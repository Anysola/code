//! Streamable HTTP servers on the fake gateway's port: the `?codemode=false` retry, transient
//! connect retries, session expiry, and OAuth sign-in (DCR and CIMD), token storage and refresh.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anysola_core::api::Core;
use anysola_core::mcp::{Mcp, Status};
use anysola_mcp::{Runtime, oauth};
use anysola_testkit::{FakeGateway, Recorded, Reply};
use common::*;
use serde_json::{Value, json};

fn reply(status: u16, body: Value, headers: Vec<(&str, String)>) -> Reply {
    Reply::Json {
        status,
        body,
        headers: headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    }
}

/// A minimal MCP server: `session` is the id it hands out on initialize.
fn answer(request: &Recorded, session: &str) -> Reply {
    if request.method != "POST" {
        return reply(405, Value::Null, Vec::new());
    }
    let message = request.json();
    let id = message["id"].clone();
    let result = match message["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake", "version": "1"},
        }),
        "tools/list" => json!({"tools": [
            {"name": "echo", "inputSchema": {"type": "object", "properties": {}}}
        ]}),
        "tools/call" => json!({"content": [{"type": "text", "text": "pong"}]}),
        _ => return reply(202, Value::Null, Vec::new()),
    };
    reply(
        200,
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
        vec![("mcp-session-id", session.to_string())],
    )
}

fn start(env: &Env, rt: &tokio::runtime::Runtime) -> (Core, Arc<Runtime>) {
    rt.block_on(async {
        let core = Core::start(&env.home.project()).await.unwrap();
        let mcp = Runtime::attach(&core);
        mcp.settled(Duration::from_secs(20)).await;
        (core, mcp)
    })
}

fn status(mcp: &Runtime, name: &str) -> Status {
    mcp.servers()
        .into_iter()
        .find(|s| s.name == name)
        .unwrap()
        .status
}

// Windows client sockets are IPv6-only by default, so the IPv4-mapped address below never connects.
#[cfg(not(windows))]
#[test]
fn codemode_retry_transient_errors_and_session_expiry() {
    let gateway = FakeGateway::start(|request, _| text(request, "unused"));
    let initializes = Arc::new(AtomicUsize::new(0));
    let counter = initializes.clone();
    gateway.route("/mcp", move |request| {
        // Servers that reject unknown query parameters.
        if request.query.contains("codemode=false") {
            return reply(404, Value::Null, Vec::new());
        }
        let method = request.json()["method"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if method == "initialize" {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return reply(503, Value::Null, Vec::new());
            }
            return answer(request, &format!("s{n}"));
        }
        // The first session is forgotten once its tools are listed.
        if method == "tools/call" && request.header("mcp-session-id") == Some("s1") {
            return reply(404, Value::Null, Vec::new());
        }
        let session = request
            .header("mcp-session-id")
            .unwrap_or_default()
            .to_string();
        answer(request, &session)
    });
    let env = env(&gateway.url);
    // An IPv4-mapped address is not one of the loopback names that skip `?codemode=false`.
    let port = gateway.url.rsplit(':').next().unwrap().to_string();
    let url = format!("http://[::ffff:127.0.0.1]:{port}/mcp");
    env.home.write(
        "proj/opencode.json",
        &json!({"mcp": {"servers": {"remote": {"type": "remote", "url": url, "oauth": false}}}})
            .to_string(),
    );
    let rt = runtime();
    let (_core, mcp) = start(&env, &rt);
    assert_eq!(status(&mcp, "remote"), Status::Connected);
    let result = rt
        .block_on(mcp.call_tool("remote", "echo", json!({}), "ses_x"))
        .unwrap();
    assert_eq!(result["content"][0]["text"], "pong");
    rt.block_on(mcp.shutdown());
    let posts: Vec<(String, String)> = gateway
        .requests_to("/mcp")
        .into_iter()
        .filter(|r| r.method == "POST")
        .map(|r| {
            let method = r.json()["method"].as_str().unwrap_or_default().to_string();
            (r.query.clone(), method)
        })
        .collect();
    let initialize = |query: &str| (query.to_string(), "initialize".to_string());
    assert_eq!(
        posts[..4],
        [
            initialize("codemode=false"),
            initialize(""),
            initialize("codemode=false"),
            initialize("")
        ]
    );
    let deletes: Vec<Option<String>> = gateway
        .requests_to("/mcp")
        .into_iter()
        .filter(|r| r.method == "DELETE")
        .map(|r| r.header("mcp-session-id").map(str::to_string))
        .collect();
    assert_eq!(deletes, [Some("s2".to_string())]);
    let user_agent = gateway.requests_to("/mcp")[0]
        .header("user-agent")
        .map(str::to_string);
    assert_eq!(user_agent.as_deref(), Some("opencode/latest/2.2.5/cli"));
}

