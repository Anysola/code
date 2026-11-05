//! System clipboard reads through the platform's paste tool: `image/png` first, then text.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(3);
/// Larger clipboard payloads are refused, as 0.5.1's configured read limit does.
const LIMIT: usize = 20 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum Content {
    Image(Vec<u8>),
    Text(String),
}

/// Reads the clipboard; `Ok(None)` when it is empty or holds nothing readable.
pub fn read() -> Result<Option<Content>, String> {
    for (program, args) in image_tools() {
        if let Some(bytes) = run(program, args)?
            && bytes.starts_with(b"\x89PNG")
        {
            return Ok(Some(Content::Image(bytes)));
        }
    }
    for (program, args) in text_tools() {
        if let Some(bytes) = run(program, args)? {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            return Ok((!text.is_empty()).then_some(Content::Text(text)));
        }
    }
    Ok(None)
}

type Tool = (&'static str, &'static [&'static str]);

#[cfg(target_os = "macos")]
fn image_tools() -> Vec<Tool> {
    vec![(
        "osascript",
        &["-e", "get the clipboard as «class PNGf»"] as &[&str],
    )]
}

#[cfg(windows)]
fn image_tools() -> Vec<Tool> {
    vec![(
        "powershell.exe",
        &[
            "-NoProfile",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms; $i = [System.Windows.Forms.Clipboard]::GetImage(); if ($i) { $m = New-Object System.IO.MemoryStream; $i.Save($m, [System.Drawing.Imaging.ImageFormat]::Png); [Console]::OpenStandardOutput().Write($m.ToArray(), 0, $m.Length) }",
        ] as &[&str],
    )]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn image_tools() -> Vec<Tool> {
    vec![
        (
            "wl-paste",
            &["--no-newline", "--type", "image/png"] as &[&str],
        ),
        (
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        ),
    ]
}

#[cfg(target_os = "macos")]
fn text_tools() -> Vec<Tool> {
    vec![("pbpaste", &[] as &[&str])]
}

#[cfg(windows)]
fn text_tools() -> Vec<Tool> {
    vec![(
        "powershell.exe",
        &["-NoProfile", "-Command", "Get-Clipboard -Raw"] as &[&str],
    )]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn text_tools() -> Vec<Tool> {
    vec![
        ("wl-paste", &["--no-newline"] as &[&str]),
        ("xclip", &["-selection", "clipboard", "-o"]),
        ("xsel", &["--clipboard", "--output"]),
    ]
}

/// Output of a tool that ran and succeeded; `Ok(None)` when it is missing or failed.
fn run(program: &str, args: &[&str]) -> Result<Option<Vec<u8>>, String> {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Ok(None);
    };
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout
            .by_ref()
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Clipboard read timed out".into());
            }
        }
    };
    let bytes = reader.join().unwrap_or_default();
    if bytes.len() > LIMIT {
        return Err("Clipboard content exceeded configured read or image conversion limits".into());
    }
    Ok((status.success() && !bytes.is_empty()).then_some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tool_is_not_an_error() {
        assert_eq!(run("anysola-no-such-tool", &[]), Ok(None));
    }

    #[test]
    fn captures_output() {
        assert_eq!(run("echo", &["-n", "hi"]), Ok(Some(b"hi".to_vec())));
    }
}
