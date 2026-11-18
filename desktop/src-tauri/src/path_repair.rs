//! PATH for an app started from the desktop. A desktop file (Linux) or the Start menu (Windows)
//! hands the app the PATH of the session as it was at login, without what shell startup files
//! add (nvm, volta, cargo, `~/.local/bin`) or what an installer added since. Without it MCP
//! servers (`npx`, `uvx`), git, gh, editors and the agent's own commands aren't found.
//!
//! [`repair`] runs first thing in `main`, before any thread starts: the core takes its tools'
//! environment from the process (`Core::start`), so the repaired PATH reaches all of them.

#[cfg(unix)]
use std::time::Duration;

/// How long the login shell may take to print its PATH.
#[cfg(unix)]
const SHELL_WAIT: Duration = Duration::from_secs(3);
/// Brackets the value in the shell's output.
const MARK: &str = "__ANYSOLA_PATH__";

/// The login shell didn't answer in time: said in the log once it is open.
static SLOW_SHELL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Logs what [`repair`] couldn't do; called once the log is open.
pub fn report() {
    if SLOW_SHELL.load(std::sync::atomic::Ordering::Relaxed) {
        anysola_core::log::warn(
            "the login shell didn't print its PATH in time; started without it",
            &[],
        );
    }
}

/// Merges the user's full PATH into the process's. Never fails and never waits long: whatever
/// can't be read is left out.
pub fn repair() {
    // This runs before the window on every start: whatever goes wrong in it (a registry value
    // no one expected, a shell that prints nonsense) must leave the app starting as it did.
    let current = std::env::var("PATH").unwrap_or_default();
    let Ok(Some(path)) = std::panic::catch_unwind(|| full(&current)) else {
        return;
    };
    if settable(&path) && path != current {
        // SAFETY: called at the top of `main`, before any other thread exists.
        unsafe { std::env::set_var("PATH", &path) };
    }
}

/// What the system takes as a variable's value: `set_var` panics on a NUL, and Windows refuses
/// a value past its limit (32,767 characters).
fn settable(path: &str) -> bool {
    !path.is_empty() && !path.contains('\0') && path.encode_utf16().count() < 32_767
}

#[cfg(unix)]
fn full(current: &str) -> Option<String> {
    use std::io::IsTerminal;
    // Started from a terminal: its PATH is the user's already.
    if std::io::stdin().is_terminal() {
        return None;
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| login_path(&s));
    let known: Vec<String> = home
        .as_deref()
        .map(well_known)
        .unwrap_or_default()
        .into_iter()
        .filter(|dir| std::path::Path::new(dir).is_dir())
        .collect();
    let lists = [
        split(shell.as_deref().unwrap_or(""), false),
        split(current, false),
        known,
    ];
    Some(join(&merge(&lists, false), false))
}

/// The login shell's PATH: `$SHELL -ilc`, as a terminal would start it.
#[cfg(unix)]
fn login_path(shell: &str) -> Option<String> {
    use std::io::Read;
    use std::sync::{Arc, Mutex, mpsc};
    let mut command = std::process::Command::new(shell);
    command
        .args(["-ilc", &format!("printf '\\n{MARK}%s{MARK}\\n' \"$PATH\"")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    // Its own group: an rc file that grabs the terminal can't stop the app, and a time limit
    // stops whatever the rc files started with it.
    crate::tree::own_group(&mut command);
    let mut child = command.spawn().ok()?;
    // Read as it comes, so a chatty rc file can't fill the pipe and stall the shell. Only the
    // end is kept: the value is printed last.
    let kept = Arc::new(Mutex::new(Vec::new()));
    let (ended, end) = mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        let kept = kept.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(n) = stdout.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
                kept.extend_from_slice(&buffer[..n]);
                if kept.len() > KEEP {
                    let extra = kept.len() - KEEP;
                    kept.drain(..extra);
                }
            }
            let _ = ended.send(());
        });
    }
    let deadline = std::time::Instant::now() + SHELL_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                crate::tree::kill(&mut child);
                SLOW_SHELL.store(true, std::sync::atomic::Ordering::Relaxed);
                return None;
            }
        }
    }
    // A process the shell left behind may hold the pipe open: what came is taken after a
    // short wait for the end, never more.
    let _ = end.recv_timeout(Duration::from_millis(200));
    let out = kept.lock().unwrap_or_else(|e| e.into_inner()).clone();
    parse_probe(&String::from_utf8_lossy(&out))
}

