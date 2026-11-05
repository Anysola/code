//! The `/diff` viewer: the repository's changes against its base branch, a file tree and the
//! selected file's patch, with reviewed marks.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Branch commits and local changes.
    All,
    Committed,
    Uncommitted,
    /// What the session's latest turn changed (only in a session).
    Turn,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Source::All => "All",
            Source::Committed => "Committed",
            Source::Uncommitted => "Uncommitted",
            Source::Turn => "Last turn",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Source::All => "Branch + local changes",
            Source::Committed => "Branch commits only",
            Source::Uncommitted => "Local changes only",
            Source::Turn => "Latest session turn",
        }
    }

    pub fn next(self) -> Source {
        match self {
            Source::All => Source::Committed,
            Source::Committed => Source::Uncommitted,
            Source::Uncommitted | Source::Turn => Source::All,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub path: String,
    pub patch: String,
    pub reviewed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    pub source: Source,
    /// `None` while loading; `Err` when git failed.
    pub files: Option<Result<Vec<File>, String>>,
    pub selected: usize,
    pub scroll: usize,
    pub tree: bool,
    /// One file at a time (`s`); otherwise every patch in one scroll.
    pub single: bool,
    /// The right-click menu on a tree row: `(column, row, file index)`.
    pub menu: Option<(u16, u16, usize)>,
    /// The branch `All` and `Committed` compare against, chosen in the picker.
    pub base: Option<String>,
    /// Full screen even where a session could dock it (`f`, `diffs.fullscreen`).
    pub fullscreen: bool,
    /// Drawn as the session's right pane this frame (the app keeps it current).
    pub docked: bool,
}

impl Diff {
    pub fn new(source: Source) -> Diff {
        Diff {
            source,
            files: None,
            selected: 0,
            scroll: 0,
            tree: true,
            single: false,
            menu: None,
            base: None,
            fullscreen: false,
            docked: false,
        }
    }

    /// Where each file's rows start in the all-files scroll.
    pub fn starts(&self, screen: u16, theme: &crate::theme::Theme) -> Vec<usize> {
        let mut starts = Vec::new();
        let mut at = 0;
        for file in self.list() {
            starts.push(at);
            let options = crate::view::patch_options(self, screen, &file.path);
            at += 3 + match crate::transcript::diff::virtual_chunks(&file.patch) {
                Some(_) => crate::transcript::diff::counts(&file.patch).1,
                None => crate::transcript::diff::render(&file.patch, &options, theme).len(),
            };
        }
        starts
    }

    /// The file the all-files scroll is in.
    pub fn current(&self, screen: u16, theme: &crate::theme::Theme) -> usize {
        if self.single {
            return self.selected;
        }
        self.starts(screen, theme)
            .iter()
            .rposition(|start| *start <= self.scroll)
            .unwrap_or(0)
    }

    /// The scrolled rows as the viewer lays them out on a screen this wide: the selected patch,
    /// or every file as a bold path, a blank row, its patch and a blank row.
    pub fn rendered(
        &self,
        screen: u16,
        theme: &crate::theme::Theme,
    ) -> Vec<crate::transcript::markdown::Line> {
        self.rendered_window(screen, theme, 0..0)
    }

    /// [`Diff::rendered`] with virtualized patches laid out only in the 128-line chunks around
    /// `window` (plus one on each side); the others are blank rows of the same height.
    pub fn rendered_window(
        &self,
        screen: u16,
        theme: &crate::theme::Theme,
        window: std::ops::Range<usize>,
    ) -> Vec<crate::transcript::markdown::Line> {
        use crate::transcript::diff::{VIRTUAL_CHUNK_LINES, counts, render, virtual_chunks};
        let render = |file: &File, at: usize, out: &mut Vec<crate::transcript::markdown::Line>| {
            let mut options = crate::view::patch_options(self, screen, &file.path);
            let Some(chunks) = virtual_chunks(&file.patch) else {
                out.extend(render(&file.patch, &options, theme));
                return;
            };
            // Chunks lay out unwrapped so each is exactly as tall as its lines.
            options.wrap = false;
            options.digits = ((chunks.len() - 1) * VIRTUAL_CHUNK_LINES
                + counts(chunks.last().unwrap()).1)
                .to_string()
                .len();
            let first = window.start.saturating_sub(at) / VIRTUAL_CHUNK_LINES;
            let last = window.end.saturating_sub(at) / VIRTUAL_CHUNK_LINES;
            for (index, chunk) in chunks.iter().enumerate() {
                if !window.is_empty() && index + 1 >= first && index <= last + 1 {
                    out.extend(render(chunk, &options, theme));
                } else {
                    out.extend(std::iter::repeat_n(Vec::new(), counts(chunk).1));
                }
            }
        };
        let mut lines = Vec::new();
        if self.single {
            if let Some(file) = self.list().get(self.selected) {
                render(file, 0, &mut lines);
            }
            return lines;
        }
        let bold = ratatui::style::Style::new()
            .fg(theme.fg("text.base"))
            .add_modifier(ratatui::style::Modifier::BOLD);
        for file in self.list() {
            lines.push(vec![ratatui::text::Span::styled(file.path.clone(), bold)]);
            lines.push(Vec::new());
            let at = lines.len();
            render(file, at, &mut lines);
            lines.push(Vec::new());
        }
        lines
    }

    /// Rows where the selected patch's hunks start (`]`/`[`).
    pub fn hunks(&self, screen: u16, theme: &crate::theme::Theme) -> Vec<usize> {
        crate::transcript::diff::hunk_rows(&self.rendered(screen, theme))
    }

    /// The selected patch's last row (`G`).
    pub fn length(&self, screen: u16, theme: &crate::theme::Theme) -> usize {
        self.rendered(screen, theme).len().saturating_sub(1)
    }

    pub fn list(&self) -> &[File] {
        match &self.files {
            Some(Ok(files)) => files,
            _ => &[],
        }
    }
}

fn git(directory: &Path, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The commit the branch started from: the merge base with the remote's default branch, or a
/// local `main`/`master`.
fn base(directory: &Path) -> Option<String> {
    [
        "origin/HEAD",
        "origin/main",
        "origin/master",
        "main",
        "master",
    ]
    .iter()
    .find_map(|branch| {
        git(directory, &["merge-base", "HEAD", branch])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Splits `git diff` output into one patch per file.
pub fn split(output: &str) -> Vec<File> {
    let mut files: Vec<File> = Vec::new();
    for chunk in output.split("diff --git ").skip(1) {
        let path = chunk
            .lines()
            .find_map(|l| {
                l.strip_prefix("+++ b/")
                    .or_else(|| l.strip_prefix("--- a/"))
            })
            .map(str::to_string)
            .or_else(|| {
                chunk
                    .lines()
                    .next()
                    .and_then(|l| l.split(" b/").nth(1))
                    .map(str::to_string)
            })
            .unwrap_or_default();
        files.push(File {
            path,
            patch: format!("diff --git {chunk}"),
            reviewed: false,
        });
    }
    files
}

/// Local and remote branches for the base picker (`git branch -a`), at most 100.
pub fn branches(directory: &Path) -> Result<Vec<String>, String> {
    let output = git(
        directory,
        &[
            "branch",
            "-a",
            "--format=%(refname:short)",
            "--sort=-committerdate",
        ],
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|b| !b.is_empty() && !b.ends_with("/HEAD") && !b.starts_with('('))
        .take(100)
        .map(str::to_string)
        .collect())
}

/// The source's patches; `chosen` replaces the detected base branch.
pub fn load(directory: &Path, source: Source, chosen: Option<&str>) -> Result<Vec<File>, String> {
    let base = match chosen {
        Some(branch) => Some(
            git(directory, &["merge-base", "HEAD", branch])
                .map(|s| s.trim().to_string())
                .map_err(|e| e.to_string())?,
        ),
        None => base(directory),
    };
    let output = match (source, base) {
        (Source::Uncommitted | Source::Turn, _) => git(directory, &["diff", "HEAD"])?,
        (Source::Committed, Some(base)) => git(directory, &["diff", &base, "HEAD"])?,
        (Source::All, Some(base)) => git(directory, &["diff", &base])?,
        (_, None) => git(directory, &["diff", "HEAD"])?,
    };
    Ok(split(&output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_per_file() {
        let output = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/b.rs b/b.rs\nnew file mode 100644\n--- /dev/null\n+++ b/b.rs\n@@ -0,0 +1 @@\n+z\n";
        let files = split(output);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "b.rs");
        assert!(files[0].patch.ends_with("+y\n"));
    }

    #[test]
    fn all_files_scroll_and_single_patch() {
        let theme = crate::theme::Catalog::bundled()
            .load("anysola", crate::theme::Mode::Dark)
            .unwrap();
        let output = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+y\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -1 +1 @@\n-z\n+w\n";
        let mut view = Diff::new(Source::All);
        view.files = Some(Ok(split(output)));
        let starts = view.starts(120, &theme);
        let lines = view.rendered(120, &theme);
        let text = |line: &crate::transcript::markdown::Line| {
            line.iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(text(&lines[starts[0]]), "a.rs");
        assert_eq!(text(&lines[starts[1]]), "b.rs");
        view.scroll = starts[1];
        assert_eq!(view.current(120, &theme), 1);
        view.single = true;
        view.selected = 0;
        assert!(view.rendered(120, &theme).len() < lines.len());
    }

    #[test]
    fn branches_list_local_ones() {
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| git(dir.path(), args).unwrap();
        run(&["init", "-q", "-b", "main"]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ]);
        run(&["branch", "feature"]);
        let mut names = branches(dir.path()).unwrap();
        names.sort();
        assert_eq!(names, ["feature", "main"]);
        assert!(
            load(dir.path(), Source::All, Some("feature"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn long_new_files_lay_out_only_the_chunks_in_view() {
        let theme = crate::theme::Catalog::bundled()
            .load("anysola", crate::theme::Mode::Dark)
            .unwrap();
        let body: String = (1..=3500).map(|n| format!("+line {n}\n")).collect();
        let patch = format!("--- /dev/null\n+++ b/big.txt\n@@ -0,0 +1,3500 @@\n{body}");
        let mut view = Diff::new(Source::Uncommitted);
        view.single = true;
        view.files = Some(Ok(vec![File {
            path: "big.txt".into(),
            patch,
            reviewed: false,
        }]));
        assert_eq!(view.length(120, &theme), 3499);
        view.scroll = 3000;
        let lines = view.rendered_window(120, &theme, 3000..3040);
        let text = |n: usize| {
            lines[n]
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert!(text(3000).contains("3001") && text(3000).contains("line 3001"));
        assert!(
            text(2900).contains("line 2901"),
            "the chunk before stays laid out"
        );
        assert!(text(10).is_empty() && text(3499).is_empty());
    }
}
