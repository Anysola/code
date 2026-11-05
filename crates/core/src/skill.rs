//! Skills: markdown instructions the model loads with the `skill` tool. Discovered from the
//! compatibility directories (`.claude/skills`, `.agents/skills`), every config directory's
//! `skill/` and `skills/`, and the config `skills` list (paths and `http(s)` index URLs); a later
//! source replaces a skill with the same id.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

use crate::config::Config;
use crate::paths::Paths;
use crate::permission::{Effect, Rule, evaluate};

const MAX_DEPTH: usize = 6;
const SAMPLED_FILES: usize = 10;
const IGNORED_DIRECTORIES: [&str; 5] = ["node_modules", ".git", ".venv", "venv", "__pycache__"];

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// `false` keeps the skill out of the system prompt listing.
    pub autoinvoke: Option<bool>,
    /// The skill's markdown file.
    pub path: PathBuf,
    pub content: String,
}

fn truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        v == "1" || v == "true"
    })
}

fn boolean(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::Number(n) => match n.as_f64() {
            Some(1.0) => Some(true),
            Some(0.0) => Some(false),
            _ => None,
        },
        serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// A skill file: id = file stem for a `*.md` directly in `root`, else the parent directory name.
fn parse(root: &Path, file: &Path, content: &str) -> Option<Skill> {
    let (meta, body) = crate::markdown::parse(content)?;
    let top_level = file.parent() == Some(root) && file.file_name() != Some("SKILL.md".as_ref());
    let id = if top_level {
        file.file_stem()
    } else {
        file.parent().and_then(Path::file_name)
    }
    .map(|s| s.to_string_lossy().into_owned())
    .unwrap_or_default();
    let text = |key: &str| meta.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let autoinvoke = meta
        .get("metadata")
        .and_then(|m| m.get("opencode/autoinvoke"))
        .and_then(boolean)
        .or_else(|| {
            meta.get("disable-model-invocation")
                .and_then(boolean)
                .filter(|disabled| *disabled)
                .map(|_| false)
        });
    Some(Skill {
        name: text("name").unwrap_or_else(|| id.clone()),
        id,
        description: text("description"),
        autoinvoke,
        path: file.to_path_buf(),
        content: body.trim().to_string(),
    })
}

/// Breadth-first: a directory holding `SKILL.md` is one skill and is not descended; `*.md` files
/// directly in the root are skills too. Dot entries and dependency folders are skipped.
fn scan(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut queue = vec![(root.to_path_buf(), 0)];
    let mut index = 0;
    while let Some((dir, depth)) = queue.get(index).cloned() {
        index += 1;
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<(String, PathBuf, bool, bool)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || IGNORED_DIRECTORIES.contains(&name.as_str()) {
                    return None;
                }
                let path = entry.path();
                let meta = std::fs::metadata(&path).ok()?;
                Some((name, path, meta.is_file(), meta.is_dir()))
            })
            .collect();
        children.sort_by(|a, b| a.0.cmp(&b.0));
        if let Some((_, skill, _, _)) = children
            .iter()
            .find(|(name, _, file, _)| *file && name == "SKILL.md")
        {
            files.push(skill.clone());
            continue;
        }
        if depth == 0 {
            files.extend(
                children
                    .iter()
                    .filter(|(name, _, file, _)| *file && name.ends_with(".md"))
                    .map(|(_, path, _, _)| path.clone()),
            );
        }
        if depth < MAX_DEPTH {
            queue.extend(
                children
                    .iter()
                    .filter(|(_, _, _, dir)| *dir)
                    .map(|(_, path, _, _)| (path.clone(), depth + 1)),
            );
        }
    }
    files.sort();
    files
}

fn load_directory(root: &Path, into: &mut Vec<Skill>) {
    for file in scan(root) {
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        let Some(skill) = parse(root, &file, &content) else {
            continue;
        };
        match into.iter_mut().find(|s| s.id == skill.id) {
            Some(existing) => *existing = skill,
            None => into.push(skill),
        }
    }
}

