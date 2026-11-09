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
    /// The file's bytes are not UTF-8 (Latin-1, Shift-JIS and the like): decoding them lossily
    /// would replace every such byte on the way back.
    NotUtf8,
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

/// Reads a file (following symlinks) as UTF-8 without its leading BOMs.
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
    let decoded = String::from_utf8(bytes).map_err(|_| ReadFailure::NotUtf8)?;
    let (bom, text) = split_bom(&decoded);
    Ok(TextFile {
        bom,
        text: text.to_string(),
    })
}

/// Writes `content`, creating missing parent directories. The bytes go to a temporary file next
/// to the target that then replaces it, so an interrupted write never leaves a truncated file; a
/// symlink keeps pointing at the replaced file and that file keeps its permissions.
pub(crate) async fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let target = match tokio::fs::canonicalize(path).await {
        Ok(target) => target,
        Err(error) if is_missing(&error) => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    let content = content.to_string();
    tokio::task::spawn_blocking(move || replace(&target, content.as_bytes()))
        .await
        .map_err(std::io::Error::other)?
}

fn replace(target: &Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let permissions = match std::fs::metadata(target) {
        Ok(meta) => Some(meta.permissions()),
        Err(error) if is_missing(&error) => None,
        Err(error) => return Err(error),
    };
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(
        ".{name}.{}.{:08x}.tmp",
        std::process::id(),
        rand::random::<u32>()
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        drop(file);
        std::fs::rename(&temporary, target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Runs the configured formatters on a written file. When one succeeded, the file's BOM state is
/// restored; returns the file's text (without BOM) as it now is.
pub(crate) async fn format(
    ctx: &super::super::ToolContext,
    path: &Path,
    bom: bool,
) -> Option<String> {
    let formatters = ctx.host.formatters()?;
    if !formatters.format(path, &ctx.directory, &ctx.worktree).await {
        return None;
    }
    let file = read_text(path).await.ok()?;
    if file.bom != bom {
        let _ = write_file(path, &join_bom(&file.text, bom)).await;
    }
    Some(file.text)
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
