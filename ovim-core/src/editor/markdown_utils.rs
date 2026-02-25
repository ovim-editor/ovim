//! Minimal markdown conceal helpers shared by the editor core for wrap/cursor math.

/// Returns a line with inline link/image URLs removed, leaving only labels/alt text.
/// Used to keep wrap/cursor calculations consistent with renderer conceal.
pub fn conceal_for_wrap(line: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut out = String::new();

    while i < len {
        if bytes[i] == b'[' {
            if let Some(close_label) = line[i + 1..].find(']') {
                let label_end = i + 1 + close_label;
                // [label](url)
                if label_end + 1 < len && bytes[label_end + 1] == b'(' {
                    if let Some(close_paren_rel) = line[label_end + 2..].find(')') {
                        let url_end = label_end + 2 + close_paren_rel;
                        out.push_str(&line[i + 1..label_end]);
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        } else if bytes[i] == b'!' && i + 1 < len && bytes[i + 1] == b'[' {
            if let Some(close_label) = line[i + 2..].find(']') {
                let label_end = i + 2 + close_label;
                if label_end + 1 < len && bytes[label_end + 1] == b'(' {
                    if let Some(close_paren_rel) = line[label_end + 2..].find(')') {
                        let url_end = label_end + 2 + close_paren_rel;
                        let alt = line[i + 2..label_end].trim();
                        if alt.is_empty() {
                            out.push('⧉');
                        } else {
                            out.push_str("⧉ ");
                            out.push_str(alt);
                        }
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
                    out.push_str(display);
                    i = end + 1;
                    continue;
                }
            }
        }

        // default: copy current char
        out.push(line[i..].chars().next().unwrap());
        i += line[i..].chars().next().unwrap().len_utf8();
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conceals_link_url() {
        let out = conceal_for_wrap("[label](https://example.com/path)");
        assert_eq!(out, "label");
    }

    #[test]
    fn conceals_image_alt() {
        let out = conceal_for_wrap("![alt text](img.png)");
        assert_eq!(out, "⧉ alt text");
    }

    #[test]
    fn leaves_plain_text() {
        let s = "nothing special";
        assert_eq!(conceal_for_wrap(s), s);
    }
}
