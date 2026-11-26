//! Children of ours, and what becomes of them when the app dies without stopping them (a crash,
//! `kill -9`, the Windows updater's exit).
//!
//! Every program the app starts on its own account and expects to stop with it goes through
//! here, right after it was spawned: the shell tool, hooks, ``!`cmd` `` placeholders, schedule
//! checks, `!` commands, formatters, MCP stdio servers, LSP servers, a worktree's setup command,
//! the embedded terminal's shell (Linux only, see [`adopt_session`]). What must outlive the app
//! never does: the updater's installer, "Open in editor", the browser `opener` opens, `reveal`,
//! the restarted app. The app's own process is never put in a Job Object: whatever it starts
//! itself would inherit it, the installer first.
//!
//! - Windows: [`adopt`] puts the child in one app-wide Job Object that kills what it holds when
//!   its last handle closes, which the system does when the app's process ends, however it ends.
//!   A child's children are in it too unless created with `CREATE_BREAKAWAY_FROM_JOB` (allowed:
//!   a program that asks for that means to outlive us, and without the limit its start would
//!   fail). [`Tree`] gives one child and its descendants a Job of their own, so they can be
//!   stopped together while the app goes on, even after the child itself has ended. The Job
//!   acts on every exit, a normal one too: something a command or hook left running on purpose
//!   (a browser `start` opened, `code .`) ends with the app, as the brief for 1.4.3 chose.
//! - Linux: a child that leads a process group of its own is written down under the data
//!   directory ([`keep_records`]); the next start's [`sweep`] kills what is left of each group
//!   whose app is gone. A process is only killed when its group (or session) matches, it
//!   started no earlier than the group's leader, and its environment carries that child's own
//!   [`MARKER`] token; anything it can't verify is left alone. A record whose app still runs
//!   (another copy, a test, a development build sharing the folder) is never touched.
//!
//! Every function exists on every platform with the same signature; what a platform doesn't need
//! does nothing there.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The environment variable every child of ours carries: a token of that child's own.
pub const MARKER: &str = "ANYSOLA_CHILD_OF";

/// A child's token (random, one per child: a group id reused by a later child of the same run
/// must not pass for the earlier one's). Never in the app's own environment.
pub struct Mark(String);

impl Mark {
    pub fn new() -> Mark {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        Mark(
            (0..32)
                .map(|_| char::from(b"0123456789abcdefghijklmnopqrstuvwxyz"[rng.gen_range(0..36)]))
                .collect(),
        )
    }

    /// The value for [`MARKER`] (for a command built without std or tokio, a PTY's).
    pub fn value(&self) -> &str {
        &self.0
    }
}

impl Default for Mark {
    fn default() -> Mark {
        Mark::new()
    }
}

/// Gives the command a new [`MARKER`]; hand what it returns to `adopt`/`tree`. Call it after
/// the command's `env_clear`/`envs`.
pub fn mark(command: &mut std::process::Command) -> Mark {
    let mark = Mark::new();
    command.env(MARKER, mark.value());
    mark
}

/// [`mark`] for a tokio command.
pub fn mark_tokio(command: &mut tokio::process::Command) -> Mark {
    let mark = Mark::new();
    command.env(MARKER, mark.value());
    mark
}

/// What the child leads, for the Linux record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leads {
    /// A process group of its own (`process_group(0)`): what is left of it is swept.
    Group,
    /// Nothing of its own (it runs in the app's group): no record; on Windows the Job still.
    Nothing,
}

static RECORDS: OnceLock<PathBuf> = OnceLock::new();

/// Where this run writes its records (Linux): the data directory's, so an isolated home keeps
/// its own. Without it nothing is written. The first call wins.
pub fn keep_records(dir: PathBuf) {
    let _ = RECORDS.set(dir);
}

/// A child of ours, joined (see the module). Dropping it removes its record; it stops nothing.
#[must_use = "dropping it removes the child's record"]
pub struct Ours {
    #[cfg(target_os = "linux")]
    record: Option<PathBuf>,
}

impl Drop for Ours {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(path) = self.record.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Joins a child just spawned (std). Call it before the child is waited for.
pub fn adopt(child: &std::process::Child, leads: Leads, mark: Mark) -> Ours {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        job::join_app(child.as_raw_handle());
    }
    joined(Some(child.id()), leads, mark)
}

