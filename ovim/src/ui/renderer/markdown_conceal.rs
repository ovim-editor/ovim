//! Lightweight markdown conceal helpers used by the renderer.

/// A span of source bytes to replace with a string in display space.
#[derive(Debug, Clone)]
pub struct ConcealSpan {
    pub src_start: usize,
    pub src_end: usize,
    pub replacement: String,
}

/// Result of conceal application.
#[derive(Debug, Clone)]
pub struct LineTransform {
    pub text: String,
    /// src byte offset -> view char index (len = src.len()+1)
    pub src_to_view: Vec<usize>,
}

impl LineTransform {
    pub fn identity(src: &str) -> Self {
        let mut src_to_view = Vec::with_capacity(src.len() + 1);
        let mut view_idx = 0;
        for (_byte_idx, _) in src.char_indices() {
            src_to_view.push(view_idx);
            view_idx += 1;
        }
        src_to_view.push(view_idx);
        Self {
            text: src.to_string(),
            src_to_view,
        }
    }
}

/// Apply non-overlapping conceal spans to `src`.
pub fn apply_conceal(src: &str, spans: &[ConcealSpan]) -> LineTransform {
    if spans.is_empty() {
        return LineTransform::identity(src);
    }

    let mut out = String::new();
    let mut src_to_view = vec![0usize; src.len() + 1];
    let mut view_idx = 0usize;
    let mut cursor = 0usize;

    for span in spans {
        let start = span.src_start.min(src.len());
        let end = span.src_end.min(src.len());

        // copy text before the span
        if cursor < start {
            for (b, ch) in src[cursor..start].char_indices() {
                src_to_view[cursor + b] = view_idx;
                out.push(ch);
                view_idx += 1;
            }
            cursor = start;
        }

        // map concealed bytes to current view index (start of replacement)
        for b in cursor..end {
            src_to_view[b] = view_idx;
        }

        for ch in span.replacement.chars() {
            out.push(ch);
            view_idx += 1;
        }

        cursor = end;
    }

    // tail
    if cursor < src.len() {
        for (b, ch) in src[cursor..].char_indices() {
            src_to_view[cursor + b] = view_idx;
            out.push(ch);
            view_idx += 1;
        }
        cursor = src.len();
    }

    src_to_view[cursor] = view_idx;

    LineTransform {
        text: out,
        src_to_view,
    }
}

/// Simple link/image conceal scanner.
pub fn scan_markdown_conceal(line: &str) -> Vec<ConcealSpan> {
    let mut spans = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'[' {
            if let Some(close) = line[i + 1..].find(']') {
                let label_end = i + 1 + close;
                if label_end + 1 < len && bytes[label_end + 1] == b'(' {
                    if let Some(paren) = line[label_end + 2..].find(')') {
                        let url_end = label_end + 2 + paren;
                        let label = &line[i + 1..label_end];
                        spans.push(ConcealSpan {
                            src_start: i,
                            src_end: url_end + 1,
                            replacement: label.to_string(),
                        });
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        } else if bytes[i] == b'!' && i + 1 < len && bytes[i + 1] == b'[' {
            if let Some(close) = line[i + 2..].find(']') {
                let label_end = i + 2 + close;
                if label_end + 1 < len && bytes[label_end + 1] == b'(' {
                    if let Some(paren) = line[label_end + 2..].find(')') {
                        let url_end = label_end + 2 + paren;
                        let alt = line[i + 2..label_end].trim();
                        let replacement = if alt.is_empty() {
                            "⧉".to_string()
                        } else {
                            format!("⧉ {}", alt)
                        };
                        spans.push(ConcealSpan {
                            src_start: i,
                            src_end: url_end + 1,
                            replacement,
                        });
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        } else if bytes[i] == b'<' {
            if let Some(close) = line[i + 1..].find('>') {
                let end = i + 1 + close;
                let inner = &line[i + 1..end];
                if inner.starts_with("http://") || inner.starts_with("https://") {
                    let display = inner
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_end_matches('/');
                    spans.push(ConcealSpan {
                        src_start: i,
                        src_end: end + 1,
                        replacement: display.to_string(),
                    });
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conceal_link() {
        let spans = scan_markdown_conceal("[label](http://example.com)");
        let t = apply_conceal("[label](http://example.com)", &spans);
        assert_eq!(t.text, "label");
        // first byte maps to view 0
        assert_eq!(t.src_to_view[0], 0);
    }

    #[test]
    fn conceal_image_alt() {
        let spans = scan_markdown_conceal("![alt text](img.png)");
        let t = apply_conceal("![alt text](img.png)", &spans);
        assert!(t.text.starts_with("⧉ alt"));
    }
}