/// How much of the shell's output is kept, from its end.
#[cfg(unix)]
const KEEP: usize = 1 << 20;

/// What rc files print around it is noise: only the marked value counts.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_probe(output: &str) -> Option<String> {
    let start = output.rfind(MARK)?;
    let before = &output[..start];
    let open = before.rfind(MARK)?;
    let value = &output[open + MARK.len()..start];
    (!value.trim().is_empty() && !value.contains('\n')).then(|| value.to_string())
}

/// Folders where tools are commonly installed for the user, whether or not a shell adds them.
#[cfg_attr(not(unix), allow(dead_code))]
fn well_known(home: &str) -> Vec<String> {
    let mut dirs: Vec<String> = [
        ".local/bin",
        "bin",
        ".cargo/bin",
        ".volta/bin",
        ".bun/bin",
        ".deno/bin",
        "go/bin",
        ".npm-global/bin",
        ".local/share/pnpm",
        ".local/share/JetBrains/Toolbox/scripts",
    ]
    .iter()
    .map(|rest| format!("{}/{rest}", home.trim_end_matches('/')))
    .collect();
    dirs.extend(["/usr/local/bin", "/snap/bin"].map(str::to_string));
    dirs
}

#[cfg(windows)]
fn full(current: &str) -> Option<String> {
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};
    let read = |key: &windows_registry::Key, path: &str| -> Option<String> {
        key.open(path)
            .ok()?
            .get_hstring("Path")
            .ok()
            .map(|value| value.to_string_lossy())
    };
    let machine = read(
        LOCAL_MACHINE,
        r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment",
    );
    let user = read(CURRENT_USER, "Environment");
    if machine.is_none() && user.is_none() {
        return None;
    }
    let lookup = |name: &str| std::env::var(name).ok();
    Some(windows_path(
        current,
        machine.as_deref(),
        user.as_deref(),
        &lookup,
    ))
}

