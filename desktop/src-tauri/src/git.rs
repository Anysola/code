//! The session header's git menu: the folder's branch and changes, and the steps from there to
//! a pull request (new branch, commit, push, open the PR). Runs the user's own `git` (and `gh`
//! when installed), so their credentials and hooks apply.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};
use tauri::{AppHandle, State};

use crate::{Answer, App, core_for, text};

/// git with no console window on Windows and no credential prompt on a terminal nobody sees:
/// a push that needs one fails with git's message instead of hanging.
fn command(program: &str, directory: &str, args: &[&str]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(directory)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn run(program: &str, directory: &str, args: &[&str]) -> Result<Output, String> {
    command(program, directory, args).output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{program} isn’t installed, or isn’t on PATH.")
        } else {
            format!("Couldn’t run {program}: {e}")
        }
    })
}

/// stdout on success; git's own message (stderr, else stdout) on failure.
fn ok(program: &str, directory: &str, args: &[&str]) -> Result<String, String> {
    let out = run(program, directory, args)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let message = if err.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    } else {
        err
    };
    Err(if message.is_empty() {
        format!("{program} {} failed.", args.first().unwrap_or(&""))
    } else {
        message
    })
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(text)?
}

/// `## main...origin/main [ahead 1, behind 2]` from `git status -b`.
fn parse_branch(line: &str) -> (String, Option<String>, u64, u64) {
    let line = line.trim_start_matches("## ");
    let (head, counts) = match line.split_once(" [") {
        Some((head, counts)) => (head, counts.trim_end_matches(']')),
        None => (line, ""),
    };
    let (branch, upstream) = match head.split_once("...") {
        Some((branch, upstream)) => (branch.to_string(), Some(upstream.to_string())),
        None => (head.to_string(), None),
    };
    let branch = branch
        .strip_prefix("No commits yet on ")
        .unwrap_or(&branch)
        .to_string();
    let count = |key: &str| {
        counts
            .split(", ")
            .find_map(|part| part.strip_prefix(key))
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0)
    };
    (branch, upstream, count("ahead "), count("behind "))
}

/// `git status --porcelain=v1 -b -z` as the branch line and `{file, status}` entries.
fn parse_status(raw: &str) -> (Option<String>, Vec<Value>) {
    let mut parts = raw.split('\0').filter(|p| !p.is_empty());
    let mut branch = None;
    let mut files = Vec::new();
    while let Some(part) = parts.next() {
        if let Some(line) = part.strip_prefix("## ") {
            branch = Some(line.to_string());
            continue;
        }
        if part.len() < 4 {
            continue;
        }
        let (code, file) = part.split_at(3);
        let code = code.trim();
        // A rename's old path follows as its own entry.
        if code.starts_with('R') || code.starts_with('C') {
            parts.next();
        }
        let status = match code {
            "??" => "added",
            c if c.contains('D') => "deleted",
            c if c.contains('A') => "added",
            _ => "modified",
        };
        files.push(json!({"file": file, "status": status}));
    }
    (branch, files)
}

/// The folder's branch, how far it is from its upstream, its uncommitted files and whether a
/// pull request can be opened with `gh`. `{"repo": false}` outside git.
#[tauri::command]
pub async fn git_status(directory: String) -> Answer {
    blocking(move || {
        let inside = run("git", &directory, &["rev-parse", "--is-inside-work-tree"]);
        if !matches!(&inside, Ok(out) if out.status.success()) {
            return Ok(json!({"repo": false}));
        }
        let raw = ok(
            "git",
            &directory,
            &[
                "status",
                "--porcelain=v1",
                "-b",
                "-z",
                "--untracked-files=all",
            ],
        )?;
        let (line, files) = parse_status(&raw);
        let (branch, upstream, ahead, behind) = parse_branch(line.as_deref().unwrap_or(""));
        let remote = ok("git", &directory, &["remote"])
            .ok()
            .and_then(|r| r.lines().next().map(str::to_string));
        let gh = run("gh", &directory, &["--version"]).is_ok_and(|o| o.status.success());
        Ok(json!({
            "repo": true,
            "branch": branch,
            "detached": branch.starts_with("HEAD"),
            "upstream": upstream,
            "ahead": ahead,
            "behind": behind,
            "files": files,
            "remote": remote,
            "gh": gh,
        }))
    })
    .await
}

/// A new branch from here, taking the uncommitted changes along.
#[tauri::command]
pub async fn git_branch(directory: String, name: String) -> Answer {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("A branch needs a name.".into());
    }
    blocking(move || {
        ok("git", &directory, &["check-ref-format", "--branch", &name])
            .map_err(|_| format!("“{name}” isn’t a valid branch name."))?;
        ok("git", &directory, &["switch", "-c", &name])?;
        Ok(Value::Null)
    })
    .await
}

/// Commits every change in the folder (`git add -A`), hooks included.
#[tauri::command]
pub async fn git_commit(directory: String, message: String) -> Answer {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("A commit needs a message.".into());
    }
    blocking(move || {
        ok("git", &directory, &["add", "-A"])?;
        ok("git", &directory, &["commit", "-q", "-m", &message])?;
        let head = ok("git", &directory, &["rev-parse", "--short", "HEAD"])?;
        Ok(json!({"commit": head}))
    })
    .await
}

