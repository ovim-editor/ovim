use crate::ai::chat_types::StreamChunk;
use crate::ai::config::AiProfileConfig;
use crate::ai::extract::extract_response;
use crate::ai::stream_parsers;
use crate::ai::types::{AiJobResult, AiProviderKind, AiRequest};
use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

pub async fn request_ai_edit(
    profile: &AiProfileConfig,
    request: &AiRequest,
) -> Result<AiJobResult> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to create AI HTTP client")?;

    let response_text = match profile.provider {
        AiProviderKind::OpenAi => request_openai(&client, profile, request).await?,
        AiProviderKind::Anthropic => request_anthropic(&client, profile, request).await?,
        AiProviderKind::Ollama => request_ollama(&client, profile, request).await?,
    };

    let extracted = extract_response(request.extraction, &response_text)
        .context("failed to extract AI response")?;

    Ok(AiJobResult {
        replacement: extracted.replacement,
        top_insertions: extracted.top_insertions,
        log_lines: extracted.log_lines,
        raw_output: response_text,
        provider: profile.provider,
        profile_name: profile.name.clone(),
        model: profile.model.clone(),
    })
}

async fn request_openai(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
) -> Result<String> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1");
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let api_key = read_api_key(profile)?;

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}")).context("invalid OpenAI API key")?,
    );

    let mut messages = Vec::new();
    if let Some(system_prompt) = &profile.system_prompt {
        messages.push(json!({ "role": "system", "content": system_prompt }));
    }
    messages.push(json!({
        "role": "user",
        "content": build_user_prompt(request),
    }));

    let mut body = json!({
        "model": profile.model,
        "messages": messages,
    });
    if let Some(temp) = profile.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(max_tokens) = profile.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }

    let value = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .context("OpenAI request failed")?
        .error_for_status()
        .context("OpenAI returned error status")?
        .json::<Value>()
        .await
        .context("failed to decode OpenAI response")?;

    parse_openai_content(&value).context("invalid OpenAI response payload")
}

async fn request_anthropic(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
) -> Result<String> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let api_key = read_api_key(profile)?;

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(&api_key).context("invalid Anthropic API key")?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

    let mut body = json!({
        "model": profile.model,
        "max_tokens": profile.max_tokens.unwrap_or(2048),
        "messages": [
            {
                "role": "user",
                "content": build_user_prompt(request),
            }
        ]
    });
    if let Some(system_prompt) = &profile.system_prompt {
        body["system"] = json!(system_prompt);
    }
    if let Some(temp) = profile.temperature {
        body["temperature"] = json!(temp);
    }

    let value = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .context("Anthropic request failed")?
        .error_for_status()
        .context("Anthropic returned error status")?
        .json::<Value>()
        .await
        .context("failed to decode Anthropic response")?;

    let content = value
        .get("content")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing content[0].text"))?;
    Ok(content.to_string())
}

async fn request_ollama(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    request: &AiRequest,
) -> Result<String> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:11434");
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let mut body = json!({
        "model": profile.model,
        "stream": false,
        "messages": [
            {
                "role": "system",
                "content": profile.system_prompt.clone().unwrap_or_default(),
            },
            {
                "role": "user",
                "content": build_user_prompt(request),
            }
        ]
    });
    if profile.system_prompt.is_none() {
        body["messages"] = json!([
            {
                "role": "user",
                "content": build_user_prompt(request),
            }
        ]);
    }
    if let Some(temp) = profile.temperature {
        body["options"] = json!({ "temperature": temp });
    }

    let value = client
        .post(url)
        .json(&body)
        .send()
        .await
        .context("Ollama request failed")?
        .error_for_status()
        .context("Ollama returned error status")?
        .json::<Value>()
        .await
        .context("failed to decode Ollama response")?;

    let content = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message.content"))?;
    Ok(content.to_string())
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
) -> Result<()> {
    // No timeout — streaming connections are long-lived.
    let client = reqwest::Client::builder()
        .build()
        .context("failed to create AI HTTP client")?;

    match profile.provider {
        AiProviderKind::OpenAi => {
            stream_openai_chat(&client, profile, messages, system_prompt, tools, tx).await
        }
        AiProviderKind::Anthropic => {
            stream_anthropic_chat(&client, profile, messages, system_prompt, tools, tx).await
        }
        AiProviderKind::Ollama => {
            stream_ollama_chat(&client, profile, messages, system_prompt, tools, tx).await
        }
    }
}