/// An OAuth-protected MCP server at `path` whose authorization server supports CIMD or not.
fn protect(gateway: &FakeGateway, path: &str, cimd: bool, tokens: Arc<Mutex<Vec<String>>>) {
    let base = gateway.url.clone();
    let issuer = if cimd {
        format!("{base}/cimd")
    } else {
        base.clone()
    };
    let resource = format!("{base}{path}");
    let challenge = format!(
        "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource{path}\", scope=\"mcp.read\""
    );
    let accepted = tokens.clone();
    gateway.route(path, move |request| {
        let token = request.header("authorization").unwrap_or_default();
        let valid = accepted
            .lock()
            .unwrap()
            .iter()
            .any(|t| token == format!("Bearer {t}"));
        if !valid {
            return reply(
                401,
                Value::Null,
                vec![("www-authenticate", challenge.clone())],
            );
        }
        answer(request, "s")
    });
    let (servers, metadata) = (
        issuer.clone(),
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{issuer}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
            "token_endpoint_auth_methods_supported": ["none"],
            "client_id_metadata_document_supported": cimd,
        }),
    );
    gateway.route(
        &format!("/.well-known/oauth-protected-resource{path}"),
        move |_| {
            reply(
                200,
                json!({"resource": resource, "authorization_servers": [servers]}),
                Vec::new(),
            )
        },
    );
    let well_known = if cimd {
        "/.well-known/oauth-authorization-server/cimd"
    } else {
        "/.well-known/oauth-authorization-server"
    };
    gateway.route(well_known, move |_| {
        reply(200, metadata.clone(), Vec::new())
    });
}

