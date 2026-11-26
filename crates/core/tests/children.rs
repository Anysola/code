//! Children of ours when the app dies without stopping them (`anysola_core::children`).
//!
//! This binary plays every part itself: the test starts it again as a stand-in app, which starts
//! it as a child (through the helper) and as an "installer" (outside it; on Windows with
//! `ShellExecuteExW`, as the updater does); the child starts it as a grandchild. Every pid is a
//! real one of the platform (Git Bash's `$!` is not a Windows pid). The parts are picked by
//! `ANYSOLA_CHILDREN_ROLE`, run by the test named `role`, which does nothing otherwise.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anysola_core::children::{self, Leads};

const ROLE: &str = "ANYSOLA_CHILDREN_ROLE";
const DIR: &str = "ANYSOLA_CHILDREN_DIR";
const INDEX: &str = "ANYSOLA_CHILDREN_INDEX";
const COUNT: &str = "ANYSOLA_CHILDREN_COUNT";
/// For a child: exit once its grandchild runs (a tree whose leader is gone).
const THEN_EXIT: &str = "ANYSOLA_CHILDREN_THEN_EXIT";
/// For the app: also start a child leading a session of its own, as the embedded terminal's
/// shell does (Linux); its grandchild gets a group of its own, as a shell's job does.
#[cfg(target_os = "linux")]
const SESSION: &str = "ANYSOLA_CHILDREN_SESSION";
#[cfg(target_os = "linux")]
const OWN_GROUP: &str = "ANYSOLA_CHILDREN_OWN_GROUP";

fn me() -> PathBuf {
    std::env::current_exe().expect("the test binary")
}

/// This binary, to run the part `role` (the rest of the environment as given).
fn part(role: &str, dir: &Path) -> Command {
    let mut command = Command::new(me());
    command
        .args(["role", "--exact", "--test-threads=1", "--nocapture"])
        .env(ROLE, role)
        .env(DIR, dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_for(path: &Path, limit: Duration) -> String {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && text.ends_with('\n')
        {
            return text.trim().to_string();
        }
        assert!(started.elapsed() < limit, "{} never came", path.display());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pid_in(path: &Path) -> u32 {
    wait_for(path, Duration::from_secs(20))
        .parse()
        .expect("a pid")
}

fn write_line(path: &Path, text: &str) {
    let partial = path.with_extension("partial");
    std::fs::write(&partial, format!("{text}\n")).unwrap();
    std::fs::rename(&partial, path).unwrap();
}

/// The parts. Nothing unless this process was started as one.
#[test]
fn role() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let dir = PathBuf::from(std::env::var_os(DIR).expect("the part's folder"));
    match role.as_str() {
        "app" => app(&dir),
        "child" => child(&dir),
        "grandchild" | "installer" => {}
        other => panic!("no part {other}"),
    }
    std::thread::sleep(Duration::from_secs(120));
}

/// Starts `ANYSOLA_CHILDREN_COUNT` children through the helper (each leading a group of its
/// own) and one installer outside it, then waits to be killed.
fn app(dir: &Path) {
    children::keep_records(dir.join("records"));
    let count: usize = std::env::var(COUNT)
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(1);
    for index in 0..count {
        let mut command = part("child", dir);
        command.env(INDEX, index.to_string());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mark = children::mark(&mut command);
        let child = command.spawn().expect("the child starts");
        let ours = children::adopt(&child, Leads::Group, mark);
        // Both are kept for good: this part is only ever killed.
        std::mem::forget(ours);
        write_line(&dir.join(format!("child-{index}")), &child.id().to_string());
        std::mem::forget(child);
        // The child starts its grandchild only now that it is in the Job.
        write_line(&dir.join(format!("joined-{index}")), "yes");
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os(SESSION).is_some() {
        use std::os::unix::process::CommandExt;
        let mut command = part("child", dir);
        command.env(INDEX, "s").env(OWN_GROUP, "1");
        // SAFETY: setsid(2) between fork and exec, as portable-pty does for a terminal's shell.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mark = children::mark(&mut command);
        let child = command.spawn().expect("the session child starts");
        std::mem::forget(children::adopt_session(Some(child.id()), mark));
        write_line(&dir.join("child-s"), &child.id().to_string());
        std::mem::forget(child);
        write_line(&dir.join("joined-s"), "yes");
    }
    let installer = start_installer(dir);
    write_line(&dir.join("installer"), &installer.to_string());
}

fn child(dir: &Path) {
    let index = std::env::var(INDEX).expect("an index");
    wait_for(
        &dir.join(format!("joined-{index}")),
        Duration::from_secs(20),
    );
    let mut grandchild = part("grandchild", dir);
    #[cfg(target_os = "linux")]
    if std::env::var_os(OWN_GROUP).is_some() {
        use std::os::unix::process::CommandExt;
        grandchild.process_group(0);
    }
    let grandchild = grandchild.spawn().expect("the grandchild starts");
    write_line(
        &dir.join(format!("grandchild-{index}")),
        &grandchild.id().to_string(),
    );
    if std::env::var_os(THEN_EXIT).is_some() {
        std::process::exit(0);
    }
    std::mem::forget(grandchild);
}

/// The installer, started the way the updater starts its own: from the app's process, not
/// through the helper. Windows: `ShellExecuteExW`, as `tauri-plugin-updater` does.
fn start_installer(dir: &Path) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::GetProcessId;
        use windows_sys::Win32::UI::Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
        let wide = |s: &std::ffi::OsStr| s.encode_wide().chain([0]).collect::<Vec<u16>>();
        let file = wide(me().as_os_str());
        let parameters = wide(std::ffi::OsStr::new(
            "role --exact --test-threads=1 --nocapture",
        ));
        let verb = wide(std::ffi::OsStr::new("open"));
        // ShellExecute passes this process's environment on.
        // SAFETY: this part's threads don't read the environment meanwhile.
        unsafe {
            std::env::set_var(ROLE, "installer");
            std::env::set_var(DIR, dir);
        }
        // SAFETY: a zeroed SHELLEXECUTEINFOW is valid; the strings outlive the call.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_HIDE;
        // SAFETY: `info` is filled in as the call documents.
        assert!(
            unsafe { ShellExecuteExW(&mut info) } != 0,
            "ShellExecuteExW"
        );
        // SAFETY: the process handle SEE_MASK_NOCLOSEPROCESS gave us; closed once.
        unsafe {
            let pid = GetProcessId(info.hProcess);
            CloseHandle(info.hProcess);
            pid
        }
    }
    #[cfg(not(windows))]
    {
        let installer = part("installer", dir)
            .spawn()
            .expect("the installer starts");
        let pid = installer.id();
        std::mem::forget(installer);
        pid
    }
}

fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
        };
        // SAFETY: OpenProcess with a pid; the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return false;
        }
        // SAFETY: a handle we opened, waited on without blocking, closed once.
        unsafe {
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        // A zombie nobody reaped yet (a container's first process may never) is gone too.
        !std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            stat.rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
        })
    }
}

