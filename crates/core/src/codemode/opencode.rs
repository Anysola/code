//! The `opencode` namespace: tools for managing Anysola Code itself (sessions, worktrees, models,
//! MCP resources). Descriptions and schemas are 0.5.1's, from `opencode.json`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use super::{Called, Namespace, NamespaceTool};
use crate::permission::PermissionRequest;
use crate::tool::{ToolContext, ToolError, access};

pub const NAME: &str = "opencode";
const DESCRIPTION: &str = "Tools for managing OpenCode itself, such as working with sessions and worktrees, searching the available models, and reading MCP resources.";

#[derive(Clone, Copy)]
enum Kind {
    SessionRename,
    SessionMove,
    WorktreeCreate,
    WorktreeList,
    WorktreeRemove,
    Models,
    ListMcpResources,
    ReadMcpResource,
}

struct Tool {
    kind: Kind,
    name: String,
    description: String,
    input: Value,
    output: Value,
    pinned: bool,
    permission: Option<String>,
}

fn definitions() -> &'static [Value] {
    static DEFS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFS.get_or_init(|| {
        serde_json::from_str(include_str!("opencode.json")).expect("embedded opencode tools")
    })
}

/// The namespace with its eight tools in 0.5.1's registration order.
pub fn namespace() -> Arc<Namespace> {
    let kinds = [
        Kind::SessionRename,
        Kind::SessionMove,
        Kind::WorktreeCreate,
        Kind::WorktreeList,
        Kind::WorktreeRemove,
        Kind::Models,
        Kind::ListMcpResources,
        Kind::ReadMcpResource,
    ];
    let tools = kinds
        .into_iter()
        .zip(definitions())
        .map(|(kind, def)| {
            let text = |key: &str| def[key].as_str().unwrap_or_default().to_string();
            Arc::new(Tool {
                kind,
                name: text("name"),
                description: text("description"),
                input: def["input_schema"].clone(),
                output: def["output_schema"].clone(),
                pinned: def["pinned"] == true,
                permission: def["permission"].as_str().map(str::to_string),
            }) as Arc<dyn NamespaceTool>
        })
        .collect();
    Arc::new(Namespace {
        name: NAME.into(),
        description: Some(DESCRIPTION.into()),
        tools,
    })
}

impl NamespaceTool for Tool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> &Value {
        &self.input
    }

    fn output_schema(&self) -> Option<&Value> {
        Some(&self.output)
    }

    fn pinned(&self) -> bool {
        self.pinned
    }

    fn permission(&self) -> Option<&str> {
        self.permission.as_deref()
    }

    fn call<'a>(
        &'a self,
        input: Value,
        ctx: &'a ToolContext,
    ) -> BoxFuture<'a, Result<Called, ToolError>> {
        async move {
            match self.kind {
                Kind::SessionRename => rename(input, ctx),
                Kind::SessionMove => move_session(input, ctx),
                Kind::WorktreeCreate => worktree_create(self, input, ctx).await,
                Kind::WorktreeList => Ok(worktree_list(ctx)),
                Kind::WorktreeRemove => worktree_remove(input, ctx).await,
                Kind::Models => Ok(models(input, ctx)),
                Kind::ListMcpResources => list_mcp_resources(input, ctx).await,
                Kind::ReadMcpResource => read_mcp_resource(input, ctx).await,
            }
        }
        .boxed()
    }
}

fn text<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn session_id(input: &Value, ctx: &ToolContext) -> String {
    text(input, "sessionID").map_or_else(|| ctx.session_id.clone(), str::to_string)
}

fn rename(input: Value, ctx: &ToolContext) -> Result<Called, ToolError> {
    let id = session_id(&input, ctx);
    let title = text(&input, "title").unwrap_or_default().trim().to_string();
    if title.is_empty() {
        return Err(ToolError::execution("Session title must not be empty"));
    }
    let db = ctx.host.db();
    let renamed = matches!(db.session(&id), Ok(Some(_))) && db.set_title(&id, &title).is_ok();
    if !renamed {
        return Err(ToolError::execution(format!(
            "Unable to rename session {id}"
        )));
    }
    Ok(Called::output(json!({"sessionID": id, "title": title}))
        .with_text(format!("Renamed session {id} to {title}.")))
}

