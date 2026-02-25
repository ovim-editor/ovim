//! Lightweight markdown conceal and table helpers used by the renderer.

use crate::display::char_display_width;

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

#[derive(Debug, Clone)]
pub struct TableCell {
    pub display: String,
    pub width: usize,
}

#[derive(Debug, Clone)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
}

pub fn parse_table_row(line: &str, conceal_links: bool) -> Option<TableRow> {
    if !line.contains('|') {
        return None;
    }
    let mut cells = Vec::new();
    for token in line.trim_matches('|').split('|') {
        let raw = token.trim();
        let display = if conceal_links {
            let spans = scan_markdown_conceal(raw);
            if spans.is_empty() {
                raw.to_string()
            } else {
                apply_conceal(raw, &spans).text
            }
        } else {
            raw.to_string()
        };
        let width = display.chars().map(char_display_width).sum();
        cells.push(TableCell { display, width });
    }
    if cells.is_empty() {
        None
    } else {
        Some(TableRow { cells })
    }
}

pub fn is_separator_row(line: &str, expected_cols: usize) -> bool {
    let tokens: Vec<&str> = line.trim_matches('|').split('|').collect();
    let mut count = 0;
    for t in tokens {
        let trimmed = t.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().all(|c| c == '-' || c == ':') {
            count += 1;
        } else {
            return false;
        }
    }
    count == expected_cols && count > 0
}

pub fn render_table_block(
    rows: &[TableRow],
    widths: &[usize],
    border: bool,
) -> Vec<(String, Vec<usize>)> {
    let cols = widths.len();
    let mut out = Vec::new();

    let horiz = |left: &str, mid: &str, right: &str| -> String {
        let mut s = String::new();
        s.push_str(left);
        for (i, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(*w + 2));
            s.push_str(if i + 1 == cols { right } else { mid });
        }
        s
    };

    if border {
        out.push((horiz("╭", "┬", "╮"), vec![0]));
    }

    for (r_idx, row) in rows.iter().enumerate() {
        let mut line = String::new();
        if border {
            line.push('│');
        }
        for (c_idx, cell) in row.cells.iter().enumerate() {
            line.push(' ');
            line.push_str(&cell.display);
            let pad = widths[c_idx].saturating_sub(cell.width);
            if pad > 0 {
                line.push_str(&" ".repeat(pad));
            }
            line.push(' ');
            if border {
                line.push('│');
            } else if c_idx + 1 < cols {
                line.push('|');
            }
        }
        out.push((line, vec![0]));
        if r_idx == 0 && border {
            out.push((horiz("├", "┼", "┤"), vec![0]));
        }
    }

    if border {
        out.push((horiz("╰", "┴", "╯"), vec![0]));
    }

    out
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

    #[test]
    fn table_parses_and_renders() {
        let header = parse_table_row("| A | B |", true).unwrap();
        let sep_ok = is_separator_row("| --- | --- |", header.cells.len());
        assert!(sep_ok);
        let body = parse_table_row("| 1 | 2 |", true).unwrap();
        let widths = vec![
            header.cells[0].width.max(body.cells[0].width),
            header.cells[1].width,
        ];
        let rendered = render_table_block(&[header, body], &widths, true);
        assert!(!rendered.is_empty());
        // should have top border + header + mid + body + bottom = 5 lines
        assert_eq!(rendered.len(), 5);
    }
}
