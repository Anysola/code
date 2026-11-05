use std::path::Path;

use serde_json::json;

use super::*;
use crate::permission::PermissionError;
use crate::tool::test_support::context;

async fn run(dir: &Path, input: Value) -> (ToolResult, Vec<PermissionRequest>) {
    let test = context(dir);
    let result = ReadTool.execute(input, &test.ctx).await;
    let asked = test.gate.asked.lock().unwrap().clone();
    (result, asked)
}

async fn text(dir: &Path, input: Value) -> String {
    let output = run(dir, input).await.0.unwrap();
    match &output.content[..] {
        [ToolContent::Text { text }] => text.clone(),
        other => panic!("unexpected content {other:?}"),
    }
}

async fn error(dir: &Path, input: Value) -> (String, &'static str) {
    let error = run(dir, input).await.0.unwrap_err();
    (error.message.clone(), error.type_tag())
}

fn write(dir: &Path, name: &str, content: impl AsRef<[u8]>) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[tokio::test]
async fn small_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "line one\nline two\nline three\n");
    let (result, asked) = run(dir.path(), json!({"path": "a.txt"})).await;
    let output = result.unwrap();
    assert_eq!(
        output.content,
        [ToolContent::text(
            "Read file a.txt, lines 1-3\n1: line one\n2: line two\n3: line three"
        )]
    );
    assert_eq!(
        output.metadata,
        json!({"truncated": false}).as_object().unwrap().clone()
    );
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].action, "read");
    assert_eq!(asked[0].resources, ["a.txt"]);
    assert_eq!(asked[0].save, ["*"]);
}

#[tokio::test]
async fn whole_file_keeps_cr_and_blank_lines() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "crlf.txt", "alpha\r\nbeta\r\ngamma\r\n");
    write(dir.path(), "blank.txt", "one\n\n\n");
    write(dir.path(), "empty.txt", "");
    write(dir.path(), "newline.txt", "\n");
    write(dir.path(), "nonl.txt", "x\ny");
    write(dir.path(), "bad.txt", b"a\xffb\n");
    write(dir.path(), "bom.txt", "\u{FEFF}hi\n");
    let cases = [
        (
            "crlf.txt",
            "Read file crlf.txt, lines 1-3\n1: alpha\r\n2: beta\r\n3: gamma\r",
        ),
        (
            "blank.txt",
            "Read file blank.txt, lines 1-3\n1: one\n2: \n3: ",
        ),
        ("empty.txt", "Read file empty.txt, 0 lines"),
        ("newline.txt", "Read file newline.txt, lines 1-1\n1: "),
        ("nonl.txt", "Read file nonl.txt, lines 1-2\n1: x\n2: y"),
        ("bad.txt", "Read file bad.txt, lines 1-1\n1: a\u{FFFD}b"),
        ("bom.txt", "Read file bom.txt, lines 1-1\n1: hi"),
    ];
    for (path, expected) in cases {
        assert_eq!(text(dir.path(), json!({ "path": path })).await, expected);
    }
}

#[tokio::test]
async fn whole_file_has_no_line_cap() {
    let dir = tempfile::tempdir().unwrap();
    let content: String = (1..=3000).map(|n| format!("{n}\n")).collect();
    write(dir.path(), "many.txt", &content);
    let output = text(dir.path(), json!({"path": "many.txt"})).await;
    assert!(output.starts_with("Read file many.txt, lines 1-3000\n"));
    assert!(output.ends_with("\n3000: 3000"));
}

#[tokio::test]
async fn paging_strips_cr_and_continues() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "crlf.txt", "alpha\r\nbeta\r\ngamma\r\n");
    assert_eq!(
        text(dir.path(), json!({"path": "crlf.txt", "offset": 0})).await,
        "Read file crlf.txt, lines 1-3\n1: alpha\n2: beta\n3: gamma"
    );
    assert_eq!(
        text(dir.path(), json!({"path": "crlf.txt", "limit": 2})).await,
        "Read file crlf.txt, lines 1-2\n1: alpha\n2: beta\n[Output truncated. Continue reading with offset: 3]"
    );
    let content: String = (1..=3000).map(|n| format!("row {n}\n")).collect();
    write(dir.path(), "long.txt", &content);
    let (result, _) = run(
        dir.path(),
        json!({"path": "long.txt", "offset": 2990, "limit": 5}),
    )
    .await;
    let output = result.unwrap();
    assert_eq!(
        output.content,
        [ToolContent::text(
            "Read file long.txt, lines 2990-2994\n2990: row 2990\n2991: row 2991\n2992: row 2992\n2993: row 2993\n2994: row 2994\n[Output truncated. Continue reading with offset: 2995]"
        )]
    );
    assert_eq!(output.metadata["truncated"], true);
    let tail = text(dir.path(), json!({"path": "long.txt", "offset": 2999})).await;
    assert_eq!(
        tail,
        "Read file long.txt, lines 2999-3000\n2999: row 2999\n3000: row 3000"
    );
}

