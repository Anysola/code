//! Where 0.5.x kept its files, resolved the way 0.5.1 resolved them: XDG variables with home
//! fallbacks and no platform branch, app name `opencode`, release channel `latest`.

use std::env;
use std::path::{Path, PathBuf};

use anysola_core::paths::Paths;
use rusqlite::{Connection, OpenFlags};

pub struct Source {
    /// `$XDG_DATA_HOME/opencode` or `~/.local/share/opencode`.
    pub data: PathBuf,
    /// `$OPENCODE_CONFIG_DIR`, `$XDG_CONFIG_HOME/opencode` or `~/.config/opencode`.
    pub config: PathBuf,
    /// `$XDG_STATE_HOME/opencode` or `~/.local/state/opencode`.
    pub state: PathBuf,
    /// `<data>/opencode.db`, or `OPENCODE_DB` (relative to `<data>`); `None` for `:memory:`.
    pub database: Option<PathBuf>,
}

fn root(variable: &str, home: &Path, fallback: &[&str]) -> PathBuf {
    match env::var_os(variable).filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("opencode"),
        None => fallback
            .iter()
            .fold(home.to_path_buf(), |p, part| p.join(part))
            .join("opencode"),
    }
}

impl Source {
    pub fn locate(paths: &Paths) -> Source {
        let data = root("XDG_DATA_HOME", &paths.home, &[".local", "share"]);
        let database = match env::var("OPENCODE_DB") {
            Ok(name) if name == ":memory:" => None,
            Ok(name) if !name.is_empty() => Some(data.join(name)),
            _ => Some(data.join("opencode.db")),
        };
        Source {
            config: paths.legacy_config_dir(),
            state: root("XDG_STATE_HOME", &paths.home, &[".local", "state"]),
            database: database.filter(|path| path.is_file()),
            data,
        }
    }
}

/// A private copy of the 0.5.x database. Even a `mode=ro` open creates `-wal`/`-shm` next to a WAL
/// database, so the file and its log are copied and the copy is opened instead.
pub struct Snapshot {
    pub conn: Connection,
    // Dropped after the connection.
    _dir: tempfile::TempDir,
}

impl Snapshot {
    pub fn take(database: &Path) -> Result<Snapshot, crate::Error> {
        let dir = tempfile::tempdir()?;
        let copy = dir.path().join("opencode.db");
        std::fs::copy(database, &copy)?;
        let wal = PathBuf::from(format!("{}-wal", database.display()));
        if wal.is_file() {
            std::fs::copy(&wal, dir.path().join("opencode.db-wal"))?;
        }
        let conn = Connection::open_with_flags(
            &copy,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(crate::Error::Corrupt(check));
        }
        Ok(Snapshot { conn, _dir: dir })
    }
}
