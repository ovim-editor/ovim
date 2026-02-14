use crate::ai::types::EditFormat;
use anyhow::{anyhow, Result};
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

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

// ---------------------------------------------------------------------------
// Elision detection
// ---------------------------------------------------------------------------

static ELISION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:(?://|#|--|/\*)\s*\.\.\..*$|(?://|#|--|/\*)\s*(?:rest of|remaining|unchanged|existing code|omitted|same as|continues)|^\s*\.\.\.\s*$)",
    )
    .expect("elision regex should compile")
});

/// Scan replacement text for elision patterns (comments with `...`, placeholder
/// phrases like "rest of code", or bare `...` lines). Returns the matched lines;
/// an empty vec means no elision was detected.
pub fn detect_elision(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    for line in text.lines() {
        if !ELISION_RE.is_match(line) {
            continue;
        }

        // --- false-positive guards ---

        // Spread/rest syntax: ...args, ...rest, ..Default::default()
        if line.contains("...") {
            let trimmed = line.trim();
            // Bare `...` on its own line is a real elision — don't skip it.
            if trimmed != "..." {
                // `...identifier` patterns (JS spread, Rust ..)
                if let Some(pos) = trimmed.find("...") {
                    let after = &trimmed[pos + 3..];
                    if after.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                        // Check it's not inside a comment prefix
                        let before = trimmed[..pos].trim();
                        if !before.ends_with("//")
                            && !before.ends_with('#')
                            && !before.ends_with("--")
                            && !before.ends_with("/*")
                        {
                            continue;
                        }
                    }
                }

                // `...` inside string literals: "..." or '...'
                if (trimmed.contains("\"...\"") || trimmed.contains("'...'"))
                    && !trimmed.starts_with("//")
                    && !trimmed.starts_with('#')
                    && !trimmed.starts_with("--")
                    && !trimmed.starts_with("/*")
                {
                    continue;
                }

                // `...` appearing mid-token (e.g., `loading...`)
                if let Some(pos) = trimmed.find("...") {
                    if pos > 0 {
                        let before_char = trimmed.as_bytes()[pos - 1];
                        if before_char.is_ascii_alphanumeric() {
                            let before_prefix = trimmed[..pos].trim();
                            if !before_prefix.ends_with("//")
                                && !before_prefix.ends_with('#')
                                && !before_prefix.ends_with("--")
                                && !before_prefix.ends_with("/*")
                            {
                                continue;
                            }
                        }
                    }
                }
            }
        }

        hits.push(line.trim().to_string());
    }
    hits
}

/// Extract the AI response and check for elision patterns in the replacement text.
/// Returns the extracted response plus any elision markers found (empty = clean).
pub fn extract_and_check_elision(
    format: &EditFormat,
    raw_output: &str,
) -> Result<(AiExtractedResponse, Vec<String>)> {
    let extracted = extract_response(format, raw_output)?;
    let elision = detect_elision(&extracted.replacement);
    Ok((extracted, elision))
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

    // -----------------------------------------------------------------------
    // Elision detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_elision_comment_patterns() {
        let text = "fn foo() {\n    // ... rest of code\n}\n";
        let hits = detect_elision(text);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("rest of code"));

        let text2 = "def bar():\n    # ... remaining unchanged\n";
        let hits2 = detect_elision(text2);
        assert_eq!(hits2.len(), 1);

        let text3 = "fn baz() {\n    -- ... existing code omitted\n}\n";
        let hits3 = detect_elision(text3);
        assert_eq!(hits3.len(), 1);

        let text4 = "fn qux() {\n    /* ... rest of implementation */\n}\n";
        let hits4 = detect_elision(text4);
        assert_eq!(hits4.len(), 1);
    }

    #[test]
    fn test_detect_elision_keyword_without_dots() {
        let text = "fn foo() {\n    // remaining unchanged\n}\n";
        let hits = detect_elision(text);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("remaining unchanged"));
    }

    #[test]
    fn test_detect_elision_bare_ellipsis() {
        let text = "fn foo() {\n    do_thing();\n    ...\n    do_other();\n}\n";
        let hits = detect_elision(text);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], "...");
    }

    #[test]
    fn test_detect_elision_negative_cases() {
        // Spread/rest syntax
        assert!(detect_elision("let items = [...args, extra];").is_empty());
        assert!(detect_elision("fn foo(...rest: any[]) {}").is_empty());

        // Rust struct update syntax
        assert!(detect_elision("..Default::default()").is_empty());

        // Mid-token ellipsis
        assert!(detect_elision("let msg = \"loading...\";").is_empty());

        // Normal comments without elision patterns
        assert!(detect_elision("// This is a normal comment").is_empty());
        assert!(detect_elision("# Just a regular comment").is_empty());

        // String literal containing ellipsis
        assert!(detect_elision("let s = \"...\";").is_empty());
    }

    #[test]
    fn test_detect_elision_empty() {
        assert!(detect_elision("").is_empty());
    }

    #[test]
    fn test_extract_and_check_elision_clean() {
        let raw = r#"{"replacement":"fn x() { return 1; }", "log":["done"]}"#;
        let (extracted, elision) =
            extract_and_check_elision(&EditFormat::Json, raw).expect("should parse");
        assert_eq!(extracted.replacement, "fn x() { return 1; }");
        assert!(elision.is_empty());
    }

    #[test]
    fn test_extract_and_check_elision_with_markers() {
        let raw =
            r#"{"replacement":"fn x() {\n    // ... rest of code\n}", "log":["done"]}"#;
        let (extracted, elision) =
            extract_and_check_elision(&EditFormat::Json, raw).expect("should parse");
        assert!(!extracted.replacement.is_empty());
        assert!(!elision.is_empty());
        assert!(elision[0].contains("rest of code"));
    }
}