#[tokio::test]
async fn large_file_pages_by_bytes_and_lines() {
    let dir = tempfile::tempdir().unwrap();
    let line = "y".repeat(99);
    write(dir.path(), "big.txt", format!("{line}\n").repeat(1000));
    let output = text(dir.path(), json!({"path": "big.txt"})).await;
    let lines: Vec<&str> = output.split('\n').collect();
    assert_eq!(lines[0], "Read file big.txt, lines 1-512");
    assert_eq!(lines[1], format!("1: {line}"));
    assert_eq!(lines[512], format!("512: {line}"));
    assert_eq!(
        lines[513],
        "[Output truncated. Continue reading with offset: 513]"
    );
    assert_eq!(lines.len(), 514);

    let content: String = (1..=20000).map(|n| format!("{n}\n")).collect();
    write(dir.path(), "rows.txt", &content);
    let output = text(dir.path(), json!({"path": "rows.txt"})).await;
    assert!(output.starts_with("Read file rows.txt, lines 1-2000\n"));
    assert!(
        output.ends_with("\n2000: 2000\n[Output truncated. Continue reading with offset: 2001]")
    );
}

#[tokio::test]
async fn long_lines_are_clamped() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "wide.txt", "x".repeat(2500));
    assert_eq!(
        text(dir.path(), json!({"path": "wide.txt"})).await,
        format!(
            "Read file wide.txt, lines 1-1\n1: {}... (line truncated to 2000 chars)",
            "x".repeat(2000)
        )
    );
    // Clamping counts UTF-16 units: 1001 astral chars are 2002 units.
    write(dir.path(), "emoji.txt", "\u{1F600}".repeat(1001));
    assert_eq!(
        text(dir.path(), json!({"path": "emoji.txt"})).await,
        format!(
            "Read file emoji.txt, lines 1-1\n1: {}... (line truncated to 2000 chars)",
            "\u{1F600}".repeat(1000)
        )
    );
}

#[tokio::test]
async fn binary_and_offsets() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "bin.dat", b"ab\0cd");
    write(dir.path(), "a.bmp", b"BM\0\0\0\0");
    write(dir.path(), "a.txt", "one\ntwo\nthree\n");
    write(dir.path(), "later.txt", b"one\ntwo\nth\0ree\n");
    assert_eq!(
        error(dir.path(), json!({"path": "bin.dat"})).await,
        ("Cannot read binary file: bin.dat".to_string(), "unknown")
    );
    assert_eq!(
        error(dir.path(), json!({"path": "a.bmp"})).await.0,
        "Cannot read binary file: a.bmp"
    );
    assert_eq!(
        error(dir.path(), json!({"path": "a.txt", "offset": 10})).await,
        (
            "Offset 10 is out of range for this file (3 lines)".to_string(),
            "unknown"
        )
    );
    write(dir.path(), "one.txt", "only");
    assert_eq!(
        error(dir.path(), json!({"path": "one.txt", "offset": 2}))
            .await
            .0,
        "Offset 2 is out of range for this file (1 line)"
    );
    // A NUL after the selected page is never looked at.
    assert_eq!(
        text(dir.path(), json!({"path": "later.txt", "limit": 2})).await,
        "Read file later.txt, lines 1-2\n1: one\n2: two\n[Output truncated. Continue reading with offset: 3]"
    );
    assert_eq!(
        error(dir.path(), json!({"path": "later.txt", "offset": 3}))
            .await
            .0,
        "Cannot read binary file: later.txt"
    );
}

#[tokio::test]
async fn invalid_offset() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        error(dir.path(), json!({"path": "a.txt", "offset": -1})).await,
        (
            "Invalid arguments for tool \"read\":\n- offset: Expected a value greater than or equal to 0\n\nArguments provided:\n{\n  \"path\": \"a.txt\",\n  \"offset\": -1\n}\n\nUpdate the arguments and call the tool again.".to_string(),
            "tool.execution"
        )
    );
}

