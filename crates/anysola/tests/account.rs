//! Account commands against the fake gateway: the device login, logout, status, usage and devices,
//! their error texts and headers, and the bare `anysola` checks that precede the interactive UI.

mod common;

use anysola_core::storage::Db;
use anysola_testkit::{DEVICE_TOKEN, FakeGateway, Output, Reply, TestHome};
use common::*;
use serde_json::{Value, json};

const STORED: &str = "asc_SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS";

fn no_messages(_: &anysola_testkit::Recorded, _: usize) -> Reply {
    unreachable!("account commands make no model calls")
}

/// A home with no token at all, dates in UTC.
fn setup() -> (FakeGateway, TestHome) {
    let gateway = FakeGateway::start(no_messages);
    let mut home = TestHome::new(&gateway.url);
    home.remove_env("ANYSOLA_TOKEN");
    home.set_env("TZ", "UTC");
    (gateway, home)
}

fn db(home: &TestHome) -> Db {
    Db::open(&home.home().join(".anysola/anysola.db")).unwrap()
}

fn store_login(home: &TestHome) {
    let value = json!({"type": "oauth", "methodID": "device", "refresh": "", "access": STORED,
        "expires": 1, "metadata": {"apiURL": "x", "device": "y"}});
    db(home).insert_anysola_login(&value).unwrap();
}

fn stored(home: &TestHome) -> Option<String> {
    db(home).anysola_login().unwrap()
}

fn anysola(home: &TestHome, args: &[&str]) -> Output {
    home.run(BIN, args, None, None)
}

fn json_reply(status: u16, body: Value) -> Reply {
    Reply::Json {
        status,
        body,
        headers: Vec::new(),
    }
}

/// The lines of one `## <title>` section of the account capture, comments dropped.
fn capture(title: &str) -> String {
    let text = std::fs::read_to_string(spec("cli/account-commands.txt")).unwrap();
    let section = text
        .split("\n## ")
        .find(|s| s.starts_with(title))
        .unwrap_or_else(|| panic!("no section {title}"));
    section
        .lines()
        .skip(1)
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn framed(lines: &[&str]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push_str("│\n");
        }
        out.push_str(line);
        out.push('\n');
    }
    out + "\n"
}

