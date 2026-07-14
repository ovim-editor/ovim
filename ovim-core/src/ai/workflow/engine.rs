use super::schema::validate_json_output;
use super::spec::{
    WorkflowNestedStepSpec, WorkflowOutputFormat, WorkflowOutputSpec, WorkflowSpec,
    WorkflowStepSpec,
};
use super::template::render_template;
use super::{WorkflowProgressEvent, WorkflowRunResult, WorkflowStepProgressKind};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::future::Future;

const MAX_LOOP_ITEMS: usize = 128;
const MAX_RENDERED_PROMPT_CHARS: usize = 64 * 1024;

#[derive(Debug, Clone)]
struct PromptStepRequest {
    context: String,
    profile_override: Option<String>,
    prompt: String,
}

pub async fn execute_workflow(
    spec: WorkflowSpec,
    inputs: BTreeMap<String, Value>,
    config: crate::ai::AiConfig,
) -> Result<WorkflowRunResult> {
    execute_workflow_with_progress(spec, inputs, config, None).await
}

pub async fn execute_workflow_with_progress(
    spec: WorkflowSpec,
    inputs: BTreeMap<String, Value>,
    config: crate::ai::AiConfig,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<WorkflowProgressEvent>>,
) -> Result<WorkflowRunResult> {
    execute_workflow_with_runner(spec, inputs, &config, progress_tx, |request| async {
        run_prompt_step(&config, request).await
    })
    .await
}

async fn execute_workflow_with_runner<F, Fut>(
    spec: WorkflowSpec,
    inputs: BTreeMap<String, Value>,
    config: &crate::ai::AiConfig,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<WorkflowProgressEvent>>,
    mut runner: F,
) -> Result<WorkflowRunResult>
where
    F: FnMut(PromptStepRequest) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    validate_required_inputs(&spec, &inputs)?;

    let workflow_meta = json!({
        "name": spec.name,
        "version": spec.version,
        "description": spec.description,
    });
    let inputs_value = Value::Object(
        inputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect::<Map<String, Value>>(),
    );

    let mut steps_context = Map::new();
    let mut outputs = BTreeMap::new();
    let mut log_lines = Vec::new();

    for step in &spec.steps {
        match step {
            WorkflowStepSpec::Prompt {
                id,
                context,
                profile,
                prompt,
                output,
                ..
            } => {
                emit_progress(
                    &progress_tx,
                    WorkflowProgressEvent {
                        step_id: id.clone(),
                        kind: WorkflowStepProgressKind::Started,
                        detail: None,
                    },
                );
                let ctx = base_template_context(&workflow_meta, &inputs_value, &steps_context, id);
                let rendered = render_template(prompt, &ctx)
                    .with_context(|| format!("step '{}' template render failed", id))?;
                ensure_prompt_size(id, &rendered)?;
                let response = runner(PromptStepRequest {
                    context: context.clone().unwrap_or_else(|| "chat".to_string()),
                    profile_override: profile.clone(),
                    prompt: rendered,
                })
                .await
                .with_context(|| format!("step '{}' provider request failed", id))?;
                let parsed = parse_step_output(id, output, &response)?;
                steps_context.insert(id.clone(), json!({ "output": parsed.clone() }));
                outputs.insert(id.clone(), parsed);
                log_lines.push(format!("step '{}' completed", id));
                emit_progress(
                    &progress_tx,
                    WorkflowProgressEvent {
                        step_id: id.clone(),
                        kind: WorkflowStepProgressKind::Completed,
                        detail: None,
                    },
                );
            }
            WorkflowStepSpec::Each {
                id,
                items,
                as_name,
                index,
                step,
            } => {
                emit_progress(
                    &progress_tx,
                    WorkflowProgressEvent {
                        step_id: id.clone(),
                        kind: WorkflowStepProgressKind::Started,
                        detail: None,
                    },
                );
                let ctx = base_template_context(&workflow_meta, &inputs_value, &steps_context, id);
                let iterable = resolve_dotted_path(&ctx, items)
                    .ok_or_else(|| anyhow!("step '{}' items path '{}' not found", id, items))?;
                let items = iterable
                    .as_array()
                    .ok_or_else(|| anyhow!("step '{}' items must resolve to an array", id))?;
                if items.len() > MAX_LOOP_ITEMS {
                    bail!(
                        "step '{}' expands to {} items (max {})",
                        id,
                        items.len(),
                        MAX_LOOP_ITEMS
                    );
                }
                let as_name = as_name.as_deref().unwrap_or("item");
                let index_name = index.as_deref().unwrap_or("index");

                let mut nested_outputs = Vec::new();
                for (idx, item) in items.iter().enumerate() {
                    emit_progress(
                        &progress_tx,
                        WorkflowProgressEvent {
                            step_id: id.clone(),
                            kind: WorkflowStepProgressKind::Started,
                            detail: Some(format!("iteration {}/{}", idx + 1, items.len())),
                        },
                    );
                    let mut loop_ctx = ctx.clone();
                    let Some(obj) = loop_ctx.as_object_mut() else {
                        bail!("internal error: workflow context is not an object");
                    };
                    obj.insert(as_name.to_string(), item.clone());
                    obj.insert(index_name.to_string(), json!(idx));
                    match step {
                        WorkflowNestedStepSpec::Prompt {
                            context,
                            profile,
                            prompt,
                            output,
                            ..
                        } => {
                            let rendered =
                                render_template(prompt, &loop_ctx).with_context(|| {
                                    format!(
                                        "step '{}' iteration {} template render failed",
                                        id, idx
                                    )
                                })?;
                            ensure_prompt_size(id, &rendered)?;
                            let response = runner(PromptStepRequest {
                                context: context.clone().unwrap_or_else(|| "chat".to_string()),
                                profile_override: profile.clone(),
                                prompt: rendered,
                            })
                            .await
                            .with_context(|| {
                                format!("step '{}' iteration {} provider request failed", id, idx)
                            })?;
                            let parsed = parse_step_output(id, output, &response)?;
                            nested_outputs.push(parsed);
                        }
                    }
                    emit_progress(
                        &progress_tx,
                        WorkflowProgressEvent {
                            step_id: id.clone(),
                            kind: WorkflowStepProgressKind::Completed,
                            detail: Some(format!("iteration {}/{}", idx + 1, items.len())),
                        },
                    );
                }

                let output = Value::Array(nested_outputs);
                steps_context.insert(id.clone(), json!({ "output": output.clone() }));
                outputs.insert(id.clone(), output);
                log_lines.push(format!("step '{}' completed", id));
                emit_progress(
                    &progress_tx,
                    WorkflowProgressEvent {
                        step_id: id.clone(),
                        kind: WorkflowStepProgressKind::Completed,
                        detail: None,
                    },
                );
            }
        }
    }

    // Keep the config argument intentionally used in execution path so future
    // extensions can consume per-workflow defaults from config.
    let _ = config;

    Ok(WorkflowRunResult { outputs, log_lines })
}