#[tokio::test]
async fn media() {
    let dir = tempfile::tempdir().unwrap();
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    write(dir.path(), "i.png", png);
    write(dir.path(), "doc.pdf", b"%PDF-1.4\n");
    write(dir.path(), "w.webp", b"RIFF\0\0\0\0WEBPVP8 ");
    let output = run(dir.path(), json!({"path": "i.png"})).await.0.unwrap();
    use base64::Engine;
    let data = base64::engine::general_purpose::STANDARD.encode(png);
    assert_eq!(
        output.content,
        [
            ToolContent::text("Image read successfully"),
            ToolContent::File {
                uri: format!("data:image/png;base64,{data}"),
                mime: "image/png".into(),
                name: Some("i.png".into()),
            }
        ]
    );
    assert_eq!(output.metadata["truncated"], false);
    let pdf = run(dir.path(), json!({"path": "doc.pdf"})).await.0.unwrap();
    assert_eq!(pdf.content[0], ToolContent::text("PDF read successfully"));
    let webp = run(dir.path(), json!({"path": "w.webp"})).await.0.unwrap();
    assert!(matches!(&webp.content[1], ToolContent::File { mime, .. } if mime == "image/webp"));

    let huge = std::fs::File::create(dir.path().join("huge.png")).unwrap();
    std::io::Write::write_all(&mut &huge, png).unwrap();
    huge.set_len(file::MAX_MEDIA_BYTES + 1).unwrap();
    assert_eq!(
        error(dir.path(), json!({"path": "huge.png"})).await,
        (
            "Media exceeds 20971520 byte ingestion limit: huge.png".to_string(),
            "unknown"
        )
    );
}

#[cfg(unix)]
#[tokio::test]
async fn directory_listing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    for name in [
        "a.txt",
        "alpha-notes.txt",
        "big.txt",
        "bin.dat",
        "blank.txt",
        "crlf.txt",
        "empty.txt",
        "long.txt",
        "many.txt",
        "nonl.txt",
        "Screenshot 2026 at 10.txt",
        "wide.txt",
    ] {
        write(root, name, "");
    }
    std::os::unix::fs::symlink(root.join("a.txt"), root.join("link.txt")).unwrap();
    let (result, asked) = run(root, json!({"path": "."})).await;
    let output = result.unwrap();
    assert_eq!(
        output.content,
        [ToolContent::text(
            "Read directory ., entries 1-15\n.git/\nsub/\na.txt\nalpha-notes.txt\nbig.txt\nbin.dat\nblank.txt\ncrlf.txt\nempty.txt\nlink.txt\nlong.txt\nmany.txt\nnonl.txt\nScreenshot 2026 at 10.txt\nwide.txt"
        )]
    );
    assert_eq!(output.metadata["truncated"], false);
    assert_eq!(asked[0].resources, ["."]);
    assert_eq!(
        text(root, json!({"path": ".", "offset": 2, "limit": 2})).await,
        "Read directory ., entries 2-3\nsub/\na.txt\n[Output truncated. Continue reading with offset: 4]"
    );
    assert_eq!(
        text(root, json!({"path": ".", "offset": 99})).await,
        "Read directory ., 0 entries"
    );
}

#[test]
fn locale_order_approximation() {
    let mut names = vec!["b", "B", "a", "_x", "A", "a.txt", "alpha"];
    names.sort_by(|a, b| crate::text::locale_compare(a, b));
    assert_eq!(names, ["_x", "a", "A", "a.txt", "alpha", "b", "B"]);
}

#[tokio::test]
async fn not_found_messages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "a.txt", "x");
    write(root, "zeta.md", "x");
    assert_eq!(
        error(root, json!({"path": "nope.txt"})).await,
        ("File not found: nope.txt".to_string(), "tool.execution")
    );
    assert_eq!(
        error(root, json!({"path": "alpha.txt"})).await.0,
        "File not found: alpha.txt\n\nDid you mean one of these?\na.txt"
    );
    assert_eq!(
        error(root, json!({"path": "./x/../alpha.txt"})).await.0,
        "File not found: ./x/../alpha.txt\n\nDid you mean one of these?\na.txt"
    );
    let absolute = root.display();
    assert_eq!(
        error(root, json!({"path": "missing/dir/x.txt"})).await.0,
        format!(
            "File not found: missing/dir/x.txt\n\nThe folder {absolute}/missing does not exist. The nearest folder that does is {absolute}."
        )
    );
    std::fs::create_dir(root.join("missing-not")).unwrap();
    assert_eq!(
        error(root, json!({"path": "missing/x.txt"})).await.0,
        format!(
            "File not found: missing/x.txt\n\nThe folder {absolute}/missing does not exist. The nearest folder that does is {absolute}.\n\nDid you mean one of these?\n{absolute}/missing-not"
        )
    );
    // A file where a folder should be: no listing, so no suggestions.
    assert_eq!(
        error(root, json!({"path": "a.txt/b.txt"})).await.0,
        "File not found: a.txt/b.txt"
    );
}

