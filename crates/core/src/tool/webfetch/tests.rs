use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::tool::ToolContent;
use crate::tool::test_support::{TestContext, context};

const PAGE: &str = "<html><head><title>T</title></head><body><h1>Hello</h1><p>Some <b>bold</b> and <a href=\"/x\">link</a>.</p><ul><li>a</li><li>b</li></ul><pre><code class=\"language-js\">let a=1</code></pre><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table></body></html>";

/// Serves canned answers per path and records each request head.
async fn server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buffer = vec![0; 8192];
                let n = socket.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..n]).to_string();
                seen.lock().unwrap().push(head.clone());
                let path = head.split(' ').nth(1).unwrap_or("/").to_string();
                let agent_opencode = head.contains("user-agent: opencode\r\n");
                let (status, headers, body): (&str, String, Vec<u8>) = match path.as_str() {
                    "/html" => (
                        "200 OK",
                        "content-type: text/html; charset=utf-8\r\n".into(),
                        PAGE.into(),
                    ),
                    "/json" => (
                        "200 OK",
                        "content-type: application/json\r\n".into(),
                        b"{\"a\":1}".to_vec(),
                    ),
                    "/png" => ("200 OK", "content-type: image/png\r\n".into(), vec![0; 4]),
                    "/bin" => (
                        "200 OK",
                        "content-type: application/octet-stream\r\n".into(),
                        vec![0; 4],
                    ),
                    "/big" => (
                        "200 OK",
                        "content-type: text/plain\r\n".into(),
                        vec![b'a'; 6 * 1024 * 1024],
                    ),
                    "/redirect" => ("302 Found", "location: /json\r\n".into(), Vec::new()),
                    "/loop" => ("302 Found", "location: /loop\r\n".into(), Vec::new()),
                    "/to-denied" => ("302 Found", "location: /secret\r\n".into(), Vec::new()),
                    "/challenge" if !agent_opencode => (
                        "403 Forbidden",
                        "cf-mitigated: challenge\r\n".into(),
                        Vec::new(),
                    ),
                    "/challenge" => (
                        "200 OK",
                        "content-type: text/plain\r\n".into(),
                        b"passed".to_vec(),
                    ),
                    "/slow" => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                        ("200 OK", String::new(), Vec::new())
                    }
                    _ => ("404 Not Found", String::new(), Vec::new()),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    (base, heads)
}

async fn fetch_with(test: &TestContext, input: Value) -> ToolResult {
    WebfetchTool.execute(input, &test.ctx).await
}

fn text(output: &ToolOutput) -> &str {
    match &output.content[0] {
        ToolContent::Text { text } => text,
        other => panic!("not text: {other:?}"),
    }
}

fn unknown(message: &str) -> ToolError {
    ToolError::unknown(message)
}

#[tokio::test]
async fn formats_and_headers() {
    let (base, heads) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let test = context(dir.path());
    let markdown = fetch_with(&test, json!({"url": format!("{base}/html")}))
        .await
        .unwrap();
    assert_eq!(
        text(&markdown),
        "# Hello\n\nSome **bold** and [link](/x).\n\n- a\n\n- b\n\n```js\nlet a=1\n```\n\n| A | B |\n| --- | --- |\n| 1 | 2 |"
    );
    assert_eq!(markdown.metadata["contentType"], "text/html; charset=utf-8");
    let plain = fetch_with(
        &test,
        json!({"url": format!("{base}/html"), "format": "text"}),
    )
    .await
    .unwrap();
    assert_eq!(text(&plain), "THelloSome bold and link.ablet a=1AB12");
    let raw = fetch_with(
        &test,
        json!({"url": format!("{base}/html"), "format": "html"}),
    )
    .await
    .unwrap();
    assert_eq!(text(&raw), PAGE);
    let head = heads.lock().unwrap()[0].to_ascii_lowercase();
    assert!(head.contains(&format!(
        "user-agent: {}\r\n",
        USER_AGENT.to_ascii_lowercase()
    )));
    assert!(head.contains("accept: text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1\r\n"));
    assert!(head.contains("accept-language: en-us,en;q=0.9\r\n"));
    let asked = test.gate.asked.lock().unwrap();
    assert_eq!(
        (
            asked[0].action.as_str(),
            &asked[0].resources,
            &asked[0].save
        ),
        (
            "webfetch",
            &vec![format!("{base}/html")],
            &vec!["*".to_string()]
        )
    );
}