/// Serialize chat messages to OpenAI format (also used by Ollama).
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

async fn stream_openai_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
) -> Result<()> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com/v1");
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let api_key = read_api_key(profile)?;

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}")).context("invalid OpenAI API key")?,
    );

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
    if let Some(temp) = profile.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(max_tokens) = profile.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }

    let response = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .context("OpenAI request failed")?
        .error_for_status()
        .context("OpenAI returned error status")?;

    use futures_core::Stream;
    let byte_stream = response.bytes_stream();
    let pinned: std::pin::Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
        Box::pin(byte_stream);
    stream_parsers::parse_openai_stream(pinned, tx).await;
    Ok(())
}

async fn stream_anthropic_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
) -> Result<()> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");
    let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
    let api_key = read_api_key(profile)?;

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(&api_key).context("invalid Anthropic API key")?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

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
    if let Some(temp) = profile.temperature {
        body["temperature"] = json!(temp);
    }
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }

    let response = client
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .context("Anthropic request failed")?
        .error_for_status()
        .context("Anthropic returned error status")?;

    use futures_core::Stream;
    let byte_stream = response.bytes_stream();
    let pinned: std::pin::Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
        Box::pin(byte_stream);
    stream_parsers::parse_anthropic_stream(pinned, tx).await;
    Ok(())
}

async fn stream_ollama_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
    tools: Option<&[serde_json::Value]>,
    tx: UnboundedSender<StreamChunk>,
) -> Result<()> {
    let base_url = profile
        .base_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:11434");
    let url = format!("{}/api/chat", base_url.trim_end_matches('/'));

    let sys = system_prompt.or(profile.system_prompt.as_deref());
    let mut api_messages = Vec::new();
    if let Some(sp) = sys {
        api_messages.push(json!({ "role": "system", "content": sp }));
    }
    api_messages.extend(chat_messages_to_openai_json(messages));

    let mut body = json!({
        "model": profile.model,
        "stream": true,
        "messages": api_messages,
    });
    if let Some(temp) = profile.temperature {
        body["options"] = json!({ "temperature": temp });
    }
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }

    let response = client
        .post(url)
        .json(&body)
        .send()
        .await
        .context("Ollama request failed")?
        .error_for_status()
        .context("Ollama returned error status")?;

    use futures_core::Stream;
    let byte_stream = response.bytes_stream();
    let pinned: std::pin::Pin<Box<dyn Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send>> =
        Box::pin(byte_stream);
    stream_parsers::parse_ollama_stream(pinned, tx).await;
    Ok(())
}

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

fn read_api_key(profile: &AiProfileConfig) -> Result<String> {
    let key_env = profile
        .api_key_env
        .as_deref()
        .unwrap_or(match profile.provider {
            AiProviderKind::OpenAi => "OPENAI_API_KEY",
            AiProviderKind::Anthropic => "ANTHROPIC_API_KEY",
            AiProviderKind::Ollama => "",
        });

    if key_env.is_empty() {
        return Err(anyhow!(
            "missing api_key_env for provider {}",
            profile.provider
        ));
    }

    std::env::var(key_env).with_context(|| format!("environment variable {key_env} is not set"))
}

fn build_user_prompt(request: &AiRequest) -> String {
    let language = request.language_id.as_deref().unwrap_or("plain_text");
    let file_path = request.file_path.as_deref().unwrap_or("[No Name]");
    let mut prompt = format!(
        "Edit the selected text based on the instruction.\n\
Instruction:\n{}\n\n\
File: {}\n\
Language: {}\n\
Extraction strategy: {}\n\n\
Selected text:\n```{}\n{}\n```",
        request.prompt, file_path, language, request.extraction, language, request.selected_text
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
}
