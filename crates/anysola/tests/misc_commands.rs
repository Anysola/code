//! `models`, `debug paths|config` and `mcp add|logout|list|auth` against temp homes and testkit,
//! compared with 0.5.1's outputs (`V/cli/other-commands.txt`, `V/integrations/strings.txt` and the
//! `mcp add` files 0.5.1 wrote, kept in `fixtures/mcp-add`).

mod common;

use std::path::Path;

use anysola_core::storage::Db;
use anysola_testkit::{FakeGateway, Output, Reply, TestHome};
use common::BIN;
use serde_json::{Value, json};

fn no_messages(_: &anysola_testkit::Recorded, _: usize) -> Reply {
    Reply::Json {
        status: 500,
        body: json!({}),
        headers: Vec::new(),
    }
}

fn anysola(home: &TestHome, cwd: &Path, args: &[&str]) -> Output {
    home.run(BIN, args, Some(cwd), None)
}

#[test]
fn models_lists_the_gateway_models_and_survives_offline() {
    let gateway = FakeGateway::start(no_messages);
    let mut home = TestHome::new(&gateway.url);
    let expected = "anysola/auto\nanysola/claude-haiku-5-5\nanysola/claude-sonnet-5-5\n";
    let out = anysola(&home, &home.project(), &["models"]);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        (expected, 0),
        "{}",
        out.stderr
    );
    home.set_env("ANYSOLA_API_URL", "http://127.0.0.1:9");
    let out = anysola(&home, &home.project(), &["models"]);
    assert_eq!(out.stdout, expected, "the cached /config answers offline");

    let fresh = TestHome::new("http://127.0.0.1:9");
    let out = anysola(&fresh, &fresh.project(), &["models"]);
    assert_eq!((out.stdout.as_str(), out.code), ("", 0));
}

#[test]
fn debug_paths_point_into_the_data_dir() {
    let home = TestHome::new("http://127.0.0.1:9");
    let h = home.home().display().to_string();
    let tmp = std::env::temp_dir().join("opencode").display().to_string();
    let out = anysola(&home, &home.project(), &["debug", "paths"]);
    let expected = format!(
        "home       {h}\ndata       {h}/.anysola\ncache      {h}/.anysola/cache\nconfig     {h}/.anysola\n\
         state      {h}/.anysola/state\ntmp        {tmp}\nbin        {h}/.anysola/bin\nlog        {h}/.anysola/log\n\
         repos      {h}/.anysola/repos\ndb         {h}/.anysola/anysola.db\n"
    );
    assert_eq!((out.stdout.as_str(), out.code), (expected.as_str(), 0));
    let out = anysola(&home, &home.project(), &["debug", "paths", "db"]);
    assert_eq!(out.stdout, format!("{h}/.anysola/anysola.db\n"));
}

#[test]
fn debug_config_lists_sources_with_secrets_hidden() {
    let home = TestHome::new("http://127.0.0.1:9");
    std::fs::create_dir_all(home.home().join(".config/opencode")).unwrap();
    home.write(
        "proj/opencode.jsonc",
        r#"{
  // c
  "theme": "dark",
  "mcp": {"servers": {"x": {"type": "remote", "url": "https://u:p@h/x", "headers": {"A": "b"}},
                      "y": {"type": "local", "command": ["y"], "environment": {"API_KEY": "k", "MODE": "m"}}}}
}"#,
    );
    let out = anysola(&home, &home.project(), &["debug", "config"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let h = home.home().display().to_string();
    let expected = json!([
        {"type": "directory", "path": format!("{h}/.config/opencode")},
        {"type": "directory", "path": format!("{h}/.anysola")},
        {"type": "document", "path": format!("{h}/proj/opencode.jsonc"), "info": {
            "theme": "dark",
            "mcp": {"servers": {
                "x": {"type": "remote", "url": "***", "headers": {"A": "***"}},
                "y": {"type": "local", "command": ["y"], "environment": {"API_KEY": "***", "MODE": "m"}}
            }}
        }}
    ]);
    assert_eq!(
        out.stdout,
        format!("{}\n", anysola_core::json::pretty(&expected))
    );
}

fn fixture(name: &str) -> Option<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/mcp-add/{name}"));
    std::fs::read_to_string(path).ok()
}