enum Source {
    Directory(PathBuf),
    Url(String),
}

/// The `.claude` and `.agents` roots: home first, then the project walk farthest first (nearest
/// wins). `.claude` is skipped under `OPENCODE_DISABLE_CLAUDE_CODE[_SKILLS]`.
fn compatibility_roots(paths: &Paths, directory: &Path) -> Vec<PathBuf> {
    let project = !truthy("OPENCODE_CONFIG_PROJECT_DISABLE")
        && !truthy("OPENCODE_DISABLE_PROJECT_CONFIG")
        && directory != paths.legacy_config_dir()
        && directory != paths.data;
    let walk: Vec<PathBuf> = if project {
        crate::config::upward(directory, &paths.home)
            .into_iter()
            .rev()
            .collect()
    } else {
        Vec::new()
    };
    let claude =
        !truthy("OPENCODE_DISABLE_CLAUDE_CODE") && !truthy("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS");
    let mut roots = Vec::new();
    for (name, enabled) in [(".claude", claude), (".agents", true)] {
        if !enabled {
            continue;
        }
        let global = paths.home.join(name);
        for root in std::iter::once(global.clone()).chain(walk.iter().map(|dir| dir.join(name))) {
            if root.is_dir() && !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

fn sources(config: &Config, paths: &Paths, directory: &Path) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    let mut add = |source: Source| {
        let duplicate = out.iter().any(|known| match (known, &source) {
            (Source::Directory(a), Source::Directory(b)) => a == b,
            (Source::Url(a), Source::Url(b)) => a == b,
            _ => false,
        });
        if !duplicate {
            out.push(source);
        }
    };
    for dir in config.directories() {
        add(Source::Directory(dir.join("skill")));
        add(Source::Directory(dir.join("skills")));
    }
    for item in &config.skills {
        if item.starts_with("http://") || item.starts_with("https://") {
            add(Source::Url(item.clone()));
            continue;
        }
        let expanded = match item.strip_prefix("~/") {
            Some(rest) => paths.home.join(rest),
            None => PathBuf::from(item),
        };
        add(Source::Directory(if expanded.is_absolute() {
            expanded
        } else {
            directory.join(expanded)
        }));
    }
    out
}

/// The local folders skills are read from, for the file watcher.
pub fn local_roots(paths: &Paths, config: &Config, directory: &Path) -> Vec<PathBuf> {
    let compatibility = compatibility_roots(paths, directory)
        .into_iter()
        .map(|root| root.join("skills"));
    let configured =
        sources(config, paths, directory)
            .into_iter()
            .filter_map(|source| match source {
                Source::Directory(dir) => Some(dir),
                Source::Url(_) => None,
            });
    compatibility.chain(configured).collect()
}

/// Every skill visible from `directory`.
pub async fn discover(paths: &Paths, config: &Config, directory: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    for root in compatibility_roots(paths, directory) {
        load_directory(&root.join("skills"), &mut skills);
    }
    for source in sources(config, paths, directory) {
        match source {
            Source::Directory(dir) => load_directory(&dir, &mut skills),
            Source::Url(url) => {
                for dir in pull(&paths.data.join("cache").join("skills"), &url).await {
                    load_directory(&dir, &mut skills);
                }
            }
        }
    }
    skills
}

#[derive(Deserialize)]
struct Index {
    skills: Vec<IndexSkill>,
}

#[derive(Deserialize)]
struct IndexSkill {
    name: String,
    version: Option<String>,
    files: Vec<String>,
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\', '\0'])
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', '\0', '?', '#'])
        && !value.starts_with('/')
        && !has_scheme(value)
        && value
            .split('/')
            .all(|segment| percent_decode(segment).is_some_and(|decoded| safe_segment(&decoded)))
}

/// Parses as an absolute URL (`scheme:`), which also covers Windows drive paths.
fn has_scheme(value: &str) -> bool {
    value.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn download(http: &reqwest::Client, url: &str, destination: &Path) -> bool {
    if destination.exists() {
        return true;
    }
    let body = match http.get(url).send().await {
        Ok(response) if response.status().is_success() => response.bytes().await.ok(),
        _ => None,
    };
    let Some(body) = body else {
        crate::log::warn("failed to download skill file", &[("url", url)]);
        return false;
    };
    destination
        .parent()
        .is_some_and(|parent| std::fs::create_dir_all(parent).is_ok())
        && std::fs::write(destination, &body).is_ok()
}

/// Downloads the skills of a `<url>/index.json` index into the cache; returns the skill roots.
async fn pull(cache: &Path, url: &str) -> Vec<PathBuf> {
    let base = if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    };
    let Ok(http) = reqwest::Client::builder()
        .user_agent(crate::version::user_agent())
        .timeout(std::time::Duration::from_secs(30))
        .build()
    else {
        return Vec::new();
    };
    let index_url = format!("{base}index.json");
    let index = match http
        .get(&index_url)
        .header("accept", "application/json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => response
            .bytes()
            .await
            .ok()
            .and_then(|body| serde_json::from_slice::<Index>(&body).ok()),
        _ => None,
    };
    let Some(index) = index else {
        crate::log::warn("failed to fetch skill index", &[("url", &index_url)]);
        return Vec::new();
    };
    let source_root = cache.join(hex::encode(Sha1::digest(base.as_bytes())));
    let mut roots = Vec::new();
    for skill in index.skills {
        let main = format!("{}.md", skill.name);
        if !safe_segment(&skill.name)
            || !skill.files.iter().any(|f| f == "SKILL.md" || *f == main)
            || !skill.files.iter().all(|f| safe_relative(f))
        {
            continue;
        }
        let root = source_root.join(&skill.name);
        let skill_url = format!("{base}{}/", percent_encode(&skill.name));
        let version_file = root.join(".opencode-version");
        let current = std::fs::read_to_string(&version_file).ok();
        match &skill.version {
            Some(version) if current.as_deref() != Some(version.as_str()) => {
                let staging =
                    source_root.join(format!("{}.tmp-{}", skill.name, crate::ids::part()));
                let mut complete = true;
                for file in &skill.files {
                    complete &=
                        download(&http, &format!("{skill_url}{file}"), &staging.join(file)).await;
                }
                if complete
                    && (staging.join("SKILL.md").exists() || staging.join(&main).exists())
                    && std::fs::write(staging.join(".opencode-version"), version).is_ok()
                {
                    let _ = std::fs::remove_dir_all(&root);
                    let _ = std::fs::rename(&staging, &root);
                }
                let _ = std::fs::remove_dir_all(&staging);
            }
            _ => {
                for file in &skill.files {
                    download(&http, &format!("{skill_url}{file}"), &root.join(file)).await;
                }
            }
        }
        if root.join("SKILL.md").exists() || root.join(&main).exists() {
            roots.push(root);
        }
    }
    roots
}

/// Skills the rules do not deny.
pub fn available<'a>(skills: &'a [Skill], rules: &[Rule]) -> Vec<&'a Skill> {
    skills
        .iter()
        .filter(|s| evaluate("skill", &s.id, rules) != Effect::Deny)
        .collect()
}

