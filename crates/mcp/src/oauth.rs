//! OAuth for remote servers: stored tokens on connect (refreshed on a 401, never a browser), and
//! the sign-in attempt with a loopback callback, discovery, client registration (configured
//! client, CIMD, else DCR) and PKCE.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anysola_core::mcp::SignIn;
use anysola_core::storage::Db;
use base64::Engine;
use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use sha1::Digest;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::http::Auth;

/// The client metadata document Anysola hosts for CIMD.
pub const CLIENT_METADATA_URL: &str = "https://www.anysola.com/oauth/anysola-code/client.json";
const PAGE: &str = include_str!("callback.html");
const CLOSE_SCRIPT: &str =
    "    <script>setTimeout(function(){try{window.close()}catch(e){}},2500)</script>\n";

/// The integration a remote server's credentials belong to: keyed on name and URL, so two names
/// for one URL keep separate accounts.
pub fn integration_id(name: &str, url: &str) -> String {
    let digest = sha1::Sha1::digest(format!("{name}\0{url}").as_bytes());
    format!("mcp_{}", &hex::encode(digest)[..16])
}

/// Whether a configured server signs in with OAuth (remote, `oauth` not `false`).
pub fn capable(server: &Value) -> bool {
    server["type"] == "remote" && server["url"].is_string() && server["oauth"] != json!(false)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn success_page(provider: &str) -> String {
    PAGE.replace("{{HEADLINE}}", "Authorization successful")
        .replace("{{STATUS}}", "success")
        .replace(
            "{{MESSAGE}}",
            &format!("Anysola Code is now connected to {}.", escape(provider)),
        )
        .replace("{{DETAIL}}", " hidden>")
        .replace("{{FOOTNOTE}}", "You can close this window.")
        .replace("{{SCRIPT}}", CLOSE_SCRIPT)
}

pub fn error_page(detail: &str, provider: &str) -> String {
    let detail = detail.trim();
    PAGE.replace("{{HEADLINE}}", "Authorization failed")
        .replace("{{STATUS}}", "error")
        .replace(
            "{{MESSAGE}}",
            &format!(
                "Anysola Code couldn't finish connecting to {}.",
                escape(provider)
            ),
        )
        .replace(
            "{{DETAIL}}",
            &if detail.is_empty() {
                " hidden>".to_string()
            } else {
                format!(">{}", escape(detail))
            },
        )
        .replace(
            "{{FOOTNOTE}}",
            "Close this window and try again from Anysola Code.",
        )
        .replace("{{SCRIPT}}", "")
}

/// Where a server's authorization server lives and what it supports.
struct Discovery {
    metadata: Value,
    resource: String,
}

async fn body_json(response: reqwest::Response) -> Result<Value, String> {
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

async fn get_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let response = client
        .get(url)
        .header("accept", "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    body_json(response).await.ok()
}

fn well_known(base: &url::Url, name: &str) -> Vec<String> {
    let origin = base.origin().ascii_serialization();
    let path = base.path().trim_end_matches('/');
    if path.is_empty() {
        vec![format!("{origin}/.well-known/{name}")]
    } else {
        vec![
            format!("{origin}/.well-known/{name}{path}"),
            format!("{origin}/.well-known/{name}"),
        ]
    }
}

/// The configured URL without its fragment: the identity tokens are bound to.
fn identity(url: &str) -> String {
    url::Url::parse(url).map_or_else(
        |_| url.to_string(),
        |mut u| {
            u.set_fragment(None);
            u.to_string()
        },
    )
}

/// `auth_server_metadata_url` when configured, else RFC 9728 then RFC 8414 discovery.
async fn discover(
    client: &reqwest::Client,
    server: &Value,
    resource_metadata: Option<String>,
) -> Result<Discovery, String> {
    let url = server["url"].as_str().unwrap_or_default();
    let configured = identity(url);
    if let Some(metadata_url) = server["oauth"]["auth_server_metadata_url"].as_str() {
        let response = client
            .get(metadata_url)
            .header("accept", "application/json")
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "HTTP {} trying to load OAuth authorization server metadata",
                response.status().as_u16()
            ));
        }
        let metadata = body_json(response).await?;
        return Ok(Discovery {
            metadata,
            resource: configured,
        });
    }
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    let mut candidates: Vec<String> = resource_metadata.into_iter().collect();
    candidates.extend(well_known(&parsed, "oauth-protected-resource"));
    let mut protected = None;
    for candidate in candidates {
        if let Some(found) = get_json(client, &candidate).await {
            protected = Some(found);
            break;
        }
    }
    let issuer = protected
        .as_ref()
        .and_then(|p| p["authorization_servers"][0].as_str())
        .map_or_else(|| parsed.origin().ascii_serialization(), str::to_string);
    let resource = match protected.as_ref().and_then(|p| p["resource"].as_str()) {
        None => configured.clone(),
        Some(resource) => {
            let (a, b) = (
                url::Url::parse(resource).ok(),
                url::Url::parse(&configured).ok(),
            );
            match (a, b) {
                (Some(a), Some(b)) if a.origin() == b.origin() && a.path() == b.path() => {
                    configured.clone()
                }
                (Some(a), Some(b))
                    if a.origin() == b.origin()
                        && b.path().starts_with(a.path().trim_end_matches('/')) =>
                {
                    resource.to_string()
                }
                _ => {
                    return Err(format!(
                        "Protected resource {resource} does not cover {configured}"
                    ));
                }
            }
        }
    };
    let issuer_url = url::Url::parse(&issuer).map_err(|e| e.to_string())?;
    let mut candidates = well_known(&issuer_url, "oauth-authorization-server");
    candidates.truncate(1);
    candidates.extend(
        well_known(&issuer_url, "openid-configuration")
            .into_iter()
            .take(1),
    );
    let path = issuer_url.path().trim_end_matches('/');
    if !path.is_empty() {
        candidates.push(format!(
            "{}{path}/.well-known/openid-configuration",
            issuer_url.origin().ascii_serialization()
        ));
    }
    let mut metadata = None;
    for candidate in &candidates {
        if let Some(found) = get_json(client, candidate).await {
            metadata = Some(found);
            break;
        }
    }
    // Without metadata the SDK falls back to the conventional endpoints on the issuer.
    let metadata = metadata.unwrap_or_else(|| {
        let base = issuer.trim_end_matches('/');
        json!({
            "issuer": issuer,
            "authorization_endpoint": format!("{base}/authorize"),
            "token_endpoint": format!("{base}/token"),
            "registration_endpoint": format!("{base}/register"),
        })
    });
    Ok(Discovery { metadata, resource })
}

