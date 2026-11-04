//! Project identification: the VCS root of a directory and the project id 0.5.1 computed for it.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha1::{Digest, Sha1};

/// Nearest ancestor (inclusive) holding `.git` or `.hg`.
pub fn vcs_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists() || dir.join(".hg").exists())
        .map(Path::to_path_buf)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    /// Project root: the VCS root, else the directory itself.
    pub worktree: PathBuf,
    pub vcs: Option<&'static str>,
}

fn sha1_hex(text: &str) -> String {
    hex::encode(Sha1::digest(text.as_bytes()))
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// `host/path` of a git remote URL: lower-case host, no leading `/`, no `.git`.
fn normalize_remote(url: &str) -> Option<String> {
    let url = url.trim();
    let (host, path) = if let Some(rest) = url.split_once("://").map(|(_, r)| r) {
        let rest = rest.rsplit_once('@').map(|(_, r)| r).unwrap_or(rest);
        let (host, path) = rest.split_once('/')?;
        (
            host.split(':').next().unwrap_or(host).to_string(),
            path.to_string(),
        )
    } else {
        let rest = url.rsplit_once('@').map(|(_, r)| r).unwrap_or(url);
        let (host, path) = rest.split_once(':')?;
        (host.to_string(), path.to_string())
    };
    let path = path.trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    Some(format!("{}/{}", host.to_lowercase(), path))
}

pub fn identify(directory: &Path) -> Project {
    let Some(root) = vcs_root(directory) else {
        return Project {
            id: sha1_hex(&format!("directory:{}", directory.display())),
            worktree: directory.to_path_buf(),
            vcs: None,
        };
    };
    if root.join(".git").exists() {
        let id = git_id(&root).unwrap_or_else(|| "global".to_string());
        return Project {
            id,
            worktree: root,
            vcs: Some("git"),
        };
    }
    let cached = std::fs::read_to_string(root.join(".hg").join("opencode"))
        .ok()
        .map(|s| s.trim().to_string());
    let id = cached.filter(|s| !s.is_empty()).or_else(|| {
        let output = Command::new("hg")
            .arg("-R")
            .arg(&root)
            .args(["log", "-r", "0", "--template", "{node}"])
            .output()
            .ok()?;
        let node = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (output.status.success() && !node.is_empty()).then_some(node)
    });
    Project {
        id: id.unwrap_or_else(|| "global".to_string()),
        worktree: root,
        vcs: Some("hg"),
    }
}

fn git_id(root: &Path) -> Option<String> {
    if let Some(remote) =
        git(root, &["config", "--get", "remote.origin.url"]).and_then(|u| normalize_remote(&u))
    {
        return Some(sha1_hex(&format!("git-remote:{remote}")));
    }
    let common = git(root, &["rev-parse", "--git-common-dir"]).map(|d| {
        let path = PathBuf::from(d);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    });
    if let Some(cached) = common
        .as_ref()
        .and_then(|dir| std::fs::read_to_string(dir.join("opencode")).ok())
    {
        let cached = cached.trim();
        if !cached.is_empty() {
            return Some(cached.to_string());
        }
    }
    let roots = git(root, &["rev-list", "--max-parents=0", "HEAD"])?;
    let mut roots: Vec<&str> = roots
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    roots.sort();
    let id = roots.first()?.to_string();
    if let Some(dir) = common {
        let _ = std::fs::write(dir.join("opencode"), &id);
    }
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_normalization() {
        assert_eq!(
            normalize_remote("git@GitHub.com:Anysola/code.git").as_deref(),
            Some("github.com/Anysola/code")
        );
        assert_eq!(
            normalize_remote("https://github.com/Anysola/code.git").as_deref(),
            Some("github.com/Anysola/code")
        );
        assert_eq!(
            normalize_remote("ssh://git@host:22/a/b").as_deref(),
            Some("host/a/b")
        );
    }

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn git_ids() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("r");
        std::fs::create_dir_all(repo.join("sub")).unwrap();
        run(&repo, &["init", "-q"]);
        assert_eq!(identify(&repo).id, "global", "no commit, no remote");
        run(&repo, &["commit", "-q", "--allow-empty", "-m", "first"]);
        let first = git(&repo, &["rev-list", "--max-parents=0", "HEAD"]).unwrap();
        let project = identify(&repo.join("sub"));
        assert_eq!(
            (project.id.as_str(), project.vcs),
            (first.as_str(), Some("git"))
        );
        assert_eq!(project.worktree, repo);
        assert_eq!(
            std::fs::read_to_string(repo.join(".git/opencode")).unwrap(),
            first,
            "the root commit is cached"
        );
        std::fs::write(repo.join(".git/opencode"), "cached-id").unwrap();
        assert_eq!(identify(&repo).id, "cached-id");
        run(
            &repo,
            &["remote", "add", "origin", "git@github.com:Anysola/code.git"],
        );
        assert_eq!(
            identify(&repo).id,
            sha1_hex("git-remote:github.com/Anysola/code")
        );
        std::fs::create_dir_all(root.path().join("h/.hg")).unwrap();
        std::fs::write(root.path().join("h/.hg/opencode"), "hgnode\n").unwrap();
        let hg = identify(&root.path().join("h"));
        assert_eq!((hg.id.as_str(), hg.vcs), ("hgnode", Some("hg")));
    }

    #[test]
    fn directory_id() {
        let p = identify(Path::new("/nonexistent-anysola/x"));
        assert_eq!(p.id, sha1_hex("directory:/nonexistent-anysola/x"));
    }
}
