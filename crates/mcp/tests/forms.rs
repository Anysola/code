//! A headless turn with stdio MCP servers: a Code Mode call whose server asks for input (nobody
//! answers, the form is cancelled and the model sees that), and a `codemode: false` server whose
//! tools are direct tools.
#![cfg(unix)]

mod common;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::events::Event;
use anysola_core::mcp::FormReply;
use anysola_core::session::{Outcome, TurnOptions};
use anysola_testkit::sse::Stream;
use anysola_testkit::{FakeGateway, Recorded, Reply};
use serde_json::json;

/// A server in `sh`: one tool `ask` whose call asks the client for a name and returns the
/// client's answer as text. It answers by method and echoes the request's id, as a real server
/// does: the client may list tools again or give a request up, in any order.
const SERVER: &str = r#"while read l; do
id=$(printf '%s' "$l" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
case "$l" in
*'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"f","version":"1"}}}' ;;
*'"method":"tools/list"'*) echo '{"jsonrpc":"2.0","id":'"$id"',"result":{"tools":[{"name":"ask","inputSchema":{"type":"object","properties":{}}}]}}' ;;
*'"method":"tools/call"'*) call=$id
echo '{"jsonrpc":"2.0","id":"e1","method":"elicitation/create","params":{"mode":"form","message":"Name?","requestedSchema":{"type":"object","properties":{"name":{"type":"string"}}}}}' ;;
*'"id":"e1"'*) r=$(printf '%s' "$l" | sed 's/"/\\"/g')
echo '{"jsonrpc":"2.0","id":'"$call"',"result":{"content":[{"type":"text","text":"'"$r"'"}]}}' ;;
esac
done
"#;

fn reply(request: &Recorded, stream: Stream) -> Reply {
    let model = request.json()["model"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    Reply::Sse {
        body: stream.finish(&model, Some(1)),
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
    let env = common::env(&gateway.url);
    let home = &env.home;
    let server = json!({"type": "local", "command": ["sh", "-c", SERVER]});
    let mut direct = server.clone();
    direct["codemode"] = json!(false);
    // A project's servers wait for approval since 1.0.10; the user's run at once.
    home.write(
        ".anysola/config.json",
        &json!({"mcp": {"servers": {"f": server, "direct": direct}}}).to_string(),
    );
    let runtime = common::runtime();
    let outcome = runtime.block_on(async {
        let core = Core::start(&home.project()).await.expect("core starts");
        let mcp = anysola_mcp::Runtime::attach(&core);
        let (session, _) = core.open(&Target::New, false).unwrap();
        let mut events = core.subscribe();
        let answering = tokio::spawn({
            let core = core.clone();
            async move {
                loop {
                    match events.recv().await {
                        Ok(Event::FormAsked { form }) => {
                            core.reply_form(&form.id, FormReply::Cancelled);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                        _ => {}
                    }
                }
            }
        });
        let prompt = Prompt {
            text: "go".into(),
            files: Vec::new(),
            skills: Vec::new(),
            command: None,
        };
        let outcome = core
            .prompt(&session.id, prompt, TurnOptions::default())
            .await;
        answering.abort();
        anysola_core::mcp::Mcp::shutdown(mcp.as_ref()).await;
        core.shutdown();
        outcome
    });
    let last = gateway
        .messages()
        .into_iter()
        .rev()
        .find(|r| !r.is_title())
        .unwrap();
    assert_eq!(outcome.unwrap(), Outcome::Succeeded, "{}", last.body);
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
        json!({"name": "direct_ask", "description": "", "input_schema": {"type": "object", "properties": {}}, "eager_input_streaming": true})
    );
    assert!(!last.body.contains("tools.direct.ask"));
}