async fn token_request(
    client: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<Value, String> {
    let response = client
        .post(endpoint)
        .header("accept", "application/json")
        .form(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body: Value = body_json(response).await.unwrap_or(Value::Null);
    if !status.is_success() || !body["access_token"].is_string() {
        let message = body["error_description"]
            .as_str()
            .or(body["error"].as_str())
            .map_or_else(|| format!("HTTP {}", status.as_u16()), str::to_string);
        return Err(message);
    }
    Ok(body)
}

/// The stored credential value 0.5.1 writes for a token response.
fn credential(
    integration: &str,
    server_url: &str,
    tokens: &Value,
    issuer: Option<&str>,
    client: Option<&Value>,
) -> Value {
    let expires = tokens["expires_in"]
        .as_u64()
        .filter(|s| *s > 0)
        .map_or(0, |s| anysola_core::time::now_ms() + s * 1000);
    let mut metadata = json!({
        "serverUrl": server_url,
        "tokenType": tokens["token_type"].as_str().unwrap_or("Bearer"),
    });
    if let Some(scope) = tokens["scope"].as_str() {
        metadata["scope"] = json!(scope);
    }
    if let Some(issuer) = issuer {
        metadata["issuer"] = json!(issuer);
    }
    if let Some(client) = client {
        metadata["client"] = client.clone();
    }
    json!({
        "type": "oauth",
        "methodID": integration,
        "refresh": tokens["refresh_token"].as_str().unwrap_or_default(),
        "access": tokens["access_token"],
        "expires": expires,
        "metadata": metadata,
    })
}

/// The client fields a token request sends.
fn client_fields(client: &Value) -> Vec<(&'static str, String)> {
    let mut fields = vec![(
        "client_id",
        client["client_id"].as_str().unwrap_or_default().to_string(),
    )];
    if let Some(secret) = client["client_secret"].as_str() {
        fields.push(("client_secret", secret.to_string()));
    }
    fields
}

/// One lock per integration, so concurrent refreshes of one token in this process coalesce.
fn refresh_lock(integration: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(integration.to_string())
        .or_default()
        .clone()
}

/// Stored tokens of one server for its connections.
pub struct Provider {
    db: Arc<Db>,
    integration: String,
    server: Value,
    client: reqwest::Client,
}

impl Provider {
    pub fn new(db: Arc<Db>, name: &str, server: &Value) -> Provider {
        let url = server["url"].as_str().unwrap_or_default();
        Provider {
            db,
            integration: integration_id(name, url),
            server: server.clone(),
            client: crate::http::client(),
        }
    }

    fn url(&self) -> &str {
        self.server["url"].as_str().unwrap_or_default()
    }

    fn refuse(&self) -> String {
        let what = if self.server["oauth"]["client_id"].is_string()
            || self
                .stored()
                .is_some_and(|(_, v)| v["metadata"]["client"].is_object())
        {
            "user authorization"
        } else {
            "a login before it can register a client"
        };
        format!("MCP server \"{}\" requires {what}", self.url())
    }

    fn stored(&self) -> Option<(String, Value)> {
        self.db
            .active_credential(&self.integration)
            .ok()
            .flatten()
            .filter(|(_, v)| v["type"] == "oauth")
    }

    async fn refresh(&self, presented: &str) -> Result<(), String> {
        let lock = refresh_lock(&self.integration);
        let _held = lock.lock().await;
        let Some((id, stored)) = self.stored() else {
            return Err(self.refuse());
        };
        // Someone else already rotated it.
        if stored["access"].as_str() != Some(presented) {
            return Ok(());
        }
        let refresh = stored["refresh"].as_str().unwrap_or_default().to_string();
        if refresh.is_empty() {
            return Err(self.refuse());
        }
        let resource = identity(self.url());
        let attempt = async {
            let discovery = discover(&self.client, &self.server, None).await?;
            let endpoint = discovery.metadata["token_endpoint"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let client = stored["metadata"]["client"].clone();
            let mut form: Vec<(&str, String)> = vec![
                ("grant_type", "refresh_token".into()),
                ("refresh_token", refresh.clone()),
            ];
            form.extend(client_fields(&client));
            form.push(("resource", resource.clone()));
            let borrowed: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let mut tokens = token_request(&self.client, &endpoint, &borrowed).await?;
            if !tokens["refresh_token"].is_string() {
                tokens["refresh_token"] = json!(refresh);
            }
            Ok::<_, String>(credential(
                &self.integration,
                self.url(),
                &tokens,
                stored["metadata"]["issuer"].as_str(),
                client.is_object().then_some(&client),
            ))
        };
        match attempt.await {
            Ok(value) => self
                .db
                .update_credential(&id, &value)
                .map_err(|e| e.to_string()),
            Err(_) => {
                // Only drop the row while it still holds the refresh token this attempt used.
                let _ = self.db.remove_credential_holding(&id, &refresh);
                Err(self.refuse())
            }
        }
    }
}

impl Auth for Provider {
    fn token(&self) -> BoxFuture<'_, Option<String>> {
        let access = self
            .stored()
            .and_then(|(_, v)| v["access"].as_str().map(str::to_string));
        async move { access }.boxed()
    }

    fn rejected<'a>(&'a self, token: Option<&'a str>) -> BoxFuture<'a, Result<(), String>> {
        async move {
            match token {
                Some(token) => self.refresh(token).await,
                None => Err(self.refuse()),
            }
        }
        .boxed()
    }
}

fn random_base64url(len: usize) -> String {
    let bytes: Vec<u8> = (0..len).map(|_| rand::random::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `resource_metadata` and `scope` of the server's 401 challenge.
async fn challenge(client: &reqwest::Client, server: &Value) -> (Option<String>, Option<String>) {
    let mut request = client
        .post(server["url"].as_str().unwrap_or_default())
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"opencode"}}}"#);
    if let Some(headers) = server["headers"].as_object() {
        for (key, value) in headers {
            if let Some(value) = value.as_str() {
                request = request.header(key, value);
            }
        }
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), request.send()).await;
    let Ok(Ok(response)) = response else {
        return (None, None);
    };
    let header = response
        .headers()
        .get("www-authenticate")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let param = |name: &str| {
        let start = header.find(&format!("{name}=\""))? + name.len() + 2;
        let end = header[start..].find('"')?;
        Some(header[start..start + end].to_string())
    };
    (param("resource_metadata"), param("scope"))
}

