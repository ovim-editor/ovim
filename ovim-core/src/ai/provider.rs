use crate::ai::chat_types::StreamChunk;
use crate::ai::config::{system_prompt_for_edit_format, AiProfileConfig};
use crate::ai::extract::extract_and_check_elision;
use crate::ai::prompt::interpolate;
use crate::ai::stream_parsers;
use crate::ai::types::{AiJobResult, AiProviderKind, AiRequest, ApiKeyConfig};
use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

pub async fn request_ai_edit(
    profile: &AiProfileConfig,
    request: &AiRequest,
    registry: &HashMap<String, ApiKeyConfig>,
    prompts: &HashMap<String, String>,
    format_prompts: &HashMap<String, String>,
    project_context: &str,
) -> Result<AiJobResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to create AI HTTP client")?;

    let retry_max = profile.retry.max;
    let mut extra_messages: Vec<Value> = Vec::new();
    let mut current_format = request.edit_format.clone();
    let base_prompt =
        resolve_edit_system_prompt(profile, prompts, format_prompts, request, &current_format);
    let mut current_system_prompt = append_project_context(&base_prompt, project_context);
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=retry_max {
        // On final attempt with fallback configured: switch format and rebuild system prompt.
        if attempt > 0 && attempt == retry_max && profile.retry.fallback.is_some() {
            let fb_name = profile.retry.fallback.as_ref().unwrap();
            current_format = crate::ai::parse_edit_format_str(fb_name);
            let base_prompt = resolve_edit_system_prompt(
                profile,
                prompts,
                format_prompts,
                request,
                &current_format,
            );
            current_system_prompt = append_project_context(&base_prompt, project_context);
            extra_messages.clear(); // fresh conversation for fallback format
        }

        let response_text = match profile.provider {
            AiProviderKind::OpenAi => {
                request_openai(
                    &client,
                    profile,
                    request,
                    &current_system_prompt,
                    registry,
                    &extra_messages,
                )
                .await?
            }
            AiProviderKind::Anthropic => {
                request_anthropic(
                    &client,
                    profile,
                    request,
                    &current_system_prompt,
                    registry,
                    &extra_messages,
                )
                .await?
            }
            AiProviderKind::Ollama => {
                request_ollama(
                    &client,
                    profile,
                    request,
                    &current_system_prompt,
                    registry,
                    &extra_messages,
                )
                .await?
            }
        };

        match extract_and_check_elision(&current_format, &response_text) {
            Ok((extracted, elision)) if !elision.is_empty() && attempt < retry_max => {
                // Elision detected, still have retries — re-prompt with anti-elision instructions.
                let feedback = format!(
                    "Your response contained placeholders that omit code: {}. \
                     You MUST provide the complete replacement. Do not abbreviate with \
                     comments like '// ... rest' or '// remaining unchanged'. Output every line.",
                    elision.join("; "),
                );
                let retry_msgs = build_retry_messages(&response_text, &feedback, profile.provider);
                extra_messages.extend(retry_msgs);
                last_error = Some(anyhow!("elision detected"));
            }
            Ok((extracted, elision)) => {
                // Clean result, or retries exhausted — accept with optional warning.
                return Ok(AiJobResult {
                    replacement: extracted.replacement,
                    new_import_statements: extracted.new_import_statements,
                    log_lines: extracted.log_lines,
                    raw_output: response_text,
                    provider: profile.provider,
                    profile_name: profile.name.clone(),
                    model: profile.model.clone(),
                    retry_attempts: attempt,
                    elision_markers: elision,
                });
            }
            Err(e) if attempt < retry_max => {
                let format_hint = format_instructions_for(&current_format);
                let feedback = format!(
                    "Your response could not be parsed. Error: {}. Please respond with {}.",
                    e, format_hint,
                );
                let retry_msgs = build_retry_messages(&response_text, &feedback, profile.provider);
                extra_messages.extend(retry_msgs);
                last_error = Some(e);
            }
            Err(e) => {
                last_error = Some(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow!("retry exhausted")))
}

// ---------------------------------------------------------------------------
// System prompt resolution
// ---------------------------------------------------------------------------

/// Resolve the system prompt for edit mode, following the priority chain:
/// 1. `profile.edit_prompt` — per-profile override, interpolated
/// 2. `prompts["selection_{format}"]` — per-format global template, interpolated
/// 3. `prompts["edit"]` — catch-all global template, interpolated (backward compat)
/// 4. `format_prompts[format_name]` — from format registry (Lua formats)
/// 5. `profile.system_prompt` — raw string, no interpolation (backward compat)
/// 6. `system_prompt_for_edit_format()` — hardcoded fallback
fn resolve_edit_system_prompt(
    profile: &AiProfileConfig,
    prompts: &HashMap<String, String>,
    format_prompts: &HashMap<String, String>,
    request: &AiRequest,
    format: &crate::ai::types::EditFormat,
) -> String {
    let file = request.file_path.as_deref().unwrap_or("[No Name]");
    let language = request.language_id.as_deref().unwrap_or("plain_text");
    let vars = HashMap::from([
        ("file", file),
        ("language", language),
        ("selection", request.selected_text.as_str()),
        ("instruction", request.prompt.as_str()),
    ]);

    if let Some(ref template) = profile.edit_prompt {
        return interpolate(template, &vars);
    }

    // Per-format prompt key: "selection_codeblock", "selection_json", etc.
    let format_key = format!("selection_{}", format);
    if let Some(template) = prompts.get(&format_key) {
        return interpolate(template, &vars);
    }

    // Catch-all "edit" key (backward compat)
    if let Some(template) = prompts.get("edit") {
        return interpolate(template, &vars);
    }

    // Format-specific prompt from vim.ai.formats.register()
    let format_name = match format {
        crate::ai::types::EditFormat::Lua(name) => Some(name.as_str()),
        _ => None,
    };
    if let Some(name) = format_name {
        if let Some(prompt) = format_prompts.get(name) {
            return interpolate(prompt, &vars);
        }
    }

    if let Some(ref sp) = profile.system_prompt {
        return sp.clone();
    }

    system_prompt_for_edit_format(format).to_string()
}

/// Resolve the system prompt for chat mode, following the priority chain:
/// 1. `chat.opts.system_prompt` — per-session override (handled by caller)
/// 2. `profile.chat_prompt` — per-profile override, interpolated
/// 3. `prompts["chat"]` — global template, interpolated
/// 4. fallback built by caller (`build_chat_system_prompt`)
pub(crate) fn resolve_chat_system_prompt(
    profile: &AiProfileConfig,
    prompts: &HashMap<String, String>,
    file: &str,
    language: &str,
) -> Option<String> {
    let vars = HashMap::from([
        ("file", file),
        ("language", language),
        ("selection", ""),
        ("instruction", ""),
    ]);

    if let Some(ref template) = profile.chat_prompt {
        return Some(interpolate(template, &vars));
    }

    if let Some(template) = prompts.get("chat") {
        return Some(interpolate(template, &vars));
    }

    None
}

/// Append project context to a system prompt. Returns the original prompt
/// unchanged when `project_context` is empty.
pub(crate) fn append_project_context(system_prompt: &str, project_context: &str) -> String {
    if project_context.is_empty() {
        return system_prompt.to_string();
    }
    format!("{system_prompt}\n\n## Project Context\n{project_context}")
}

// ---------------------------------------------------------------------------
// Per-provider helpers: URL, headers, common body params
// ---------------------------------------------------------------------------

/// Build the API endpoint URL for the given provider.
fn provider_url(profile: &AiProfileConfig) -> String {
    let (default_base, path) = match profile.provider {
        AiProviderKind::OpenAi => ("https://api.openai.com/v1", "/chat/completions"),
        AiProviderKind::Anthropic => ("https://api.anthropic.com", "/v1/messages"),
        AiProviderKind::Ollama => ("http://127.0.0.1:11434", "/api/chat"),
    };
    let base = profile.base_url.as_deref().unwrap_or(default_base);
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// Build HTTP headers for the given provider (reads API key from env when needed).
fn provider_headers(
    profile: &AiProfileConfig,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    match profile.provider {
        AiProviderKind::OpenAi => {
            let api_key = read_api_key(profile, registry)?;
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {api_key}"))
                    .context("invalid OpenAI API key")?,
            );
        }
        AiProviderKind::Anthropic => {
            let api_key = read_api_key(profile, registry)?;
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&api_key).context("invalid Anthropic API key")?,
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        }
        AiProviderKind::Ollama => {
            // Ollama doesn't need authentication headers.
        }
    }
    Ok(headers)
}

