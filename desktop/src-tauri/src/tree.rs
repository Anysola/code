//! Child processes that start helpers of their own (git's hooks, ssh and credential helpers; a
//! login shell's rc files): started as a group, so a time limit stops all of them.

use std::process::{Child, Command};

/// Starts the command as the leader of a process group of its own (Unix). On Windows the tree
/// is found from the process at kill time.
pub fn own_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(not(unix))]
    let _ = command;
}

/// Kills the child and everything it started, then reaps it. Call it before the child was
/// waited for: once reaped, its id may name another process.
pub fn kill(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: kill(2) with a negative pid signals the process group the child leads.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = anysola_core::console::command("taskkill")
            .args(["/pid", &child.id().to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Whether the process is gone: ended, or a zombie nobody reaped yet (a container's first
/// process may never reap the orphans it inherits).
#[cfg(all(test, unix))]
pub fn gone(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return true;
    }
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn the_whole_group_is_stopped() {
        let dir = std::env::temp_dir().join(format!(
            "anysola-tree-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("grandchild");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!("sleep 30 & echo $! > '{}'; wait", pidfile.display()),
        ]);
        own_group(&mut command);
        let mut child = command.spawn().unwrap();
        let started = std::time::Instant::now();
        while std::fs::read_to_string(&pidfile).map_or(true, |s| !s.ends_with('\n')) {
            assert!(started.elapsed() < std::time::Duration::from_secs(5));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let grandchild: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        kill(&mut child);
        let started = std::time::Instant::now();
        while !gone(grandchild) {
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "the grandchild still runs"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