/// Moves the session's working directory; a running turn picks it up at its next step.
fn move_session(input: Value, ctx: &ToolContext) -> Result<Called, ToolError> {
    let id = session_id(&input, ctx);
    let requested = text(&input, "directory").unwrap_or_default().to_string();
    let failure = || ToolError::execution(format!("Unable to move session to {requested}"));
    let db = ctx.host.db();
    let session = db.session(&id).ok().flatten().ok_or_else(failure)?;
    let value = requested.trim();
    let expanded = if value == "~" || value.starts_with("~/") {
        ctx.paths.expand_home(value)
    } else {
        PathBuf::from(value)
    };
    let destination = crate::paths::normalize(&Path::new(&session.directory).join(expanded));
    if !destination.is_dir() {
        return Err(failure());
    }
    let directory = destination.display().to_string();
    db.set_directory(&id, &directory).map_err(|_| failure())?;
    ctx.notifier.synthetic(
        &id,
        format!("The working directory has been changed to {directory}."),
    );
    Ok(
        Called::output(json!({"sessionID": id, "directory": requested}))
            .with_text(format!("Moved session {id} to {requested}.")),
    )
}

async fn worktree_create(
    tool: &Tool,
    input: Value,
    ctx: &ToolContext,
) -> Result<Called, ToolError> {
    let raw = text(&input, "name").unwrap_or_default();
    let name = raw.trim();
    if name.is_empty() {
        return Err(ToolError::execution(crate::tool::input::format_issues(
            &super::direct_name(NAME, &tool.name),
            &[(
                "name".into(),
                "Expected a value with a length of at least 1".into(),
            )],
            &input,
        )));
    }
    let created = crate::worktree::create(
        ctx.host.db(),
        &worktree_base(ctx),
        &ctx.project_id,
        &ctx.worktree,
        name,
    )
    .await
    .map_err(|_| ToolError::execution(format!("Unable to create worktree {name}")))?;
    let directory = created.display().to_string();
    Ok(Called::output(json!({"directory": directory}))
        .with_text(format!("Created worktree in {directory}.")))
}

/// Config `worktree.directory` (`~/` is home, relative is the project root), else the data dir.
fn worktree_base(ctx: &ToolContext) -> PathBuf {
    let config = ctx.host.config();
    let configured = config
        .get("worktree")
        .and_then(|w| w.get("directory"))
        .and_then(Value::as_str);
    match configured {
        Some(directory) => match directory.strip_prefix("~/") {
            Some(rest) => ctx.paths.home.join(rest),
            None => crate::paths::normalize(&ctx.worktree.join(directory)),
        },
        None => ctx
            .paths
            .data
            .join("worktree")
            .join(ctx.project_id.chars().take(6).collect::<String>()),
    }
}

fn worktree_list(ctx: &ToolContext) -> Called {
    let worktrees: Vec<Value> = crate::worktree::list(ctx.host.db(), &ctx.project_id)
        .into_iter()
        .map(|w| match w.strategy {
            Some(strategy) => json!({"directory": w.directory, "strategy": strategy}),
            None => json!({"directory": w.directory}),
        })
        .collect();
    Called::output(json!({"worktrees": worktrees}))
}

