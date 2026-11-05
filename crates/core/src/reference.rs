//! Config `references`: extra directories the model may read, local paths or git repositories
//! kept as tracking checkouts under `~/.anysola/repos` (one per remote and branch, refreshed at
//! most daily). Block 2 lists the ones with a description in `<available_references>`; the
//! composer's `@alias[/path]` mentions resolve through [`resolve`].

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Weak};
use std::time::Duration;

use serde_json::{Value, json};

use crate::api::Shared;
use crate::config::Config;
use crate::storage::Db;
use crate::time::now_ms;

const REFRESH_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);
const CLONE_DEPTH: &str = "100";

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Local(PathBuf),
    Git {
        repository: String,
        branch: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub name: String,
    pub source: Source,
    pub description: Option<String>,
    /// Left out of the composer's `@` list.
    pub hidden: bool,
    /// The directory itself: the local path, or the checkout's path.
    pub path: PathBuf,
}

/// A remote git repository as `owner/repo`, `github:owner/repo`, `host/path`, scp-style or a URL.
#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    pub host: String,
    pub segments: Vec<String>,
    /// What `git clone` fetches.
    pub remote: String,
}

fn trim_git(text: &str) -> &str {
    text.strip_suffix(".git").unwrap_or(text)
}

fn parts(text: &str) -> Vec<String> {
    text.split('/')
        .map(|p| trim_git(p.trim()).to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn safe_segment(text: &str) -> bool {
    text != "."
        && text != ".."
        && !text.contains(':')
        && !text.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\')
}

fn host_like(text: &str) -> bool {
    text.contains('.') || text.contains(':') || text == "localhost"
}

/// GitHub clones go to `https://github.com/<path>.git`, or below `OPENCODE_REPO_CLONE_GITHUB_BASE_URL`.
fn github_remote(path: &str) -> String {
    match std::env::var("OPENCODE_REPO_CLONE_GITHUB_BASE_URL") {
        Ok(base) if !base.is_empty() => {
            let base = if base.ends_with('/') {
                base
            } else {
                format!("{base}/")
            };
            url::Url::parse(&base)
                .and_then(|b| b.join(&format!("{path}.git")))
                .map(String::from)
                .unwrap_or_else(|_| format!("{base}{path}.git"))
        }
        _ => format!("https://github.com/{path}.git"),
    }
}

fn remote(host: &str, segments: Vec<String>, remote: Option<String>) -> Option<Remote> {
    let segments: Vec<String> = segments
        .iter()
        .map(|s| trim_git(s).to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let safe_host = !host.is_empty() && !host.starts_with('-') && host.split(':').all(safe_segment);
    if !safe_host || segments.is_empty() || !segments.iter().all(|s| safe_segment(s)) {
        return None;
    }
    let host = host.to_lowercase();
    let path = segments.join("/");
    let remote = remote.unwrap_or_else(|| {
        if host == "github.com" {
            github_remote(&path)
        } else {
            format!("https://{host}/{path}.git")
        }
    });
    Some(Remote {
        host,
        segments,
        remote,
    })
}

/// Parses a repository; `None` for anything else, local `file:` URLs included (not supported).
pub fn parse(input: &str) -> Option<Remote> {
    parse_any(input).filter(|r| r.host != "file")
}

/// [`parse`], plus `file:` URLs as host `file`: what an existing checkout's origin may be.
fn parse_any(input: &str) -> Option<Remote> {
    static GITHUB: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static SCP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let trimmed = input.trim();
    let cleaned = trimmed.strip_prefix("git+").unwrap_or(trimmed);
    let cleaned = cleaned
        .split('#')
        .next()
        .unwrap_or_default()
        .trim_end_matches('/');
    if cleaned.is_empty() {
        return None;
    }
    let github = GITHUB
        .get_or_init(|| regex::Regex::new(r"^github:([^/\s]+)/([^/\s]+)$").expect("static regex"));
    if let Some(found) = github.captures(cleaned) {
        return remote("github.com", vec![found[1].into(), found[2].into()], None);
    }
    if !cleaned.contains("://") {
        let scp = SCP.get_or_init(|| {
            regex::Regex::new(r"^(?:[^@/\s]+@)?([^:/\s]+):(.+)$").expect("static regex")
        });
        if let Some(found) = scp.captures(cleaned) {
            return remote(&found[1], parts(&found[2]), Some(cleaned.to_string()));
        }
        let direct = parts(cleaned);
        if direct.len() >= 2 && host_like(&direct[0]) {
            return remote(&direct[0], direct[1..].to_vec(), None);
        }
        if direct.len() == 2 {
            return remote("github.com", direct, None);
        }
    }
    let url = url::Url::parse(cleaned).ok()?;
    if url.scheme() == "file" {
        let segments = parts(url.path());
        return (!segments.is_empty()).then(|| Remote {
            host: "file".into(),
            segments,
            remote: cleaned.to_string(),
        });
    }
    let host = match (url.host_str(), url.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        (None, _) => String::new(),
    };
    let segments = parts(url.path());
    let clone = if host == "github.com" {
        github_remote(&segments.join("/"))
    } else {
        cleaned.to_string()
    };
    remote(&host, segments, Some(clone))
}

pub fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_.-".contains(c))
        && !branch.starts_with('-')
        && !branch.contains("..")
}

/// One checkout per remote and branch; the branch is percent-encoded since it may contain `/`.
pub fn cache_path(root: &Path, remote: &Remote, branch: Option<&str>) -> PathBuf {
    let mut path = root.to_path_buf();
    path.extend(remote.host.split(':'));
    path.extend(&remote.segments);
    match branch {
        Some(branch) => {
            let mut name = path.into_os_string();
            name.push(format!("@{}", branch.replace('/', "%2F")));
            PathBuf::from(name)
        }
        None => path,
    }
}

fn valid_alias(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(|c: char| c == '/' || c == '`' || c == ',' || c.is_whitespace())
}

/// The configured references, in config order. Git references that do not parse (or name an
/// invalid branch) are left out.
pub fn list(config: &Config, home: &Path, directory: &Path, repos: &Path) -> Vec<Reference> {
    config
        .references
        .iter()
        .filter(|(name, ..)| valid_alias(name))
        .filter_map(|(name, entry, dir)| {
            let field = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
            let (local, repository) = match entry {
                Value::String(text) if text.starts_with(['.', '/', '~']) => {
                    (Some(text.clone()), None)
                }
                Value::String(text) => (None, Some(text.clone())),
                Value::Object(_) if entry.get("path").is_some() => (field("path"), None),
                Value::Object(_) => (None, field("repository")),
                _ => return None,
            };
            let (source, path) = match (local, repository) {
                (Some(local), _) => {
                    let base = dir.as_deref().unwrap_or(directory);
                    let path = match local.strip_prefix("~/") {
                        Some(rest) => home.join(rest),
                        None => crate::paths::normalize(&base.join(&local)),
                    };
                    (Source::Local(path.clone()), path)
                }
                (None, Some(repository)) => {
                    let branch = field("branch");
                    if branch.as_deref().is_some_and(|b| !valid_branch(b)) {
                        return None;
                    }
                    let path = cache_path(repos, &parse(&repository)?, branch.as_deref());
                    (Source::Git { repository, branch }, path)
                }
                (None, None) => return None,
            };
            Some(Reference {
                name: name.clone(),
                source,
                description: field("description"),
                hidden: entry
                    .get("hidden")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                path,
            })
        })
        .collect()
}

/// `@alias` or `@alias/sub/path` (without the `@`) as a path, for references the composer lists.
pub fn resolve(references: &[Reference], mention: &str) -> Option<PathBuf> {
    let (alias, rest) = mention.split_once('/').unwrap_or((mention, ""));
    let reference = references.iter().find(|r| !r.hidden && r.name == alias)?;
    Some(if rest.is_empty() {
        reference.path.clone()
    } else {
        reference.path.join(rest)
    })
}

/// The block-2 source value: references with a description, by name.
pub fn guidance(references: &[Reference]) -> Option<Value> {
    let mut listed: Vec<&Reference> = references
        .iter()
        .filter(|r| r.description.is_some())
        .collect();
    if listed.is_empty() {
        return None;
    }
    listed.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.name.cmp(&b.name))
    });
    Some(Value::Array(
        listed
            .into_iter()
            .map(|r| {
                let mut entry = json!({"name": r.name, "path": r.path.display().to_string()});
                if let Some(description) = &r.description {
                    entry["description"] = json!(description);
                }
                entry
            })
            .collect(),
    ))
}