/// The PATH the app was started with first (a folder the user put in front, a venv's or nvm's,
/// stays in front), then what the registry has that it lacks: the machine's, then the user's,
/// as Windows orders them.
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_path(
    current: &str,
    machine: Option<&str>,
    user: Option<&str>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> String {
    let lists = [
        split(current, true),
        split(&expand(machine.unwrap_or(""), lookup), true),
        split(&expand(user.unwrap_or(""), lookup), true),
    ];
    join(&merge(&lists, true), true)
}

#[cfg(not(any(unix, windows)))]
fn full(_: &str) -> Option<String> {
    None
}

/// `%NAME%` replaced by its value (names in any case); an unknown name stays as written, as
/// Windows leaves it.
#[cfg_attr(not(windows), allow(dead_code))]
fn expand(value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name).or_else(|| lookup(&name.to_uppercase())) {
                    Some(found) => out.push_str(&found),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            // `%%` or a lone `%`: kept.
            Some(_) => {
                out.push('%');
                rest = after;
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn split(path: &str, windows: bool) -> Vec<String> {
    let separator = if windows { ';' } else { ':' };
    path.split(separator)
        .map(|p| p.trim())
        .map(|p| if windows { p.trim_matches('"') } else { p })
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

fn join(dirs: &[String], windows: bool) -> String {
    dirs.join(if windows { ";" } else { ":" })
}

/// The lists in order, each folder once: on Windows compared without case or a trailing `\`.
fn merge(lists: &[Vec<String>], windows: bool) -> Vec<String> {
    let same = |dir: &str| {
        let dir = if dir.len() > 1 {
            dir.trim_end_matches(['/', '\\'])
        } else {
            dir
        };
        if windows {
            dir.replace('/', "\\").to_lowercase()
        } else {
            dir.to_string()
        }
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for dir in lists.iter().flatten() {
        if seen.insert(same(dir)) {
            out.push(dir.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    /// The real thing, on whatever this machine has (on Windows: the registry read).
    #[test]
    fn repairing_this_machine_gives_a_value_the_system_takes() {
        let current = std::env::var("PATH").unwrap_or_default();
        if let Some(path) = super::full(&current) {
            assert!(super::settable(&path), "{path:?}");
            // Nothing the app was started with is lost.
            for dir in std::env::split_paths(&current).filter(|d| !d.as_os_str().is_empty()) {
                let dir = dir.to_string_lossy().to_lowercase();
                let dir = dir.trim_end_matches(['\\', '/']);
                assert!(
                    path.to_lowercase().contains(dir),
                    "{dir} is missing from {path}"
                );
            }
        }
    }

    #[test]
    fn a_value_the_system_refuses_is_not_set() {
        assert!(!super::settable(""));
        assert!(!super::settable("a\0b"));
        assert!(!super::settable(&"x".repeat(40_000)));
        assert!(super::settable("/usr/bin:/bin"));
    }

    use super::*;

    #[test]
    fn the_marked_value_is_taken_from_noisy_output() {
        let out = "Welcome!\nnvm: using node 22\n\n__ANYSOLA_PATH__/a:/b__ANYSOLA_PATH__\nbye\n";
        assert_eq!(parse_probe(out).as_deref(), Some("/a:/b"));
        assert_eq!(parse_probe("no markers"), None);
        assert_eq!(parse_probe("__ANYSOLA_PATH____ANYSOLA_PATH__"), None);
        assert_eq!(parse_probe("__ANYSOLA_PATH__/a"), None);
    }

    #[test]
    fn unix_lists_merge_in_order_once() {
        let lists = [
            split("/home/u/.nvm/bin:/usr/bin:/bin", false),
            split("/usr/bin:/bin:/usr/local/bin/:/opt/app", false),
            vec!["/home/u/.local/bin".into(), "/usr/local/bin".into()],
        ];
        assert_eq!(
            join(&merge(&lists, false), false),
            "/home/u/.nvm/bin:/usr/bin:/bin:/usr/local/bin/:/opt/app:/home/u/.local/bin"
        );
        // Unix paths keep their case.
        assert_eq!(merge(&[vec!["/A".into(), "/a".into()]], false).len(), 2);
        assert_eq!(merge(&[split("::/x:", false)], false), ["/x"]);
        assert_eq!(merge(&[vec!["/".into()]], false), ["/"]);
    }

    #[test]
    fn windows_lists_merge_without_case_or_trailing_separator() {
        let lists = [
            split(
                r"C:\Windows\system32;C:\Windows;C:\Program Files\Git\cmd",
                true,
            ),
            split(
                r#"C:\Users\u\AppData\Roaming\npm;"C:\Program Files\nodejs\""#,
                true,
            ),
            split(
                r"c:\windows\System32\;C:\Program Files\nodejs;C:\Tools;;",
                true,
            ),
        ];
        assert_eq!(
            merge(&lists, true),
            [
                r"C:\Windows\system32",
                r"C:\Windows",
                r"C:\Program Files\Git\cmd",
                r"C:\Users\u\AppData\Roaming\npm",
                r"C:\Program Files\nodejs\",
                r"C:\Tools",
            ]
        );
        assert_eq!(
            join(&merge(&lists[..1], true), true).matches(';').count(),
            2
        );
    }

    #[test]
    fn the_windows_path_keeps_what_the_app_was_started_with_first() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some(r"C:\Users\u".to_string()),
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            _ => None,
        };
        let current = r"C:\work\.venv\Scripts;C:\Windows\system32;C:\Program Files\Git\cmd";
        let machine = r"%SystemRoot%\system32;%SystemRoot%;C:\Program Files\nodejs\";
        let user = r"%USERPROFILE%\.cargo\bin;C:\Users\u\AppData\Roaming\npm";
        assert_eq!(
            windows_path(current, Some(machine), Some(user), &lookup),
            [
                r"C:\work\.venv\Scripts",
                r"C:\Windows\system32",
                r"C:\Program Files\Git\cmd",
                r"C:\Windows",
                r"C:\Program Files\nodejs\",
                r"C:\Users\u\.cargo\bin",
                r"C:\Users\u\AppData\Roaming\npm",
            ]
            .join(";")
        );
        // Started with no PATH at all: the registry's, machine first.
        assert_eq!(
            windows_path("", Some(machine), None, &lookup),
            r"C:\Windows\system32;C:\Windows;C:\Program Files\nodejs\"
        );
        assert_eq!(windows_path(current, None, None, &lookup), current);
    }

    #[test]
    fn variables_expand() {
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some(r"C:\Users\u".to_string()),
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            _ => None,
        };
        assert_eq!(
            expand(r"%USERPROFILE%\.cargo\bin;%SystemRoot%\system32", &lookup),
            r"C:\Users\u\.cargo\bin;C:\Windows\system32"
        );
        // Another case of a name the lookup knows in upper case.
        assert_eq!(expand(r"%userprofile%\bin", &lookup), r"C:\Users\u\bin");
        assert_eq!(expand(r"%NOPE%\bin", &lookup), r"%NOPE%\bin");
        assert_eq!(expand("100%", &lookup), "100%");
        assert_eq!(expand("a%%b", &lookup), "a%%b");
        assert_eq!(expand("plain", &lookup), "plain");
        assert_eq!(
            expand(r"%USERPROFILE%%SystemRoot%", &lookup),
            r"C:\Users\uC:\Windows"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_login_shell_answers_or_is_given_up() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "anysola-path-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // A shell whose startup files talk: the command still runs, and only its value counts.
        let chatty = dir.join("chatty-shell");
        std::fs::write(
            &chatty,
            "#!/bin/sh\necho 'Welcome back'\nPATH=/opt/tool/bin:/usr/bin exec /bin/sh -c \"$2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&chatty, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            login_path(&chatty.display().to_string()).as_deref(),
            Some("/opt/tool/bin:/usr/bin")
        );
        // A shell whose startup hangs costs a few seconds, not the start of the app.
        let shell = dir.join("slow-shell");
        std::fs::write(&shell, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(login_path(&shell.display().to_string()), None);
        assert!(started.elapsed() < SHELL_WAIT + Duration::from_secs(2));
        assert_eq!(login_path("/nonexistent/shell"), None);
        // Startup files that print more than a pipe holds before the value.
        let loud = dir.join("loud-shell");
        std::fs::write(
            &loud,
            "#!/bin/sh\nhead -c 300000 /dev/zero | tr '\\0' x\necho\nPATH=/loud/bin exec /bin/sh -c \"$2\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&loud, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            login_path(&loud.display().to_string()).as_deref(),
            Some("/loud/bin")
        );
        // A hung startup's helpers are stopped with it.
        let pidfile = dir.join("helper");
        let hung = dir.join("hung-shell");
        std::fs::write(
            &hung,
            format!(
                "#!/bin/sh\nsleep 60 &\necho $! > '{}'\nwait\n",
                pidfile.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&hung, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(login_path(&hung.display().to_string()), None);
        let helper: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let started = std::time::Instant::now();
        while !crate::tree::gone(helper) {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the helper runs on"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn well_known_folders_are_under_home() {
        let dirs = well_known("/home/u/");
        assert!(dirs.contains(&"/home/u/.local/bin".to_string()));
        assert!(dirs.contains(&"/home/u/.cargo/bin".to_string()));
    }
}
