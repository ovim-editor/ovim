use crate::ai::skills::{ACTIVATED_SKILL_MARKER, ACTIVATE_SKILL_TOOL};
use crate::ai::tools::ToolResult;

use super::Editor;

impl Editor {
    pub(super) fn execute_activate_skill_tool(&self, arguments: &serde_json::Value) -> ToolResult {
        let Some(name) = arguments.get("name").and_then(serde_json::Value::as_str) else {
            return ToolResult::Error("'name' is required and must be a string".to_string());
        };
        let name = name.trim();
        let Some(skill) = self.ai_state.skill_catalog.get(name) else {
            return ToolResult::Error(format!(
                "unknown skill {name:?}; use one of the names advertised for {ACTIVATE_SKILL_TOOL}"
            ));
        };

        ToolResult::Success(format!(
            "{ACTIVATED_SKILL_MARKER}{name}\n\nSkill instructions:\n\n{}",
            skill.instructions
        ))
    }
}