fn summaries(value: &Value) -> Vec<&Value> {
    value
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn entries(references: &[&Value]) -> Vec<String> {
    references
        .iter()
        .flat_map(|r| {
            let mut lines = vec![
                "  <reference>".to_string(),
                format!("    <name>{}</name>", field(r, "name").unwrap_or_default()),
                format!("    <path>{}</path>", field(r, "path").unwrap_or_default()),
            ];
            lines.extend(
                field(r, "description").map(|d| format!("    <description>{d}</description>")),
            );
            lines.push("  </reference>".into());
            lines
        })
        .collect()
}

pub fn render_guidance(value: &Value) -> Option<String> {
    let mut lines = vec![
        "Project references provide additional directories that can be accessed when relevant."
            .to_string(),
        "<available_references>".into(),
    ];
    lines.extend(entries(&summaries(value)));
    lines.push("</available_references>".into());
    Some(lines.join("\n"))
}

/// Added, changed and removed references as deltas; the whole list again only when no entry
/// differs.
fn find<'a>(list: &[&'a Value], name: Option<&str>) -> Option<&'a Value> {
    list.iter().find(|r| field(r, "name") == name).copied()
}

pub fn guidance_update(previous: &Value, current: &Value) -> Option<String> {
    let (before, after) = (summaries(previous), summaries(current));
    let added: Vec<&Value> = after
        .iter()
        .copied()
        .filter(|r| find(&before, field(r, "name")).is_none())
        .collect();
    let changed: Vec<&Value> = after
        .iter()
        .copied()
        .filter(|r| {
            find(&before, field(r, "name")).is_some_and(|b| {
                field(b, "path") != field(r, "path")
                    || field(b, "description") != field(r, "description")
            })
        })
        .collect();
    let removed: Vec<&str> = before
        .iter()
        .filter(|r| find(&after, field(r, "name")).is_none())
        .filter_map(|r| field(r, "name"))
        .collect();
    if added.is_empty() && changed.is_empty() && removed.is_empty() {
        return Some(format!(
            "The available project references have changed. This list supersedes the previous reference list.\n{}",
            render_guidance(current)?
        ));
    }
    let mut lines = Vec::new();
    if !added.is_empty() {
        lines.push(
            "New project references are available in addition to those previously listed:"
                .to_string(),
        );
        lines.extend(entries(&added));
    }
    if !changed.is_empty() {
        lines.push("The following project references have changed. Each entry replaces the previously listed reference with the same name:".into());
        lines.extend(entries(&changed));
    }
    if !removed.is_empty() {
        lines.push(format!(
            "The following project references are no longer available and must not be used: {}.",
            removed.join(", ")
        ));
    }
    Some(lines.join("\n"))
}

