//! stderr presentation shared by commands: styles, error lines and blank-line framing.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub const DIM: &str = "\x1b[90m";
pub const NORMAL: &str = "\x1b[0m";
pub const WARNING_BOLD: &str = "\x1b[93m\x1b[1m";
pub const DANGER_BOLD: &str = "\x1b[91m\x1b[1m";

static BLANK: AtomicBool = AtomicBool::new(false);

pub fn println(text: &str) {
    BLANK.store(false, Ordering::Relaxed);
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{text}");
}

/// One blank line, unless the previous stderr line already was one.
pub fn empty() {
    if BLANK.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{NORMAL}");
}

pub fn error(message: &str) {
    let message = message.strip_prefix("Error: ").unwrap_or(message);
    println(&format!("{DANGER_BOLD}Error: {NORMAL}{message}"));
}