fn kill(pid: u32) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        // SAFETY: OpenProcess with a pid; the handle is closed below.
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if !handle.is_null() {
            // SAFETY: a handle we opened, closed once.
            unsafe {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: kill(2) with a pid of this test's own parts.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

fn gone_within(pid: u32, limit: Duration) -> bool {
    let started = Instant::now();
    while alive(pid) {
        if started.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

/// Whatever the test started dies with it, passed or not.
struct Parts {
    dir: PathBuf,
    pids: Vec<u32>,
    app: Option<Child>,
}

impl Parts {
    fn new(name: &str) -> Parts {
        let dir = std::env::temp_dir().join(format!(
            "anysola-children-{name}-{}-{}",
            std::process::id(),
            anysola_core::ids::part()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Parts {
            dir,
            pids: Vec::new(),
            app: None,
        }
    }

    fn records(&self) -> PathBuf {
        self.dir.join("records")
    }

    fn start_app(&mut self, count: usize) {
        self.start_app_with(count, &[]);
    }

    fn start_app_with(&mut self, count: usize, env: &[(&str, &str)]) {
        let app = part("app", &self.dir)
            .env(COUNT, count.to_string())
            .envs(env.iter().copied())
            .spawn()
            .expect("the app starts");
        self.app = Some(app);
    }

    fn pid(&mut self, name: &str) -> u32 {
        let pid = pid_in(&self.dir.join(name));
        self.pids.push(pid);
        pid
    }

    /// The app dies as in a crash (Windows: TerminateProcess; Unix: SIGKILL).
    fn crash(&mut self) {
        let mut app = self.app.take().expect("an app");
        app.kill().unwrap();
        app.wait().unwrap();
    }
}

impl Drop for Parts {
    fn drop(&mut self) {
        if let Some(mut app) = self.app.take() {
            let _ = app.kill();
            let _ = app.wait();
        }
        for &pid in &self.pids {
            kill(pid);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The updater's guarantee and the point of the Job: when the app dies, what it started
/// through the helper goes, grandchild included; what it started otherwise (the installer)
/// lives on. On Linux the group is left until the next start's sweep, which ends it and spares
/// the installer.
#[test]
fn a_crash_takes_our_children_and_spares_the_installer() {
    let mut parts = Parts::new("crash");
    parts.start_app(1);
    let child = parts.pid("child-0");
    let grandchild = parts.pid("grandchild-0");
    let installer = parts.pid("installer");
    assert!(alive(child) && alive(grandchild) && alive(installer));
    parts.crash();
    if cfg!(windows) {
        assert!(
            gone_within(child, Duration::from_secs(10)),
            "the child outlived the app"
        );
        assert!(
            gone_within(grandchild, Duration::from_secs(10)),
            "the grandchild outlived the app"
        );
    } else {
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            alive(child) && alive(grandchild),
            "nothing on Linux ends them at once"
        );
        assert!(children::sweep(&parts.records()) >= 2);
        assert!(
            gone_within(child, Duration::from_secs(5)),
            "the sweep left the child"
        );
        assert!(
            gone_within(grandchild, Duration::from_secs(5)),
            "the sweep left the grandchild"
        );
        assert_eq!(
            std::fs::read_dir(parts.records()).unwrap().count(),
            0,
            "the record stays after the sweep"
        );
    }
    std::thread::sleep(Duration::from_millis(500));
    assert!(alive(installer), "the installer died with the app");
}

/// A [`children::Tree`] stops what its child started also after the child itself ended (a
/// finished setup command's `npm run dev &`, an npm shim's node): `taskkill /T` and the group's
/// id can't be used then.
#[test]
fn a_tree_ends_what_its_child_left_behind() {
    let mut parts = Parts::new("tree");
    let mut command = part("child", &parts.dir);
    command.env(INDEX, "0").env(THEN_EXIT, "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mark = children::mark(&mut command);
    let mut child = command.spawn().expect("the child starts");
    let tree = children::tree(&child, mark);
    write_line(&parts.dir.join("joined-0"), "yes");
    let grandchild = parts.pid("grandchild-0");
    // The child has ended and was reaped: its id may name anything now.
    assert!(child.wait().unwrap().success());
    assert!(alive(grandchild));
    assert!(tree.kill());
    assert!(
        gone_within(grandchild, Duration::from_secs(10)),
        "the grandchild outlived its tree"
    );
}

/// The app's own process never joins the Job (its installer, editor and browser would inherit
/// it); the child it adopts does.
#[cfg(windows)]
#[test]
fn the_app_never_joins_the_job() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let parts = Parts::new("job");
    let mut grandchild = part("grandchild", &parts.dir).spawn().unwrap();
    let ours = children::adopt(&grandchild, Leads::Nothing, children::Mark::new());
    assert!(children::in_app_job(grandchild.as_raw_handle()));
    // SAFETY: the pseudo handle of this process.
    assert!(!children::in_app_job(unsafe { GetCurrentProcess() }));
    let _ = grandchild.kill();
    let _ = grandchild.wait();
    drop(ours);
}

/// Each criterion alone keeps a process alive: a record whose app still runs, another token, a
/// start time before the group's leader, another boot, a group that isn't ours, and another
/// child's group of the same run (a group id reused while a record of ours still names it).
#[cfg(target_os = "linux")]
#[test]
fn the_sweep_kills_only_what_it_can_verify() {
    use std::collections::HashMap;
    let mut parts = Parts::new("verify");
    parts.start_app(6);
    let mut pids = Vec::new();
    for index in 0..6 {
        pids.push((
            parts.pid(&format!("child-{index}")),
            parts.pid(&format!("grandchild-{index}")),
        ));
    }
    parts.pid("installer");
    let records = parts.records();
    let by_child = || -> HashMap<u32, PathBuf> {
        std::fs::read_dir(&records)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| !p.file_name().unwrap().to_string_lossy().starts_with('.'))
            .map(|path| {
                let text = std::fs::read_to_string(&path).unwrap();
                let id = text
                    .lines()
                    .find_map(|l| l.strip_prefix("id "))
                    .unwrap()
                    .parse()
                    .unwrap();
                (id, path)
            })
            .collect()
    };
    assert_eq!(by_child().len(), 6);

    // The app still runs: nothing of its is touched, and its records stay.
    assert_eq!(children::sweep(&records), 0);
    assert_eq!(by_child().len(), 6);
    assert!(pids.iter().all(|&(c, g)| alive(c) && alive(g)));

    parts.crash();
    let found = by_child();
    let rewrite = |child: u32, key: &str, value: &str| {
        let path = &found[&child];
        let text = std::fs::read_to_string(path).unwrap();
        let text: String = text
            .lines()
            .map(|line| {
                if line.split_once(' ').is_some_and(|(k, _)| k == key) {
                    format!("{key} {value}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        std::fs::write(path, text).unwrap();
    };
    // 0 is left as it is. 1: another token.
    rewrite(pids[1].0, "token", "someotherchildstoken000000000000");
    // 2: the group's leader started later than its members did.
    let start: u64 = std::fs::read_to_string(&found[&pids[2].0])
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("start "))
        .unwrap()
        .parse()
        .unwrap();
    rewrite(pids[2].0, "start", &(start + 1_000_000).to_string());
    // 3: another boot.
    rewrite(pids[3].0, "boot", "00000000-0000-0000-0000-000000000000");
    // 4: a group that isn't ours (its leader carries no token), with its real start time.
    let mut stranger = Command::new("sleep");
    stranger.arg("120");
    {
        use std::os::unix::process::CommandExt;
        stranger.process_group(0);
    }
    let mut stranger = stranger.spawn().unwrap();
    let stranger_pid = stranger.id();
    let stranger_start = std::fs::read_to_string(format!("/proc/{stranger_pid}/stat"))
        .unwrap()
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_string();
    rewrite(pids[4].0, "id", &stranger_pid.to_string());
    rewrite(pids[4].0, "start", &stranger_start);
    // 5: child 1's group (as if its id had come back to a later child), with child 5's token.
    let start_1 = std::fs::read_to_string(&found[&pids[1].0])
        .unwrap()
        .lines()
        .find_map(|l| l.strip_prefix("start "))
        .unwrap()
        .to_string();
    rewrite(pids[5].0, "id", &pids[1].0.to_string());
    rewrite(pids[5].0, "start", &start_1);

    let killed = children::sweep(&records);
    assert_eq!(killed, 2, "only the control group's two processes");
    assert!(gone_within(pids[0].0, Duration::from_secs(5)));
    assert!(gone_within(pids[0].1, Duration::from_secs(5)));
    std::thread::sleep(Duration::from_millis(300));
    for &(child, grandchild) in &pids[1..] {
        assert!(
            alive(child) && alive(grandchild),
            "{child} or {grandchild} was killed"
        );
    }
    assert!(alive(stranger_pid), "a stranger's group was killed");
    assert!(
        std::fs::read_dir(&records).unwrap().next().is_none(),
        "records stay once swept"
    );
    let _ = stranger.kill();
    let _ = stranger.wait();
}

/// What a crash leaves of the embedded terminal's session (a job in a group of its own, which
/// the kernel's hangup doesn't reach once the shell ignores it) goes at the next start, as
/// closing the terminal would have ended it.
#[cfg(target_os = "linux")]
#[test]
fn a_terminal_session_left_by_a_crash_is_swept() {
    let mut parts = Parts::new("session");
    parts.start_app_with(0, &[(SESSION, "1")]);
    let shell = parts.pid("child-s");
    let job = parts.pid("grandchild-s");
    parts.pid("installer");
    parts.crash();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(shell) && alive(job));
    assert_eq!(children::sweep(&parts.records()), 2);
    assert!(gone_within(shell, Duration::from_secs(5)));
    assert!(gone_within(job, Duration::from_secs(5)));
}

/// `PR_SET_PDEATHSIG` follows the spawning thread, not the process: a child spawned from a
/// thread that then ends dies with it. That is why only a spawn whose thread outlives the child
/// (the worktree setup command's) uses it.
#[cfg(target_os = "linux")]
#[test]
fn death_signal_follows_the_spawning_thread() {
    let spawn = || {
        let mut command = Command::new("sleep");
        command.arg("120");
        children::die_with_spawning_thread(&mut command);
        command.spawn().unwrap()
    };
    let short = std::thread::spawn(move || {
        let child = spawn();
        let pid = child.id();
        std::mem::forget(child);
        pid
    })
    .join()
    .unwrap();
    assert!(
        gone_within(short, Duration::from_secs(5)),
        "the child of an ended thread lives"
    );
    let mut kept = spawn();
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(kept.id()), "the child of a living thread died");
    let _ = kept.kill();
    let _ = kept.wait();
}