pub const REMOVED: &str =
    "Project reference guidance is no longer available. Do not use previously listed references.";

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Clones the checkout, or refreshes it when the last attempt is a day old: fetch, check out the
/// tracked branch (the remote default without one) and hard-reset to it. Attempts are recorded
/// in KV `repository-cache:<path>` before any network work, so failures wait a day too.
pub fn ensure(db: &Db, root: &Path, remote: &Remote, branch: Option<&str>) -> Result<(), String> {
    let path = cache_path(root, remote, branch);
    let key = format!("repository-cache:{}", path.display());
    let parent = path.parent().ok_or("no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // A repository above the cache path could answer for it; reuse only a checkout exactly here.
    let reuse = git(&path, &["rev-parse", "--show-toplevel"])
        .ok()
        .is_some_and(|top| std::fs::canonicalize(top).ok() == std::fs::canonicalize(&path).ok())
        && git(&path, &["remote", "get-url", "origin"])
            .ok()
            .and_then(|origin| parse_any(&origin))
            .is_some_and(|origin| {
                let target = parse_any(&remote.remote).unwrap_or_else(|| remote.clone());
                (origin.host, origin.segments) == (target.host, target.segments)
            });
    if !reuse && path.exists() {
        std::fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
    }
    let now = now_ms();
    let attempted = db
        .kv_get(&key)
        .ok()
        .flatten()
        .and_then(|v| v.get("attemptedAt").and_then(Value::as_u64));
    if reuse && attempted.is_some_and(|at| now.saturating_sub(at) < REFRESH_INTERVAL_MS) {
        return Ok(());
    }
    db.kv_set(&key, &json!({"attemptedAt": now}))
        .map_err(|e| e.to_string())?;
    if !reuse {
        let target = path.display().to_string();
        let mut args = vec!["clone", "--depth", CLONE_DEPTH];
        if let Some(branch) = branch {
            args.extend(["--branch", branch]);
        }
        args.extend(["--", &remote.remote, &target]);
        return git(parent, &args).map(drop);
    }
    git(&path, &["fetch", "--all", "--prune"])?;
    if let Some(branch) = branch {
        let spec = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        git(&path, &["fetch", "origin", &spec])?;
    }
    let tracked = match branch {
        Some(branch) => Some(branch.to_string()),
        None => git(&path, &["symbolic-ref", "refs/remotes/origin/HEAD"])
            .ok()
            .and_then(|r| r.strip_prefix("refs/remotes/origin/").map(str::to_string))
            .filter(|b| !b.is_empty()),
    };
    if let Some(branch) = &tracked {
        git(
            &path,
            &["checkout", "-B", branch, &format!("origin/{branch}")],
        )?;
    }
    let target = tracked
        .or_else(|| git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok())
        .map_or("HEAD".to_string(), |b| format!("origin/{b}"));
    git(&path, &["reset", "--hard", &target]).map(drop)
}

/// Clones or refreshes every git reference, off the request path.
pub(crate) async fn materialize(db: Arc<Db>, root: PathBuf, references: Vec<Reference>) {
    for reference in references {
        let Source::Git { repository, branch } = &reference.source else {
            continue;
        };
        let Some(remote) = parse(repository) else {
            continue;
        };
        let (db, root, branch) = (db.clone(), root.clone(), branch.clone());
        let result =
            tokio::task::spawn_blocking(move || ensure(&db, &root, &remote, branch.as_deref()))
                .await;
        if let Ok(Err(error)) = result {
            crate::log::warn(
                "failed to materialize reference",
                &[("name", &reference.name), ("error", &error)],
            );
        }
    }
}

/// Checks the references hourly while the process runs; the daily throttle keeps git work rare.
pub(crate) async fn keep_fresh(shared: Weak<Shared>) {
    loop {
        let Some(current) = shared.upgrade() else {
            return;
        };
        let work = materialize(
            current.db.clone(),
            current.paths.repos_dir(),
            current.loaded().references.clone(),
        );
        drop(current);
        work.await;
        tokio::time::sleep(CHECK_EVERY).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(input: &str) -> Option<(String, String, String)> {
        parse(input).map(|r| (r.host, r.segments.join("/"), r.remote))
    }

    #[test]
    fn repositories() {
        let gh = |path: &str| {
            Some((
                "github.com".to_string(),
                path.to_string(),
                format!("https://github.com/{path}.git"),
            ))
        };
        assert_eq!(parsed("owner/repo"), gh("owner/repo"));
        assert_eq!(parsed("github:owner/repo.git"), gh("owner/repo"));
        assert_eq!(
            parsed("git+https://github.com/owner/repo.git#main"),
            gh("owner/repo")
        );
        assert_eq!(
            parsed("git@gitlab.com:group/sub/repo.git"),
            Some((
                "gitlab.com".into(),
                "group/sub/repo".into(),
                "git@gitlab.com:group/sub/repo.git".into()
            ))
        );
        assert_eq!(
            parsed("gitlab.com/group/repo"),
            Some((
                "gitlab.com".into(),
                "group/repo".into(),
                "https://gitlab.com/group/repo.git".into()
            ))
        );
        assert_eq!(
            parsed("https://Example.com:8443/a/b/"),
            Some((
                "example.com:8443".into(),
                "a/b".into(),
                "https://Example.com:8443/a/b".into()
            ))
        );
        assert_eq!(parsed("file:///tmp/repo"), None);
        assert_eq!(parsed("single"), None);
        assert_eq!(parsed("a/../b"), None);
        assert!(
            valid_branch("release/1.0")
                && !valid_branch("-x")
                && !valid_branch("a..b")
                && !valid_branch("a b")
        );
        let remote = parse("https://example.com:8443/a/b").unwrap();
        assert_eq!(
            cache_path(Path::new("/r"), &remote, Some("feat/x")),
            PathBuf::from("/r/example.com/8443/a/b@feat%2Fx")
        );
    }

    #[test]
    fn listing_and_guidance() {
        let dir = Path::new("/proj");
        let mut config = Config::default();
        config.references = vec![
            ("docs".into(), json!("./docs"), Some(dir.to_path_buf())),
            (
                "lib".into(),
                json!({"repository": "owner/lib", "branch": "dev", "description": "The library"}),
                None,
            ),
            (
                "api".into(),
                json!({"path": "~/api", "description": "API", "hidden": true}),
                None,
            ),
            ("bad alias".into(), json!("./x"), None),
            (
                "badbranch".into(),
                json!({"repository": "owner/x", "branch": "-x"}),
                None,
            ),
        ];
        let list = list(
            &config,
            Path::new("/home/u"),
            Path::new("/cwd"),
            Path::new("/home/u/.anysola/repos"),
        );
        let names: Vec<&str> = list.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["docs", "lib", "api"]);
        assert_eq!(list[0].path, PathBuf::from("/proj/docs"));
        assert_eq!(
            list[1].path,
            PathBuf::from("/home/u/.anysola/repos/github.com/owner/lib@dev")
        );
        assert_eq!(list[2].path, PathBuf::from("/home/u/api"));
        assert_eq!(
            resolve(&list, "docs/a/b.md"),
            Some(PathBuf::from("/proj/docs/a/b.md"))
        );
        assert_eq!(resolve(&list, "api"), None);

        let value = guidance(&list).unwrap();
        assert_eq!(
            render_guidance(&value).unwrap(),
            "Project references provide additional directories that can be accessed when relevant.\n<available_references>\n  <reference>\n    <name>api</name>\n    <path>/home/u/api</path>\n    <description>API</description>\n  </reference>\n  <reference>\n    <name>lib</name>\n    <path>/home/u/.anysola/repos/github.com/owner/lib@dev</path>\n    <description>The library</description>\n  </reference>\n</available_references>"
        );
        let next = json!([{"name": "api", "path": "/x", "description": "API"}, {"name": "new", "path": "/n", "description": "N"}]);
        assert_eq!(
            guidance_update(&value, &next).unwrap(),
            "New project references are available in addition to those previously listed:\n  <reference>\n    <name>new</name>\n    <path>/n</path>\n    <description>N</description>\n  </reference>\nThe following project references have changed. Each entry replaces the previously listed reference with the same name:\n  <reference>\n    <name>api</name>\n    <path>/x</path>\n    <description>API</description>\n  </reference>\nThe following project references are no longer available and must not be used: lib."
        );
        assert!(
            guidance_update(&value, &value)
                .unwrap()
                .starts_with("The available project references have changed.")
        );
        assert_eq!(guidance(&list[..1]), None);
    }

    #[test]
    fn clones_then_refreshes_daily() {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        let run = |args: &[&str]| assert!(git(&origin, args).is_ok(), "{args:?}");
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(origin.join("a.txt"), "one").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "one",
        ]);
        let db = Db::open(&dir.path().join("db.sqlite")).unwrap();
        let root = dir.path().join("repos");
        let url = format!("file://{}", origin.display());
        let remote = Remote {
            host: "example.com".into(),
            segments: vec!["o".into(), "r".into()],
            remote: url,
        };
        ensure(&db, &root, &remote, None).unwrap();
        let checkout = cache_path(&root, &remote, None);
        assert_eq!(
            std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
            "one"
        );
        let key = format!("repository-cache:{}", checkout.display());
        assert!(db.kv_get(&key).unwrap().unwrap()["attemptedAt"].is_u64());

        std::fs::write(origin.join("a.txt"), "two").unwrap();
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qam",
            "two",
        ]);
        ensure(&db, &root, &remote, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
            "one",
            "within a day"
        );
        db.kv_set(&key, &json!({"attemptedAt": 0})).unwrap();
        ensure(&db, &root, &remote, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(checkout.join("a.txt")).unwrap(),
            "two"
        );
    }
}
