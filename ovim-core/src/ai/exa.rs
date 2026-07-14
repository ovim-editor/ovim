//! Exa-backed web tools and credential lifecycle for Ovim's direct harness.

use crate::ai::tools::ToolResult;
use crate::ai::truncate_utf8_with_notice;
use anyhow::{Context, Result};
use reqwest::blocking::{Client, Response};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DASHBOARD_URL: &str = "https://dashboard.exa.ai/api-keys";
const SEARCH_URL: &str = "https://api.exa.ai/search";
const CONTENTS_URL: &str = "https://api.exa.ai/contents";
const MAX_TOOL_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct ExaAuthFile {
    version: u8,
    api_key: Option<String>,
    onboarding_dismissed: bool,
    credential_invalid: bool,
}

impl Default for ExaAuthFile {
    fn default() -> Self {
        Self {
            version: 1,
            api_key: None,
            onboarding_dismissed: false,
            credential_invalid: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    Environment,
    Stored,
}

#[derive(Debug, Clone)]
pub struct Credential {
    pub key: String,
    pub source: CredentialSource,
}

#[derive(Debug)]
pub struct WebToolOutcome {
    pub result: ToolResult,
    pub credential_rejected: bool,
    pub environment_override: bool,
}

pub fn credential() -> Option<Credential> {
    if let Ok(value) = std::env::var("EXA_API_KEY") {
        let key = value.trim();
        if !key.is_empty() {
            return Some(Credential {
                key: key.to_string(),
                source: CredentialSource::Environment,
            });
        }
    }
    let state = read_auth_file().unwrap_or_default();
    if state.credential_invalid {
        return None;
    }
    state
        .api_key
        .filter(|key| !key.trim().is_empty())
        .map(|key| Credential {
            key,
            source: CredentialSource::Stored,
        })
}

pub fn should_offer_onboarding() -> bool {
    if credential().is_some() {
        return false;
    }
    !read_auth_file().unwrap_or_default().onboarding_dismissed
}

pub fn save_key(key: &str) -> Result<()> {
    let key = key.trim();
    if key.len() < 8 || key.chars().any(char::is_whitespace) {
        anyhow::bail!("Enter a complete Exa API key without spaces")
    }
    let mut state = read_auth_file().unwrap_or_default();
    state.api_key = Some(key.to_string());
    state.onboarding_dismissed = true;
    state.credential_invalid = false;
    write_auth_file(&state)
}

pub fn dismiss_onboarding() -> Result<()> {
    let mut state = read_auth_file().unwrap_or_default();
    state.onboarding_dismissed = true;
    write_auth_file(&state)
}

pub fn mark_rejected(source: CredentialSource) -> Result<()> {
    if source == CredentialSource::Stored {
        let mut state = read_auth_file().unwrap_or_default();
        state.credential_invalid = true;
        state.onboarding_dismissed = false;
        write_auth_file(&state)?;
    }
    Ok(())
}

pub fn execute(name: &str, arguments: &Value) -> WebToolOutcome {
    let Some(credential) = credential() else {
        return WebToolOutcome {
            result: ToolResult::Error(format!(
                "Exa web access is not configured. Open /exa to add a key from {DASHBOARD_URL}."
            )),
            credential_rejected: false,
            environment_override: false,
        };
    };
    let source = credential.source;
    let response = match name {
        "web_search" => search(&credential.key, arguments),
        "web_fetch" => fetch(&credential.key, arguments),
        _ => Err(ExaError::Other(format!("unknown Exa tool: {name}"))),
    };
    match response {
        Ok(text) => WebToolOutcome {
            result: ToolResult::Success(truncate_utf8_with_notice(&text, MAX_TOOL_OUTPUT)),
            credential_rejected: false,
            environment_override: source == CredentialSource::Environment,
        },
        Err(error) => {
            let rejected = matches!(error, ExaError::InvalidKey(_));
            if rejected {
                let _ = mark_rejected(source);
            }
            WebToolOutcome {
                result: ToolResult::Error(error.user_message()),
                credential_rejected: rejected,
                environment_override: source == CredentialSource::Environment,
            }
        }
    }
}

#[derive(Debug)]
enum ExaError {
    InvalidKey(String),
    Credits(String),
    RateLimited(String),
    AccessDenied(String),
    Other(String),
}

impl ExaError {
    fn user_message(self) -> String {
        match self {
            Self::InvalidKey(detail) => format!(
                "Exa rejected the API key ({detail}). Replace it with /exa. API keys: {DASHBOARD_URL}"
            ),
            Self::Credits(detail) => format!(
                "Exa web search has no available credits or reached its budget ({detail}). Manage billing and limits at {DASHBOARD_URL}."
            ),
            Self::RateLimited(detail) => format!(
                "Exa rate-limited the request after a retry ({detail}). Try again shortly."
            ),
            Self::AccessDenied(detail) => format!("Exa denied this request: {detail}"),
            Self::Other(detail) => format!("Exa web request failed: {detail}"),
        }
    }
}

fn client() -> Result<Client, ExaError> {
    Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("ovim/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| ExaError::Other(error.to_string()))
}

fn search(key: &str, args: &Value) -> Result<String, ExaError> {
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if query.is_empty() {
        return Err(ExaError::Other("'query' is required".into()));
    }
    let count = args
        .get("num_results")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .clamp(1, 10);
    let mut body = json!({
        "query": query,
        "type": "auto",
        "numResults": count,
        "contents": {
            "highlights": { "numSentences": 3, "highlightsPerUrl": 2 },
            "text": { "maxCharacters": 4000 }
        }
    });
    copy_string_array(args, &mut body, "include_domains", "includeDomains");
    copy_string_array(args, &mut body, "exclude_domains", "excludeDomains");
    let value = send_json_with_retry(&client()?, SEARCH_URL, key, &body)?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| ExaError::Other("response did not contain a results array".into()))?;
    let mut out = format!("Exa search results for {query:?}:\n");
    for (index, item) in results.iter().enumerate() {
        let title = string_field(item, "title", "Untitled");
        let url = string_field(item, "url", "");
        out.push_str(&format!("\n{}. {}\nURL: {}\n", index + 1, title, url));
        for field in ["publishedDate", "author"] {
            if let Some(value) = item
                .get(field)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push_str(&format!("{}: {}\n", field, value));
            }
        }
        if let Some(highlights) = item.get("highlights").and_then(Value::as_array) {
            let snippets = highlights
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !snippets.is_empty() {
                out.push_str(&format!("Relevant excerpts: {}\n", snippets.join(" … ")));
            }
        } else if let Some(text) = item.get("text").and_then(Value::as_str) {
            out.push_str(&format!(
                "Excerpt: {}\n",
                truncate_utf8_with_notice(text, 2000)
            ));
        }
    }
    Ok(out)
}

fn fetch(key: &str, args: &Value) -> Result<String, ExaError> {
    let url = args.get("url").and_then(Value::as_str).unwrap_or("").trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(ExaError::Other(
            "'url' must be an absolute http(s) URL".into(),
        ));
    }
    let body = json!({
        "urls": [url],
        "text": { "maxCharacters": 50000 },
        "livecrawl": "always"
    });
    let value = send_json_with_retry(&client()?, CONTENTS_URL, key, &body)?;
    let item = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| ExaError::Other("no page content was returned".into()))?;
    let title = string_field(item, "title", "Untitled");
    let canonical_url = string_field(item, "url", url);
    let text = item.get("text").and_then(Value::as_str).unwrap_or("");
    if text.is_empty() {
        return Err(ExaError::Other(format!(
            "no readable content was extracted from {url}"
        )));
    }
    Ok(format!("{title}\nURL: {canonical_url}\n\n{text}"))
}

