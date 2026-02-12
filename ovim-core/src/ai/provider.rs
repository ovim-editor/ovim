use crate::ai::config::AiProfileConfig;
use crate::ai::extract::extract_response;
use crate::ai::types::{AiJobResult, AiProviderKind, AiRequest};
use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};
use std::time::Duration;

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
// Multi-turn chat API
// ---------------------------------------------------------------------------

pub async fn request_ai_chat(
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("failed to create AI HTTP client")?;

    match profile.provider {
        AiProviderKind::OpenAi => {
            request_openai_chat(&client, profile, messages, system_prompt).await
        }
        AiProviderKind::Anthropic => {
            request_anthropic_chat(&client, profile, messages, system_prompt).await
        }
        AiProviderKind::Ollama => {
            request_ollama_chat(&client, profile, messages, system_prompt).await
        }
    }
}

fn chat_messages_to_json(messages: &[super::chat_types::ChatMessage]) -> Vec<Value> {
    messages
        .iter()
        .filter(|m| m.role != super::chat_types::ChatRole::Error)
        .map(|m| {
            let role = match m.role {
                super::chat_types::ChatRole::User => "user",
                super::chat_types::ChatRole::Assistant => "assistant",
                super::chat_types::ChatRole::Error => unreachable!(),
            };
            json!({ "role": role, "content": m.content })
        })
        .collect()
}

async fn request_openai_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
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

    let sys = system_prompt.or(profile.system_prompt.as_deref());
    let mut api_messages = Vec::new();
    if let Some(sp) = sys {
        api_messages.push(json!({ "role": "system", "content": sp }));
    }
    api_messages.extend(chat_messages_to_json(messages));

    let mut body = json!({
        "model": profile.model,
        "messages": api_messages,
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

async fn request_anthropic_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
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
        "messages": chat_messages_to_json(messages),
    });
    let sys = system_prompt.or(profile.system_prompt.as_deref());
    if let Some(sp) = sys {
        body["system"] = json!(sp);
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

async fn request_ollama_chat(
    client: &reqwest::Client,
    profile: &AiProfileConfig,
    messages: &[super::chat_types::ChatMessage],
    system_prompt: Option<&str>,
) -> Result<String> {
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
    api_messages.extend(chat_messages_to_json(messages));

    let mut body = json!({
        "model": profile.model,
        "stream": false,
        "messages": api_messages,
    });
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
