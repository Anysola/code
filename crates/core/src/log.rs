//! The shared log file `~/.anysola/log/anysola.log`: logfmt lines (`timestamp`, `level`, `run`,
//! `message`, fields, `role`) appended by every process. Above 50 MiB it is cut to its last ~25 MiB
//! when a process starts.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const TRIM_ABOVE: u64 = 50 * 1024 * 1024;
const TRIM_TO: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
    Off,
}

impl Level {
    /// `--log-level` choices and `ANYSOLA_LOG_LEVEL` values.
    pub fn parse(text: &str) -> Option<Level> {
        Some(match text.to_lowercase().as_str() {
            "all" | "trace" | "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" | "warning" => Level::Warn,
            "error" | "fatal" => Level::Error,
            "none" => Level::Off,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error | Level::Off => "ERROR",
        }
    }
}

struct Logger {
    file: PathBuf,
    level: Level,
    print: bool,
    run: String,
    lock: Mutex<()>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

fn trim(file: &PathBuf) {
    let Ok(meta) = std::fs::metadata(file) else {
        return;
    };
    if meta.len() <= TRIM_ABOVE {
        return;
    }
    let Ok(bytes) = std::fs::read(file) else {
        return;
    };
    let start = bytes.len() - TRIM_TO as usize;
    let start = bytes[start..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(start, |p| start + p + 1);
    let _ = std::fs::write(file, &bytes[start..]);
}

/// Sets up logging for this process; `level` and `print` come from flags, else the environment.
pub fn init(paths: &crate::paths::Paths, level: Option<&str>, print: bool) {
    let level = level
        .and_then(Level::parse)
        .or_else(|| {
            crate::env::var("LOG_LEVEL")
                .as_deref()
                .and_then(Level::parse)
        })
        .unwrap_or(Level::Info);
    let print = print || crate::env::var("PRINT_LOGS").is_some_and(|v| v == "1" || v == "true");
    let file = paths.log_file();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    trim(&file);
    let run = format!("{:08x}", rand::random::<u32>());
    let _ = LOGGER.set(Logger {
        file,
        level,
        print,
        run,
        lock: Mutex::new(()),
    });
}

fn value(text: &str) -> String {
    if text.is_empty() || text.contains([' ', '=', '"', '\\', '\n']) {
        serde_json::to_string(text).unwrap_or_default()
    } else {
        text.to_string()
    }
}

pub fn write(level: Level, message: &str, fields: &[(&str, &str)]) {
    let Some(logger) = LOGGER.get() else { return };
    if level < logger.level || logger.level == Level::Off {
        return;
    }
    let mut line = format!(
        "timestamp={} level={} run={} message={}",
        crate::time::iso(crate::time::now_ms()),
        level.label(),
        logger.run,
        value(message)
    );
    for (key, text) in fields {
        line.push_str(&format!(" {key}={}", value(text)));
    }
    line.push_str(" role=cli\n");
    let _guard = logger.lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&logger.file)
    {
        let _ = file.write_all(line.as_bytes());
    }
    if logger.print {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

pub fn info(message: &str, fields: &[(&str, &str)]) {
    write(Level::Info, message, fields);
}

pub fn warn(message: &str, fields: &[(&str, &str)]) {
    write(Level::Warn, message, fields);
}

pub fn error(message: &str, fields: &[(&str, &str)]) {
    write(Level::Error, message, fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logfmt_values() {
        assert_eq!(value("plain"), "plain");
        assert_eq!(value("two words"), "\"two words\"");
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert!(Level::parse("loud").is_none());
    }

    #[test]
    fn trims_to_whole_lines_above_50_mib() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("anysola.log");
        let line = format!("{}\n", "x".repeat(99));
        let lines = (51 * 1024 * 1024) / line.len() + 1;
        std::fs::write(&file, line.repeat(lines)).unwrap();
        trim(&file);
        let kept = std::fs::read(&file).unwrap();
        assert!(kept.len() as u64 <= TRIM_TO);
        assert!(kept.len() as u64 >= TRIM_TO - 100);
        assert_eq!(kept.len() % line.len(), 0, "cut at a line start");
        std::fs::write(&file, "small\n").unwrap();
        trim(&file);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "small\n");
    }
}