fn form(body: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

#[test]
fn oauth_sign_in_storage_and_refresh() {
    let gateway = FakeGateway::start(|request, _| text(request, "unused"));
    let tokens = Arc::new(Mutex::new(vec!["tok-1".to_string()]));
    protect(&gateway, "/mcp", false, tokens.clone());
    protect(&gateway, "/mcp2", true, tokens.clone());
    gateway.route("/register", |request| {
        let mut client = request.json();
        client["client_id"] = json!("dyn-client-1");
        reply(201, client, Vec::new())
    });
    let refreshes = Arc::new(AtomicUsize::new(0));
    let counter = refreshes.clone();
    gateway.route("/token", move |request| {
        let fields = form(&request.body);
        let grant = fields.iter().find(|(k, _)| k == "grant_type").unwrap().1.clone();
        let (access, refresh) = if grant == "refresh_token" {
            counter.fetch_add(1, Ordering::SeqCst);
            ("tok-2", "ref-2")
        } else {
            ("tok-1", "ref-1")
        };
        reply(
            200,
            json!({"access_token": access, "refresh_token": refresh, "token_type": "Bearer", "expires_in": 3600}),
            Vec::new(),
        )
    });
    let env = env(&gateway.url);
    let url = format!("{}/mcp", gateway.url);
    let url2 = format!("{}/mcp2", gateway.url);
    env.home.write(
        "proj/opencode.json",
        &json!({"mcp": {"servers": {
            "oauth1": {"type": "remote", "url": url},
            "oauth2": {"type": "remote", "url": url2},
        }}})
        .to_string(),
    );
    let rt = runtime();
    let (core, mcp) = start(&env, &rt);
    assert_eq!(
        status(&mcp, "oauth1"),
        Status::NeedsAuth {
            error: format!("MCP server \"{url}\" requires a login before it can register a client")
        }
    );

    let attempt = rt.block_on(mcp.authorize("oauth1")).unwrap();
    assert_eq!(
        attempt.instructions,
        "Authorize oauth1 in your browser. This window will close automatically."
    );
    let authorize = url::Url::parse(&attempt.url).unwrap();
    let params: Vec<(String, String)> = authorize
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let keys: Vec<&str> = params.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "response_type",
            "client_id",
            "code_challenge",
            "code_challenge_method",
            "redirect_uri",
            "state",
            "scope",
            "resource"
        ]
    );
    let param = |key: &str| params.iter().find(|(k, _)| k == key).unwrap().1.clone();
    assert_eq!(param("client_id"), "dyn-client-1");
    assert_eq!(param("scope"), "mcp.read");
    assert_eq!(param("resource"), url);
    let redirect = param("redirect_uri");
    assert!(redirect.starts_with("http://127.0.0.1:") && redirect.ends_with("/callback"));
    assert_eq!(
        gateway.requests_to("/register")[0].body,
        format!(
            r#"{{"redirect_uris":["{redirect}"],"client_name":"Anysola Code","client_uri":"https://www.anysola.com/code","grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none","application_type":"native","scope":"mcp.read"}}"#
        )
    );
    let pages = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/verbatim/integrations/"
    );
    let page = |name: &str| std::fs::read_to_string(format!("{pages}{name}")).unwrap();
    let get = |target: String| {
        rt.block_on(async {
            let response = reqwest::get(target).await.unwrap();
            (response.status().as_u16(), response.text().await.unwrap())
        })
    };
    let origin = redirect.trim_end_matches("/callback").to_string();
    assert_eq!(
        get(format!("{origin}/elsewhere")),
        (404, "Not found".into())
    );
    assert_eq!(
        get(redirect.clone()),
        (400, page("oauth-callback-error-missing-code.html"))
    );
    let callback = format!("{redirect}?code=c1&state={}", param("state"));
    assert_eq!(get(callback), (200, page("oauth-callback-success.html")));
    assert_eq!(rt.block_on(attempt.done).unwrap(), Ok(()));

    let integration = oauth::integration_id("oauth1", &url);
    let (_, stored) = core.db().active_credential(&integration).unwrap().unwrap();
    assert!(stored["expires"].as_u64().unwrap() > 0);
    let mut shape = stored.clone();
    shape["expires"] = json!(0);
    shape["metadata"]["client"] = json!(shape["metadata"]["client"]["client_id"]);
    assert_eq!(
        shape,
        json!({"type": "oauth", "methodID": integration, "refresh": "ref-1", "access": "tok-1", "expires": 0,
               "metadata": {"serverUrl": url, "tokenType": "Bearer", "issuer": gateway.url, "client": "dyn-client-1"}})
    );
    let token = gateway.requests_to("/token")[0].body.clone();
    assert!(token.starts_with("grant_type=authorization_code&code=c1&code_verifier="));

    // The server now takes only the rotated token: concurrent calls refresh once.
    rt.block_on(mcp.settled(Duration::from_secs(20)));
    assert_eq!(status(&mcp, "oauth1"), Status::Connected);
    *tokens.lock().unwrap() = vec!["tok-2".to_string()];
    let calls = rt.block_on(async {
        futures_util::future::join_all(
            (0..3).map(|_| mcp.call_tool("oauth1", "echo", json!({}), "ses_x")),
        )
        .await
    });
    assert!(calls.iter().all(Result::is_ok), "{calls:?}");
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    let (_, rotated) = core.db().active_credential(&integration).unwrap().unwrap();
    assert_eq!(
        (rotated["access"].clone(), rotated["refresh"].clone()),
        (json!("tok-2"), json!("ref-2"))
    );

    // CIMD: the hosted client document stands in for registration; a wrong state fails the attempt.
    let registrations = gateway.requests_to("/register").len();
    let attempt = rt.block_on(mcp.authorize("oauth2")).unwrap();
    let authorize = url::Url::parse(&attempt.url).unwrap();
    let param = |key: &str| {
        authorize
            .query_pairs()
            .find(|(k, _)| k == key)
            .unwrap()
            .1
            .into_owned()
    };
    assert_eq!(param("client_id"), oauth::CLIENT_METADATA_URL);
    assert_eq!(gateway.requests_to("/register").len(), registrations);
    let (status, _) = get(format!("{}?code=c2&state=wrong", param("redirect_uri")));
    assert_eq!(status, 400);
    assert_eq!(
        rt.block_on(attempt.done).unwrap(),
        Err("OAuth state mismatch".to_string())
    );
    rt.block_on(mcp.shutdown());
}

