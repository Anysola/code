//! The set of tools and the tool array of a request: direct tools sorted by name, then `execute`
//! (Code Mode) last. A tool is left out when the agent's rules wholly deny its permission action.

use std::sync::{Arc, OnceLock, RwLock};

use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolDefinition};
use crate::codemode::execute::{DirectTool, ExecuteTool};
use crate::codemode::{Inventory, Namespace, opencode};
use crate::permission::{Rule, wholly_disabled};

/// The verbatim description and input schema of a built-in tool, from `defs/<name>.json`.
pub struct Def {
    pub description: String,
    pub input_schema: Value,
}

pub fn parse_def(json: &str) -> Def {
    let value: Value = serde_json::from_str(json).expect("embedded tool definition is valid JSON");
    Def {
        description: value["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        input_schema: value["input_schema"].clone(),
    }
}

/// Declares a lazily parsed embedded definition: `embedded_def!(DEF, "read")`.
#[macro_export]
macro_rules! embedded_def {
    ($name:ident, $file:literal) => {
        fn $name() -> &'static $crate::tool::registry::Def {
            static DEF: std::sync::OnceLock<$crate::tool::registry::Def> =
                std::sync::OnceLock::new();
            DEF.get_or_init(|| {
                $crate::tool::registry::parse_def(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/src/tool/defs/",
                    $file,
                    ".json"
                )))
            })
        }
    };
}

pub struct Registry {
    tools: Vec<Arc<dyn Tool>>,
    /// Code Mode namespaces; MCP servers add theirs as they connect.
    namespaces: RwLock<Vec<Arc<Namespace>>>,
}

impl Registry {
    /// The built-in tools; `todowrite` only with config `experimental.todo`.
    pub fn builtin() -> Self {
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(super::edit::EditTool),
            Arc::new(super::glob::GlobTool),
            Arc::new(super::grep::GrepTool),
            Arc::new(super::question::QuestionTool),
            Arc::new(super::read::ReadTool),
            Arc::new(super::shell::ShellTool),
            Arc::new(super::skill::SkillTool),
            Arc::new(super::subagent::SubagentTool),
            Arc::new(super::webfetch::WebfetchTool),
            Arc::new(super::websearch::WebsearchTool),
            Arc::new(super::write::WriteTool),
            Arc::new(super::todowrite::TodowriteTool),
        ];
        Registry {
            tools,
            namespaces: RwLock::new(vec![opencode::namespace_with_schedules()]),
        }
    }

    /// Adds a namespace, replacing one with the same name.
    pub fn set_namespace(&self, namespace: Arc<Namespace>) {
        let mut namespaces = self.namespaces.write().unwrap_or_else(|e| e.into_inner());
        namespaces.retain(|n| n.name != namespace.name);
        namespaces.push(namespace);
    }

    pub fn remove_namespace(&self, name: &str) {
        let mut namespaces = self.namespaces.write().unwrap_or_else(|e| e.into_inner());
        namespaces.retain(|n| n.name != name);
    }

    /// The Code Mode tools an agent with these rules can reach; `None` when `execute` is denied.
    pub fn code_mode(&self, rules: &[Rule]) -> Option<Inventory> {
        self.code_mode_gated(rules, Gates::default())
    }

    pub fn code_mode_gated(&self, rules: &[Rule], gates: Gates) -> Option<Inventory> {
        (!wholly_disabled("execute", rules)).then(|| self.inventory(false, gates).visible(rules))
    }

    /// The Code Mode namespaces (`direct: false`) or the direct ones, without gated-off tools.
    fn inventory(&self, direct: bool, gates: Gates) -> Inventory {
        Inventory::new(
            self.namespaces
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|n| n.direct == direct)
                .map(|n| gates.apply(n))
                .collect(),
        )
    }

    /// Tools advertised to an agent with these rules, in request order.
    pub fn active(&self, rules: &[Rule]) -> Vec<Arc<dyn Tool>> {
        self.active_gated(rules, Gates::default())
    }

    pub fn active_gated(&self, rules: &[Rule], gates: Gates) -> Vec<Arc<dyn Tool>> {
        let enabled = |tool: &Arc<dyn Tool>| {
            // 0.5.1 advertises `skill` even to agents whose rules end in `* deny` (explore); the
            // skill tool still asks `skill:<id>` per call.
            tool.name() == "skill" || !wholly_disabled(tool.permission(), rules)
        };
        let mut direct: Vec<_> = self.tools.iter().filter(|t| enabled(t)).cloned().collect();
        let code_mode = self.code_mode_gated(rules, gates);
        // Without `execute` the namespace tools would be unreachable, so they become direct.
        let mut exposed = self
            .inventory(true, gates)
            .visible(rules)
            .namespaces()
            .to_vec();
        if code_mode.is_none() {
            exposed.extend(
                self.inventory(false, gates)
                    .visible(rules)
                    .namespaces()
                    .to_vec(),
            );
        }
        for namespace in &exposed {
            for tool in &namespace.tools {
                direct.push(Arc::new(DirectTool::new(namespace.clone(), tool.clone())));
            }
        }
        direct.sort_by(|a, b| a.name().cmp(b.name()));
        if let Some(inventory) = code_mode {
            direct.push(Arc::new(ExecuteTool::new(inventory)));
        }
        direct
    }

    pub fn definitions(tools: &[Arc<dyn Tool>], env: &DefinitionEnv) -> Vec<ToolDefinition> {
        tools.iter().map(|t| t.definition(env)).collect()
    }
}

/// Process-start year for the websearch description.
/// Config flags that add tools to a namespace.
#[derive(Debug, Clone, Copy, Default)]
pub struct Gates {
    /// `experimental.schedules`: the `schedule_*` tools of the `opencode` namespace.
    pub schedules: bool,
}

impl Gates {
    pub fn from_config(config: &crate::config::Config) -> Gates {
        Gates {
            schedules: crate::schedule::enabled(config),
        }
    }

    fn apply(&self, namespace: &Arc<Namespace>) -> Arc<Namespace> {
        if self.schedules || namespace.name != opencode::NAME {
            return namespace.clone();
        }
        Arc::new(Namespace {
            name: namespace.name.clone(),
            description: namespace.description.clone(),
            direct: namespace.direct,
            tools: namespace
                .tools
                .iter()
                .filter(|t| !t.name().starts_with("schedule_"))
                .cloned()
                .collect(),
        })
    }
}

pub fn process_year() -> i32 {
    static YEAR: OnceLock<i32> = OnceLock::new();
    *YEAR.get_or_init(crate::time::local_year)
}
