use crate::ai::types::ExtractionStrategy;
use anyhow::{anyhow, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct AiExtractedResponse {
    pub replacement: String,
    pub top_insertions: Vec<String>,
    pub log_lines: Vec<String>,
}

pub fn extract_response(strategy: ExtractionStrategy, raw_output: &str) -> Result<AiExtractedResponse> {
    match strategy {
        ExtractionStrategy::Json => extract_json(raw_output),
        ExtractionStrategy::Codeblock => extract_codeblock(raw_output),
        ExtractionStrategy::Raw => Ok(AiExtractedResponse {
            replacement: raw_output.to_string(),
            top_insertions: Vec::new(),
            log_lines: vec!["Used raw extraction strategy".to_string()],
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

    let top_insertions = match value.get("top_insertions") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(single)) => vec![single.clone()],
        _ => Vec::new(),
    };

    let log_lines = match value.get("log") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect(),
        Some(Value::String(single)) => vec![single.clone()],
        _ => vec!["Used json extraction strategy".to_string()],
    };

    Ok(AiExtractedResponse {
        replacement,
        top_insertions,
        log_lines,
    })
}

fn extract_codeblock(raw_output: &str) -> Result<AiExtractedResponse> {
    let code = first_codeblock(raw_output).ok_or_else(|| anyhow!("no fenced code block found"))?;
    Ok(AiExtractedResponse {
        replacement: code.trim_end_matches('\n').to_string(),
        top_insertions: Vec::new(),
        log_lines: vec!["Used codeblock extraction strategy".to_string()],
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
        let raw = r#"{"replacement":"fn x() {}", "top_insertions":["use std::fmt;"], "log":["done"]}"#;
        let parsed = extract_response(ExtractionStrategy::Json, raw).expect("json parse");
        assert_eq!(parsed.replacement, "fn x() {}");
        assert_eq!(parsed.top_insertions, vec!["use std::fmt;"]);
        assert_eq!(parsed.log_lines, vec!["done"]);
    }

    #[test]
    fn codeblock_extract_basic() {
        let raw = "text\n```rust\nfn y() {}\n```\n";
        let parsed =
            extract_response(ExtractionStrategy::Codeblock, raw).expect("codeblock parse");
        assert_eq!(parsed.replacement, "fn y() {}");
        assert!(parsed.top_insertions.is_empty());
    }

    #[test]
    fn raw_extract_basic() {
        let raw = "hello";
        let parsed = extract_response(ExtractionStrategy::Raw, raw).expect("raw parse");
        assert_eq!(parsed.replacement, "hello");
    }
}