async fn worktree_remove(input: Value, ctx: &ToolContext) -> Result<Called, ToolError> {
    let requested = text(&input, "directory").unwrap_or_default();
    let target = access::resolve(ctx, requested, Some(access::Kind::Directory));
    // Removing a worktree deletes files, so it needs the same approval as editing them.
    ctx.ask(PermissionRequest {
        action: "edit".into(),
        resources: vec![target.resource.clone()],
        save: vec!["*".into()],
        metadata: Value::Null,
    })
    .await?;
    let shown = target.absolute.display().to_string();
    let force = input.get("force").and_then(Value::as_bool).unwrap_or(false);
    crate::worktree::remove(ctx.host.db(), &ctx.project_id, &target.absolute, force)
        .await
        .map_err(|error| {
            use crate::worktree::RemoveError;
            ToolError::execution(match error {
                RemoveError::Unavailable | RemoveError::Dirty => {
                    format!("Unable to remove worktree {shown}: {error}.")
                }
                RemoveError::NotWorktree => format!(
                    "Unable to remove {shown}: {error}. Use worktree_list to see the project's worktrees."
                ),
                RemoveError::Failed(_) => format!("Unable to remove worktree {shown}."),
            })
        })?;
    Ok(Called::output(json!({"directory": shown, "removed": true}))
        .with_text(format!("Removed worktree {shown}.")))
}

fn models(input: Value, ctx: &ToolContext) -> Called {
    let offset = input.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let terms: Vec<String> = text(&input, "query")
        .map(|q| {
            q.to_lowercase()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let provider = text(&input, "provider").map(str::to_lowercase);
    let provider_name = "Anysola";
    // Anysola serves only chat models.
    let chat = text(&input, "type").is_none_or(|t| t == "chat");
    let models = ctx.host.models();
    let matching: Vec<&crate::model::Model> = models
        .iter()
        .filter(|_| chat)
        .filter(|_| {
            provider
                .as_deref()
                .is_none_or(|p| p == crate::model::PROVIDER || p == provider_name.to_lowercase())
        })
        .filter(|m| {
            let text = format!("{}/{} {}", crate::model::PROVIDER, m.id, m.name).to_lowercase();
            terms.iter().all(|t| text.contains(t.as_str()))
        })
        .collect();
    let page: Vec<Value> = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|m| {
            json!({
                "id": format!("{}/{}", crate::model::PROVIDER, m.id),
                "name": m.name,
                "released": 0,
                "variants": m.variants.iter().map(|v| v.id.clone()).collect::<Vec<_>>(),
                "cost": [],
                "status": "active",
            })
        })
        .collect();
    let providers = if page.is_empty() {
        Vec::new()
    } else {
        vec![json!({"id": crate::model::PROVIDER, "name": provider_name, "models": page})]
    };
    let next = if offset + limit < matching.len() {
        json!(offset + limit)
    } else {
        Value::Null
    };
    Called::output(json!({"providers": providers, "total": matching.len(), "next": next}))
}

// M5: MCP servers; until then there are none, so every server name is unknown.
fn servers() -> Vec<String> {
    Vec::new()
}

async fn list_mcp_resources(input: Value, ctx: &ToolContext) -> Result<Called, ToolError> {
    let servers = match text(&input, "server") {
        Some(server) => vec![server.to_string()],
        None => servers(),
    };
    ctx.ask(PermissionRequest {
        action: "opencode_list_mcp_resources".into(),
        resources: servers.clone(),
        save: servers,
        metadata: json!({}),
    })
    .await?;
    if let Some(server) = text(&input, "server") {
        return Err(ToolError::execution(format!(
            "MCP server not found: {server}"
        )));
    }
    Ok(Called::output(json!({"resources": [], "templates": []})))
}

async fn read_mcp_resource(input: Value, ctx: &ToolContext) -> Result<Called, ToolError> {
    let server = text(&input, "server").unwrap_or_default();
    let uri = text(&input, "uri").unwrap_or_default();
    ctx.ask(PermissionRequest {
        action: "opencode_read_mcp_resource".into(),
        resources: vec![format!("{server}:{uri}")],
        save: vec![format!("{server}:*")],
        metadata: json!({"server": server, "uri": uri}),
    })
    .await?;
    Err(ToolError::execution(format!(
        "MCP server not found: {server}"
    )))
}