#[test]
fn mcp_add_writes_the_bytes_0_5_1_wrote() {
    let remote = ["--url", "https://x/y", "--header", "K=V"];
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "new-file",
            "opencode.json",
            &[
                "ctx",
                "--",
                "npx",
                "-y",
                "@upstash/context7-mcp",
                "--api-key",
                "K",
            ],
        ),
        (
            "comments",
            "opencode.jsonc",
            &[
                "web",
                "--url",
                "https://mcp.example.com/mcp",
                "--header",
                "Authorization=Bearer x",
                "--header",
                "X-A=1",
            ],
        ),
        (
            "empty",
            "opencode.json",
            &["e", "--env", "A=1", "--env", "B=2", "--", "node", "s.js"],
        ),
        ("compact", "opencode.json", &["e", "--", "node"]),
        (
            "tabs",
            ".opencode/opencode.json",
            &["e", "--url", "http://h/x"],
        ),
        ("replace", "opencode.json", &["e", "--url", "http://new/x"]),
        (
            "crlf",
            "opencode.jsonc",
            &["n", remote[0], remote[1], remote[2], remote[3]],
        ),
        (
            "header-comment",
            "opencode.jsonc",
            &["n", remote[0], remote[1], remote[2], remote[3]],
        ),
        (
            "arrays",
            "opencode.jsonc",
            &["n", remote[0], remote[1], remote[2], remote[3]],
        ),
        (
            "blank",
            "opencode.jsonc",
            &["n", remote[0], remote[1], remote[2], remote[3]],
        ),
    ];
    for (name, file, args) in cases {
        let home = TestHome::new("http://127.0.0.1:9");
        if let Some(input) = fixture(&format!("{name}.in")) {
            home.write(&format!("proj/{file}"), &input);
        }
        let mut argv = vec!["mcp", "add"];
        argv.extend_from_slice(args);
        let out = anysola(&home, &home.project(), &argv);
        let path = home.project().join(file);
        assert_eq!(
            (out.stdout, out.code),
            (
                format!("MCP server \"{}\" added to {}\n", args[0], path.display()),
                0
            ),
            "{name}: {}",
            out.stderr
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            fixture(&format!("{name}.out")).unwrap(),
            "{name}"
        );
    }

    let home = TestHome::new("http://127.0.0.1:9");
    let out = anysola(
        &home,
        &home.project(),
        &["mcp", "add", "g", "--global", "--url", "http://g/x"],
    );
    let global = home.home().join(".anysola/config.json");
    assert_eq!(
        out.stdout,
        format!("MCP server \"g\" added to {}\n", global.display())
    );
    let written: Value = serde_json::from_str(&std::fs::read_to_string(global).unwrap()).unwrap();
    assert_eq!(
        written,
        json!({"mcp": {"servers": {"g": {"type": "remote", "url": "http://g/x"}}}})
    );
}

#[test]
fn mcp_add_validation() {
    let home = TestHome::new("http://127.0.0.1:9");
    let cases: &[(&[&str], &str)] = &[
        (&["--url", "nope"], "Invalid URL: nope"),
        (&[], "Provide either --url <url> or a command after --"),
        (
            &["--url", "http://x", "--", "y"],
            "Provide either --url <url> or a command after --, not both",
        ),
        (
            &["--url", "http://x", "--env", "A=1"],
            "--env is only valid for local MCP servers",
        ),
        (
            &["--header", "A=1", "--", "y"],
            "--header is only valid for remote MCP servers",
        ),
    ];
    for (args, message) in cases {
        let mut argv = vec!["mcp", "add", "e"];
        argv.extend_from_slice(args);
        let out = anysola(&home, &home.project(), &argv);
        assert_eq!(
            (out.stderr, out.code),
            (format!("Error: {message}\n"), 1),
            "{args:?}"
        );
    }
    home.write("proj/opencode.json", "[1]");
    let out = anysola(
        &home,
        &home.project(),
        &["mcp", "add", "e", "--url", "http://x"],
    );
    assert_eq!(
        out.stderr,
        "Error: Can not add index to parent of type array\n"
    );
    assert_eq!(
        std::fs::read_to_string(home.project().join("opencode.json")).unwrap(),
        "[1]"
    );
    let out = anysola(
        &home,
        &home.project(),
        &["mcp", "add", "e", "--header", "noeq", "--url", "http://x"],
    );
    assert!(out.stderr.ends_with(
        "\nERROR\n  Invalid value for flag --header: \"noeq\". Expected: Invalid key=value format. Expected format: key=value, got: noeq\n"
    ));
}