/// The client a sign-in uses: configured, CIMD, an earlier registration, else a new DCR one.
async fn register(
    client: &reqwest::Client,
    server: &Value,
    metadata: &Value,
    redirect: &str,
    scope: Option<&str>,
    previous: Option<Value>,
) -> Result<Value, String> {
    let oauth = &server["oauth"];
    if let Some(id) = oauth["client_id"].as_str() {
        let mut client = json!({"client_id": id});
        if let Some(secret) = oauth["client_secret"].as_str() {
            client["client_secret"] = json!(secret);
        }
        return Ok(client);
    }
    let cimd = !oauth["redirect_uri"].is_string()
        && metadata["client_id_metadata_document_supported"] == json!(true)
        && metadata["token_endpoint_auth_methods_supported"]
            .as_array()
            .is_some_and(|m| m.iter().any(|v| v == "none"));
    if cimd {
        return Ok(json!({"client_id": CLIENT_METADATA_URL}));
    }
    if let Some(previous) = previous.filter(|p| {
        p["redirect_uris"]
            .as_array()
            .is_some_and(|r| r.iter().any(|u| u == redirect))
    }) {
        return Ok(previous);
    }
    let endpoint = metadata["registration_endpoint"]
        .as_str()
        .ok_or("Incompatible auth server: does not support dynamic client registration")?;
    let mut body = json!({
        "redirect_uris": [redirect],
        "client_name": "Anysola Code",
        "client_uri": "https://www.anysola.com/code",
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "application_type": "native",
    });
    if let Some(scope) = scope {
        body["scope"] = json!(scope);
    }
    let response = client
        .post(endpoint)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Dynamic client registration failed: HTTP {}",
            response.status().as_u16()
        ));
    }
    body_json(response).await
}

