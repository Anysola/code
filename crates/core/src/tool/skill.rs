//! The `skill` tool: loads a discovered skill's instructions and a sample of its files.

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;

use super::{
    DefinitionEnv, Tool, ToolContext, ToolDefinition, ToolError, ToolOutput, ToolResult,
    parse_input,
};
use crate::embedded_def;
use crate::permission::PermissionRequest;

embedded_def!(def, "skill");

#[derive(Deserialize)]
struct Input {
    id: String,
}

pub struct SkillTool;

impl Tool for SkillTool {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn definition(&self, _: &DefinitionEnv) -> ToolDefinition {
        let def = def();
        ToolDefinition {
            name: self.name().into(),
            description: def.description.clone(),
            input_schema: def.input_schema.clone(),
        }
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolResult> {
        async move {
            let input: Input = parse_input(self.name(), &def().input_schema, input)?;
            let Some(skill) = ctx.session.skill(&input.id) else {
                return Err(ToolError::execution(format!(
                    "Unable to load skill {}",
                    input.id
                )));
            };
            ctx.ask(PermissionRequest {
                action: self.name().into(),
                resources: vec![skill.id.clone()],
                save: vec![skill.id.clone()],
                ..Default::default()
            })
            .await?;
            let directory = skill
                .path
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            Ok(ToolOutput::text(crate::skill::content(&skill))
                .with_metadata("name", skill.name.clone())
                .with_metadata("directory", directory))
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission::PermissionError;
    use crate::tool::test_support::context;
    use serde_json::json;

    #[tokio::test]
    async fn loads_and_asks() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("skills/demo");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Do the demo thing.\n").unwrap();
        let test = context(dir.path());
        test.session
            .skills
            .lock()
            .unwrap()
            .push(crate::skill::Skill {
                id: "demo".into(),
                name: "demo".into(),
                description: None,
                autoinvoke: None,
                path: skill_dir.join("SKILL.md"),
                content: "Do the demo thing.".into(),
            });
        let output = SkillTool
            .execute(json!({"id": "demo"}), &test.ctx)
            .await
            .unwrap();
        assert_eq!(output.metadata["name"], "demo");
        let asked = test.gate.asked.lock().unwrap().clone();
        assert_eq!(
            (
                asked[0].action.as_str(),
                &asked[0].resources,
                &asked[0].save
            ),
            (
                "skill",
                &vec!["demo".to_string()],
                &vec!["demo".to_string()]
            )
        );
        let missing = SkillTool
            .execute(json!({"id": "missing"}), &test.ctx)
            .await
            .unwrap_err();
        assert_eq!(
            missing,
            ToolError::execution("Unable to load skill missing")
        );
        *test.gate.answer.lock().unwrap() = Some(PermissionError::Blocked {
            action: "skill".into(),
            reason: None,
        });
        let denied = SkillTool
            .execute(json!({"id": "demo"}), &test.ctx)
            .await
            .unwrap_err();
        assert_eq!(
            (denied.type_tag(), denied.message.as_str()),
            ("permission.rejected", "Permission denied: skill")
        );
    }
}
