//! The `/diff` viewer: the repository's changes against its base branch, a file tree and the
//! selected file's patch, with reviewed marks.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Branch commits and local changes.
    All,
    Committed,
    Uncommitted,
}

impl Source {
    pub fn title(self) -> &'static str {
        match self {
            Source::All => "All",
            Source::Committed => "Committed",
            Source::Uncommitted => "Uncommitted",
        }
    }

    pub fn next(self) -> Source {
        match self {
            Source::All => Source::Committed,
            Source::Committed => Source::Uncommitted,
            Source::Uncommitted => Source::All,
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
}

impl Diff {
    pub fn new(source: Source) -> Diff {
        Diff {
            source,
            files: None,
            selected: 0,
            scroll: 0,
            tree: true,
        }
    }

    /// The selected patch as the viewer lays it out on a screen this wide.
    fn rendered(
        &self,
        screen: u16,
        theme: &crate::theme::Theme,
    ) -> Vec<crate::transcript::markdown::Line> {
        let Some(file) = self.list().get(self.selected) else {
            return Vec::new();
        };
        let options = crate::view::patch_options(self, screen, &file.path);
        crate::transcript::diff::render(&file.patch, &options, theme)
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

pub fn load(directory: &Path, source: Source) -> Result<Vec<File>, String> {
    let output = match (source, base(directory)) {
        (Source::Uncommitted, _) => git(directory, &["diff", "HEAD"])?,
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
}