/// Provider label for error messages.
fn provider_label(provider: AiProviderKind) -> &'static str {
    match provider {
        AiProviderKind::OpenAi => "OpenAI",
        AiProviderKind::Anthropic => "Anthropic",
        AiProviderKind::Ollama => "Ollama",
    }
}

/// Apply common optional params (temperature, max_tokens, tools) to a JSON body.
fn apply_optional_params(body: &mut Value, profile: &AiProfileConfig, tools: Option<&[Value]>) {
    if let Some(temp) = profile.temperature {
        match profile.provider {
            AiProviderKind::Ollama => {
                body["options"] = json!({ "temperature": temp });
            }
            _ => {
                body["temperature"] = json!(temp);
            }
        }
    }
    if let Some(max_tokens) = profile.max_tokens {
        // GPT-5+ models require max_completion_tokens instead of max_tokens.
        let key = match profile.provider {
            AiProviderKind::OpenAi => "max_completion_tokens",
            _ => "max_tokens",
        };
        body[key] = json!(max_tokens);
    }
    // OpenAI reasoning_effort: enable extended thinking and strip incompatible params.
    if profile.provider == AiProviderKind::OpenAi {
        if let Some(ref effort) = profile.reasoning_effort {
            if effort != "none" {
                body["reasoning"] = json!({ "effort": effort });
                body.as_object_mut().unwrap().remove("temperature");
                body.as_object_mut().unwrap().remove("top_p");
            }
        }
    }

    // OpenAI verbosity (5.2+ parameter).
    if profile.provider == AiProviderKind::OpenAi {
        if let Some(ref verbosity) = profile.verbosity {
            body["text"] = json!({ "verbosity": verbosity });
        }
    }

    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }
}

// ---------------------------------------------------------------------------
// Single-shot request functions (one user prompt → one response)
// ---------------------------------------------------------------------------

async fn request_openai(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
    system_prompt: &str,
    registry: &HashMap<String, ApiKeyConfig>,
    extra_messages: &[Value],
) -> Result<String> {
    let url = provider_url(profile);
    let headers = provider_headers(profile, registry)?;

    let mut messages = vec![json!({ "role": "system", "content": system_prompt })];
    messages.push(json!({ "role": "user", "content": build_user_prompt(request) }));
    messages.extend_from_slice(extra_messages);

    let mut body = json!({ "model": profile.model, "messages": messages });
    // When expecting JSON, ask the API to enforce valid JSON output.
    if request.edit_format == crate::ai::types::EditFormat::Json {
        body["response_format"] = json!({ "type": "json_object" });
    }
    apply_optional_params(&mut body, profile, None);

    let label = provider_label(profile.provider);
    let value = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("{label} request failed"))?
        .error_for_status()
        .with_context(|| format!("{label} returned error status"))?
        .json::<Value>()
        .await
        .with_context(|| format!("failed to decode {label} response"))?;

    parse_openai_content(&value).context("invalid OpenAI response payload")
}