/// Starts signing in to `name`: the callback server listens, the client is registered, and the
/// returned URL waits for the browser. On success the new credential replaces the old ones.
pub async fn authorize(db: Arc<Db>, name: &str, server: &Value) -> Result<SignIn, String> {
    let client = crate::http::client();
    let url = server["url"].as_str().unwrap_or_default().to_string();
    let integration = integration_id(name, &url);
    let oauth = &server["oauth"];
    let configured_redirect = oauth["redirect_uri"]
        .as_str()
        .and_then(|r| url::Url::parse(r).ok());
    let path = configured_redirect
        .as_ref()
        .map_or("/callback".to_string(), |r| r.path().to_string());
    let port = oauth["callback_port"]
        .as_u64()
        .or_else(|| {
            configured_redirect
                .as_ref()
                .and_then(|r| r.port())
                .map(u64::from)
        })
        .unwrap_or(0);
    let listener = TcpListener::bind(("127.0.0.1", port as u16))
        .await
        .map_err(|e| e.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not determine MCP OAuth callback port".to_string())?
        .port();
    let redirect = oauth["redirect_uri"]
        .as_str()
        .map_or_else(|| format!("http://127.0.0.1:{port}{path}"), str::to_string);
    let (resource_metadata, challenge_scope) = challenge(&client, server).await;
    let discovery = discover(&client, server, resource_metadata).await?;
    let scope = oauth["scope"]
        .as_str()
        .map(str::to_string)
        .or(challenge_scope);
    let previous = db
        .active_credential(&integration)
        .ok()
        .flatten()
        .map(|(_, v)| v["metadata"]["client"].clone())
        .filter(Value::is_object);
    let registered = register(
        &client,
        server,
        &discovery.metadata,
        &redirect,
        scope.as_deref(),
        previous,
    )
    .await?;
    let authorization_endpoint = discovery.metadata["authorization_endpoint"]
        .as_str()
        .unwrap_or_default();
    let mut authorization = url::Url::parse(authorization_endpoint).map_err(|e| e.to_string())?;
    if !matches!(authorization.scheme(), "http" | "https") {
        return Err(format!(
            "MCP server \"{url}\" returned a {}: authorization URL; only http and https are supported",
            authorization.scheme()
        ));
    }
    let verifier = random_base64url(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    let state = random_base64url(32);
    {
        let mut query = authorization.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair(
                "client_id",
                registered["client_id"].as_str().unwrap_or_default(),
            )
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("redirect_uri", &redirect)
            .append_pair("state", &state);
        if let Some(scope) = &scope {
            query.append_pair("scope", scope);
        }
        query.append_pair("resource", &discovery.resource);
    }
    let (sender, done) = oneshot::channel();
    let exchange = Exchange {
        client,
        db,
        name: name.to_string(),
        url,
        integration,
        token_endpoint: discovery.metadata["token_endpoint"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        issuer: discovery.metadata["issuer"].as_str().map(str::to_string),
        registered,
        verifier,
        redirect,
        resource: discovery.resource,
    };
    tokio::spawn(async move {
        let _ = sender.send(serve(listener, &path, &state, &exchange).await);
    });
    Ok(SignIn {
        url: authorization.to_string(),
        instructions: format!(
            "Authorize {name} in your browser. This window will close automatically."
        ),
        done,
    })
}

/// What the code exchange needs once the callback arrives.
struct Exchange {
    client: reqwest::Client,
    db: Arc<Db>,
    name: String,
    url: String,
    integration: String,
    token_endpoint: String,
    issuer: Option<String>,
    registered: Value,
    verifier: String,
    redirect: String,
    resource: String,
}

impl Exchange {
    async fn run(&self, code: &str) -> Result<(), String> {
        let mut form: Vec<(&str, String)> = vec![
            ("grant_type", "authorization_code".into()),
            ("code", code.to_string()),
            ("code_verifier", self.verifier.clone()),
            ("redirect_uri", self.redirect.clone()),
        ];
        form.extend(client_fields(&self.registered));
        form.push(("resource", self.resource.clone()));
        let borrowed: Vec<(&str, &str)> = form.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let tokens = token_request(&self.client, &self.token_endpoint, &borrowed)
            .await
            .map_err(|_| format!("MCP server \"{}\" did not return OAuth tokens", self.name))?;
        let value = credential(
            &self.integration,
            &self.url,
            &tokens,
            self.issuer.as_deref(),
            Some(&self.registered),
        );
        self.db
            .remove_integration_credentials(&self.integration)
            .map_err(|e| e.to_string())?;
        self.db
            .insert_credential(&self.integration, &self.name, &value)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

async fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// The request target of one HTTP request.
async fn target(stream: &mut TcpStream) -> Option<url::Url> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 64 * 1024 {
        let n = stream.read(&mut buffer).await.ok()?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buffer[..n]);
    }
    let line = String::from_utf8_lossy(&head);
    let path = line.lines().next()?.split(' ').nth(1)?;
    url::Url::parse(&format!("http://127.0.0.1{path}")).ok()
}

/// Answers callbacks until one ends the attempt.
async fn serve(
    listener: TcpListener,
    path: &str,
    state: &str,
    exchange: &Exchange,
) -> Result<(), String> {
    let provider = exchange.name.as_str();
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return Err("Could not determine MCP OAuth callback port".into());
        };
        let Some(url) = target(&mut stream).await else {
            continue;
        };
        if url.path() != path {
            respond(&mut stream, "404 Not Found", "text/plain", "Not found").await;
            continue;
        }
        let params: Map<String, Value> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), json!(v)))
            .collect();
        let text = |key: &str| params.get(key).and_then(Value::as_str);
        let fail = |reason: &str| error_page(reason, provider);
        if let Some(error) = text("error_description").or(text("error")) {
            respond(&mut stream, "400 Bad Request", "text/html", &fail(error)).await;
            return Err(error.to_string());
        }
        let code = text("code").filter(|c| !c.is_empty());
        // Browsers reach the callback without parameters too (prefetches, reloads); only a
        // redirect from the authorization server ends the attempt.
        if text("state").is_none() && code.is_none() {
            let page = fail("Missing authorization code");
            respond(&mut stream, "400 Bad Request", "text/html", &page).await;
            continue;
        }
        let reason = if text("state") != Some(state) {
            "OAuth state mismatch"
        } else if code.is_none() {
            "Missing authorization code"
        } else {
            ""
        };
        if !reason.is_empty() {
            respond(&mut stream, "400 Bad Request", "text/html", &fail(reason)).await;
            return Err(reason.to_string());
        }
        // The page waits for the token exchange so the browser never reports success for a
        // rejected code.
        let result = exchange.run(code.unwrap_or_default()).await;
        match &result {
            Ok(()) => respond(&mut stream, "200 OK", "text/html", &success_page(provider)).await,
            Err(error) => respond(&mut stream, "400 Bad Request", "text/html", &fail(error)).await,
        }
        return result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_match_the_captures() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../spec/verbatim/integrations/"
        );
        let read = |name: &str| std::fs::read_to_string(format!("{dir}{name}")).unwrap();
        assert_eq!(success_page("oauth1"), read("oauth-callback-success.html"));
        assert_eq!(
            error_page("Missing authorization code", "oauth1"),
            read("oauth-callback-error-missing-code.html")
        );
    }

    #[test]
    fn integration_ids_hash_name_and_url() {
        assert_eq!(
            integration_id("oauth1", "http://127.0.0.1:8852/mcp"),
            "mcp_f97485b09636c533"
        );
    }
}