#[tokio::test]
async fn filename_recovery() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Screenshot 2026\u{202F}at 10.txt", "shot\n");
    write(dir.path(), "it\u{2019}s.txt", "quote\n");
    let (result, asked) = run(dir.path(), json!({"path": "Screenshot 2026 at 10.txt"})).await;
    assert_eq!(
        result.unwrap().content,
        [ToolContent::text(
            "Read file Screenshot 2026\u{202F}at 10.txt, lines 1-1\n1: shot"
        )]
    );
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0].resources, ["Screenshot 2026 at 10.txt"]);
    assert_eq!(asked[1].action, "read");
    assert_eq!(asked[1].resources, ["Screenshot 2026\u{202F}at 10.txt"]);
    assert_eq!(
        text(dir.path(), json!({"path": "./it's.txt"})).await,
        "Read file it\u{2019}s.txt, lines 1-1\n1: quote"
    );
}

#[tokio::test]
async fn external_read_asks_directory_first_and_skips_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "AGENTS.md", "rules");
    write(outside.path(), "o.txt", "x");
    let path = outside.path().join("o.txt");
    let (result, asked) = run(dir.path(), json!({"path": path.to_string_lossy()})).await;
    let output = result.unwrap();
    assert_eq!(
        output.content,
        [ToolContent::text(format!(
            "Read file {}, lines 1-1\n1: x",
            path.display()
        ))]
    );
    assert!(!output.metadata.contains_key("instructions"));
    assert_eq!(asked[0].action, "external_directory");
    assert_eq!(
        asked[0].resources,
        [access::slash(&outside.path().join("*"))]
    );
    assert_eq!(asked[1].action, "read");
    assert_eq!(asked[1].resources, [access::slash(&path)]);
}

#[tokio::test]
async fn permission_rejection_propagates() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt", "x");
    let test = context(dir.path());
    let message = "This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action.";
    *test.gate.answer.lock().unwrap() = Some(PermissionError::Corrected(message.into()));
    let error = ReadTool
        .execute(json!({"path": "a.txt"}), &test.ctx)
        .await
        .unwrap_err();
    assert_eq!(error.message, message);
    assert_eq!(error.type_tag(), "permission.rejected");
}

#[tokio::test]
async fn nested_instructions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "AGENTS.md", "root");
    write(root, "sub/AGENTS.md", "sub");
    write(root, "sub/deeper/AGENTS.md", "deeper");
    write(root, "sub/deeper/f.txt", "x");
    write(root, "sub/other/g.txt", "x");
    let canonical = |p: &str| {
        std::fs::canonicalize(root.join(p))
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    let metadata = |input: Value| async move { run(root, input).await.0.unwrap().metadata };
    assert_eq!(
        metadata(json!({"path": "sub/deeper/f.txt"})).await["instructions"],
        json!([
            canonical("sub/deeper/AGENTS.md"),
            canonical("sub/AGENTS.md")
        ])
    );
    assert_eq!(
        metadata(json!({"path": "sub/other/g.txt"})).await["instructions"],
        json!([canonical("sub/AGENTS.md")])
    );
    assert_eq!(
        metadata(json!({"path": "sub"})).await["instructions"],
        json!([canonical("sub/AGENTS.md")])
    );
    assert!(
        !metadata(json!({"path": "sub/AGENTS.md"}))
            .await
            .contains_key("instructions")
    );
    assert!(
        !metadata(json!({"path": "AGENTS.md"}))
            .await
            .contains_key("instructions")
    );
}

#[test]
fn node_paths() {
    assert_eq!(js_dirname("a.txt"), ".");
    assert_eq!(js_dirname("a/b.txt"), "a");
    assert_eq!(js_dirname("/a"), "/");
    assert_eq!(js_dirname("a/b/"), "a");
    assert_eq!(js_basename("a/b/"), "b");
    assert_eq!(js_join(".", "x"), "x");
    assert_eq!(js_join("./a/../b", "x"), "b/x");
    assert_eq!(js_join("/", "x"), "/x");
    assert_eq!(js_join("../a", "x"), "../a/x");
}

#[cfg(unix)]
#[tokio::test]
async fn other_file_kinds_are_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(dir.path().join("sock")).unwrap();
    assert_eq!(
        error(dir.path(), json!({"path": "sock"})).await,
        (
            "Path is not a file or directory: sock".to_string(),
            "unknown"
        )
    );
}