/// The skill guidance source of system block 2: `{id, name, description}` of each listed skill,
/// sorted by id (0.5.1's stored shape); `None` when no skill qualifies.
pub fn guidance(skills: &[Skill], rules: &[Rule]) -> Option<Value> {
    let mut listed: Vec<(&str, &str, &str)> = available(skills, rules)
        .into_iter()
        .filter(|s| s.autoinvoke != Some(false))
        .filter_map(|s| {
            s.description
                .as_deref()
                .map(|d| (s.id.as_str(), s.name.as_str(), d))
        })
        .collect();
    if listed.is_empty() {
        return None;
    }
    listed.sort_by(|a, b| crate::text::locale_compare(a.0, b.0));
    Some(Value::Array(
        listed
            .into_iter()
            .map(|(id, name, description)| json!({"id": id, "name": name, "description": description}))
            .collect(),
    ))
}

fn summaries(value: &Value) -> Vec<&Value> {
    value
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn field<'a>(skill: &'a Value, key: &str) -> &'a str {
    skill.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn entries(skills: &[&Value]) -> Vec<String> {
    skills
        .iter()
        .flat_map(|skill| {
            [
                "  <skill>".to_string(),
                format!("    <id>{}</id>", field(skill, "id")),
                format!("    <name>{}</name>", field(skill, "name")),
                format!(
                    "    <description>{}</description>",
                    field(skill, "description")
                ),
                "  </skill>".to_string(),
            ]
        })
        .collect()
}