/// A server for the 2026-07-28 revision answers `server/discover` and takes no initialize; a
/// 2025 server answers it with "method not found".
fn era(request: &Recorded, modern: bool) -> Reply {
    let message = request.json();
    let id = message["id"].clone();
    let method = message["method"].as_str().unwrap_or_default();
    let result = match (method, modern) {
        ("server/discover", true) => json!({
            "supportedVersions": ["2026-07-28"],
            "capabilities": {"tools": {}},
            "instructions": "Modern server.",
        }),
        ("server/discover", false) => {
            return reply(
                200,
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "Method not found"}}),
                Vec::new(),
            );
        }
        _ => return answer(request, ""),
    };
    reply(
        200,
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Vec::new(),
    )
}

#[cfg(not(windows))]
#[test]
fn protocol_auto_and_pinned_negotiate_with_server_discover() {
    let gateway = FakeGateway::start(|request, _| text(request, "unused"));
    gateway.route("/modern", |request| era(request, true));
    gateway.route("/legacy", |request| era(request, false));
    gateway.route("/pinned", |request| era(request, false));
    let env = env(&gateway.url);
    let server = |path: &str, protocol: &str| json!({"type": "remote", "url": format!("{}{path}", gateway.url), "oauth": false, "protocol": protocol});
    env.home.write(
        "proj/opencode.json",
        &json!({"mcp": {"servers": {
            "modern": server("/modern", "auto"),
            "legacy": server("/legacy", "auto"),
            "pinned": server("/pinned", "2026-07-28"),
        }}})
        .to_string(),
    );
    let rt = runtime();
    let (_core, mcp) = start(&env, &rt);
    assert_eq!(status(&mcp, "modern"), Status::Connected);
    assert_eq!(status(&mcp, "legacy"), Status::Connected);
    assert_eq!(
        status(&mcp, "pinned"),
        Status::Failed {
            error: "Version negotiation failed: the server did not offer pinned protocol version 2026-07-28 via server/discover (no fallback in pin mode)".into()
        }
    );
    rt.block_on(mcp.call_tool("modern", "echo", json!({}), "ses_x"))
        .unwrap();
    rt.block_on(mcp.shutdown());

    let methods = |path: &str| -> Vec<String> {
        gateway
            .requests_to(path)
            .iter()
            .filter(|r| r.method == "POST")
            .map(|r| r.json()["method"].as_str().unwrap_or_default().to_string())
            .collect()
    };
    let modern = gateway.requests_to("/modern");
    assert_eq!(methods("/modern")[0], "server/discover");
    assert!(!methods("/modern").contains(&"initialize".to_string()));
    let probe = modern[0].json();
    assert_eq!(
        probe["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    assert_eq!(
        probe["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
        json!({"elicitation": {"form": {}, "url": {}}, "roots": {}})
    );
    let call = modern
        .iter()
        .find(|r| r.json()["method"] == "tools/call")
        .unwrap();
    assert_eq!(call.header("mcp-protocol-version"), Some("2026-07-28"));
    assert_eq!(call.header("mcp-method"), Some("tools/call"));
    assert_eq!(call.header("mcp-name"), Some("echo"));
    assert_eq!(
        call.json()["params"]["_meta"]["io.modelcontextprotocol/clientInfo"]["name"],
        "opencode"
    );

    assert_eq!(methods("/legacy")[..2], ["server/discover", "initialize"]);
    let initialize = gateway.requests_to("/legacy")[1].json();
    assert_eq!(initialize["params"]["protocolVersion"], "2025-11-25");
    assert!(initialize["params"].get("_meta").is_none());
    assert_eq!(methods("/pinned"), ["server/discover"]);
}
