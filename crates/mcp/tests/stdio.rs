//! A stdio MCP server through the whole stack: statuses, the Code Mode catalog and instructions in
//! block 2, a tool call as the model sees it, a tool list that grows mid-turn, prompts as
//! commands, resources, and a server process that exits.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anysola_core::api::{Core, Prompt, Target};
use anysola_core::mcp::{Mcp, Status};
use anysola_core::session::{Outcome, TurnOptions};
use anysola_mcp::Runtime;
use anysola_testkit::FakeGateway;
use anysola_testkit::sse::Stream;
use common::*;
use serde_json::json;

#[test]
fn fake_mcp() {
    serve_if_asked();
}

fn execute(code: &str) -> Stream {
    Stream::new().tool("toolu_x", "execute", json!({"code": code}))
}

#[test]
fn server_through_the_stack() {
    let steps = Arc::new(AtomicUsize::new(0));
    let counter = steps.clone();
    let gateway = FakeGateway::start(move |request, _| {
        if request.is_title() {
            return text(request, "title");
        }
        match counter.fetch_add(1, Ordering::SeqCst) {
            0 => sse(
                request,
                execute("return await tools.testsrv.echo({text: 'hi'})"),
            ),
            1 => sse(
                request,
                execute("return await tools.testsrv.echo({text: 'grow'})"),
            ),
            2 => sse(
                request,
                Stream::new().tool("toolu_s", "shell", json!({"command": "sleep 0.5"})),
            ),
            3 => sse(
                request,
                execute("return await tools.testsrv.echo({text: 'quit'})"),
            ),
            _ => text(request, "done"),
        }
    });
    let env = env(&gateway.url);
    let config = json!({"mcp": {"servers": {
        "testsrv": fake_server("fake_mcp"),
        "glob1": {"type": "local", "command": ["echo", "hi"]},
        "bad1": {"type": "local", "command": ["/nonexistent/cmd", "--flag"]},
        "off": {"type": "local", "command": ["x"], "disabled": true},
    }}});
    env.home.write("proj/opencode.json", &config.to_string());
    let rt = runtime();
    let (core, mcp): (Core, Arc<Runtime>) = rt.block_on(async {
        let core = Core::start(&env.home.project()).await.unwrap();
        let mcp = Runtime::attach(&core);
        mcp.settled(Duration::from_secs(20)).await;
        (core, mcp)
    });
    let statuses: Vec<(String, Status)> = mcp
        .servers()
        .into_iter()
        .map(|s| (s.name, s.status))
        .collect();
    assert_eq!(
        statuses,
        vec![
            (
                "bad1".into(),
                Status::Failed {
                    error: "NotFound: ChildProcess.spawn (/nonexistent/cmd --flag)".into()
                }
            ),
            (
                "glob1".into(),
                Status::Failed {
                    error: "Connection closed: MCP server process exited with code 0".into()
                }
            ),
            ("off".into(), Status::Disabled),
            ("testsrv".into(), Status::Connected),
        ]
    );
    let greet = core.commands().get("testsrv:greet").cloned().unwrap();
    assert_eq!(greet.description.as_deref(), Some("Greets"));

    let (session, _) = core.open(&Target::New, false).unwrap();
    let prompt = Prompt {
        text: "hi".into(),
        files: Vec::new(),
        skills: Vec::new(),
    };
    let outcome = rt
        .block_on(core.prompt(&session.id, prompt, TurnOptions::default()))
        .unwrap();
    assert_eq!(outcome, Outcome::Succeeded);
    let requests: Vec<_> = gateway
        .messages()
        .into_iter()
        .filter(|r| !r.is_title())
        .collect();
    let block2 = requests[0].json()["system"][2]["text"]
        .as_str()
        .unwrap()
        .to_string();
    let captured =
        include_str!("../../../spec/verbatim/integrations/codemode-catalog-with-mcp.txt");
    assert!(
        block2.starts_with(&format!("{}\n\n", captured.trim_end())),
        "{block2}"
    );
    let echoed = format!(
        r#"echo: {{\"name\":\"echo\",\"arguments\":{{\"text\":\"hi\"}},\"_meta\":{{\"ai.anysola/sessionID\":\"{}\",\"progressToken\":3}}}}"#,
        session.id
    );
    assert!(requests[1].body.contains(&echoed), "{}", requests[1].body);
    let grown = "The Code Mode tool catalog has changed.\\n\\nNew tools are available in addition to those previously listed:\\n  - tools.testsrv.add(): Promise<unknown> // Added later";
    assert!(requests[3].body.contains(grown), "{}", requests[3].body);
    assert!(
        requests[4]
            .body
            .contains("MCP tool \\\"echo\\\" on server \\\"testsrv\\\" failed: Connection closed: MCP server process exited with code 3: bye"),
        "{}",
        requests[4].body
    );
    assert_eq!(
        mcp.servers()
            .into_iter()
            .find(|s| s.name == "testsrv")
            .unwrap()
            .status,
        Status::Failed {
            error: "Connection closed: MCP server process exited with code 3: bye".into()
        }
    );
    // The next request starts the exited process again, three times per process; after that
    // the server stays failed until connected again from /mcps.
    let call =
        |text: &str| rt.block_on(mcp.call_tool("testsrv", "echo", json!({"text": text}), "ses_x"));
    for restart in 1..=3 {
        assert!(call("hi").is_ok(), "restart {restart}");
        let status = mcp.servers().into_iter().find(|s| s.name == "testsrv");
        assert_eq!(status.unwrap().status, Status::Connected);
        assert!(call("quit").is_err());
    }
    assert_eq!(
        call("hi"),
        Err("MCP server \"testsrv\" is not connected: Connection closed: MCP server process exited with code 3: bye. Reconnect it from /mcps.".into())
    );
    rt.block_on(async {
        mcp.connect("testsrv");
        mcp.settled(Duration::from_secs(20)).await
    });
    let resources = rt.block_on(mcp.list_resources(None)).unwrap();
    assert_eq!(
        resources,
        json!({"resources": [{"server": "testsrv", "name": "readme", "uri": "file:///readme", "mimeType": "text/plain"}], "templates": []})
    );
    let read = rt
        .block_on(mcp.read_resource("testsrv", "file:///readme"))
        .unwrap();
    assert_eq!(
        read,
        Some(
            json!({"server": "testsrv", "uri": "file:///readme", "contents": [{"type": "text", "uri": "file:///readme", "text": "read me"}]})
        )
    );
    let expanded = rt
        .block_on(mcp.expand_prompt(&mcp.prompts()[0], vec!["Ada".into()]))
        .unwrap();
    assert_eq!(expanded, "Hello \"Ada\" \"\"");
    rt.block_on(mcp.shutdown());
}

