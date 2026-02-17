use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref PRIVATE_KEY_BLOCK_RE: Regex = Regex::new(
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----"
    )
    .expect("valid private key regex");
    static ref BEARER_TOKEN_RE: Regex =
        Regex::new(r"(?i)(authorization\s*:\s*bearer\s+)[A-Za-z0-9\-._~+/]+=*")
            .expect("valid bearer regex");
    static ref SECRET_ASSIGNMENT_RE: Regex = Regex::new(
        r#"(?im)\b(api[_-]?key|token|secret|password)\b\s*[:=]\s*["']?([A-Za-z0-9_\-./+=]{8,})["']?"#
    )
    .expect("valid secret assignment regex");
    static ref AWS_ACCESS_KEY_RE: Regex =
        Regex::new(r"\bAKIA[0-9A-Z]{16}\b").expect("valid aws key regex");
    static ref GITHUB_TOKEN_RE: Regex =
        Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b").expect("valid github token regex");
}

/// Redact common high-risk token patterns.
pub fn redact_high_risk_tokens(input: &str) -> String {
    let mut out = input.to_string();
    out = PRIVATE_KEY_BLOCK_RE
        .replace_all(&out, "[REDACTED_PRIVATE_KEY]")
        .into_owned();
    out = BEARER_TOKEN_RE
        .replace_all(&out, "${1}[REDACTED_TOKEN]")
        .into_owned();
    out = SECRET_ASSIGNMENT_RE
        .replace_all(&out, "$1=[REDACTED]")
        .into_owned();
    out = AWS_ACCESS_KEY_RE
        .replace_all(&out, "[REDACTED_AWS_KEY]")
        .into_owned();
    out = GITHUB_TOKEN_RE
        .replace_all(&out, "[REDACTED_GITHUB_TOKEN]")
        .into_owned();
    out
}

/// Truncate by byte budget on UTF-8 char boundaries and append a notice.
pub fn truncate_utf8_with_notice(input: &str, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    const NOTICE: &str = "\n[truncated]";
    let notice_bytes = NOTICE.len();
    let budget = max_bytes.saturating_sub(notice_bytes).max(1);

    let mut end = 0usize;
    for (idx, ch) in input.char_indices() {
        let next = idx + ch.len_utf8();
        if next > budget {
            break;
        }
        end = next;
    }
    if end == 0 {
        return NOTICE.to_string();
    }

    let mut out = input[..end].to_string();
    out.push_str(NOTICE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_private_key_block() {
        let s = "-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----";
        assert_eq!(redact_high_risk_tokens(s), "[REDACTED_PRIVATE_KEY]");
    }

    #[test]
    fn redacts_assignment_tokens() {
        let s = "API_KEY=sk-secret-value";
        assert_eq!(redact_high_risk_tokens(s), "API_KEY=[REDACTED]");
    }

    #[test]
    fn truncates_on_utf8_boundary() {
        let s = "abcådef";
        let out = truncate_utf8_with_notice(s, 6);
        assert!(out.ends_with("[truncated]"));
    }
}
