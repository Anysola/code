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
        std::iter::once(String::new())
            .chain(pathext.split(';').map(str::to_string))
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
