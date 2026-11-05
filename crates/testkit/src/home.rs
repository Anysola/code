//! An isolated home directory and environment for running the `anysola` binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct TestHome {
    _dir: tempfile::TempDir,
    root: PathBuf,
    env: Vec<(String, String)>,
}

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Variables a child keeps from the test's environment. Windows cannot open sockets without
/// `SystemRoot`, and resolves programs and temp files through the others.
const INHERITED: &[&str] = if cfg!(windows) {
    &[
        "PATH",
        "PATHEXT",
        "SystemRoot",
        "SystemDrive",
        "windir",
        "ComSpec",
        "TEMP",
        "TMP",
    ]
} else {
    &["PATH"]
};

/// The real path of the temp dir, so it matches what a child sees as its working directory
/// (macOS temp dirs live under the `/var` -> `/private/var` symlink). Windows keeps the
/// path as is: canonical paths there carry a `\\?\` prefix nothing else uses.
fn canonical(path: &Path) -> PathBuf {
    if cfg!(windows) {
        path.to_path_buf()
    } else {
        path.canonicalize().expect("canonical temp dir")
    }
}

impl TestHome {
    /// A home with a `proj` directory, pointed at the gateway with a test token.
    pub fn new(gateway_url: &str) -> TestHome {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = canonical(dir.path());
        std::fs::create_dir_all(root.join("home/proj")).expect("project dir");
        let home = root.join("home");
        let env = vec![
            ("HOME".into(), home.display().to_string()),
            ("USERPROFILE".into(), home.display().to_string()),
            (
                "XDG_CONFIG_HOME".into(),
                home.join(".config").display().to_string(),
            ),
            (
                "XDG_DATA_HOME".into(),
                home.join(".local/share").display().to_string(),
            ),
            (
                "XDG_STATE_HOME".into(),
                home.join(".local/state").display().to_string(),
            ),
            (
                "XDG_CACHE_HOME".into(),
                home.join(".cache").display().to_string(),
            ),
            ("ANYSOLA_API_URL".into(), gateway_url.to_string()),
            ("ANYSOLA_TOKEN".into(), format!("asc_{}", "T".repeat(40))),
            ("SHELL".into(), "/bin/bash".into()),
        ];
        TestHome {
            _dir: dir,
            root,
            env,
        }
    }

    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    pub fn project(&self) -> PathBuf {
        self.home().join("proj")
    }

    /// The variables a run gets, for driving the core in-process.
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    pub fn set_env(&mut self, name: &str, value: &str) {
        self.env.retain(|(n, _)| n != name);
        self.env.push((name.into(), value.into()));
    }

    pub fn remove_env(&mut self, name: &str) {
        self.env.retain(|(n, _)| n != name);
    }

    pub fn write(&self, relative: &str, content: &str) -> PathBuf {
        let path = self.home().join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, content).expect("write");
        path
    }

    /// Runs `binary args…` in `cwd` (default: the project) with `stdin` (default: `/dev/null`).
    pub fn run(
        &self,
        binary: &str,
        args: &[&str],
        cwd: Option<&Path>,
        stdin: Option<&str>,
    ) -> Output {
        let mut command = Command::new(binary);
        command
            .args(args)
            .current_dir(cwd.map(Path::to_path_buf).unwrap_or_else(|| self.project()));
        command.env_clear();
        for name in INHERITED {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command.envs(self.env.iter().map(|(k, v)| (k, v)));
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        command.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = command.spawn().expect("spawn anysola");
        if let Some(text) = stdin {
            let mut pipe = child.stdin.take().expect("stdin pipe");
            pipe.write_all(text.as_bytes()).expect("write stdin");
        }
        let output = child.wait_with_output().expect("wait");
        Output {
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            code: output.status.code().unwrap_or(-1),
        }
    }
}