/// The guidance as the model first sees it.
pub fn render_guidance(value: &Value) -> Option<String> {
    let skills = summaries(value);
    let mut lines = vec![
        "Skills provide specialized instructions and workflows for specific tasks.".to_string(),
        "Use the skill tool to load a skill when a task matches its description.".into(),
        "The user may also invoke a skill directly. When that happens, its instructions appear in the conversation as a <skill_content> block, the same shape the skill tool returns. A skill that is already present this way does not need to be invoked again.".into(),
    ];
    if skills.is_empty() {
        lines.push("No skills are currently available.".into());
    } else {
        lines.push("<available_skills>".into());
        lines.extend(entries(&skills));
        lines.push("</available_skills>".into());
    }
    Some(lines.join("\n"))
}

/// Added, changed and removed skills as deltas; the whole list again when only the order moved.
pub fn guidance_update(previous: &Value, current: &Value) -> Option<String> {
    let (before, after) = (summaries(previous), summaries(current));
    let find = |list: &[&'_ Value], id: &str| list.iter().position(|s| field(s, "id") == id);
    let added: Vec<&Value> = after
        .iter()
        .copied()
        .filter(|s| find(&before, field(s, "id")).is_none())
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .map(|s| field(s, "id"))
        .filter(|id| find(&after, id).is_none())
        .collect();
    let changed: Vec<&Value> = after
        .iter()
        .copied()
        .filter(|s| {
            find(&before, field(s, "id")).is_some_and(|i| {
                field(before[i], "name") != field(s, "name")
                    || field(before[i], "description") != field(s, "description")
            })
        })
        .collect();
    if added.is_empty() && changed.is_empty() && removed.is_empty() {
        return Some(format!(
            "The available skills have changed. This list supersedes the previous available skills list.\n{}",
            render_guidance(current)?
        ));
    }
    let mut lines = Vec::new();
    if !added.is_empty() {
        lines.push("New skills are available in addition to those previously listed:".to_string());
        lines.extend(entries(&added));
    }
    if !changed.is_empty() {
        lines.push("The following skills have changed. Each entry replaces the previously listed skill with the same ID:".into());
        lines.extend(entries(&changed));
    }
    if !removed.is_empty() {
        lines.push(format!(
            "The following skill IDs are no longer available and must not be used: {}.",
            removed.join(", ")
        ));
    }
    Some(lines.join("\n"))
}

/// Up to ten files of a `SKILL.md` skill's directory (sorted, `SKILL.md` and dependency folders
/// left out); a single-file skill lists none.
fn sampled_files(skill: &Skill) -> Vec<String> {
    let Some(directory) = skill.path.parent() else {
        return Vec::new();
    };
    if skill.path.file_name() != Some("SKILL.md".as_ref()) {
        return Vec::new();
    }
    let mut files: Vec<String> = ignore::WalkBuilder::new(directory)
        .standard_filters(false)
        .filter_entry(|entry| {
            !IGNORED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref())
        })
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| e.file_name() != "SKILL.md")
        .map(|e| e.path().display().to_string())
        .collect();
    files.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    files.truncate(SAMPLED_FILES);
    files
}

