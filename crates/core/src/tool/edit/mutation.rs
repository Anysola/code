//! Text file reads and writes shared by edit and write: UTF-8 BOM handling and the lock key.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const BOM: char = '\u{FEFF}';

/// A file's text with its leading BOMs split off.
pub(crate) struct TextFile {
    pub bom: bool,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadFailure {
    Missing,
    Directory,
    /// Any other failure, including paths that are neither files nor directories.
    Failed,
}

/// `text` without its leading BOMs, and whether it had any.
pub(crate) fn split_bom(text: &str) -> (bool, &str) {
    let stripped = text.trim_start_matches(BOM);
    (stripped.len() != text.len(), stripped)
}

/// `text` with exactly one leading BOM when `bom`, none otherwise.
pub(crate) fn join_bom(text: &str, bom: bool) -> String {
    let (_, stripped) = split_bom(text);
    if bom {
        format!("{BOM}{stripped}")
    } else {
        stripped.to_string()
    }
}

pub(crate) fn is_missing(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
}

/// Reads a file (following symlinks) as lossy UTF-8 without its leading BOMs.
pub(crate) async fn read_text(path: &Path) -> Result<TextFile, ReadFailure> {
    let failure = |error: std::io::Error| {
        if is_missing(&error) {
            ReadFailure::Missing
        } else {
            ReadFailure::Failed
        }
    };
    let meta = tokio::fs::metadata(path).await.map_err(failure)?;
    if meta.is_dir() {
        return Err(ReadFailure::Directory);
    }
    if !meta.is_file() {
        return Err(ReadFailure::Failed);
    }
    let bytes = tokio::fs::read(path).await.map_err(failure)?;
    let decoded = String::from_utf8_lossy(&bytes);
    let (bom, text) = split_bom(&decoded);
    Ok(TextFile {
        bom,
        text: text.to_string(),
    })
}

/// Writes `content`, creating missing parent directories.
pub(crate) async fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, content).await
}

/// Mutations of one file serialize on its canonical path (the path itself when it does not exist).
pub(crate) async fn lock_key(path: &Path) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_split_and_join() {
        assert_eq!(split_bom("\u{FEFF}\u{FEFF}a"), (true, "a"));
        assert_eq!(split_bom("a\u{FEFF}"), (false, "a\u{FEFF}"));
        assert_eq!(join_bom("\u{FEFF}\u{FEFF}a", true), "\u{FEFF}a");
        assert_eq!(join_bom("\u{FEFF}a", false), "a");
        assert_eq!(join_bom("a", true), "\u{FEFF}a");
    }
}