#[test]
fn device_login_matches_the_capture() {
    let (gateway, home) = setup();
    let out = anysola(&home, &["login"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, capture("anysola login (signed out") + "\n");
    assert_eq!(out.stderr, "");
    assert_eq!(stored(&home).as_deref(), Some(DEVICE_TOKEN));
    let row: (String, Option<String>, String) = rusqlite_row(&home);
    assert_eq!(row.0, "Anysola account");
    assert!(row.1.is_none());
    let value: Value = serde_json::from_str(&row.2).unwrap();
    assert_eq!(value["type"], "oauth");
    assert_eq!(value["methodID"], "device");
    assert_eq!(value["refresh"], "");
    assert_eq!(value["expires"], 1_825_286_400_000_i64);
    assert_eq!(value["metadata"]["apiURL"], gateway.url.as_str());

    let start = &gateway.requests_to("/v1/code/device/code")[0];
    assert_eq!(start.method, "POST");
    assert_eq!(
        start.json()["client"],
        format!("anysola-code/{}", env!("CARGO_PKG_VERSION"))
    );
    assert!(start.json()["deviceName"].as_str().is_some());
    assert_eq!(start.header("content-type"), Some("application/json"));
    assert_eq!(start.header("authorization"), None);
    let polls = gateway.requests_to("/v1/code/device/token");
    assert_eq!(polls.len(), 2);
    assert_eq!(polls[0].json(), json!({"device_code": "D".repeat(43)}));
    // Sleep first: the first poll waits one interval after the start.
    assert!(polls[0].at.duration_since(start.at).as_millis() >= 1000);
}

/// `(label, method_id, value)` of the stored credential.
fn rusqlite_row(home: &TestHome) -> (String, Option<String>, String) {
    let conn = rusqlite::Connection::open(home.home().join(".anysola/anysola.db")).unwrap();
    conn.query_row("SELECT label, method_id, value FROM credential", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .unwrap()
}

#[test]
fn slow_down_adds_five_seconds() {
    let (gateway, home) = setup();
    gateway.set_device_poll(|index| match index {
        0 => json_reply(400, json!({"error": "slow_down", "interval": 30})),
        _ => json_reply(200, json!({"access_token": DEVICE_TOKEN})),
    });
    let out = anysola(&home, &["login"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    let polls = gateway.requests_to("/v1/code/device/token");
    let gap = polls[1].at.duration_since(polls[0].at).as_millis();
    assert!((6000..9000).contains(&gap), "{gap}");
}

fn login_failure(poll: impl Fn(usize) -> Reply + Send + Sync + 'static, message: &str) {
    let (gateway, home) = setup();
    gateway.set_device_poll(poll);
    let out = anysola(&home, &["login"]);
    assert_eq!(out.code, 1);
    let expected = framed(&[
        "┌  Sign in to Anysola",
        "◇  Authorization started",
        "●  Open https://account.anysola.com/device and enter WXYZ-1234",
        "●  https://account.anysola.com/device",
        "◇  Waiting for authorization...",
        &format!("■  {message}"),
        "└  Failed",
    ]);
    assert_eq!(out.stdout, expected);
    assert_eq!(stored(&home), None);
}

#[test]
fn expired_code() {
    login_failure(
        |_| json_reply(400, json!({"error": "expired_token"})),
        "The code expired before it was approved. Run `anysola login` again.",
    );
}

#[test]
fn denied() {
    login_failure(
        |_| json_reply(400, json!({"error": "access_denied"})),
        "Login was denied.",
    );
}

#[test]
fn other_poll_error() {
    login_failure(
        |_| json_reply(400, json!({"error": "invalid_grant"})),
        "Login failed: invalid_grant",
    );
}

#[test]
fn malformed_token() {
    login_failure(
        |_| json_reply(200, json!({"access_token": "asc_not-valid"})),
        "Login failed: the server returned no usable token.",
    );
}

#[test]
fn three_server_errors_abort() {
    login_failure(
        |_| {
            json_reply(
                503,
                json!({"error": {"type": "overloaded_error", "message": "Busy."}}),
            )
        },
        "Login failed: Busy.",
    );
}

#[test]
fn rate_limited_start() {
    let (gateway, home) = setup();
    gateway.set_reply(
        "/v1/code/device/code",
        json_reply(429, json!({"error": "slow_down"})),
    );
    let out = anysola(&home, &["login"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stdout,
        framed(&[
            "┌  Sign in to Anysola",
            "■  Authentication failed",
            "■  Too many login attempts. Try again later.",
            "└  Failed",
        ])
    );
}

#[test]
fn unusable_start() {
    let (gateway, home) = setup();
    gateway.set_reply(
        "/v1/code/device/code",
        json_reply(200, json!({"user_code": "X"})),
    );
    let out = anysola(&home, &["login"]);
    assert!(
        out.stdout
            .contains("■  Could not start login: unexpected response from the server.\n")
    );
    gateway.set_reply(
        "/v1/code/device/code",
        json_reply(400, json!({"error": "invalid_request"})),
    );
    let out = anysola(&home, &["login"]);
    assert!(
        out.stdout
            .contains("■  Could not start login: invalid_request\n")
    );
}

#[test]
fn already_signed_in() {
    let (gateway, home) = setup();
    store_login(&home);
    let out = anysola(&home, &["login"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        capture("anysola login (already signed in)").replace("FFFF...FFFF", "SSSS...SSSS") + "\n"
    );
    assert!(gateway.requests().is_empty());
}

#[test]
fn force_revokes_the_old_token_first() {
    let (gateway, home) = setup();
    store_login(&home);
    let out = anysola(&home, &["login", "-f"]);
    assert_eq!(out.code, 0, "{}", out.stdout);
    assert_eq!(out.stdout, capture("anysola login (signed out") + "\n");
    let revoke = &gateway.requests_to("/v1/code/tokens/self/revoke")[0];
    assert_eq!(
        revoke.header("authorization"),
        Some(&*format!("Bearer {STORED}"))
    );
    assert_eq!(stored(&home).as_deref(), Some(DEVICE_TOKEN));

    store_login(&home);
    gateway.set_reply("/v1/code/tokens/self/revoke", json_reply(500, json!({})));
    let out = anysola(&home, &["login", "--force"]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains(
        "│\n▲  Could not revoke the old token (HTTP 500); revoke it at https://account.anysola.com/apps\n│\n◇  Authorization started\n"
    ), "{}", out.stdout);
}

#[test]
fn env_token_login_and_logout() {
    let (gateway, mut home) = setup();
    home.set_env("ANYSOLA_TOKEN", "asc_env");
    let out = anysola(&home, &["login"]);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (
            0,
            framed(&[
                "┌  Sign in to Anysola",
                "●  Signed in through ANYSOLA_TOKEN; unset it to use a device login.",
                "└  Done",
            ])
            .as_str()
        )
    );
    let out = anysola(&home, &["logout"]);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (
            0,
            framed(&[
                "┌  Sign out of Anysola",
                "●  Signed in through ANYSOLA_TOKEN; unset it to sign out.",
                "└  Done",
            ])
            .as_str()
        )
    );
    assert!(gateway.requests().is_empty());

    store_login(&home);
    let out = anysola(&home, &["logout"]);
    assert_eq!(
        (out.code, out.stdout.as_str()),
        (
            0,
            framed(&[
                "┌  Sign out of Anysola",
                "●  ANYSOLA_TOKEN is still set and stays in use; unset it to sign out.",
                "└  Signed out. The token is revoked and removed from this machine.",
            ])
            .as_str()
        )
    );
    let revoke = &gateway.requests_to("/v1/code/tokens/self/revoke")[0];
    assert_eq!(
        revoke.header("authorization"),
        Some(&*format!("Bearer {STORED}"))
    );
    assert_eq!(stored(&home), None);
}

#[test]
fn logout_outcomes() {
    let (gateway, home) = setup();
    let out = anysola(&home, &["logout"]);
    assert_eq!(
        out.stdout,
        framed(&["┌  Sign out of Anysola", "●  Not signed in.", "└  Done"])
    );

    store_login(&home);
    let out = anysola(&home, &["logout"]);
    assert_eq!(
        (out.code, out.stdout),
        (0, capture("anysola logout") + "\n")
    );
    assert_eq!(stored(&home), None);

    store_login(&home);
    gateway.set_reply("/v1/code/tokens/self/revoke", json_reply(401, json!({})));
    assert_eq!(anysola(&home, &["logout"]).code, 0);
    assert_eq!(stored(&home), None);

    store_login(&home);
    gateway.set_reply("/v1/code/tokens/self/revoke", json_reply(502, json!({})));
    let out = anysola(&home, &["logout"]);
    assert_eq!(out.code, 1);
    assert_eq!(
        out.stdout,
        "┌  Sign out of Anysola\n│\n■  Could not revoke the token: HTTP 502\n│\n●  The token was kept. Try again, or revoke it at https://account.anysola.com/apps.\n"
    );
    assert_eq!(stored(&home).as_deref(), Some(STORED));
}

#[test]
fn status_shows_the_real_me_shape() {
    let (gateway, home) = setup();
    let out = anysola(&home, &["status"]);
    assert_eq!(
        (out.code, out.stderr.as_str()),
        (1, "Not signed in. Run `anysola login`.\n")
    );

    store_login(&home);
    let out = anysola(&home, &["status"]);
    assert_eq!(out.code, 0);
    assert_eq!(
        out.stdout,
        "Plan     Team\nCredits  731 available, renews 2026-11-23\nToday    3 requests · 9 credits\nToken    asc_SSSS...SSSS\n"
    );
    let me = &gateway.requests_to("/v1/code/me")[0];
    assert_eq!(
        me.header("authorization"),
        Some(&*format!("Bearer {STORED}"))
    );
    assert_eq!(me.header("accept"), Some("application/json"));
    assert_eq!(
        me.header("x-anysola-version"),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(me.header("user-agent"), Some("opencode/latest/2.2.5/cli"));
    assert_eq!(me.header("x-anysola-client"), None);

    gateway.set_reply("/v1/code/me", json_reply(200, json!({"plan": "free", "access": "free", "credits": null,
        "freeQuota": {"usedUsd": 0.01, "dailyUsd": 0.25, "turns": 3, "dailyTurns": 30, "resetsAt": "2026-11-05T00:00:00Z"},
        "usageToday": {"requests": 1234, "credits": 0},
        "event": {"mode": "unlimited", "label": "Launch week", "until": "2026-11-10T15:30:00Z"}})));
    let out = anysola(&home, &["status"]);
    assert_eq!(
        out.stdout,
        "Plan        Free\nFree quota  3/30 turns today\nToday       1,234 requests · 0 credits\nEvent       Launch week until Nov 10, 03:30 PM\nToken       asc_SSSS...SSSS\n"
    );

    gateway.set_reply("/v1/code/me", json_reply(500, json!({})));
    let out = anysola(&home, &["status"]);
    assert_eq!(out.code, 0);
    assert_eq!(out.stderr, "Could not load status: HTTP 500\n");
    assert_eq!(out.stdout, "Token  asc_SSSS...SSSS\n");

    gateway.set_reply("/v1/code/me", json_reply(401, json!({})));
    let out = anysola(&home, &["status"]);
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (
            1,
            "",
            "The token was revoked or expired. Run `anysola login --force`.\n"
        )
    );
}

#[test]
fn env_token_wins_with_a_stored_login() {
    let (gateway, mut home) = setup();
    store_login(&home);
    home.set_env("ANYSOLA_TOKEN", "x");
    let out = anysola(&home, &["status"]);
    assert!(out.stdout.ends_with("Token    x...x\n"), "{}", out.stdout);
    let me = &gateway.requests_to("/v1/code/me")[0];
    assert_eq!(me.header("authorization"), Some("Bearer x"));
}

#[test]
fn usage_tables_and_json() {
    let (gateway, home) = setup();
    store_login(&home);
    let out = anysola(&home, &["usage"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let expected = capture("anysola usage").replace("Nov 23, 03:00 AM", "Nov 23, 12:00 AM");
    assert_eq!(out.stdout, expected);
    assert_eq!(gateway.requests_to("/v1/code/usage")[0].query, "days=7");

    let out = anysola(&home, &["usage", "--days", "30", "--format", "json"]);
    assert_eq!(gateway.requests_to("/v1/code/usage")[1].query, "days=30");
    assert_eq!(
        out.stdout,
        serde_json::to_string_pretty(&anysola_testkit::default_usage()).unwrap() + "\n"
    );
    assert!(
        out.stdout
            .starts_with("{\n  \"days\": 7,\n  \"totals\": {\n    \"requests\": 42,")
    );
}

#[test]
fn devices_and_revoke() {
    let (gateway, home) = setup();
    store_login(&home);
    let out = anysola(&home, &["devices"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let expected = capture("anysola devices").replace("01:00 PM", "10:00 AM");
    assert_eq!(out.stdout, expected);

    let out = anysola(&home, &["devices", "revoke", "tok_2"]);
    assert_eq!(
        (out.code, out.stdout),
        (0, capture("anysola devices revoke tok_2"))
    );
    let out = anysola(&home, &["devices", "revoke", "a b/c"]);
    assert_eq!(out.code, 0);
    assert!(
        gateway
            .requests()
            .iter()
            .any(|r| r.path == "/v1/code/tokens/a%20b%2Fc/revoke")
    );

    gateway.set_reply(
        "/v1/code/tokens/nope/revoke",
        json_reply(404, json!({"type": "error", "error": {"type": "not_found_error", "message": "No such token."}})),
    );
    let out = anysola(&home, &["devices", "revoke", "nope"]);
    assert_eq!(
        (out.code, out.stderr.as_str()),
        (
            1,
            "No signed-in device with ID nope. Run `anysola devices` to list them.\n"
        )
    );

    gateway.set_reply("/v1/code/tokens", json_reply(200, json!({"tokens": []})));
    assert_eq!(
        anysola(&home, &["devices"]).stdout,
        "No devices are signed in.\n"
    );
    gateway.set_reply("/v1/code/tokens", json_reply(404, json!({})));
    assert_eq!(
        anysola(&home, &["devices"]).stderr,
        "This Anysola server does not offer this yet.\n"
    );
    gateway.set_reply(
        "/v1/code/tokens",
        json_reply(
            403,
            json!({"type": "error", "error": {"type": "permission_error", "message": "Nope."}}),
        ),
    );
    assert_eq!(anysola(&home, &["devices"]).stderr, "Nope.\n");
    gateway.set_reply("/v1/code/tokens", json_reply(500, json!({})));
    assert_eq!(
        anysola(&home, &["devices"]).stderr,
        "Request failed (HTTP 500)\n"
    );
    gateway.set_reply("/v1/code/tokens", json_reply(401, json!({})));
    let out = anysola(&home, &["devices"]);
    assert_eq!(
        (out.code, out.stderr.as_str()),
        (
            1,
            "The token was revoked or expired. Run `anysola login --force`.\n"
        )
    );
}

#[test]
fn unreachable_gateway_and_no_login() {
    let (_gateway, mut home) = setup();
    for args in [&["usage"][..], &["devices"], &["devices", "revoke", "x"]] {
        let out = anysola(&home, args);
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (1, "Not signed in. Run `anysola login`.\n")
        );
    }
    store_login(&home);
    home.set_env("ANYSOLA_API_URL", "http://127.0.0.1:9");
    for args in [&["usage"][..], &["devices"], &["status"]] {
        let out = anysola(&home, args);
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (1, "Cannot reach http://127.0.0.1:9\n"),
            "{args:?}"
        );
    }
}

#[test]
fn bare_command_checks_before_the_ui() {
    let (_gateway, home) = setup();
    let out = anysola(&home, &["no-such-dir"]);
    assert_eq!(
        (out.code, out.stdout.as_str(), out.stderr.as_str()),
        (1, "", "Error: No such directory: no-such-dir\n")
    );
    for args in [&[][..], &["."], &["-c"]] {
        let out = anysola(&home, args);
        assert_eq!(
            (out.code, out.stderr.as_str()),
            (
                1,
                "Error: The interactive UI needs a terminal. Use `anysola run` for non-interactive use.\n"
            )
        );
    }
}