/// [`adopt`] for a tokio child.
pub fn adopt_tokio(child: &tokio::process::Child, leads: Leads, mark: Mark) -> Ours {
    #[cfg(windows)]
    if let Some(handle) = child.raw_handle() {
        job::join_app(handle);
    }
    joined(child.id(), leads, mark)
}

/// The embedded terminal's shell (it leads a session of its own on Unix). Linux: recorded, so
/// what of the session survives a crash goes at the next start, as closing the terminal would
/// have ended it. `mark` is the one the shell's environment got. Windows: nothing; closing the
/// pseudo console ends what is attached to it, and like any terminal window it leaves alone a
/// program that left it (a GUI app, `code .`, `start`). That the app's death does the same
/// (the console host going once its pipes break) is assumed, not tested on Windows here.
pub fn adopt_session(pid: Option<u32>, mark: Mark) -> Ours {
    #[cfg(target_os = "linux")]
    {
        Ours {
            record: pid.and_then(|pid| linux::record(pid, linux::Kind::Session, &mark.0)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, mark);
        Ours {}
    }
}

fn joined(pid: Option<u32>, leads: Leads, mark: Mark) -> Ours {
    #[cfg(target_os = "linux")]
    {
        Ours {
            record: pid
                .filter(|_| leads == Leads::Group)
                .and_then(|pid| linux::record(pid, linux::Kind::Group, &mark.0)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, leads, mark);
        Ours {}
    }
}

/// One child and everything it starts, stopped together by [`Tree::kill`] or by dropping it,
/// also after the child itself ended (a setup command's `npm run dev &`; an npm `.cmd` shim's
/// node; rustup's proxy). Windows: a Job of its own. Linux: the process group the child leads
/// (spawn it with `process_group(0)` and [`mark`]), verified as the sweep does, and recorded.
#[must_use = "dropping it stops the child and what it started"]
pub struct Tree {
    #[cfg(windows)]
    job: Option<job::Job>,
    #[cfg(target_os = "linux")]
    group: Option<linux::Group>,
    #[cfg(target_os = "linux")]
    record: Option<PathBuf>,
    #[cfg(all(unix, not(target_os = "linux")))]
    pid: Option<u32>,
}

/// A [`Tree`] prepared before spawning the child (Windows: the Job Object is created ahead of
/// time so `AssignProcessToJobObject` runs with the smallest gap after `spawn()`).
pub struct PreparedTree {
    #[cfg(windows)]
    job: Option<job::Job>,
}

/// Prepares a [`Tree`] before spawning its child; hand the spawned child to [`PreparedTree::attach`]
/// or [`PreparedTree::attach_tokio`].
pub fn prepare_tree() -> PreparedTree {
    PreparedTree {
        #[cfg(windows)]
        job: job::Job::new(),
    }
}

impl PreparedTree {
    /// Attaches a child just spawned (std).
    pub fn attach(self, child: &std::process::Child, mark: Mark) -> Tree {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let _ = mark;
            Tree {
                job: self.job.filter(|job| job.assign(child.as_raw_handle())),
            }
        }
        #[cfg(not(windows))]
        tree_of(Some(child.id()), mark)
    }

    /// Attaches a child just spawned (tokio).
    pub fn attach_tokio(self, child: &tokio::process::Child, mark: Mark) -> Tree {
        #[cfg(windows)]
        {
            let _ = mark;
            Tree {
                job: self
                    .job
                    .filter(|job| child.raw_handle().is_some_and(|h| job.assign(h))),
            }
        }
        #[cfg(not(windows))]
        tree_of(child.id(), mark)
    }
}

/// A [`Tree`] for a child just spawned (std).
pub fn tree(child: &std::process::Child, mark: Mark) -> Tree {
    prepare_tree().attach(child, mark)
}

/// [`tree`] for a tokio child.
pub fn tree_tokio(child: &tokio::process::Child, mark: Mark) -> Tree {
    prepare_tree().attach_tokio(child, mark)
}

#[cfg(not(windows))]
fn tree_of(pid: Option<u32>, mark: Mark) -> Tree {
    #[cfg(target_os = "linux")]
    {
        Tree {
            group: pid.and_then(|pid| linux::Group::of(pid, &mark.0)),
            record: pid.and_then(|pid| linux::record(pid, linux::Kind::Group, &mark.0)),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = mark;
        Tree { pid }
    }
}

impl Tree {
    /// Kills the child and what it started. Whether it could act (false: no Job on Windows; the
    /// caller falls back to what it did before).
    pub fn kill(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.as_ref().is_some_and(job::Job::terminate)
        }
        #[cfg(target_os = "linux")]
        {
            self.group.as_ref().is_some_and(|group| {
                linux::kill_group(group);
                true
            })
        }
        #[cfg(all(unix, not(target_os = "linux")))]
        {
            // No /proc to verify against: the group while its leader may still be ours.
            self.pid.is_some_and(|pid| {
                crate::tool::shell::exec::force_kill(pid);
                true
            })
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // Windows: closing the Job's handle kills what it holds.
        #[cfg(target_os = "linux")]
        {
            if let Some(group) = self.group.take() {
                linux::kill_group(&group);
            }
            if let Some(path) = self.record.take() {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Makes the child get SIGKILL when the thread spawning it ends (Linux `PR_SET_PDEATHSIG`; the
/// kernel ties it to that thread, not to the process). Only for a spawn whose thread outlives
/// the child, e.g. one that waits for it: from a short-lived thread (tokio's blocking pool, a
/// runtime made for one call) the child would die early. It reaches the child alone, not what
/// the child starts. Nothing elsewhere.
pub fn die_with_spawning_thread(command: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: getpid(2) has no preconditions.
        let parent = unsafe { libc::getpid() };
        // SAFETY: the closure runs between fork and exec and only makes async-signal-safe calls.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have gone before the line above: then nobody would signal us.
                if libc::getppid() != parent {
                    return Err(std::io::Error::other("the app ended while starting it"));
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

/// Kills what the records in `dir` name and their apps left behind (Linux; see the module), and
/// removes those records. A record whose app still runs is kept. Returns the processes killed.
pub fn sweep(dir: &Path) -> usize {
    #[cfg(target_os = "linux")]
    {
        linux::sweep(dir)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        0
    }
}

/// Whether `process` is in the app-wide Job (Windows; false elsewhere). For tests: the app's own
/// process must never be.
#[doc(hidden)]
pub fn in_app_job(process: *mut std::ffi::c_void) -> bool {
    #[cfg(windows)]
    {
        job::in_app(process)
    }
    #[cfg(not(windows))]
    {
        let _ = process;
        false
    }
}

#[cfg(windows)]
mod job {
    //! The Windows Job Objects. Handles are created without inheritance (null security
    //! attributes): a child holding one would keep its Job open after the app died.

    use std::ffi::c_void;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
    };

    pub struct Job(HANDLE);

    // SAFETY: a Job handle is a kernel handle usable from any thread.
    unsafe impl Send for Job {}
    // SAFETY: the calls made with it (assign, terminate, query) are thread-safe in the kernel.
    unsafe impl Sync for Job {}

    impl Job {
        /// A Job that kills what it holds when its last handle closes. `BREAKAWAY_OK`: a
        /// program creating a process with `CREATE_BREAKAWAY_FROM_JOB` would otherwise fail
        /// with access denied; one that asks means its process to outlive us.
        pub fn new() -> Option<Job> {
            // SAFETY: null attributes and name: an unnamed Job with a non-inheritable handle.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            let job = Job(handle);
            // SAFETY: a zeroed limit structure is valid; only the flags are set.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            // SAFETY: the pointer and size describe `limits`, alive for the call.
            let set = unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast::<c_void>(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            // Without kill-on-close the Job would be useless: never use one like that.
            (set != FALSE).then_some(job)
        }

        pub fn assign(&self, process: HANDLE) -> bool {
            // SAFETY: both handles are valid for the call; the child's comes from its spawn.
            unsafe { AssignProcessToJobObject(self.0, process) != FALSE }
        }

        pub fn terminate(&self) -> bool {
            // SAFETY: a valid Job handle.
            unsafe { TerminateJobObject(self.0, 1) != FALSE }
        }

        fn contains(&self, process: HANDLE) -> bool {
            let mut result = FALSE;
            // SAFETY: valid handles and an out pointer alive for the call.
            let ok = unsafe { IsProcessInJob(process, self.0, &mut result) };
            ok != FALSE && result != FALSE
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle is ours and closed once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// The app-wide Job, made on first use. `None` if Windows wouldn't make one.
    fn app() -> Option<&'static Job> {
        static APP: OnceLock<Option<Job>> = OnceLock::new();
        APP.get_or_init(|| {
            let job = Job::new();
            if job.is_none() {
                let error = std::io::Error::last_os_error().to_string();
                crate::log::warn(
                    "no Job Object: children may outlive a crash",
                    &[("error", &error)],
                );
            }
            job
        })
        .as_ref()
    }

    pub fn join_app(process: HANDLE) {
        let Some(job) = app() else {
            return;
        };
        if !job.assign(process) {
            let error = std::io::Error::last_os_error().to_string();
            crate::log::warn("a child couldn't join the Job Object", &[("error", &error)]);
        }
    }

    pub fn in_app(process: HANDLE) -> bool {
        app().is_some_and(|job| job.contains(process))
    }
}

#[cfg(target_os = "linux")]
mod linux {
    //! Records and the verified kill, from `/proc`.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Kind {
        Group,
        Session,
    }

    /// A process group (or session) of ours: its id, its leader's start time, the token its
    /// members carry, the boot it belongs to.
    #[derive(Debug, PartialEq, Eq)]
    pub struct Group {
        pub kind: Kind,
        pub id: i32,
        pub start: u64,
        pub token: String,
        pub boot: String,
    }

    impl Group {
        /// The group `pid` leads, now (it must be alive, or an unreaped zombie).
        pub fn of(pid: u32, token: &str) -> Option<Group> {
            Group::led_by(pid, Kind::Group, token)
        }

        fn led_by(pid: u32, kind: Kind, token: &str) -> Option<Group> {
            let id = i32::try_from(pid).ok()?;
            Some(Group {
                kind,
                id,
                start: stat(id)?.start,
                token: token.to_string(),
                boot: boot_id()?,
            })
        }
    }

    pub struct Stat {
        pub zombie: bool,
        pub group: i32,
        pub session: i32,
        pub start: u64,
    }

    /// From `/proc/<pid>/stat`: after the command name in parentheses come state, ppid, pgrp,
    /// session, and the start time is the 20th field after it.
    pub fn stat(pid: i32) -> Option<Stat> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (_, rest) = text.rsplit_once(')')?;
        let fields: Vec<&str> = rest.split_whitespace().collect();
        Some(Stat {
            zombie: fields.first().is_some_and(|s| *s == "Z"),
            group: fields.get(2)?.parse().ok()?,
            session: fields.get(3)?.parse().ok()?,
            start: fields.get(19)?.parse().ok()?,
        })
    }

    fn boot_id() -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Whether the process's environment (as it was exec'd) holds `MARKER=token`. Unreadable
    /// (not ours, a zombie, gone): no.
    fn carries(pid: i32, token: &str) -> bool {
        let Ok(environ) = std::fs::read(format!("/proc/{pid}/environ")) else {
            return false;
        };
        let wanted = format!("{}={token}", super::MARKER);
        environ
            .split(|b| *b == 0)
            .any(|entry| entry == wanted.as_bytes())
    }

    fn verified(group: &Group, pid: i32) -> bool {
        let Some(stat) = stat(pid) else {
            return false;
        };
        let member = match group.kind {
            Kind::Group => stat.group == group.id,
            Kind::Session => stat.session == group.id,
        };
        member && stat.start >= group.start && carries(pid, &group.token)
    }

    fn pids() -> Vec<i32> {
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
            .collect()
    }

    /// SIGKILL to every process of the group that passes [`verified`]: each by its own pid
    /// (a pidfd taken before the check where the kernel has them), never the group's id. A few
    /// rounds, for what forked meanwhile. Returns how many were signalled.
    pub fn kill_group(group: &Group) -> usize {
        if boot_id().as_deref() != Some(group.boot.as_str()) {
            return 0;
        }
        // SAFETY: getpid(2) has no preconditions.
        let me = unsafe { libc::getpid() };
        let mut signalled = std::collections::HashSet::new();
        for _ in 0..3 {
            let mut round = 0;
            let candidates: Vec<i32> = pids()
                .into_iter()
                .filter(|pid| *pid != me && !signalled.contains(pid))
                .collect();
            for pid in candidates {
                // SAFETY: pidfd_open(2) with a pid and no flags; the descriptor is closed below.
                let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
                if !verified(group, pid) {
                    if fd >= 0 {
                        // SAFETY: closing the descriptor just opened.
                        unsafe { libc::close(fd) };
                    }
                    continue;
                }
                // With a pidfd the signal can only reach the process just checked (its pid
                // isn't reused while the descriptor is open); without one the window between
                // the check and kill(2) is the pid being freed and handed out again.
                let sent = if fd >= 0 {
                    // SAFETY: pidfd_send_signal(2) on our descriptor, no info, no flags.
                    let r = unsafe {
                        libc::syscall(
                            libc::SYS_pidfd_send_signal,
                            fd,
                            libc::SIGKILL,
                            std::ptr::null::<libc::siginfo_t>(),
                            0,
                        )
                    };
                    // SAFETY: closing our descriptor.
                    unsafe { libc::close(fd) };
                    r == 0
                } else {
                    // SAFETY: kill(2) with a verified pid and a signal number.
                    unsafe { libc::kill(pid, libc::SIGKILL) == 0 }
                };
                if sent {
                    signalled.insert(pid);
                    round += 1;
                }
            }
            if round == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        signalled.len()
    }

    /// This process and its start, as a record names its app.
    fn me() -> Option<(i32, u64)> {
        // SAFETY: getpid(2) has no preconditions.
        let pid = unsafe { libc::getpid() };
        Some((pid, stat(pid)?.start))
    }

    /// Writes the record of what `pid` leads, if this run keeps records.
    pub fn record(pid: u32, kind: Kind, token: &str) -> Option<PathBuf> {
        let dir = super::RECORDS.get()?;
        record_in(dir, pid, kind, token)
    }

    fn record_in(dir: &Path, pid: u32, kind: Kind, token: &str) -> Option<PathBuf> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let group = Group::led_by(pid, kind, token)?;
        let (owner, owner_start) = me()?;
        std::fs::create_dir_all(dir).ok()?;
        let name = format!(
            "{owner}-{}-{}",
            group.id,
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let text = format!(
            "boot {}\nowner {owner} {owner_start}\nkind {}\nid {}\nstart {}\ntoken {}\n",
            group.boot,
            match kind {
                Kind::Group => "group",
                Kind::Session => "session",
            },
            group.id,
            group.start,
            group.token,
        );
        let path = dir.join(&name);
        let partial = dir.join(format!(".{name}"));
        std::fs::write(&partial, text).ok()?;
        std::fs::rename(&partial, &path).ok()?;
        Some(path)
    }

    struct Record {
        owner: i32,
        owner_start: u64,
        group: Group,
    }

    fn parse(text: &str) -> Option<Record> {
        let mut fields = std::collections::HashMap::new();
        for line in text.lines() {
            let (key, value) = line.split_once(' ')?;
            fields.insert(key, value);
        }
        let (owner, owner_start) = fields.get("owner")?.split_once(' ')?;
        let token = fields.get("token")?.to_string();
        // A record can only name a token a run of ours makes.
        if token.len() < 16 || !token.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        let id: i32 = fields.get("id")?.parse().ok()?;
        if id <= 1 {
            return None;
        }
        Some(Record {
            owner: owner.parse().ok()?,
            owner_start: owner_start.parse().ok()?,
            group: Group {
                kind: match *fields.get("kind")? {
                    "group" => Kind::Group,
                    "session" => Kind::Session,
                    _ => return None,
                },
                id,
                start: fields.get("start")?.parse().ok()?,
                token,
                boot: fields.get("boot")?.to_string(),
            },
        })
    }

    pub fn sweep(dir: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let boot = boot_id();
        let mut killed = 0;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let name = entry.file_name();
            // A record being written (or one cut off while it was).
            if name.to_string_lossy().starts_with('.') {
                if entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t.elapsed().is_ok_and(|age| age.as_secs() > 60))
                {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            let Some(record) = std::fs::read_to_string(&path)
                .ok()
                .as_deref()
                .and_then(parse)
            else {
                // Unreadable: nothing is killed for it.
                let _ = std::fs::remove_file(&path);
                continue;
            };
            if boot.as_deref() == Some(record.group.boot.as_str()) {
                let owner_runs =
                    stat(record.owner).is_some_and(|s| !s.zombie && s.start == record.owner_start);
                if owner_runs {
                    continue;
                }
                killed += kill_group(&record.group);
            }
            // From another boot nothing of it runs.
            let _ = std::fs::remove_file(&path);
        }
        killed
    }
}