fn validate_required_inputs(spec: &WorkflowSpec, inputs: &BTreeMap<String, Value>) -> Result<()> {
    for (name, input_spec) in &spec.inputs {
        if input_spec.required && !inputs.contains_key(name) {
            bail!("missing required workflow input '{}'", name);
        }
    }
    Ok(())
}

fn emit_progress(
    tx: &Option<tokio::sync::mpsc::UnboundedSender<WorkflowProgressEvent>>,
    event: WorkflowProgressEvent,
) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

fn ensure_prompt_size(step_id: &str, rendered_prompt: &str) -> Result<()> {
    if rendered_prompt.chars().count() > MAX_RENDERED_PROMPT_CHARS {
        bail!(
            "step '{}' rendered prompt too large ({} chars, max {})",
            step_id,
            rendered_prompt.chars().count(),
            MAX_RENDERED_PROMPT_CHARS
        );
    }
    Ok(())
}

fn base_template_context(
    workflow_meta: &Value,
    inputs_value: &Value,
    steps_context: &Map<String, Value>,
    current_step: &str,
) -> Value {
    json!({
        "workflow": workflow_meta,
        "inputs": inputs_value,
        "steps": Value::Object(steps_context.clone()),
        "state": {
            "current_step": current_step,
        }
    })
}

fn parse_step_output(step_id: &str, output: &WorkflowOutputSpec, raw: &str) -> Result<Value> {
    match output.format {
        WorkflowOutputFormat::Text => Ok(Value::String(raw.to_string())),
        WorkflowOutputFormat::Json => {
            let parsed: Value = serde_json::from_str(raw).with_context(|| {
                format!("step '{}' expected JSON output but parsing failed", step_id)
            })?;
            if let Some(schema) = output.schema.as_ref() {
                validate_json_output(&parsed, schema)
                    .with_context(|| format!("step '{}' schema validation failed", step_id))?;
            }
            Ok(parsed)
        }
    }
}

fn resolve_dotted_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        if segment.is_empty() {
            return None;
        }
        current = current.get(segment)?;
    }
    Some(current)
}

