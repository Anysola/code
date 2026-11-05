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
