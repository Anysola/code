//! Executable lookup on PATH.

use std::env;
use std::path::{Path, PathBuf};

/// PATH lookup of a bare command name (with PATHEXT on Windows).
pub fn which(command: &str) -> Option<PathBuf> {
    which_in(command, None)
}

/// [`which`] over PATH and then `extra`.
pub fn which_in(command: &str, extra: Option<&Path>) -> Option<PathBuf> {
    let path = env::var_os("PATH").or_else(|| env::var_os("Path"));
    let mut dirs: Vec<PathBuf> = path.iter().flat_map(env::split_paths).collect();
    dirs.extend(extra.map(Path::to_path_buf));
    let extensions: Vec<String> = if cfg!(windows) {
        let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".into());
        // A bare name is never run as is: npm's bin folder keeps an extensionless sh script beside
        // each `.cmd` shim, and Windows cannot start it.
        let bare = Path::new(command).extension().is_some();
        bare.then(String::new)
            .into_iter()
            .chain(
                pathext
                    .split(';')
                    .filter(|e| !e.is_empty())
                    .map(str::to_string),
            )
            .collect()
    } else {
        vec![String::new()]
    };
    dirs.into_iter()
        .flat_map(|dir| {
            extensions
                .iter()
                .map(move |ext| dir.join(format!("{command}{ext}")))
        })
        .find(|candidate| is_executable(candidate))
}

/// The program to start for `name`. On Windows a bare name is looked up with PATHEXT first, so
/// `.cmd` shims such as `npx`, `npm` or `code` start (the standard library only tries `.exe`).
pub fn program(name: &str) -> std::ffi::OsString {
    if cfg!(windows)
        && !name.contains(['/', '\\'])
        && let Some(found) = which(name)
    {
        return found.into_os_string();
    }
    name.into()
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}
