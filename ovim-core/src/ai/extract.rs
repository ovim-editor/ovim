use crate::ai::types::EditFormat;
use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct AiExtractedResponse {
    pub replacement: String,
    pub new_import_statements: Vec<String>,
    pub log_lines: Vec<String>,
}

pub fn extract_response(format: &EditFormat, raw_output: &str) -> Result<AiExtractedResponse> {
    match format {
        EditFormat::Json => extract_json(raw_output),
        EditFormat::Codeblock => extract_codeblock(raw_output),
        EditFormat::Raw => Ok(AiExtractedResponse {
            replacement: raw_output.to_string(),
            new_import_statements: Vec::new(),
            log_lines: vec!["Used raw edit format".to_string()],
        }),
        EditFormat::ApplyPatch => Err(anyhow!("apply_patch edit format not yet implemented")),
        EditFormat::StrReplace => Err(anyhow!("str_replace edit format not yet implemented")),
        EditFormat::Lua(name) => Ok(AiExtractedResponse {
            replacement: raw_output.to_string(),
            new_import_statements: Vec::new(),
            log_lines: vec![format!("lua:{} — deferred to main thread", name)],
        }),
    }
}

fn extract_json(raw_output: &str) -> Result<AiExtractedResponse> {
    let trimmed = raw_output.trim();
    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(_) => {
            let fenced =
                first_codeblock(trimmed).ok_or_else(|| anyhow!("expected JSON payload"))?;
            serde_json::from_str(fenced.trim())?
        }
    };

    let replacement = value
        .get("replacement")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing JSON field: replacement"))?
        .to_string();

    // Accept both "new_import_statements" (primary) and "top_insertions" (compat alias).
    let new_import_statements = value
        .get("new_import_statements")
        .or_else(|| value.get("top_insertions"))
        .and_then(|v| match v {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToString::to_string)
                    .collect(),
            ),
            Value::String(single) => Some(vec![single.clone()]),
            _ => None,
        })
        .unwrap_or_default();

    let log_lines = match value.get("log") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(single)) => vec![single.clone()],
        _ => vec!["Used json edit format".to_string()],
    };

    Ok(AiExtractedResponse {
        replacement,
        new_import_statements,
        log_lines,
    })
}

fn extract_codeblock(raw_output: &str) -> Result<AiExtractedResponse> {
    let code = first_codeblock(raw_output).ok_or_else(|| anyhow!("no fenced code block found"))?;
    Ok(AiExtractedResponse {
        replacement: code.trim_end_matches('\n').to_string(),
        new_import_statements: Vec::new(),
        log_lines: vec!["Used codeblock edit format".to_string()],
    })
}

fn first_codeblock(text: &str) -> Option<&str> {
    let start = text.find("```")?;
    let mut body_start = start + 3;

    if let Some(line_end) = text[body_start..].find('\n') {
        body_start += line_end + 1;
    } else {
        return None;
    }

    let body = &text[body_start..];
    let end = body.find("```")?;
    Some(&body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_extract_basic() {
        let raw = r#"{"replacement":"fn x() {}", "new_import_statements":["use std::fmt;"], "log":["done"]}"#;
        let parsed = extract_response(&EditFormat::Json, raw).expect("json parse");
        assert_eq!(parsed.replacement, "fn x() {}");
        assert_eq!(parsed.new_import_statements, vec!["use std::fmt;"]);
        assert_eq!(parsed.log_lines, vec!["done"]);
    }

    #[test]
    fn json_extract_compat_top_insertions() {
        let raw =
            r#"{"replacement":"fn x() {}", "top_insertions":["use std::fmt;"], "log":["done"]}"#;
        let parsed = extract_response(&EditFormat::Json, raw).expect("json parse");
        assert_eq!(parsed.new_import_statements, vec!["use std::fmt;"]);
    }

    #[test]
    fn codeblock_extract_basic() {
        let raw = "text\n```rust\nfn y() {}\n```\n";
        let parsed = extract_response(&EditFormat::Codeblock, raw).expect("codeblock parse");
        assert_eq!(parsed.replacement, "fn y() {}");
        assert!(parsed.new_import_statements.is_empty());
    }

    #[test]
    fn raw_extract_basic() {
        let raw = "hello";
        let parsed = extract_response(&EditFormat::Raw, raw).expect("raw parse");
        assert_eq!(parsed.replacement, "hello");
    }

    #[test]
    fn lua_format_defers_extraction() {
        let raw = "fn deferred() {}";
        let parsed =
            extract_response(&EditFormat::Lua("test".to_string()), raw).expect("lua deferred");
        assert_eq!(parsed.replacement, raw, "should pass through raw text");
        assert!(parsed.new_import_statements.is_empty());
        assert!(
            parsed.log_lines[0].contains("deferred"),
            "log should mention deferral"
        );
    }
}