async fn request_anthropic(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
    system_prompt: &str,
    registry: &HashMap<String, ApiKeyConfig>,
    extra_messages: &[Value],
) -> Result<String> {
    let url = provider_url(profile);
    let headers = provider_headers(profile, registry)?;

    let mut messages = vec![json!({ "role": "user", "content": build_user_prompt(request) })];
    messages.extend_from_slice(extra_messages);

    let mut body = json!({
        "model": profile.model,
        "max_tokens": profile.max_tokens.unwrap_or(2048),
        "messages": messages,
        "system": system_prompt,
    });
    apply_optional_params(&mut body, profile, None);

    let label = provider_label(profile.provider);
    let value = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("{label} request failed"))?
        .error_for_status()
        .with_context(|| format!("{label} returned error status"))?
        .json::<Value>()
        .await
        .with_context(|| format!("failed to decode {label} response"))?;

    parse_anthropic_content(&value)
}

async fn request_ollama(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
    system_prompt: &str,
    registry: &HashMap<String, ApiKeyConfig>,
    extra_messages: &[Value],
) -> Result<String> {
    let url = provider_url(profile);
    let headers = provider_headers(profile, registry)?;

    let mut messages = vec![json!({ "role": "system", "content": system_prompt })];
    messages.push(json!({ "role": "user", "content": build_user_prompt(request) }));
    messages.extend_from_slice(extra_messages);

    let mut body = json!({ "model": profile.model, "stream": false, "messages": messages });
    apply_optional_params(&mut body, profile, None);

    let label = provider_label(profile.provider);
    let value = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("{label} request failed"))?
        .error_for_status()
        .with_context(|| format!("{label} returned error status"))?
        .json::<Value>()
        .await
        .with_context(|| format!("failed to decode {label} response"))?;

    parse_ollama_content(&value)
}

// ---------------------------------------------------------------------------
// Multi-turn streaming chat API
// ---------------------------------------------------------------------------

