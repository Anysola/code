//! `run` with stdio MCP servers: a Code Mode call whose server asks for input (the form is
//! cancelled and the run exits 1), and a `codemode: false` server whose tools are direct tools.
#![cfg(unix)]

mod common;

use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply, TestHome};
use common::BIN;
use serde_json::json;

/// A server in `sh`: one tool `ask` whose call asks the client for a name and returns the
/// client's answer as text.
const SERVER: &str = r#"read l
echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"1"}}}'
read l
read l
echo '{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"ask","inputSchema":{"type":"object","properties":{}}}]}}'
read l
echo '{"jsonrpc":"2.0","id":"e1","method":"elicitation/create","params":{"mode":"form","message":"Name?","requestedSchema":{"type":"object","properties":{"name":{"type":"string"}}}}}'
read r
r=$(printf '%s' "$r" | sed 's/"/\\"/g')
echo '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"'"$r"'"}]}}'
cat >/dev/null
"#;

fn reply(request: &Recorded, stream: Stream) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Reply::Sse {
        body: stream.finish(&model, Some(1)),
        before: None,
    }
}

#[test]
fn elicitation_is_cancelled_and_direct_tools_are_advertised() {
    let gateway = FakeGateway::start(|request, _| {
        if request.is_title() {
            return reply(request, Stream::new().text("Title"));
        }
        let body = request.body.as_str();
        // Servers connect while the run starts; wait for them as a model would.
        if !body.contains("tools.f.ask") || !body.contains("\"direct_ask\"") {
            return reply(
                request,
                Stream::new().tool("w", "shell", json!({"command": "sleep 0.2"})),
            );
        }
        if !body.contains("\"tool_use_id\":\"t\"") {
            return reply(
                request,
                Stream::new().tool(
                    "t",
                    "execute",
                    json!({"code": "return await tools.f.ask({})"}),
                ),
            );
        }
        reply(request, Stream::new().text("done"))
    });
    let home = TestHome::new(&gateway.url);
    let server = json!({"type": "local", "command": ["sh", "-c", SERVER]});
    let mut direct = server.clone();
    direct["codemode"] = json!(false);
    home.write(
        "proj/opencode.json",
        &json!({"mcp": {"servers": {"f": server, "direct": direct}}}).to_string(),
    );
    let out = home.run(BIN, &["run", "go"], None, None);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let last = gateway
        .messages()
        .into_iter()
        .rev()
        .find(|r| !r.is_title())
        .unwrap();
    assert!(
        last.body
            .contains(r#"\"result\": {\n    \"action\": \"cancel\"\n  }"#),
        "{}",
        last.body
    );
    let tools = last.json()["tools"].clone();
    let direct = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "direct_ask")
        .unwrap()
        .clone();
    assert_eq!(
        direct,
        json!({"name": "direct_ask", "description": "", "input_schema": {"type": "object", "properties": {}}})
    );
    assert!(!last.body.contains("tools.direct.ask"));
}