fn send_json_with_retry(
    client: &Client,
    url: &str,
    key: &str,
    body: &Value,
) -> Result<Value, ExaError> {
    let mut retried = false;
    loop {
        let response = client
            .post(url)
            .header("x-api-key", key)
            .json(body)
            .send()
            .map_err(|error| ExaError::Other(error.to_string()))?;
        if !retried
            && (response.status() == StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error())
        {
            let delay = retry_delay(&response);
            std::thread::sleep(delay);
            retried = true;
            continue;
        }
        return decode_response(response);
    }
}

fn retry_delay(response: &Response) -> Duration {
    response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(2)))
        .unwrap_or_else(|| Duration::from_millis(400))
}

fn decode_response(response: Response) -> Result<Value, ExaError> {
    let status = response.status();
    let text = response.text().unwrap_or_default();
    if status.is_success() {
        return serde_json::from_str(&text)
            .map_err(|error| ExaError::Other(format!("invalid JSON response: {error}")));
    }
    let detail = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| truncate_utf8_with_notice(&text, 1000));
    match status.as_u16() {
        401 => Err(ExaError::InvalidKey(detail)),
        402 => Err(ExaError::Credits(detail)),
        403 => Err(ExaError::AccessDenied(detail)),
        429 => Err(ExaError::RateLimited(detail)),
        _ => Err(ExaError::Other(format!("HTTP {status}: {detail}"))),
    }
}

fn copy_string_array(args: &Value, body: &mut Value, source: &str, target: &str) {
    if let Some(values) = args.get(source).and_then(Value::as_array) {
        let values = values
            .iter()
            .filter_map(Value::as_str)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        if !values.is_empty() {
            body[target] = json!(values);
        }
    }
}

fn string_field<'a>(value: &'a Value, name: &str, fallback: &'a str) -> &'a str {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback)
}

fn auth_path() -> Result<PathBuf> {
    dirs::config_dir()
        .map(|dir| dir.join("ovim").join("exa-auth.json"))
        .context("platform configuration directory is unavailable")
}

fn read_auth_file() -> Result<ExaAuthFile> {
    let path = auth_path()?;
    if !path.exists() {
        return Ok(ExaAuthFile::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_auth_file(state: &ExaAuthFile) -> Result<()> {
    write_auth_file_at(&auth_path()?, state)
}

fn write_auth_file_at(path: &Path, state: &ExaAuthFile) -> Result<()> {
    let parent = path.parent().context("Exa auth path has no parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".exa-auth-{}.tmp", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(state)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_auth_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/exa-auth.json");
        let state = ExaAuthFile {
            api_key: Some("test-key-secret".into()),
            onboarding_dismissed: true,
            ..Default::default()
        };
        write_auth_file_at(&path, &state).unwrap();
        let decoded: ExaAuthFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(decoded.api_key.as_deref(), Some("test-key-secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
