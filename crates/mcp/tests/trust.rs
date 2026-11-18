//! A project's MCP server is not started until the user approved it; then it starts without a
//! restart.

mod common;

use std::time::{Duration, Instant};

use anysola_core::api::{Core, TrustKind};
use anysola_core::mcp::{Mcp, Status};
use anysola_mcp::Runtime;
use anysola_testkit::FakeGateway;
use common::*;
use serde_json::json;

#[test]
fn fake_mcp() {
    serve_if_asked();
}

fn status(mcp: &Runtime, name: &str) -> Option<Status> {
    mcp.servers()
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| s.status)
}

#[test]
fn a_project_server_starts_only_once_approved() {
    let gateway = FakeGateway::start(|request, _| text(request, "unused"));
    let env = env(&gateway.url);
    let marker = env.home.home().join("started");
    let mut server = fake_server("fake_mcp");
    server["environment"]["FAKE_MCP_MARKER"] = json!(marker.display().to_string());
    env.home.write(
        "proj/anysola.json",
        &json!({"mcp": {"servers": {"gated": server}}}).to_string(),
    );
    let rt = runtime();
    let (core, mcp): (Core, std::sync::Arc<Runtime>) = rt.block_on(async {
        let core = Core::start(&env.home.project()).await.unwrap();
        let mcp = Runtime::attach(&core);
        mcp.settled(Duration::from_secs(5)).await;
        (core, mcp)
    });
    std::thread::sleep(Duration::from_millis(500));
    assert!(!marker.exists(), "the server process was not started");
    assert_eq!(status(&mcp, "gated"), None);
    let item = core
        .trust_items()
        .into_iter()
        .find(|i| i.kind == TrustKind::Mcp)
        .unwrap();
    rt.block_on(async {
        core.trust(&item.path, TrustKind::Mcp, &item.digest, true)
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while status(&mcp, "gated") != Some(Status::Connected) {
        assert!(Instant::now() < deadline, "the approved server connects");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(marker.exists());
    rt.block_on(mcp.shutdown());
}