/// Pushes the branch, setting its upstream the first time.
#[tauri::command]
pub async fn git_push(directory: String) -> Answer {
    blocking(move || {
        let upstream = ok(
            "git",
            &directory,
            &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        );
        match upstream {
            Ok(_) => ok("git", &directory, &["push", "-q"])?,
            Err(_) => {
                let remote = ok("git", &directory, &["remote"])?
                    .lines()
                    .next()
                    .map(str::to_string)
                    .ok_or("This repository has no remote to push to.")?;
                ok("git", &directory, &["push", "-q", "-u", &remote, "HEAD"])?
            }
        };
        Ok(Value::Null)
    })
    .await
}

/// `https://github.com/o/r` from `git@github.com:o/r.git` or `https://github.com/o/r.git`.
fn web_url(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches(".git");
    if let Some(rest) = remote.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return Some(format!("https://{host}/{path}"));
    }
    if let Some(rest) = remote.strip_prefix("ssh://git@") {
        return Some(format!("https://{rest}"));
    }
    let rest = remote.strip_prefix("https://")?;
    // Credentials in the URL never go to the browser.
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    Some(format!("https://{host}/{path}"))
}

/// Opens a pull request for the pushed branch: with `gh` when installed (its URL comes back),
/// else the host's compare page in the browser.
#[tauri::command]
pub async fn git_pr(handle: AppHandle, directory: String, title: String, body: String) -> Answer {
    let found = {
        let directory = directory.clone();
        blocking(move || {
            // gh opens it outright; signed out of gh, or for a host gh doesn't serve, the
            // host's compare page opens instead.
            let gh = run("gh", &directory, &["--version"]).is_ok_and(|o| o.status.success());
            let mut gh_error = None;
            if gh {
                match ok(
                    "gh",
                    &directory,
                    &["pr", "create", "--title", title.trim(), "--body", &body],
                ) {
                    Ok(url) => {
                        return Ok((url.lines().last().unwrap_or_default().to_string(), true));
                    }
                    Err(error) => gh_error = Some(error),
                }
            }
            let compare = || -> Result<String, String> {
                let branch = ok("git", &directory, &["branch", "--show-current"])?;
                let remote = ok("git", &directory, &["remote"])?
                    .lines()
                    .next()
                    .map(str::to_string)
                    .ok_or("This repository has no remote.")?;
                let url = ok("git", &directory, &["remote", "get-url", &remote])?;
                let base = web_url(&url).ok_or("Can’t tell this remote’s web address.")?;
                Ok(format!("{base}/compare/{branch}?expand=1"))
            };
            match (compare(), gh_error) {
                (Ok(url), _) => Ok((url, false)),
                (Err(_), Some(error)) | (Err(error), None) => Err(error),
            }
        })
        .await?
    };
    let (url, created) = found;
    if !created {
        use tauri_plugin_opener::OpenerExt;
        handle.opener().open_url(&url, None::<&str>).map_err(text)?;
    }
    Ok(json!({"url": url, "created": created}))
}

/// A commit message for the session's changes, from the session itself (a one-off answer that
/// adds nothing to the session).
#[tauri::command]
pub async fn suggest_commit_message(
    handle: AppHandle,
    app: State<'_, App>,
    directory: String,
    session_id: String,
) -> Answer {
    let core = core_for(&handle, &app, &directory).await?;
    let answer = core
        .aside(
            &session_id,
            "Write a git commit message for the changes made in this session: a subject line \
             under 72 characters in the imperative mood, then, only if it helps, a blank line \
             and a short body. Reply with the message only, no code fences.",
        )
        .await?;
    Ok(json!(answer.trim().trim_matches('`').trim()))
}

/// Opens the folder in the system's file manager.
#[tauri::command]
pub async fn reveal(handle: AppHandle, directory: String) -> Answer {
    use tauri_plugin_opener::OpenerExt;
    if !Path::new(&directory).is_dir() {
        return Err(format!("{directory} is no longer on this computer."));
    }
    handle
        .opener()
        .open_path(&directory, None::<&str>)
        .map_err(text)?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_lines() {
        assert_eq!(
            parse_branch("## main...origin/main [ahead 2, behind 1]"),
            ("main".into(), Some("origin/main".into()), 2, 1)
        );
        assert_eq!(parse_branch("## feature"), ("feature".into(), None, 0, 0));
        assert_eq!(
            parse_branch("## No commits yet on main"),
            ("main".into(), None, 0, 0)
        );
    }

    #[test]
    fn status_entries() {
        let raw = "## main\0 M src/a.rs\0?? new.txt\0R  b.rs\0old.rs\0 D gone.rs\0";
        let (branch, files) = parse_status(raw);
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(
            files,
            [
                json!({"file": "src/a.rs", "status": "modified"}),
                json!({"file": "new.txt", "status": "added"}),
                json!({"file": "b.rs", "status": "modified"}),
                json!({"file": "gone.rs", "status": "deleted"}),
            ]
        );
    }

    #[test]
    fn web_urls() {
        assert_eq!(
            web_url("git@github.com:Anysola/code.git").as_deref(),
            Some("https://github.com/Anysola/code")
        );
        assert_eq!(
            web_url("https://github.com/Anysola/code.git").as_deref(),
            Some("https://github.com/Anysola/code")
        );
        assert_eq!(
            web_url("https://x-access-token:abc@github.com/Anysola/code").as_deref(),
            Some("https://github.com/Anysola/code")
        );
        assert_eq!(web_url("/srv/repo.git"), None);
    }
}