pub async fn stream_ai_chat(
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<()> {
    // No timeout — streaming connections are long-lived.
    let client = reqwest::Client::builder()
        .build()
        .context("failed to create AI HTTP client")?;

    match profile.provider {
        AiProviderKind::OpenAi => {
            stream_openai_chat(
                &client,
                profile,
                messages,
                system_prompt,
                tools,
                tx,
                registry,
            )
            .await
        }
        AiProviderKind::Anthropic => {
            stream_anthropic_chat(
                &client,
                profile,
                messages,
                system_prompt,
                tools,
                tx,
                registry,
            )
            .await
        }
        AiProviderKind::Ollama => {
            stream_ollama_chat(
                &client,
                profile,
                messages,
                system_prompt,
                tools,
                tx,
                registry,
            )
            .await
        }
    }
}

/// Serialize chat messages to OpenAI format.
fn chat_messages_to_openai_json(messages: &[super::chat_types::ChatMessage]) -> Vec<Value> {
    use super::chat_types::ChatRole;
    messages
        .iter()
        .filter(|m| m.role != ChatRole::Error && m.role != ChatRole::Thinking)
        .map(|m| match m.role {
            ChatRole::User => json!({ "role": "user", "content": m.content }),
            ChatRole::Assistant => {
                if m.tool_calls.is_empty() {
                    json!({ "role": "assistant", "content": m.content })
                } else {
                    let tc: Vec<Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    let mut msg = json!({ "role": "assistant", "tool_calls": tc });
                    if !m.content.is_empty() {
                        msg["content"] = json!(m.content);
                    }
                    msg
                }
            }
            ChatRole::Tool => {
                json!({
                    "role": "tool",
                    "content": m.content,
                    "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                })
            }
            ChatRole::Error | ChatRole::Thinking => unreachable!(),
        })
        .collect()
}

/// Serialize chat messages to Ollama native `/api/chat` format.
///
/// Key differences from OpenAI:
/// - Tool response messages use `tool_name` instead of `tool_call_id`
/// - Function arguments are objects, not stringified JSON
fn chat_messages_to_ollama_json(messages: &[super::chat_types::ChatMessage]) -> Vec<Value> {
    use super::chat_types::ChatRole;

    // Build a map from tool_call_id → tool_name so we can emit `tool_name`
    // on Tool role messages (Ollama doesn't use `tool_call_id`).
    let mut id_to_name: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for m in messages {
        if m.role == ChatRole::Assistant {
            for tc in &m.tool_calls {
                id_to_name.insert(&tc.id, &tc.name);
            }
        }
    }

    messages
        .iter()
        .filter(|m| m.role != ChatRole::Error && m.role != ChatRole::Thinking)
        .map(|m| match m.role {
            ChatRole::User => json!({ "role": "user", "content": m.content }),
            ChatRole::Assistant => {
                if m.tool_calls.is_empty() {
                    json!({ "role": "assistant", "content": m.content })
                } else {
                    let tc: Vec<Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    let mut msg = json!({ "role": "assistant", "tool_calls": tc });
                    if !m.content.is_empty() {
                        msg["content"] = json!(m.content);
                    }
                    msg
                }
            }
            ChatRole::Tool => {
                let tool_name = m
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| id_to_name.get(id).copied())
                    .unwrap_or("");
                json!({
                    "role": "tool",
                    "content": m.content,
                    "tool_name": tool_name,
                })
            }
            ChatRole::Error | ChatRole::Thinking => unreachable!(),
        })
        .collect()
}

/// Serialize chat messages to Anthropic format.
fn chat_messages_to_anthropic_json(messages: &[super::chat_types::ChatMessage]) -> Vec<Value> {
    use super::chat_types::ChatRole;

    // Anthropic expects tool results as user messages containing tool_result blocks.
    // We need to merge consecutive Tool messages into a single user message.
    let filtered: Vec<_> = messages
        .iter()
        .filter(|m| m.role != ChatRole::Error && m.role != ChatRole::Thinking)
        .collect();

    let mut result = Vec::new();
    let mut i = 0;
    while i < filtered.len() {
        let m = filtered[i];
        match m.role {
            ChatRole::User => {
                result.push(json!({ "role": "user", "content": m.content }));
            }
            ChatRole::Assistant => {
                let mut content_blocks = Vec::new();
                if !m.content.is_empty() {
                    content_blocks.push(json!({ "type": "text", "text": m.content }));
                }
                for tc in &m.tool_calls {
                    content_blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                if content_blocks.is_empty() {
                    content_blocks.push(json!({ "type": "text", "text": "" }));
                }
                result.push(json!({ "role": "assistant", "content": content_blocks }));
            }
            ChatRole::Tool => {
                // Collect consecutive Tool messages into one user message
                let mut tool_result_blocks = Vec::new();
                while i < filtered.len() && filtered[i].role == ChatRole::Tool {
                    let tm = filtered[i];
                    tool_result_blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tm.tool_call_id.as_deref().unwrap_or(""),
                        "content": tm.content,
                    }));
                    i += 1;
                }
                result.push(json!({ "role": "user", "content": tool_result_blocks }));
                continue; // Skip the i += 1 at the end
            }
            ChatRole::Error | ChatRole::Thinking => {}
        }
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Streaming chat functions (multi-turn → streamed response)
// ---------------------------------------------------------------------------

/// Send a streaming POST and return the pinned byte stream.
async fn send_streaming(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    body: &Value,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<
    std::pin::Pin<
        Box<dyn futures_core::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>,
    >,
> {
    let url = provider_url(profile);
    let headers = provider_headers(profile, registry)?;
    let label = provider_label(profile.provider);

    let response = client
        .post(url)
        .headers(headers)
        .json(body)
        .send()
        .await
        .with_context(|| format!("{label} request failed"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // Try to extract the error message from the JSON response body.
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(String::from))
            .unwrap_or(body);
        anyhow::bail!("{label} returned {status}: {detail}");
    }

    Ok(Box::pin(response.bytes_stream()))
}

async fn stream_openai_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<()> {
    let sys = system_prompt.or(profile.system_prompt.as_deref());
    let mut api_messages = Vec::new();
    if let Some(sp) = sys {
        api_messages.push(json!({ "role": "system", "content": sp }));
    }
    api_messages.extend(chat_messages_to_openai_json(messages));

    let mut body = json!({
        "model": profile.model,
        "messages": api_messages,
        "stream": true,
    });
    apply_optional_params(&mut body, profile, tools);

    let stream = send_streaming(client, profile, &body, registry).await?;
    stream_parsers::parse_openai_stream(stream, tx).await;
    Ok(())
}

async fn stream_anthropic_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<()> {
    let mut body = json!({
        "model": profile.model,
        "max_tokens": profile.max_tokens.unwrap_or(2048),
        "messages": chat_messages_to_anthropic_json(messages),
        "stream": true,
    });
    let sys = system_prompt.or(profile.system_prompt.as_deref());
    if let Some(sp) = sys {
        body["system"] = json!(sp);
    }
    apply_optional_params(&mut body, profile, tools);

    let stream = send_streaming(client, profile, &body, registry).await?;
    stream_parsers::parse_anthropic_stream(stream, tx).await;
    Ok(())
}

async fn stream_ollama_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<()> {
    let sys = system_prompt.or(profile.system_prompt.as_deref());
    let mut api_messages = Vec::new();
    if let Some(sp) = sys {
        api_messages.push(json!({ "role": "system", "content": sp }));
    }
    api_messages.extend(chat_messages_to_ollama_json(messages));

    let mut body = json!({
        "model": profile.model,
        "stream": true,
        "messages": api_messages,
    });
    apply_optional_params(&mut body, profile, tools);

    let stream = send_streaming(client, profile, &body, registry).await?;
    stream_parsers::parse_ollama_stream(stream, tx).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Response parsing helpers
// ---------------------------------------------------------------------------

fn parse_openai_content(value: &Value) -> Result<String> {
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .ok_or_else(|| anyhow!("missing choices[0].message.content"))?;

    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }

    if let Some(items) = content.as_array() {
        let mut output = String::new();
        for item in items {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                output.push_str(text);
            }
        }
        if !output.is_empty() {
            return Ok(output);
        }
    }

    Err(anyhow!("unexpected OpenAI content type"))
}

fn parse_anthropic_content(value: &Value) -> Result<String> {
    value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing content[0].text in Anthropic response"))
}

fn parse_ollama_content(value: &Value) -> Result<String> {
    value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing message.content in Ollama response"))
}

fn read_api_key(
    profile: &AiProfileConfig,
    registry: &HashMap<String, ApiKeyConfig>,
) -> Result<String> {
    // If the profile references a named key in the registry, resolve it.
    if let Some(key_name) = &profile.api_key {
        let key_config = registry.get(key_name).ok_or_else(|| {
            anyhow!(
                "API key '{}' referenced by profile '{}' is not registered \
                 (call vim.api_keys.register('{}', {{ env_var = ... }}))",
                key_name,
                profile.name,
                key_name,
            )
        })?;
        return resolve_api_key_config(key_name, key_config);
    }

    // Fallback: use api_key_env (existing behavior).
    let env_var_name: String = match &profile.api_key_env {
        Some(name) if !name.is_empty() => name.clone(),
        _ => super::config::default_api_key_env(profile.provider).ok_or_else(|| {
            anyhow!(
                "no API key environment variable configured for provider {} \
                 (set api_key_env in your profile or export the default env var)",
                profile.provider
            )
        })?,
    };

    read_env_var_with_diagnostics(&env_var_name)
}

/// Resolve an API key from a registry entry: try env_var first, then file.
fn resolve_api_key_config(key_name: &str, config: &ApiKeyConfig) -> Result<String> {
    if let Some(ref env_var) = config.env_var {
        if let Ok(val) = std::env::var(env_var) {
            return Ok(val);
        }
        // If file is also set, fall through to try it.
        if config.file.is_none() {
            return read_env_var_with_diagnostics(env_var);
        }
    }

    if let Some(ref file_path) = config.file {
        let content = std::fs::read_to_string(file_path).with_context(|| {
            format!(
                "failed to read API key file '{}' for key '{}'",
                file_path, key_name
            )
        })?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            anyhow::bail!(
                "API key file '{}' for key '{}' is empty",
                file_path,
                key_name
            );
        }
        return Ok(trimmed);
    }

    // Should not reach here because setup_api_keys_api validates at least one is set,
    // but handle it defensively.
    anyhow::bail!("API key '{}' has no env_var or file configured", key_name)
}

/// Read an environment variable, providing helpful diagnostics on failure.
fn read_env_var_with_diagnostics(env_var_name: &str) -> Result<String> {
    std::env::var(env_var_name).with_context(|| {
        let related: Vec<String> = std::env::vars()
            .filter(|(k, _)| {
                k.contains("OPENAI")
                    || k.contains("ANTHROPIC")
                    || k.contains("OVIM")
                    || k.contains("API_KEY")
            })
            .map(|(k, _)| k)
            .collect();
        let hint = if related.is_empty() {
            "no related env vars visible to this process (OPENAI/ANTHROPIC/OVIM/API_KEY)"
                .to_string()
        } else {
            format!("env vars visible to process: {}", related.join(", "))
        };
        format!(
            "environment variable {env_var_name} is not set — {hint}. \
             Export it in your shell before launching ovim."
        )
    })
}

fn build_user_prompt(request: &AiRequest) -> String {
    let language = request.language_id.as_deref().unwrap_or("plain_text");
    let file_path = request.file_path.as_deref().unwrap_or("[No Name]");
    let mut prompt = format!(
        "Edit the selected text based on the instruction.\n\
Instruction:\n{}\n\n\
File: {}\n\
Language: {}\n\n\
Selected text:\n```{}\n{}\n```",
        request.prompt, file_path, language, language, request.selected_text
    );

    if let Some(context_pack) = &request.context_pack {
        if !context_pack.symbol_facts.is_empty() {
            prompt.push_str("\n\nNearby symbols:\n");
            for symbol in context_pack.symbol_facts.iter().take(12) {
                prompt.push_str(&format!(
                    "- {} [{}] at {}:{}\n",
                    symbol.name, symbol.kind, symbol.line, symbol.character
                ));
            }
        }

        if !context_pack.diagnostics.is_empty() {
            prompt.push_str("\nDiagnostics overlapping selection:\n");
            for diag in context_pack.diagnostics.iter().take(12) {
                let severity = diag.severity.as_deref().unwrap_or("unknown");
                prompt.push_str(&format!(
                    "- {} ({} at {}:{}-{})\n",
                    diag.message, severity, diag.line, diag.start_character, diag.end_character
                ));
            }
        }

        for slice in context_pack.surrounding.iter().take(3) {
            let slice_language = slice.language.as_deref().unwrap_or(language);
            prompt.push_str(&format!(
                "\nContext slice [{} lines {}-{}]:\n```{}\n{}\n```",
                slice.label, slice.start_line, slice.end_line, slice_language, slice.content
            ));
        }

        for slice in context_pack.related_slices.iter().take(3) {
            let slice_language = slice.language.as_deref().unwrap_or(language);
            prompt.push_str(&format!(
                "\nRelated slice [{} lines {}-{}]:\n```{}\n{}\n```",
                slice.label, slice.start_line, slice.end_line, slice_language, slice.content
            ));
        }
    }

    prompt
}

// ---------------------------------------------------------------------------
// Retry helpers
// ---------------------------------------------------------------------------

/// Return a short description of the expected response format for retry feedback.
fn format_instructions_for(format: &crate::ai::types::EditFormat) -> &'static str {
    use crate::ai::types::EditFormat;
    match format {
        EditFormat::Json => "a valid JSON object with a \"replacement\" field",
        EditFormat::Codeblock => "your code inside a single fenced code block (```)",
        EditFormat::Raw => "the replacement text directly",
        _ => "the replacement text in the expected format",
    }
}