/// The `<skill_content>` block the skill tool returns (and user-invoked skills prepend).
pub fn content(skill: &Skill) -> String {
    let directory = skill
        .path
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let mut lines = vec![
        format!("<skill_content name=\"{}\">", skill.name),
        format!("# Skill: {}", skill.name),
        String::new(),
        skill.content.trim().to_string(),
        String::new(),
        format!("Base directory for this skill: {directory}"),
        "Relative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.".into(),
        "Note: file list is sampled.".into(),
        String::new(),
        "<skill_files>".into(),
    ];
    lines.extend(
        sampled_files(skill)
            .into_iter()
            .map(|f| format!("<file>{f}</file>")),
    );
    lines.push("</skill_files>".into());
    lines.push("</skill_content>".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn scan_rules() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(&root.join("top.md"), "x");
        write(&root.join("a/SKILL.md"), "x");
        write(&root.join("a/nested/SKILL.md"), "x");
        write(&root.join("b/c/SKILL.md"), "x");
        write(&root.join("b/c/d.md"), "x");
        write(&root.join("b/loose.md"), "x");
        write(&root.join(".hidden/SKILL.md"), "x");
        write(&root.join("node_modules/x/SKILL.md"), "x");
        write(&root.join("1/2/3/4/5/6/SKILL.md"), "x");
        write(&root.join("1/2/3/4/5/6/7/SKILL.md"), "x");
        let found: Vec<String> = scan(root)
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .map(|p| p.replace('\\', "/"))
            .collect();
        assert_eq!(
            found,
            vec![
                "1/2/3/4/5/6/SKILL.md",
                "a/SKILL.md",
                "b/c/SKILL.md",
                "top.md"
            ]
        );
    }

    #[test]
    fn frontmatter_and_ids() {
        let root = Path::new("/s");
        let skill = parse(
            root,
            Path::new("/s/demo/SKILL.md"),
            "---\nname: Demo\ndescription: Does demos\nmetadata:\n  opencode/autoinvoke: \"no\"\n---\n\nBody\n",
        )
        .unwrap();
        assert_eq!(
            (skill.id.as_str(), skill.name.as_str(), skill.autoinvoke),
            ("demo", "Demo", Some(false))
        );
        assert_eq!(skill.description.as_deref(), Some("Does demos"));
        let skill = parse(
            root,
            Path::new("/s/top.md"),
            "---\ndisable-model-invocation: true\n---\nx",
        )
        .unwrap();
        assert_eq!((skill.id.as_str(), skill.autoinvoke), ("top", Some(false)));
        let skill = parse(root, Path::new("/s/top.md"), "plain").unwrap();
        assert_eq!((skill.name.as_str(), skill.autoinvoke), ("top", None));
    }

    #[test]
    fn content_matches_capture() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("proj/.claude/skills").join("demo");
        write(
            &base.join("SKILL.md"),
            "---\nname: demo\ndescription: Demo skill\n---\nDo the demo thing.\n",
        );
        write(&base.join("notes.txt"), "n");
        let mut skills = Vec::new();
        load_directory(&dir.path().join("proj/.claude/skills"), &mut skills);
        let notes = base.join("notes.txt");
        let (base, notes) = (base.display(), notes.display());
        assert_eq!(
            content(&skills[0]),
            format!(
                "<skill_content name=\"demo\">\n# Skill: demo\n\nDo the demo thing.\n\nBase directory for this skill: {base}\nRelative paths in this skill (e.g., scripts/, reference/) are relative to this base directory.\nNote: file list is sampled.\n\n<skill_files>\n<file>{notes}</file>\n</skill_files>\n</skill_content>"
            )
        );
    }

    #[test]
    fn at_most_ten_files() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("s/SKILL.md"), "x");
        for i in 0..12 {
            write(&dir.path().join(format!("s/f{i:02}.txt")), "x");
        }
        write(&dir.path().join("s/.git/HEAD"), "x");
        let mut skills = Vec::new();
        load_directory(dir.path(), &mut skills);
        let text = content(&skills[0]);
        assert_eq!(text.matches("<file>").count(), 10);
        assert!(text.contains("f00.txt") && !text.contains("f10.txt"));
    }

    #[test]
    fn guidance_listing() {
        let skill = |id: &str, description: Option<&str>, autoinvoke: Option<bool>| Skill {
            id: id.into(),
            name: id.into(),
            description: description.map(str::to_string),
            autoinvoke,
            path: PathBuf::from("/x/SKILL.md"),
            content: String::new(),
        };
        let skills = vec![
            skill("zeta", Some("Z"), None),
            skill("Alpha", Some("A"), None),
            skill("quiet", None, None),
            skill("manual", Some("M"), Some(false)),
            skill("denied", Some("D"), None),
        ];
        let rules = vec![
            Rule::new("*", "*", Effect::Allow),
            Rule::new("skill", "denied", Effect::Deny),
        ];
        let text = render_guidance(&guidance(&skills, &rules).unwrap()).unwrap();
        assert!(text.ends_with("<available_skills>\n  <skill>\n    <id>Alpha</id>\n    <name>Alpha</name>\n    <description>A</description>\n  </skill>\n  <skill>\n    <id>zeta</id>\n    <name>zeta</name>\n    <description>Z</description>\n  </skill>\n</available_skills>"));
        assert_eq!(
            guidance(&skills, &[Rule::new("*", "*", Effect::Deny)]),
            None
        );
    }

    #[test]
    fn guidance_updates() {
        let skill =
            |id: &str, description: &str| json!({"id": id, "name": id, "description": description});
        let before = json!([skill("a", "A"), skill("b", "B"), skill("c", "C")]);
        let after = json!([skill("b", "B2"), skill("c", "C"), skill("d", "D")]);
        assert_eq!(
            guidance_update(&before, &after).unwrap(),
            "New skills are available in addition to those previously listed:\n  <skill>\n    <id>d</id>\n    <name>d</name>\n    <description>D</description>\n  </skill>\nThe following skills have changed. Each entry replaces the previously listed skill with the same ID:\n  <skill>\n    <id>b</id>\n    <name>b</name>\n    <description>B2</description>\n  </skill>\nThe following skill IDs are no longer available and must not be used: a."
        );
        let reordered = json!([skill("c", "C"), skill("b", "B"), skill("a", "A")]);
        let restated = guidance_update(&before, &reordered).unwrap();
        assert!(restated.starts_with("The available skills have changed. This list supersedes the previous available skills list.\nSkills provide specialized"));
        assert!(
            render_guidance(&json!([]))
                .unwrap()
                .ends_with("again.\nNo skills are currently available.")
        );
    }

    #[test]
    fn precedence_and_bound() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("outer");
        let home = outer.join("home");
        let project = home.join("proj");
        let skill = |dir: &Path, text: &str| {
            write(
                &dir.join("dup/SKILL.md"),
                &format!("---\ndescription: {text}\n---\nbody"),
            )
        };
        skill(&outer.join(".claude/skills"), "above home");
        skill(&home.join(".claude/skills"), "home claude");
        skill(&project.join(".claude/skills"), "project claude");
        skill(&project.join(".agents/skills"), "project agents");
        write(
            &outer.join(".claude/skills/above/SKILL.md"),
            "---\ndescription: x\n---\n",
        );
        let paths = Paths::isolated(home.clone());
        let roots = compatibility_roots(&paths, &project);
        assert_eq!(
            roots,
            vec![
                home.join(".claude"),
                project.join(".claude"),
                project.join(".agents")
            ]
        );
        let config = Config::default();
        let skills = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(discover(&paths, &config, &project));
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description.as_deref(), Some("project agents"));
    }

    #[test]
    fn index_paths_are_checked() {
        assert!(safe_relative("SKILL.md") && safe_relative("scripts/run.sh"));
        for bad in [
            "",
            "/abs",
            "../x",
            "a/%2e%2e/b",
            "c:/x",
            "a\\b",
            "x?y",
            "https://e/x",
        ] {
            assert!(!safe_relative(bad), "{bad}");
        }
    }
}