#[tokio::test]
async fn captured_errors() {
    let (base, _) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let test = context(dir.path());
    let json_body = fetch_with(&test, json!({"url": format!("{base}/redirect")}))
        .await
        .unwrap();
    assert_eq!(text(&json_body), "{\"a\":1}");
    let cases = [
        (
            "/404",
            format!("StatusCode: non 2xx status code (404 GET {base}/404)"),
        ),
        (
            "/png",
            "Unsupported fetched image content type: image/png".into(),
        ),
        (
            "/bin",
            "Unsupported fetched file content type: application/octet-stream".into(),
        ),
        (
            "/big",
            "Response too large (exceeds 5242880 byte limit)".into(),
        ),
        ("/loop", "Too many redirects (more than 10)".into()),
    ];
    for (path, message) in cases {
        let error = fetch_with(&test, json!({"url": format!("{base}{path}")}))
            .await
            .unwrap_err();
        assert_eq!(error, unknown(&message), "{path}");
    }
    let error = fetch_with(&test, json!({"url": "ftp://example.com/x"}))
        .await
        .unwrap_err();
    assert_eq!(error, unknown("URL must use http:// or https://"));
    let error = fetch_with(&test, json!({"url": "not a url"}))
        .await
        .unwrap_err();
    assert_eq!(error, unknown("Invalid URL"));
    let error = fetch_with(&test, json!({"url": "http://127.0.0.1:1/x"}))
        .await
        .unwrap_err();
    assert_eq!(error, unknown("Transport error (GET http://127.0.0.1:1/x)"));
    let error = fetch_with(&test, json!({"url": format!("{base}/slow"), "timeout": 1}))
        .await
        .unwrap_err();
    assert_eq!(error, unknown("Request timed out"));
}

#[tokio::test]
async fn redirects_are_checked_and_challenges_retried() {
    let (base, heads) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let test = context(dir.path());
    test.gate
        .denied
        .lock()
        .unwrap()
        .push(format!("{base}/secret"));
    let error = fetch_with(&test, json!({"url": format!("{base}/to-denied")}))
        .await
        .unwrap_err();
    assert_eq!(
        (error.type_tag(), error.message.as_str()),
        ("permission.rejected", "Permission denied: webfetch")
    );
    assert!(!heads.lock().unwrap().iter().any(|h| h.contains("/secret")));
    let passed = fetch_with(&test, json!({"url": format!("{base}/challenge")}))
        .await
        .unwrap();
    assert_eq!(text(&passed), "passed");
}

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/tool/webfetch/fixtures")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Outputs of 0.5.1's converter and text extractor for the same inputs.
#[test]
fn converter_matches_the_typescript_outputs() {
    let mut failures = Vec::new();
    for case in fixture("corpus.json").as_array().unwrap() {
        let html = case["html"].as_str().unwrap();
        for (kind, actual) in [
            ("markdown", markdown::to_markdown(html)),
            ("text", markdown::to_text(html)),
        ] {
            if actual != case[kind].as_str().unwrap() {
                failures.push(format!("{kind} {html:?}: {actual:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn fnv1a(value: &str) -> String {
    let hash = value.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// Large inputs (output caps, nesting fallback, 64 KiB write boundaries), compared by size and hash.
#[test]
fn large_inputs_match_the_typescript_outputs() {
    let mut failures = Vec::new();
    for case in fixture("large.json").as_array().unwrap() {
        let count = case["count"].as_u64().unwrap() as usize;
        let html = format!(
            "{}{}{}",
            case["prefix"].as_str().unwrap(),
            case["unit"].as_str().unwrap().repeat(count),
            case["suffix"].as_str().unwrap()
        );
        for (kind, actual) in [
            ("markdown", markdown::to_markdown(&html)),
            ("text", markdown::to_text(&html)),
        ] {
            let expected = &case[kind];
            if actual.len() as u64 != expected["bytes"].as_u64().unwrap()
                || fnv1a(&actual) != expected["hash"].as_str().unwrap()
            {
                failures.push(format!("{} {kind}", case["name"]));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
