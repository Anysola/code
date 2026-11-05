//! `anysola update` (alias `upgrade`): installs the latest release the way this copy was installed,
//! by running the install script again or through npm.

use std::time::Duration;

use anysola_core::gateway::Gateway;
use anysola_core::paths::Paths;
use anysola_core::version::{VERSION, compare};

use crate::clack;
use crate::cli::UpdateArgs;
use crate::install::{self, Platform};

/// 0.5.1's `released()`: anything starting with a numeric version except `0.0.0*`.
fn released(version: &str) -> bool {
    let mut parts = version.splitn(4, '.');
    let numeric =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let major = parts.next();
    let minor = parts.next();
    let patch = parts.next().map(|p| {
        p.split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap_or_default()
    });
    numeric(major) && numeric(minor) && numeric(patch) && !version.starts_with("0.0.0")
}

pub async fn update(args: UpdateArgs) -> i32 {
    clack::intro("Update Anysola Code");
    let info = Gateway::new(None).version(Duration::from_secs(5)).await;
    let home = Paths::from_env().home;
    let platform = Platform::current();
    let command = install::update_command(platform, install::by_script(&home));
    let shown = install::display(&command);
    if let Some(info) = info {
        clack::info(&format!("Installed {VERSION}, latest {}", info.latest));
        if let Some(notes) = info.notes.filter(|n| !n.is_empty()) {
            clack::message(&notes);
        }
        if released(VERSION) && compare(&info.latest, VERSION).is_le() && !args.force {
            clack::outro("Already up to date");
            return 0;
        }
    }
    if args.print {
        clack::message(&shown);
        clack::outro("Run the command above to update");
        return 0;
    }
    clack::step(&shown);
    let bin = &command[0];
    // npm on Windows is npm.cmd, which only runs through a shell.
    let mut process = if platform == Platform::Windows && bin == "npm" {
        let mut shell = std::process::Command::new("cmd");
        shell.arg("/c").args(&command);
        shell
    } else {
        let mut direct = std::process::Command::new(bin);
        direct.args(&command[1..]);
        direct
    };
    #[cfg(windows)]
    let _retained = retain_image(&Paths::from_env().data.join("cache"));
    let failure = match process.status() {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!(
            "{bin} failed with exit code {}",
            status.code().map_or("null".to_string(), |c| c.to_string())
        )),
        Err(error) => Some(format!("{bin} failed: {error}")),
    };
    match failure {
        Some(message) => {
            clack::error(&message);
            clack::info(&format!("Run it yourself: {shown}"));
            1
        }
        None => {
            clack::outro("Updated. Restart Anysola Code to use the new version.");
            0
        }
    }
}

/// Windows refuses to delete the last hard link of a running executable, so the installer could
/// not replace this one (or the TUI that started it). A second link in the data folder, held
/// while the installer runs, lets it. Links of runs that have exited are swept first.
#[cfg(windows)]
struct Retained(Option<std::path::PathBuf>);

#[cfg(windows)]
impl Drop for Retained {
    fn drop(&mut self) {
        // The last link of a still-running image refuses deletion; a later sweep removes it.
        if let Some(path) = &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
fn retain_image(directory: &std::path::Path) -> Retained {
    let _ = std::fs::create_dir_all(directory);
    for entry in std::fs::read_dir(directory).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_prefix("anysola-upgrade-")
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok());
        if let Some(pid) = pid
            && !anysola_core::storage::process_alive(pid)
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let target = directory.join(format!(
        "anysola-upgrade-{}-{nonce:08x}.exe",
        std::process::id()
    ));
    let linked = std::env::current_exe()
        .and_then(|exe| std::fs::hard_link(exe, &target))
        .is_ok();
    Retained(linked.then_some(target))
}

#[cfg(test)]
mod tests {
    #[test]
    fn released_versions() {
        for (version, expected) in [
            ("1.0.0", true),
            ("1.0.0-dev", true),
            ("1.0.0-rc.1", true),
            ("0.0.0-local", false),
            ("local", false),
            ("1.0", false),
        ] {
            assert_eq!(super::released(version), expected, "{version}");
        }
    }
}