async fn run_prompt_step(
    config: &crate::ai::AiConfig,
    request: PromptStepRequest,
) -> Result<String> {
    let profile_name = request
        .profile_override
        .or_else(|| config.contexts.get(&request.context).cloned())
        .unwrap_or_else(|| config.default_profile.clone());
    let profile = config
        .resolve_profile(&profile_name)
        .cloned()
        .ok_or_else(|| anyhow!("unknown profile '{}'", profile_name))?;

    let system_prompt = crate::ai::resolve_chat_system_prompt(
        &profile,
        &config.prompts,
        "[workflow]",
        "plain_text",
    )
    .or(profile.system_prompt.clone())
    .unwrap_or_else(|| {
        "You are executing an ovim workflow step. Reply only with requested output.".to_string()
    });

    let messages = vec![crate::ai::ChatMessage {
        role: crate::ai::ChatRole::User,
        content: request.prompt,
        model: None,
        timestamp: std::time::Instant::now(),
        images: vec![],
        tool_calls: vec![],
        tool_call_id: None,
    }];

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let profile_clone = profile.clone();
    let registry = config.api_key_registry.clone();
    let system_prompt_clone = system_prompt.clone();
    let task = tokio::spawn(async move {
        if let Err(e) = crate::ai::stream_ai_chat(
            &profile_clone,
            &messages,
            Some(&system_prompt_clone),
            None,
            None,
            None,
            None,
            tx.clone(),
            &registry,
        )
        .await
        {
            let _ = tx.send(crate::ai::StreamChunk::Error(e.to_string()));
        }
    });

    let mut content = String::new();
    while let Some(chunk) = rx.recv().await {
        match chunk {
            crate::ai::StreamChunk::Content(delta) => content.push_str(&delta),
            crate::ai::StreamChunk::Done => break,
            crate::ai::StreamChunk::Error(err) => {
                let _ = task.await;
                return Err(anyhow!("provider error: {}", err));
            }
            crate::ai::StreamChunk::Thinking(_)
            | crate::ai::StreamChunk::AgentMessageComplete
            | crate::ai::StreamChunk::ToolCall { .. }
            | crate::ai::StreamChunk::ToolCallComplete { .. }
            | crate::ai::StreamChunk::DynamicToolRequest { .. }
            | crate::ai::StreamChunk::SteerAccepted { .. }
            | crate::ai::StreamChunk::SteerRejected { .. } => {}
        }
    }
    let _ = task.await;
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::workflow::spec::WorkflowSpec;

    #[test]
    fn dotted_path_resolves_nested_values() {
        let root = json!({
            "steps": {
                "a": {
                    "output": [1, 2, 3]
                }
            }
        });
        let v = resolve_dotted_path(&root, "steps.a.output").expect("path");
        assert!(v.is_array());
    }

    #[tokio::test]
    async fn executes_prompt_and_each_with_stub_runner() {
        let spec: WorkflowSpec = serde_yaml::from_str(
            r#"
version: 1
name: demo
steps:
  - id: first
    kind: prompt
    prompt: "hello"
    output:
      format: json
  - id: second
    kind: each
    items: steps.first.output
    as: item
    index: idx
    step:
      kind: prompt
      prompt: "{{ item }}-{{ idx }}"
"#,
        )
        .expect("parse");
        spec.validate().expect("valid");

        let cfg = crate::ai::AiConfig::default();
        let result =
            execute_workflow_with_runner(spec, BTreeMap::new(), &cfg, None, |request| async move {
                if request.prompt == "hello" {
                    Ok("[\"a\", \"b\"]".to_string())
                } else {
                    Ok(request.prompt)
                }
            })
            .await
            .expect("execute");

        assert!(result.outputs.contains_key("first"));
        assert!(result.outputs.contains_key("second"));
        let second = result.outputs.get("second").expect("second output");
        assert_eq!(second, &json!(["a-0", "b-1"]));
    }

    #[tokio::test]
    async fn each_step_respects_max_items_guard() {
        let spec: WorkflowSpec = serde_yaml::from_str(
            r#"
version: 1
name: bounded
steps:
  - id: first
    kind: prompt
    prompt: "seed"
    output:
      format: json
  - id: second
    kind: each
    items: steps.first.output
    step:
      kind: prompt
      prompt: "{{ item }}"
"#,
        )
        .expect("parse");
        spec.validate().expect("valid");

        let cfg = crate::ai::AiConfig::default();
        let err =
            execute_workflow_with_runner(spec, BTreeMap::new(), &cfg, None, |request| async move {
                if request.prompt == "seed" {
                    let body = (0..129usize)
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    Ok(format!("[{}]", body))
                } else {
                    Ok(request.prompt)
                }
            })
            .await
            .expect_err("should fail on max items");

        assert!(err.to_string().contains("max 128"));
    }

    #[tokio::test]
    async fn prompt_step_respects_render_size_guard() {
        let spec: WorkflowSpec = serde_yaml::from_str(
            r#"
version: 1
name: oversized
steps:
  - id: first
    kind: prompt
    prompt: "{{ inputs.big }}"
"#,
        )
        .expect("parse");
        spec.validate().expect("valid");

        let mut inputs = BTreeMap::new();
        inputs.insert(
            "big".to_string(),
            Value::String("x".repeat(MAX_RENDERED_PROMPT_CHARS + 1)),
        );
        let cfg = crate::ai::AiConfig::default();
        let err = execute_workflow_with_runner(spec, inputs, &cfg, None, |request| async move {
            Ok(request.prompt)
        })
        .await
        .expect_err("should fail on prompt size");

        assert!(err.to_string().contains("rendered prompt too large"));
    }
}