/// Build provider-appropriate retry messages (assistant echo + user feedback).
fn build_retry_messages(
    response_text: &str,
    feedback: &str,
    provider: AiProviderKind,
) -> Vec<Value> {
    match provider {
        AiProviderKind::Anthropic => vec![
            json!({"role": "assistant", "content": [{"type": "text", "text": response_text}]}),
            json!({"role": "user", "content": feedback}),
        ],
        _ => vec![
            json!({"role": "assistant", "content": response_text}),
            json!({"role": "user", "content": feedback}),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::chat_types::{ChatMessage, ChatRole, ToolCallInfo};
    use std::time::Instant;

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::User,
            content: content.to_string(),
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: content.to_string(),
            model: Some("test".to_string()),
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn assistant_msg_with_tools(content: &str, tool_calls: Vec<ToolCallInfo>) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Assistant,
            content: content.to_string(),
            model: Some("test".to_string()),
            timestamp: Instant::now(),
            tool_calls,
            tool_call_id: None,
        }
    }

    fn tool_msg(tool_call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: ChatRole::Tool,
            content: content.to_string(),
            model: None,
            timestamp: Instant::now(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id.to_string()),
        }
    }

    #[test]
    fn openai_json_basic_messages() {
        let msgs = vec![user_msg("hello"), assistant_msg("hi")];
        let json = chat_messages_to_openai_json(&msgs);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"], "hello");
        assert_eq!(json[1]["role"], "assistant");
        assert_eq!(json[1]["content"], "hi");
    }

    #[test]
    fn openai_json_filters_error_and_thinking() {
        let msgs = vec![
            user_msg("hello"),
            ChatMessage {
                role: ChatRole::Thinking,
                content: "hmm".to_string(),
                model: None,
                timestamp: Instant::now(),
                tool_calls: vec![],
                tool_call_id: None,
            },
            ChatMessage {
                role: ChatRole::Error,
                content: "oops".to_string(),
                model: None,
                timestamp: Instant::now(),
                tool_calls: vec![],
                tool_call_id: None,
            },
            assistant_msg("hi"),
        ];
        let json = chat_messages_to_openai_json(&msgs);
        assert_eq!(json.len(), 2);
    }

    #[test]
    fn openai_json_assistant_with_tool_calls() {
        let tc = ToolCallInfo {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"start_line": 1}),
        };
        let msgs = vec![
            user_msg("check file"),
            assistant_msg_with_tools("Let me check.", vec![tc]),
        ];
        let json = chat_messages_to_openai_json(&msgs);
        assert_eq!(json[1]["role"], "assistant");
        assert!(json[1]["tool_calls"].is_array());
        assert_eq!(json[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(json[1]["tool_calls"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn openai_json_tool_role_messages() {
        let msgs = vec![
            user_msg("check file"),
            assistant_msg_with_tools(
                "",
                vec![ToolCallInfo {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({}),
                }],
            ),
            tool_msg("call_1", "file contents here"),
        ];
        let json = chat_messages_to_openai_json(&msgs);
        assert_eq!(json[2]["role"], "tool");
        assert_eq!(json[2]["content"], "file contents here");
        assert_eq!(json[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn anthropic_json_basic_messages() {
        let msgs = vec![user_msg("hello"), assistant_msg("hi")];
        let json = chat_messages_to_anthropic_json(&msgs);
        assert_eq!(json.len(), 2);
        assert_eq!(json[0]["role"], "user");
        assert_eq!(json[0]["content"], "hello");
        assert_eq!(json[1]["role"], "assistant");
    }

    #[test]
    fn anthropic_json_tool_results_as_user() {
        let tc = ToolCallInfo {
            id: "toolu_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({}),
        };
        let msgs = vec![
            user_msg("check"),
            assistant_msg_with_tools("", vec![tc]),
            tool_msg("toolu_1", "file content"),
        ];
        let json = chat_messages_to_anthropic_json(&msgs);
        assert_eq!(json.len(), 3); // user, assistant, user (tool result)
                                   // Tool results become a user message with tool_result blocks
        assert_eq!(json[2]["role"], "user");
        let content = json[2]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "tool_result");
        assert_eq!(content[0]["tool_use_id"], "toolu_1");
        assert_eq!(content[0]["content"], "file content");
    }

    // -----------------------------------------------------------------------
    // apply_optional_params tests
    // -----------------------------------------------------------------------

    fn test_profile(provider: AiProviderKind) -> AiProfileConfig {
        use crate::ai::types::{
            AgentLoopConfig, ContextGatheringPolicy, EditFormat, ProfileScope, RetryPolicy,
        };
        AiProfileConfig {
            name: "test".to_string(),
            provider,
            model: "test-model".to_string(),
            base_url: None,
            api_key: None,
            api_key_env: None,
            system_prompt: None,
            temperature: Some(0.7),
            max_tokens: None,
            edit_format: EditFormat::default(),
            chat_edit_format: None,
            context: ContextGatheringPolicy::default(),
            agent_loop: AgentLoopConfig::default(),
            tools: vec![],
            scope: ProfileScope::default(),
            edit_prompt: None,
            chat_prompt: None,
            chat_edit_prompt: None,
            reasoning_effort: None,
            verbosity: None,
            syntax_check: None,
            retry: RetryPolicy::default(),
        }
    }

    #[test]
    fn reasoning_effort_openai() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.reasoning_effort = Some("low".to_string());
        let mut body = json!({ "model": "o3" });
        apply_optional_params(&mut body, &profile, None);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_effort_none_is_noop() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.reasoning_effort = Some("none".to_string());
        let mut body = json!({ "model": "o3" });
        apply_optional_params(&mut body, &profile, None);
        assert!(body.get("reasoning").is_none());
        // temperature should still be present
        assert!(body.get("temperature").is_some());
    }

    #[test]
    fn reasoning_effort_ignored_for_anthropic() {
        let mut profile = test_profile(AiProviderKind::Anthropic);
        profile.reasoning_effort = Some("high".to_string());
        let mut body = json!({ "model": "claude" });
        apply_optional_params(&mut body, &profile, None);
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn verbosity_openai() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.verbosity = Some("high".to_string());
        let mut body = json!({ "model": "gpt" });
        apply_optional_params(&mut body, &profile, None);
        assert_eq!(body["text"]["verbosity"], "high");
    }

    #[test]
    fn verbosity_ignored_for_ollama() {
        let mut profile = test_profile(AiProviderKind::Ollama);
        profile.verbosity = Some("high".to_string());
        let mut body = json!({ "model": "llama" });
        apply_optional_params(&mut body, &profile, None);
        assert!(body.get("text").is_none());
    }

    #[test]
    fn anthropic_json_assistant_with_tool_use() {
        let tc = ToolCallInfo {
            id: "toolu_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"start_line": 1}),
        };
        let msgs = vec![
            user_msg("check"),
            assistant_msg_with_tools("Let me check.", vec![tc]),
        ];
        let json = chat_messages_to_anthropic_json(&msgs);
        assert_eq!(json[1]["role"], "assistant");
        let content = json[1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2); // text + tool_use
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "Let me check.");
        assert_eq!(content[1]["type"], "tool_use");
        assert_eq!(content[1]["id"], "toolu_1");
        assert_eq!(content[1]["name"], "read_file");
    }

    // -----------------------------------------------------------------------
    // read_api_key / API key registry tests
    // -----------------------------------------------------------------------

    #[test]
    fn api_key_registry_env_var() {
        let mut registry = HashMap::new();
        registry.insert(
            "test_key".to_string(),
            ApiKeyConfig {
                env_var: Some("OVIM_TEST_API_KEY_12345".to_string()),
                file: None,
            },
        );
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.api_key = Some("test_key".to_string());

        // Set the env var, read the key, then clean up.
        std::env::set_var("OVIM_TEST_API_KEY_12345", "sk-secret");
        let result = read_api_key(&profile, &registry);
        std::env::remove_var("OVIM_TEST_API_KEY_12345");

        assert_eq!(result.unwrap(), "sk-secret");
    }

    #[test]
    fn api_key_registry_missing_name() {
        let registry = HashMap::new(); // empty
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.api_key = Some("nonexistent".to_string());

        let result = read_api_key(&profile, &registry);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("nonexistent"),
            "error should mention the missing key name: {msg}"
        );
    }

    #[test]
    fn api_key_fallback_to_api_key_env() {
        let registry = HashMap::new();
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.api_key = None;
        profile.api_key_env = Some("OVIM_TEST_FALLBACK_KEY_67890".to_string());

        std::env::set_var("OVIM_TEST_FALLBACK_KEY_67890", "sk-fallback");
        let result = read_api_key(&profile, &registry);
        std::env::remove_var("OVIM_TEST_FALLBACK_KEY_67890");

        assert_eq!(result.unwrap(), "sk-fallback");
    }

    #[test]
    fn api_key_registry_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = dir.path().join("api_key.txt");
        std::fs::write(&key_file, "  sk-from-file  \n").unwrap();

        let mut registry = HashMap::new();
        registry.insert(
            "file_key".to_string(),
            ApiKeyConfig {
                env_var: None,
                file: Some(key_file.to_string_lossy().to_string()),
            },
        );
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.api_key = Some("file_key".to_string());

        let result = read_api_key(&profile, &registry);
        assert_eq!(result.unwrap(), "sk-from-file");
    }

    // -----------------------------------------------------------------------
    // resolve_edit_system_prompt tests
    // -----------------------------------------------------------------------

    fn test_request() -> AiRequest {
        use crate::ai::types::EditFormat;
        AiRequest {
            prompt: "fix this".to_string(),
            selected_text: "let x = 1;".to_string(),
            language_id: Some("rust".to_string()),
            file_path: Some("main.rs".to_string()),
            edit_format: EditFormat::Json,
            context_pack: None,
        }
    }

    #[test]
    fn resolve_edit_system_prompt_profile_override() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.edit_prompt = Some("You are a {{language}} expert.".to_string());
        let prompts = HashMap::new();
        let request = test_request();

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "You are a rust expert.");
    }

    #[test]
    fn resolve_edit_system_prompt_global_template() {
        let profile = test_profile(AiProviderKind::OpenAi);
        let mut prompts = HashMap::new();
        prompts.insert(
            "edit".to_string(),
            "Edit {{file}} ({{language}})".to_string(),
        );
        let request = test_request();

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Edit main.rs (rust)");
    }

    #[test]
    fn resolve_edit_system_prompt_hardcoded_fallback() {
        let profile = test_profile(AiProviderKind::OpenAi);
        let prompts = HashMap::new();
        let request = test_request();

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        // Should get the default JSON edit format prompt
        assert!(
            result.contains("JSON"),
            "expected JSON fallback, got: {result}"
        );
    }

    #[test]
    fn resolve_edit_system_prompt_profile_edit_prompt_wins_over_global() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.edit_prompt = Some("Profile override".to_string());
        let mut prompts = HashMap::new();
        prompts.insert("edit".to_string(), "Global template".to_string());
        let request = test_request();

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Profile override");
    }

    #[test]
    fn resolve_edit_system_prompt_raw_system_prompt_fallback() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.system_prompt = Some("Custom raw prompt".to_string());
        let prompts = HashMap::new();
        let request = test_request();

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Custom raw prompt");
    }

    #[test]
    fn resolve_edit_system_prompt_format_prompt() {
        let profile = test_profile(AiProviderKind::OpenAi);
        let prompts = HashMap::new();
        let mut format_prompts = HashMap::new();
        format_prompts.insert(
            "upper".to_string(),
            "Return code as-is for {{language}}.".to_string(),
        );
        let mut request = test_request();
        request.edit_format = crate::ai::types::EditFormat::Lua("upper".to_string());

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &format_prompts,
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Return code as-is for rust.");
    }

    #[test]
    fn resolve_edit_system_prompt_profile_wins_over_format() {
        let mut profile = test_profile(AiProviderKind::OpenAi);
        profile.edit_prompt = Some("Profile wins".to_string());
        let prompts = HashMap::new();
        let mut format_prompts = HashMap::new();
        format_prompts.insert("upper".to_string(), "Format prompt".to_string());
        let mut request = test_request();
        request.edit_format = crate::ai::types::EditFormat::Lua("upper".to_string());

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &format_prompts,
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Profile wins");
    }

    #[test]
    fn resolve_edit_system_prompt_format_specific_key() {
        let profile = test_profile(AiProviderKind::OpenAi);
        let mut prompts = HashMap::new();
        prompts.insert(
            "selection_codeblock".to_string(),
            "Codeblock prompt for {{language}}".to_string(),
        );
        // Also set a catch-all "edit" — should NOT be used
        prompts.insert("edit".to_string(), "Generic edit prompt".to_string());
        let mut request = test_request();
        request.edit_format = crate::ai::types::EditFormat::Codeblock;

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Codeblock prompt for rust");
    }

    #[test]
    fn resolve_edit_system_prompt_edit_key_still_works() {
        let profile = test_profile(AiProviderKind::OpenAi);
        let mut prompts = HashMap::new();
        // Only "edit" key, no format-specific key
        prompts.insert("edit".to_string(), "Catch-all for {{file}}".to_string());
        let mut request = test_request();
        request.edit_format = crate::ai::types::EditFormat::Codeblock;

        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &request.edit_format,
        );
        assert_eq!(result, "Catch-all for main.rs");
    }

    // -----------------------------------------------------------------------
    // Retry helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn format_instructions_for_all_variants() {
        use crate::ai::types::EditFormat;

        let json_hint = format_instructions_for(&EditFormat::Json);
        assert!(json_hint.contains("JSON"), "got: {json_hint}");

        let cb_hint = format_instructions_for(&EditFormat::Codeblock);
        assert!(cb_hint.contains("code block"), "got: {cb_hint}");

        let raw_hint = format_instructions_for(&EditFormat::Raw);
        assert!(!raw_hint.is_empty());

        // Other variants use the generic fallback.
        let lua_hint = format_instructions_for(&EditFormat::Lua("custom".to_string()));
        assert!(!lua_hint.is_empty());
    }

    #[test]
    fn build_retry_messages_openai_shape() {
        let msgs = build_retry_messages("bad output", "please fix", AiProviderKind::OpenAi);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "assistant");
        assert_eq!(msgs[0]["content"], "bad output");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "please fix");
    }

    #[test]
    fn build_retry_messages_anthropic_shape() {
        let msgs = build_retry_messages("bad output", "please fix", AiProviderKind::Anthropic);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "assistant");
        // Anthropic wraps assistant content in a content block array.
        let content = msgs[0]["content"].as_array().expect("should be array");
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "bad output");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "please fix");
    }

    #[test]
    fn build_retry_messages_ollama_same_as_openai() {
        let msgs = build_retry_messages("out", "fix", AiProviderKind::Ollama);
        assert_eq!(msgs[0]["content"], "out");
        // Ollama uses plain string content, same as OpenAI.
        assert!(msgs[0]["content"].is_string());
    }

    #[test]
    fn resolve_edit_system_prompt_with_different_format() {
        // Verify the refactored function respects the explicit format parameter
        // even when it differs from request.edit_format.
        let profile = test_profile(AiProviderKind::OpenAi);
        let prompts = HashMap::new();
        let request = test_request(); // edit_format = Json

        // Pass Codeblock as the explicit format — should get codeblock fallback, not JSON.
        let result = resolve_edit_system_prompt(
            &profile,
            &prompts,
            &HashMap::new(),
            &request,
            &crate::ai::types::EditFormat::Codeblock,
        );
        assert!(
            !result.contains("JSON"),
            "should use codeblock format, not JSON. got: {result}"
        );
    }

    #[test]
    fn extraction_feedback_for_invalid_json() {
        use crate::ai::extract::extract_response;
        use crate::ai::types::EditFormat;

        let bad_response = "Sure! Here is the code:\nfn main() {}";
        let result = extract_response(&EditFormat::Json, bad_response);
        assert!(result.is_err(), "should fail to parse non-JSON response");

        // Verify we can construct a meaningful feedback message from the error.
        let err = result.unwrap_err();
        let feedback = format!(
            "Your response could not be parsed. Error: {}. Please respond with {}.",
            err,
            format_instructions_for(&EditFormat::Json),
        );
        assert!(feedback.contains("JSON"));
    }

    #[test]
    fn extraction_feedback_for_missing_codeblock() {
        use crate::ai::extract::extract_response;
        use crate::ai::types::EditFormat;

        let bad_response = "Here is the fix:\nfn main() { println!(\"hello\"); }";
        let result = extract_response(&EditFormat::Codeblock, bad_response);
        assert!(result.is_err(), "should fail without fenced code block");

        let err = result.unwrap_err();
        let feedback = format!(
            "Your response could not be parsed. Error: {}. Please respond with {}.",
            err,
            format_instructions_for(&EditFormat::Codeblock),
        );
        assert!(feedback.contains("code block"));
    }
}