#[test]
fn config_changes_restart_only_changed_servers() {
    let gateway = FakeGateway::start(|request, _| text(request, "unused"));
    let env = env(&gateway.url);
    let write = |echo: serde_json::Value| {
        let config =
            json!({"mcp": {"servers": {"testsrv": fake_server("fake_mcp"), "glob1": echo}}});
        env.home.write("proj/opencode.json", &config.to_string());
    };
    write(json!({"type": "local", "command": ["echo", "hi"]}));
    let rt = runtime();
    let (core, mcp) = rt.block_on(async {
        let core = Core::start(&env.home.project()).await.unwrap();
        let mcp = Runtime::attach(&core);
        mcp.settled(Duration::from_secs(20)).await;
        (core, mcp)
    });
    let mut events = core.subscribe();
    write(json!({"type": "local", "command": ["echo", "hi"], "disabled": true}));
    let changed = rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut changed = Vec::new();
            while let Ok(event) = events.recv().await {
                if let anysola_core::events::Event::McpStatusChanged { server } = event {
                    let done = server.name == "glob1" && server.status == Status::Disabled;
                    changed.push(server.name);
                    if done {
                        return changed;
                    }
                }
            }
            changed
        })
        .await
        .unwrap()
    });
    assert_eq!(changed, ["glob1"]);
    assert_eq!(
        mcp.servers()
            .into_iter()
            .map(|s| s.status)
            .collect::<Vec<_>>(),
        [Status::Disabled, Status::Connected]
    );
    rt.block_on(mcp.shutdown());
}
