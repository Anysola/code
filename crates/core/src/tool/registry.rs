//! The set of tools and the tool array of a request: direct tools sorted by name, then `execute`
//! (Code Mode) last. A tool is left out when the agent's rules wholly deny its permission action.

use std::sync::{Arc, OnceLock};

use serde_json::Value;

use super::{DefinitionEnv, Tool, ToolDefinition};
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
}

impl Registry {
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
            Arc::new(super::stubs::WebfetchTool),
            Arc::new(super::stubs::WebsearchTool),
            Arc::new(super::write::WriteTool),
            Arc::new(super::stubs::ExecuteTool),
        ];
        Registry { tools }
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    /// Tools advertised to an agent with these rules, in request order.
    pub fn active(&self, rules: &[Rule]) -> Vec<Arc<dyn Tool>> {
        let enabled = |tool: &Arc<dyn Tool>| {
            // 0.5.1 advertises `skill` even to agents whose rules end in `* deny` (explore); the
            // skill tool still asks `skill:<id>` per call.
            tool.name() == "skill" || !wholly_disabled(tool.permission(), rules)
        };
        let mut direct: Vec<_> = self
            .tools
            .iter()
            .filter(|t| !t.code_mode() && enabled(t))
            .cloned()
            .collect();
        direct.sort_by(|a, b| a.name().cmp(b.name()));
        direct.extend(
            self.tools
                .iter()
                .filter(|t| t.code_mode() && enabled(t))
                .cloned(),
        );
        direct
    }

    pub fn definitions(tools: &[Arc<dyn Tool>], env: &DefinitionEnv) -> Vec<ToolDefinition> {
        tools.iter().map(|t| t.definition(env)).collect()
    }
}

/// Process-start year for the websearch description.
pub fn process_year() -> i32 {
    static YEAR: OnceLock<i32> = OnceLock::new();
    *YEAR.get_or_init(crate::time::local_year)
}
