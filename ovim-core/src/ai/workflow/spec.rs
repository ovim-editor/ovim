use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

use super::template::validate_template;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowSpec {
    pub version: u32,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub inputs: BTreeMap<String, WorkflowInputSpec>,
    pub steps: Vec<WorkflowStepSpec>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowInputSpec {
    #[serde(rename = "type", default = "default_input_type")]
    pub type_name: String,
    #[serde(default)]
    pub required: bool,
    pub description: Option<String>,
}

fn default_input_type() -> String {
    "string".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowStepSpec {
    Prompt {
        id: String,
        mode: Option<String>,
        context: Option<String>,
        profile: Option<String>,
        prompt: String,
        #[serde(default)]
        output: WorkflowOutputSpec,
    },
    Each {
        id: String,
        items: String,
        #[serde(rename = "as")]
        as_name: Option<String>,
        index: Option<String>,
        step: WorkflowNestedStepSpec,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkflowNestedStepSpec {
    Prompt {
        mode: Option<String>,
        context: Option<String>,
        profile: Option<String>,
        prompt: String,
        #[serde(default)]
        output: WorkflowOutputSpec,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkflowOutputSpec {
    #[serde(default)]
    pub format: WorkflowOutputFormat,
    #[serde(default)]
    pub schema: Option<Value>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowOutputFormat {
    #[default]
    Text,
    Json,
}

impl WorkflowSpec {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!(
                "workflow '{}' has unsupported version {} (expected 1)",
                self.name,
                self.version
            );
        }
        if self.name.trim().is_empty() {
            bail!("workflow name must be non-empty");
        }
        if self.steps.is_empty() {
            bail!("workflow '{}' must contain at least one step", self.name);
        }

        let mut seen_ids = HashSet::new();
        for step in &self.steps {
            match step {
                WorkflowStepSpec::Prompt { id, prompt, .. } => {
                    if id.trim().is_empty() {
                        bail!(
                            "workflow '{}' contains prompt step with empty id",
                            self.name
                        );
                    }
                    if prompt.trim().is_empty() {
                        bail!("workflow '{}': step '{}' has empty prompt", self.name, id);
                    }
                    validate_template(prompt).with_context(|| {
                        format!(
                            "workflow '{}': step '{}' has invalid prompt template",
                            self.name, id
                        )
                    })?;
                    if !seen_ids.insert(id.clone()) {
                        bail!("workflow '{}': duplicate step id '{}'", self.name, id);
                    }
                }
                WorkflowStepSpec::Each {
                    id,
                    items,
                    as_name,
                    index,
                    step,
                } => {
                    if id.trim().is_empty() {
                        bail!("workflow '{}' contains each step with empty id", self.name);
                    }
                    if !seen_ids.insert(id.clone()) {
                        bail!("workflow '{}': duplicate step id '{}'", self.name, id);
                    }
                    let referenced = referenced_step_from_items_path(items).ok_or_else(|| {
                        anyhow!(
                            "workflow '{}': step '{}' items must be a dotted path like \
                             'steps.<id>.output'",
                            self.name,
                            id
                        )
                    })?;
                    if !seen_ids.contains(referenced) {
                        bail!(
                            "workflow '{}': step '{}' references '{}' before it is defined",
                            self.name,
                            id,
                            referenced
                        );
                    }
                    let loop_var = as_name.as_deref().unwrap_or("item");
                    if loop_var.trim().is_empty() {
                        bail!(
                            "workflow '{}': step '{}' has empty 'as' variable",
                            self.name,
                            id
                        );
                    }
                    let idx_var = index.as_deref().unwrap_or("index");
                    if idx_var.trim().is_empty() {
                        bail!(
                            "workflow '{}': step '{}' has empty 'index' variable",
                            self.name,
                            id
                        );
                    }
                    match step {
                        WorkflowNestedStepSpec::Prompt { prompt, .. } => {
                            if prompt.trim().is_empty() {
                                bail!(
                                    "workflow '{}': step '{}' nested prompt is empty",
                                    self.name,
                                    id
                                );
                            }
                            validate_template(prompt).with_context(|| {
                                format!(
                                    "workflow '{}': step '{}' has invalid nested template",
                                    self.name, id
                                )
                            })?;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

pub fn referenced_step_from_items_path(path: &str) -> Option<&str> {
    let trimmed = path.trim();
    if !trimmed.starts_with("steps.") || !trimmed.ends_with(".output") {
        return None;
    }
    let inner = &trimmed["steps.".len()..trimmed.len() - ".output".len()];
    if inner.is_empty() || inner.contains('.') {
        return None;
    }
    Some(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_invalid_prompt_template() {
        let spec: WorkflowSpec = serde_yaml::from_str(
            r#"
version: 1
name: broken
steps:
  - id: one
    kind: prompt
    prompt: "{% if inputs.foo %}ok"
"#,
        )
        .expect("parse");
        let err = spec.validate().expect_err("invalid template should fail");
        assert!(err.to_string().contains("invalid prompt template"));
    }

    #[test]
    fn validate_rejects_invalid_nested_prompt_template() {
        let spec: WorkflowSpec = serde_yaml::from_str(
            r#"
version: 1
name: broken_nested
steps:
  - id: seed
    kind: prompt
    prompt: "[]"
    output:
      format: json
  - id: loop
    kind: each
    items: steps.seed.output
    step:
      kind: prompt
      prompt: "{{ item"
"#,
        )
        .expect("parse");
        let err = spec
            .validate()
            .expect_err("invalid nested template should fail");
        assert!(err.to_string().contains("invalid nested template"));
    }
}