#[test]
fn mcp_logout_removes_the_servers_credentials() {
    let home = TestHome::new("http://127.0.0.1:9");
    home.write(
        "proj/opencode.json",
        r#"{"mcp": {"servers": {
            "remote": {"type": "remote", "url": "https://mcp.example.com/mcp"},
            "anonymous": {"type": "remote", "url": "https://a/mcp", "oauth": false},
            "local": {"type": "local", "command": ["x"]}}}}"#,
    );
    let db = Db::open(&home.home().join(".anysola/anysola.db")).unwrap();
    let token = json!({"type": "oauth", "methodID": "mcp_47cdc06a49fb1e78", "access": "a", "refresh": "r", "expires": 0});
    db.insert_credential("mcp_47cdc06a49fb1e78", "remote", &token)
        .unwrap();
    db.insert_credential("mcp_47cdc06a49fb1e78", "remote", &token)
        .unwrap();
    drop(db);
    let project = home.project();
    let logout = |name: &str| anysola(&home, &project, &["mcp", "logout", name]);
    let out = logout("remote");
    assert_eq!(
        (out.stdout.as_str(), out.code),
        ("Removed OAuth credentials for remote\n", 0)
    );
    let out = logout("remote");
    assert_eq!(out.stdout, "No stored credentials for remote\n");
    for name in ["anonymous", "local"] {
        assert_eq!(
            logout(name).stdout,
            format!("No stored credentials for {name}\n")
        );
    }
    let out = logout("nope");
    assert_eq!(
        (out.stderr.as_str(), out.code),
        ("Error: MCP server not found: nope\n", 1)
    );
}

#[test]
fn mcp_list_shows_live_statuses_and_auth_refuses_without_a_server() {
    let home = TestHome::new("http://127.0.0.1:9");
    let project = home.project();
    let out = anysola(&home, &project, &["mcp", "list"]);
    assert_eq!(out.stdout, "No MCP servers configured\n");
    home.write(
        "proj/opencode.json",
        r#"{"mcp": {"servers": {
            "glob1": {"type": "local", "command": ["echo", "hi"]},
            "bad1": {"type": "local", "command": ["/nonexistent/cmd", "--flag"]},
            "off": {"type": "local", "command": ["x"], "disabled": true}}}}"#,
    );
    let out = anysola(&home, &project, &["mcp", "list"]);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        (
            "✗ bad1   failed: NotFound: ChildProcess.spawn (/nonexistent/cmd --flag)\n\
             ✗ glob1  failed: Connection closed: MCP server process exited with code 0\n\
             ○ off    disabled\n",
            0
        )
    );
    let out = anysola(&home, &project, &["mcp", "auth"]);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        (
            "│\n■  Pass an MCP server name when running without an interactive terminal\n│\n└  Failed\n\n",
            1
        )
    );
    let out = anysola(&home, &project, &["mcp", "auth", "glob1"]);
    assert_eq!(
        (out.stdout.as_str(), out.code),
        (
            "┌  Authenticate an MCP server\n│\n■  MCP server \"glob1\" is not an OAuth-capable remote server\n│\n└  Failed\n\n",
            1
        )
    );
}
