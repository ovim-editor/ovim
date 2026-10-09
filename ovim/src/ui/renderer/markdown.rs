//! Markdown parser for hover window rendering
//!
//! Parses LSP hover markdown and converts it to styled text spans for ratatui.
//! Supports links, images, emphasis, `inline code`, ```code blocks```, and basic structure.

use crate::syntax::{HighlightGroup, SyntaxHighlighter, Theme};
use ovim_core::language_catalog::LanguageCatalog;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::ops::Range;
use unicode_width::UnicodeWidthStr;

/// Colors for markdown rendering (Catppuccin-inspired)
pub mod colors {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(30, 30, 46);
    pub const TEXT: Color = Color::Rgb(205, 214, 244);
    pub const BORDER: Color = Color::Rgb(137, 180, 250);
    pub const BOLD: Color = Color::Rgb(245, 194, 231);
    pub const CODE_SPAN_BG: Color = Color::Rgb(49, 50, 68);
    pub const CODE_SPAN_FG: Color = Color::Rgb(148, 226, 213);
    pub const CODE_BLOCK_BG: Color = Color::Rgb(24, 24, 37);
    pub const CODE_BLOCK_FG: Color = Color::Rgb(166, 227, 161);
    pub const HEADING: Color = Color::Rgb(245, 194, 231);
    pub const PARAM: Color = Color::Rgb(250, 179, 135);
    pub const RETURN: Color = Color::Rgb(166, 227, 161);
    pub const LINK: Color = Color::Rgb(137, 180, 250);
    pub const MUTED: Color = Color::Rgb(127, 132, 156);
}

/// Parsed markdown element
#[derive(Debug, Clone)]
pub enum MarkdownElement {
    /// Plain text
    Text(String),
    /// Bold text (**text**)
    Bold(String),
    /// Italic text (`_text_` or `*text*`)
    Italic(String),
    /// Inline code (`code`)
    InlineCode(String),
    /// Link (`[label](destination)`).
    Link { label: String, destination: String },
    /// Image (`![alt](source)`). Terminal previews use a compact text substitute.
    Image(String),
    /// Code block with optional language
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    /// Display math delimited by `$$...$$` or `\[...\]`.
    DisplayMath(String),
    /// GitHub-flavored Markdown table.
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Heading (# Title)
    Heading(#[allow(dead_code)] u8, String),
    /// Horizontal rule (---)
    HorizontalRule,
    /// Line break
    LineBreak,
}

/// Parse markdown text into elements
pub fn parse_markdown(text: &str) -> Vec<MarkdownElement> {
    let mut elements = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lang: Option<String> = None;
    let mut code_block_content = String::new();
    let mut math_block: Option<(&'static str, &'static str, String)> = None;

    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        // Handle code blocks
        if math_block.is_none() && line.starts_with("```") {
            if in_code_block {
                // End of code block
                elements.push(MarkdownElement::CodeBlock {
                    language: code_block_lang.take(),
                    code: code_block_content.trim_end().to_string(),
                });
                code_block_content.clear();
                in_code_block = false;
            } else {
                // Start of code block
                in_code_block = true;
                let lang = line.trim_start_matches('`').trim();
                code_block_lang = if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                };
            }

            continue;
        }

        if in_code_block {
            if !code_block_content.is_empty() {
                code_block_content.push('\n');
            }
            code_block_content.push_str(line);
            continue;
        }

        if let Some(separator) = lines.peek().copied() {
            let headers = split_table_row(line);
            let separators = split_table_row(separator);
            if headers.len() >= 2
                && headers.len() == separators.len()
                && separators.iter().all(|cell| is_table_separator(cell))
            {
                lines.next();
                let mut rows = Vec::new();
                while let Some(candidate) = lines.peek().copied() {
                    let mut cells = split_table_row(candidate);
                    if cells.len() < 2 || candidate.trim().is_empty() {
                        break;
                    }
                    lines.next();
                    if cells.len() > headers.len() {
                        let overflow = cells.split_off(headers.len() - 1);
                        cells.push(overflow.join(" | "));
                    }
                    cells.resize(headers.len(), String::new());
                    rows.push(cells);
                }
                elements.push(MarkdownElement::Table { headers, rows });
                continue;
            }
        }

        if let Some((opener, closer, mut content)) = math_block.take() {
            if line.trim() == closer {
                elements.push(MarkdownElement::DisplayMath(content.trim().to_string()));
            } else {
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(line);
                math_block = Some((opener, closer, content));
            }
            continue;
        }

        let trimmed = line.trim();
        let delimiter = if trimmed.starts_with("$$") {
            Some(("$$", "$$"))
        } else if trimmed.starts_with("\\[") {
            Some(("\\[", "\\]"))
        } else {
            None
        };
        if let Some((opener, closer)) = delimiter {
            let remainder = trimmed.strip_prefix(opener).unwrap_or_default();
            if let Some(math) = remainder.strip_suffix(closer) {
                if !math.trim().is_empty() {
                    elements.push(MarkdownElement::DisplayMath(math.trim().to_string()));
                }
            } else {
                math_block = Some((opener, closer, remainder.to_string()));
            }
            continue;
        }

        // Handle headings
        if line.starts_with('#') {
            let level = line.chars().take_while(|c| *c == '#').count() as u8;
            let text = line.trim_start_matches('#').trim();
            elements.push(MarkdownElement::Heading(level, text.to_string()));
            continue;
        }

        // Handle horizontal rules
        if line.trim() == "---" || line.trim() == "***" || line.trim() == "___" {
            elements.push(MarkdownElement::HorizontalRule);
            continue;
        }

        // Handle empty lines
        if line.trim().is_empty() {
            elements.push(MarkdownElement::LineBreak);
            continue;
        }

        // Parse inline elements
        parse_inline_elements(line, &mut elements);
        elements.push(MarkdownElement::LineBreak);
    }

    // Handle unclosed code block
    if in_code_block && !code_block_content.is_empty() {
        elements.push(MarkdownElement::CodeBlock {
            language: code_block_lang,
            code: code_block_content,
        });
    }
    if let Some((opener, _, content)) = math_block {
        elements.push(MarkdownElement::Text(opener.to_string()));
        elements.push(MarkdownElement::LineBreak);
        for line in content.lines() {
            elements.push(MarkdownElement::Text(line.to_string()));
            elements.push(MarkdownElement::LineBreak);
        }
    }

    elements
}

fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let inner = trimmed
        .strip_prefix('|')
        .unwrap_or(trimmed)
        .strip_suffix('|')
        .unwrap_or_else(|| trimmed.strip_prefix('|').unwrap_or(trimmed));
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    let mut in_code = false;
    while let Some(character) = chars.next() {
        match character {
            '`' => {
                in_code = !in_code;
                cell.push(character);
            }
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cell.push('|');
            }
            '|' if !in_code => {
                cells.push(cell.trim().to_string());
                cell.clear();
            }
            _ => cell.push(character),
        }
    }
    cells.push(cell.trim().to_string());
    cells
}

fn is_table_separator(cell: &str) -> bool {
    let dashes = cell.trim().trim_start_matches(':').trim_end_matches(':');
    dashes.len() >= 3 && dashes.chars().all(|character| character == '-')
}

/// Parse inline markdown elements from a line.
fn parse_inline_elements(line: &str, elements: &mut Vec<MarkdownElement>) {
    let mut current_text = String::new();
    let chars = line.chars().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                if !current_text.is_empty() {
                    elements.push(MarkdownElement::Text(std::mem::take(&mut current_text)));
                }
                let content_start = index + 2;
                let mut end = content_start;
                while end + 1 < chars.len() {
                    if chars[end] == '*' && chars[end + 1] == '*' {
                        break;
                    }
                    end += 1;
                }

                if end + 1 < chars.len() {
                    let bold_text = chars[content_start..end].iter().collect::<String>();
                    push_bold_elements(&bold_text, elements);
                    index = end + 2;
                } else {
                    let bold_text = chars[content_start..].iter().collect::<String>();
                    elements.push(MarkdownElement::Text(format!("**{bold_text}")));
                    index = chars.len();
                }
            }
            '`' => {
                if !current_text.is_empty() {
                    elements.push(MarkdownElement::Text(std::mem::take(&mut current_text)));
                }
                let content_start = index + 1;
                if let Some(relative_end) = chars[content_start..]
                    .iter()
                    .position(|character| *character == '`')
                {
                    let end = content_start + relative_end;
                    let code_text = chars[content_start..end].iter().collect::<String>();
                    elements.push(MarkdownElement::InlineCode(code_text));
                    index = end + 1;
                } else {
                    let code_text = chars[content_start..].iter().collect::<String>();
                    elements.push(MarkdownElement::Text(format!("`{code_text}")));
                    index = chars.len();
                }
            }
            '*' | '_' if is_emphasis_opener(&chars, index) => {
                let delimiter = chars[index];
                if let Some(end) = find_emphasis_closer(&chars, index + 1, delimiter) {
                    if !current_text.is_empty() {
                        elements.push(MarkdownElement::Text(std::mem::take(&mut current_text)));
                    }
                    let text = chars[index + 1..end].iter().collect::<String>();
                    elements.push(MarkdownElement::Italic(text));
                    index = end + 1;
                } else {
                    current_text.push(delimiter);
                    index += 1;
                }
            }
            '!' | '[' => {
                if let Some((element, next_index)) = parse_link_or_image(&chars, index) {
                    if !current_text.is_empty() {
                        elements.push(MarkdownElement::Text(std::mem::take(&mut current_text)));
                    }
                    elements.push(element);
                    index = next_index;
                } else {
                    current_text.push(chars[index]);
                    index += 1;
                }
            }
            _ => {
                current_text.push(chars[index]);
                index += 1;
            }
        }
    }

    if !current_text.is_empty() {
        elements.push(MarkdownElement::Text(current_text));
    }
}

fn is_emphasis_opener(chars: &[char], index: usize) -> bool {
    let delimiter = chars[index];
    let Some(next) = chars.get(index + 1) else {
        return false;
    };
    if (index > 0 && chars[index - 1] == '\\') || next.is_whitespace() || *next == delimiter {
        return false;
    }
    delimiter != '_' || index == 0 || !chars[index - 1].is_alphanumeric()
}

fn find_emphasis_closer(chars: &[char], start: usize, delimiter: char) -> Option<usize> {
    (start..chars.len()).find(|&index| {
        chars[index] == delimiter
            && chars[index - 1] != '\\'
            && !chars[index - 1].is_whitespace()
            && (delimiter != '_'
                || chars
                    .get(index + 1)
                    .is_none_or(|next| !next.is_alphanumeric()))
    })
}

fn parse_link_or_image(chars: &[char], start: usize) -> Option<(MarkdownElement, usize)> {
    let is_image = chars.get(start) == Some(&'!');
    let open_bracket = start + usize::from(is_image);
    if chars.get(open_bracket) != Some(&'[') {
        return None;
    }

    let label_start = open_bracket + 1;
    let close_bracket = find_unescaped(chars, label_start, ']')?;
    if chars.get(close_bracket + 1) != Some(&'(') {
        return None;
    }

    let destination_start = close_bracket + 2;
    let mut index = destination_start;
    let mut depth = 1usize;
    while index < chars.len() {
        match chars[index] {
            '\\' => index = (index + 2).min(chars.len()),
            '(' => {
                depth += 1;
                index += 1;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let label = chars[label_start..close_bracket].iter().collect::<String>();
                    let destination = chars[destination_start..index].iter().collect::<String>();
                    let destination = markdown_destination(&destination);
                    let element = if is_image {
                        MarkdownElement::Image(label)
                    } else {
                        MarkdownElement::Link { label, destination }
                    };
                    return Some((element, index + 1));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn find_unescaped(chars: &[char], start: usize, needle: char) -> Option<usize> {
    let mut index = start;
    while index < chars.len() {
        if chars[index] == '\\' {
            index += 2;
        } else if chars[index] == needle {
            return Some(index);
        } else {
            index += 1;
        }
    }
    None
}

/// Preserve inline-code semantics inside a bold run instead of displaying
/// its backticks literally. The surrounding pieces remain bold; the code span
/// uses the stronger code treatment, matching ordinary inline code.
fn push_bold_elements(text: &str, elements: &mut Vec<MarkdownElement>) {
    for (index, part) in text.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        if index % 2 == 0 {
            elements.push(MarkdownElement::Bold(part.to_string()));
        } else {
            elements.push(MarkdownElement::InlineCode(part.to_string()));
        }
    }
}

/// Highlights a code block using tree-sitter syntax highlighting
/// Returns None if language is unknown or highlighting fails
type LineHighlights = Vec<Vec<(Range<usize>, HighlightGroup)>>;

/// Resolves the fence language against the process-wide catalog so plugin
/// languages highlight in chat, hover, and walkthrough markdown too.
fn highlight_code_block(language: &str, code: &str) -> Option<LineHighlights> {
    highlight_code_block_with(&LanguageCatalog::process(), language, code)
}

fn highlight_code_block_with(
    catalog: &LanguageCatalog,
    language: &str,
    code: &str,
) -> Option<LineHighlights> {
    let definition = catalog.detect_from_info_string(language)?;
    let syntax = definition.syntax.as_ref()?;
    let mut highlighter = SyntaxHighlighter::from_definition(definition.id(), syntax).ok()?;
    highlighter.parse(code);
    Some(highlighter.highlights_for_all_lines(code))
}

/// Renders a single code line with syntax highlights
fn render_code_line_with_highlights(
    line: &str,
    highlights: &[(Range<usize>, HighlightGroup)],
    theme: &Theme,
) -> Line<'static> {
    use unicode_segmentation::UnicodeSegmentation;

    let mut spans = Vec::new();
    spans.push(Span::styled(
        " ",
        Style::default().bg(colors::CODE_BLOCK_BG),
    )); // Leading padding

    let style_for = |group: Option<HighlightGroup>| {
        if let Some(g) = group {
            Style::default()
                .fg(crate::key_convert::convert_core_color(theme.get_color(g)))
                .bg(colors::CODE_BLOCK_BG)
        } else {
            Style::default()
                .fg(colors::CODE_BLOCK_FG)
                .bg(colors::CODE_BLOCK_BG)
        }
    };

    // Highlight ranges are BYTE offsets relative to the line start (see
    // `highlights_for_all_lines`), so group lookups use grapheme byte
    // positions. Preserve the full line for the caller’s style-aware wrapping.
    let mut run_group: Option<HighlightGroup> = None;
    let mut run_text = String::new();
    let mut run_started = false;

    for (byte_idx, grapheme) in line.grapheme_indices(true) {
        let group = highlights
            .iter()
            .find(|(range, _)| range.contains(&byte_idx))
            .map(|(_, g)| *g);
        if run_started && group != run_group {
            spans.push(Span::styled(
                std::mem::take(&mut run_text),
                style_for(run_group),
            ));
        }
        run_started = true;
        run_group = group;
        run_text.push_str(grapheme);
    }
    if !run_text.is_empty() {
        spans.push(Span::styled(run_text, style_for(run_group)));
    }

    spans.push(Span::styled(
        " ",
        Style::default().bg(colors::CODE_BLOCK_BG),
    )); // Trailing padding

    Line::from(spans)
}

fn table_border(left: char, join: char, right: char, widths: &[usize]) -> Line<'static> {
    let mut border = String::new();
    border.push(left);
    for (index, width) in widths.iter().enumerate() {
        border.push_str(&"─".repeat(width + 2));
        border.push(if index + 1 == widths.len() {
            right
        } else {
            join
        });
    }
    Line::from(Span::styled(border, Style::default().fg(colors::BORDER)))
}

/// A link's byte range in an unwrapped rendered line.
#[derive(Debug, Clone)]
pub(super) struct MarkdownLink {
    pub row: usize,
    pub bytes: Range<usize>,
    pub destination: String,
}

fn markdown_destination(source: &str) -> String {
    let source = source.trim();
    let destination = if let Some(source) = source.strip_prefix('<') {
        source
            .split_once('>')
            .map_or(source, |(destination, _)| destination)
    } else {
        source.split_whitespace().next().unwrap_or("")
    };
    let mut output = String::new();
    let mut chars = destination.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' && chars.peek().is_some_and(char::is_ascii_punctuation) {
            output.push(chars.next().unwrap());
        } else {
            output.push(character);
        }
    }
    output
}

fn append_link(
    spans: &mut Vec<Span<'static>>,
    links: &mut Vec<MarkdownLink>,
    row: usize,
    label: &str,
    destination: &str,
) {
    let start = spans.iter().map(|span| span.content.len()).sum();
    let text = format!("{} ↗", link_label(label));
    links.push(MarkdownLink {
        row,
        bytes: start..start + text.len(),
        destination: destination.to_string(),
    });
    spans.push(Span::styled(
        link_label(label).to_string(),
        Style::default()
            .fg(colors::LINK)
            .add_modifier(Modifier::UNDERLINED),
    ));
    spans.push(Span::styled(
        " ↗",
        Style::default()
            .fg(colors::MUTED)
            .add_modifier(Modifier::ITALIC),
    ));
}

fn append_table_cell_spans(
    cell: &str,
    text_style: Style,
    bold_style: Style,
    code_style: Style,
    spans: &mut Vec<Span<'static>>,
    links: &mut Vec<MarkdownLink>,
    row: usize,
) {
    let mut elements = Vec::new();
    parse_inline_elements(cell, &mut elements);
    for element in elements {
        let span = match element {
            MarkdownElement::Text(text) => Span::styled(text, text_style),
            MarkdownElement::Bold(text) => Span::styled(text, bold_style),
            MarkdownElement::Italic(text) => {
                Span::styled(text, text_style.add_modifier(Modifier::ITALIC))
            }
            MarkdownElement::InlineCode(code) => Span::styled(code, code_style),
            MarkdownElement::Link { label, destination } => {
                append_link(spans, links, row, &label, &destination);
                continue;
            }
            MarkdownElement::Image(alt) => Span::styled(
                format!("Image: {}", image_label(&alt)),
                Style::default()
                    .fg(colors::MUTED)
                    .add_modifier(Modifier::ITALIC),
            ),
            _ => continue,
        };
        spans.push(span);
    }
}

fn table_cell_width(cell: &str) -> usize {
    let mut elements = Vec::new();
    parse_inline_elements(cell, &mut elements);
    elements
        .iter()
        .map(|element| match element {
            MarkdownElement::Text(text)
            | MarkdownElement::Bold(text)
            | MarkdownElement::Italic(text)
            | MarkdownElement::InlineCode(text) => UnicodeWidthStr::width(text.as_str()),
            MarkdownElement::Link { label, .. } => UnicodeWidthStr::width(link_label(label)) + 2,
            MarkdownElement::Image(alt) => {
                UnicodeWidthStr::width(image_label(alt)) + "Image: ".len()
            }
            _ => 0,
        })
        .sum()
}

fn link_label(label: &str) -> &str {
    if label.trim().is_empty() {
        "link"
    } else {
        label
    }
}

fn image_label(alt: &str) -> &str {
    if alt.trim().is_empty() {
        "image"
    } else {
        alt
    }
}

fn render_table(
    headers: &[String],
    rows: &[Vec<String>],
    max_width: usize,
    text_style: Style,
    bold_style: Style,
    code_style: Style,
    links: &mut Vec<MarkdownLink>,
    first_row: usize,
) -> Vec<Line<'static>> {
    if headers.is_empty() {
        return Vec::new();
    }

    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(column, header)| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .fold(table_cell_width(header), |width, cell| {
                    width.max(table_cell_width(cell))
                })
        })
        .collect();
    let table_width = 1 + widths.iter().map(|width| width + 3).sum::<usize>();

    if table_width <= max_width {
        let mut lines = vec![table_border('┌', '┬', '┐', &widths)];
        let mut header_spans = vec![Span::styled("│ ", Style::default().fg(colors::BORDER))];
        for (index, header) in headers.iter().enumerate() {
            append_table_cell_spans(
                header,
                bold_style,
                bold_style,
                code_style,
                &mut header_spans,
                links,
                first_row + lines.len(),
            );
            let padding = widths[index].saturating_sub(table_cell_width(header));
            header_spans.push(Span::styled(
                format!(
                    "{} │{}",
                    " ".repeat(padding),
                    if index + 1 == headers.len() { "" } else { " " }
                ),
                Style::default().fg(colors::BORDER),
            ));
        }
        lines.push(Line::from(header_spans));
        lines.push(table_border('├', '┼', '┤', &widths));
        for row in rows {
            let mut spans = vec![Span::styled("│ ", Style::default().fg(colors::BORDER))];
            for (index, width) in widths.iter().enumerate() {
                let cell = row.get(index).map_or("", String::as_str);
                append_table_cell_spans(
                    cell,
                    text_style,
                    bold_style,
                    code_style,
                    &mut spans,
                    links,
                    first_row + lines.len(),
                );
                let padding = width.saturating_sub(table_cell_width(cell));
                spans.push(Span::styled(
                    format!(
                        "{} │{}",
                        " ".repeat(padding),
                        if index + 1 == widths.len() { "" } else { " " }
                    ),
                    Style::default().fg(colors::BORDER),
                ));
            }
            lines.push(Line::from(spans));
        }
        lines.push(table_border('└', '┴', '┘', &widths));
        return lines;
    }

    // Narrow layouts become vertical records. Each field remains one logical
    // line so the caller's style-aware wrapper can adapt it to any width.
    let mut lines = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        for (column, header) in headers.iter().enumerate() {
            let mut spans = Vec::new();
            append_table_cell_spans(
                header,
                bold_style,
                bold_style,
                code_style,
                &mut spans,
                links,
                first_row + lines.len(),
            );
            spans.push(Span::styled(": ", text_style));
            append_table_cell_spans(
                row.get(column).map_or("", String::as_str),
                text_style,
                bold_style,
                code_style,
                &mut spans,
                links,
                first_row + lines.len(),
            );
            lines.push(Line::from(spans));
        }
        if row_index + 1 < rows.len() {
            lines.push(Line::default());
        }
    }
    lines
}

/// Convert parsed markdown elements to styled ratatui Lines.
/// Text is preserved in full; callers wrap the styled lines to their viewport.
pub fn render_markdown(
    elements: &[MarkdownElement],
    max_width: usize,
    theme: Option<&Theme>,
) -> Vec<Line<'static>> {
    render_markdown_with_links(elements, max_width, theme).0
}

pub(super) fn render_markdown_with_links(
    elements: &[MarkdownElement],
    max_width: usize,
    theme: Option<&Theme>,
) -> (Vec<Line<'static>>, Vec<MarkdownLink>) {
    let mut links = Vec::new();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();

    let text_style = Style::default().fg(colors::TEXT);
    let bold_style = Style::default()
        .fg(colors::BOLD)
        .add_modifier(Modifier::BOLD);
    let italic_style = text_style.add_modifier(Modifier::ITALIC);
    let code_style = Style::default()
        .fg(colors::CODE_SPAN_FG)
        .bg(colors::CODE_SPAN_BG);
    let muted_style = Style::default()
        .fg(colors::MUTED)
        .add_modifier(Modifier::ITALIC);
    let heading_style = Style::default()
        .fg(colors::HEADING)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let code_block_style = Style::default()
        .fg(colors::CODE_BLOCK_FG)
        .bg(colors::CODE_BLOCK_BG);

    for element in elements {
        match element {
            MarkdownElement::Text(text) => {
                // Check for @param and @return annotations
                let styled_text = if text.contains("@param") || text.starts_with("@param") {
                    Span::styled(text.clone(), Style::default().fg(colors::PARAM))
                } else if text.contains("@return") || text.starts_with("@return") {
                    Span::styled(text.clone(), Style::default().fg(colors::RETURN))
                } else {
                    Span::styled(text.clone(), text_style)
                };
                current_spans.push(styled_text);
            }
            MarkdownElement::Bold(text) => {
                current_spans.push(Span::styled(text.clone(), bold_style));
            }
            MarkdownElement::Italic(text) => {
                current_spans.push(Span::styled(text.clone(), italic_style));
            }
            MarkdownElement::InlineCode(code) => {
                current_spans.push(Span::styled(code.clone(), code_style));
            }
            MarkdownElement::Link { label, destination } => {
                append_link(
                    &mut current_spans,
                    &mut links,
                    lines.len(),
                    label,
                    destination,
                );
            }
            MarkdownElement::Image(alt) => {
                current_spans.push(Span::styled("Image: ", muted_style));
                current_spans.push(Span::styled(image_label(alt).to_string(), text_style));
            }
            MarkdownElement::CodeBlock { language, code } => {
                // Flush current line
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                }

                // Try to get syntax highlights if we have a language and theme
                let highlights = language
                    .as_ref()
                    .and_then(|lang| highlight_code_block(lang, code));

                // Add code block lines
                for (line_idx, code_line) in code.lines().enumerate() {
                    // Try to render with syntax highlighting
                    if let (Some(hl), Some(theme)) = (&highlights, theme) {
                        if let Some(line_hl) = hl.get(line_idx) {
                            lines.push(render_code_line_with_highlights(code_line, line_hl, theme));
                            continue;
                        }
                    }

                    // Fallback: plain green style
                    lines.push(Line::from(Span::styled(
                        format!(" {code_line} "),
                        code_block_style,
                    )));
                }
            }
            MarkdownElement::Table { headers, rows } => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                }
                lines.extend(render_table(
                    headers,
                    rows,
                    max_width,
                    text_style,
                    bold_style,
                    code_style,
                    &mut links,
                    lines.len(),
                ));
            }
            MarkdownElement::DisplayMath(math) => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                }
                lines.push(Line::from(Span::styled("\\[", text_style)));
                for math_line in math.lines() {
                    lines.push(Line::from(Span::styled(math_line.to_string(), text_style)));
                }
                lines.push(Line::from(Span::styled("\\]", text_style)));
            }
            MarkdownElement::Heading(_, text) => {
                // Flush current line
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                }
                lines.push(Line::from(Span::styled(text.clone(), heading_style)));
            }
            MarkdownElement::HorizontalRule => {
                // Flush current line
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                }
                lines.push(Line::from(Span::styled(
                    "─".repeat(max_width.saturating_sub(2)),
                    Style::default().fg(colors::BORDER),
                )));
            }
            MarkdownElement::LineBreak => {
                if !current_spans.is_empty() {
                    lines.push(Line::from(current_spans.clone()));
                    current_spans.clear();
                } else {
                    lines.push(Line::from("")); // Empty line
                }
            }
        }
    }

    // Flush remaining spans
    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    (lines, links)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_destinations_survive_titles_parentheses_and_escapes() {
        for (source, expected) in [
            (
                r#"[Guide](https://example.com/a_(b) "Title")"#,
                "https://example.com/a_(b)",
            ),
            (r"[Source](src/main.rs:42:3)", "src/main.rs:42:3"),
            (r#"[Source](<my file.rs:2> "Title")"#, "my file.rs:2"),
            (
                r"[Guide](https://example.com/a\(b\))",
                "https://example.com/a(b)",
            ),
        ] {
            let elements = parse_markdown(source);
            let (_, links) = render_markdown_with_links(&elements, 80, None);
            assert_eq!(links.len(), 1, "{source}");
            assert_eq!(links[0].destination, expected);
        }
    }

    #[test]
    fn table_syntax_inside_code_fences_stays_code_and_has_no_links() {
        let source =
            "```markdown\n| A | B |\n| --- | --- |\n| [Guide](https://example.com) | Text |\n```";
        let elements = parse_markdown(source);
        assert!(
            matches!(elements.as_slice(), [MarkdownElement::CodeBlock { code, .. }] if code.contains("[Guide](https://example.com)"))
        );
        assert!(render_markdown_with_links(&elements, 80, None).1.is_empty());
    }

    #[test]
    fn test_parse_bold() {
        let elements = parse_markdown("Hello **world**!");
        assert!(elements
            .iter()
            .any(|e| matches!(e, MarkdownElement::Bold(s) if s == "world")));
    }

    #[test]
    fn test_parse_inline_code() {
        let elements = parse_markdown("Use `println!` for output");
        assert!(elements
            .iter()
            .any(|e| matches!(e, MarkdownElement::InlineCode(s) if s == "println!")));
    }

    #[test]
    fn test_parse_inline_code_nested_in_bold() {
        let elements = parse_markdown("**Use `k` while history is focused.**");
        assert!(elements
            .iter()
            .any(|element| matches!(element, MarkdownElement::Bold(text) if text == "Use ")));
        assert!(elements
            .iter()
            .any(|element| matches!(element, MarkdownElement::InlineCode(text) if text == "k")));
        assert!(elements.iter().any(|element| {
            matches!(element, MarkdownElement::Bold(text) if text == " while history is focused.")
        }));
    }

    #[test]
    fn parses_emphasis_without_mangling_snake_case_identifiers() {
        let elements = parse_markdown("_available_ and *portable*, but keep some_name_here");

        assert!(elements.iter().any(
            |element| matches!(element, MarkdownElement::Italic(text) if text == "available")
        ));
        assert!(elements
            .iter()
            .any(|element| matches!(element, MarkdownElement::Italic(text) if text == "portable")));
        assert!(elements.iter().any(
            |element| matches!(element, MarkdownElement::Text(text) if text.contains("some_name_here"))
        ));
    }

    #[test]
    fn inline_code_does_not_add_padding_for_concealed_backticks() {
        let elements = parse_markdown("Use `code` now");
        let lines = render_markdown(&elements, 80, None);
        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert_eq!(rendered, "Use code now");
        let code = lines[0]
            .spans
            .iter()
            .find(|span| span.content == "code")
            .expect("inline code span");
        assert_eq!(code.style.bg, Some(colors::CODE_SPAN_BG));
    }

    #[test]
    fn lsp_links_and_images_render_without_markdown_syntax_or_payloads() {
        let markdown = concat!(
            "The `span` element is a generic inline container.\n\n",
            "![Baseline icon](data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iMTgi) _Widely available_\n\n",
            "![](data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUA)\n\n",
            "[MDN Reference](https://developer.mozilla.org/docs/Web/HTML/Reference/Elements/span)"
        );
        let elements = parse_markdown(markdown);
        let lines = render_markdown(&elements, 100, None);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("The span element is a generic inline container."));
        assert!(rendered.contains("Image: Baseline icon"));
        assert!(rendered.contains("Image: image"));
        assert!(rendered.contains("Widely available"));
        assert!(rendered.contains("MDN Reference ↗"));
        assert!(!rendered.contains("!["));
        assert!(!rendered.contains("]("));
        assert!(!rendered.contains("data:image"));

        let italic = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == "Widely available")
            .expect("rendered italic text");
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));

        let link = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == "MDN Reference")
            .expect("rendered link label");
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn unmatched_inline_delimiters_remain_visible() {
        let elements = parse_markdown("Keep `this and **that");
        assert!(elements.iter().any(
            |element| matches!(element, MarkdownElement::Text(text) if text == "`this and **that")
        ));
    }

    #[test]
    fn test_parse_code_block() {
        let elements = parse_markdown("```rust\nfn main() {}\n```");
        assert!(elements.iter().any(|e| matches!(e,
            MarkdownElement::CodeBlock { language: Some(lang), code }
            if lang == "rust" && code.contains("fn main")
        )));
    }

    #[test]
    fn bold_style_boundary_does_not_split_a_logical_line() {
        let elements = parse_markdown(
            "So C420 **does not produce a contradiction and does not prove existence**. It merely narrows the range.",
        );
        let lines = render_markdown(&elements, 30, None);

        assert_eq!(lines.len(), 1);
        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(
            rendered,
            "So C420 does not produce a contradiction and does not prove existence. It merely narrows the range."
        );
    }

    #[test]
    fn parses_github_style_table_and_escaped_pipes() {
        let elements = parse_markdown(
            "| Area | Nula | Nushell |\n|:---|---:|---|\n| Primary role | Embedded data transformation | Interactive system shell |\n| Syntax | `a \\| b` | **pipelines** |",
        );
        let table = elements
            .iter()
            .find_map(|element| match element {
                MarkdownElement::Table { headers, rows } => Some((headers, rows)),
                _ => None,
            })
            .expect("parsed table");
        assert_eq!(table.0, &["Area", "Nula", "Nushell"]);
        assert_eq!(table.1.len(), 2);
        assert_eq!(table.1[1][1], "`a | b`");
    }

    #[test]
    fn table_uses_bordered_columns_when_it_fits() {
        let elements = parse_markdown(
            "| Name | Role |\n|---|---|\n| **Nula** | `Data` |\n| Nushell | Shell |",
        );
        let lines = render_markdown(&elements, 40, None);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(rendered.first().is_some_and(|line| line.starts_with('┌')));
        assert!(rendered
            .iter()
            .all(|line| UnicodeWidthStr::width(line.as_str()) <= 40));
        assert!(rendered.iter().any(|line| line.contains("Nushell")));
    }

    #[test]
    fn wide_table_becomes_wrappable_vertical_records() {
        let elements = parse_markdown(
            "| Area | Nula | Nushell |\n|---|---|---|\n| Primary role | Embedded data transformation | Interactive system shell |",
        );
        let lines = render_markdown(&elements, 30, None);
        let rendered = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[0], "Area: Primary role");
        assert_eq!(rendered[1], "Nula: Embedded data transformation");
        assert_eq!(rendered[2], "Nushell: Interactive system shell");
        assert!(rendered.iter().all(|line| !line.contains('│')));
    }

    #[test]
    fn parses_multiline_bracket_display_math() {
        let elements = parse_markdown("Before\n\\[\nF(R) \\le \\theta F(2R)\n\\]\nAfter");
        assert!(elements.iter().any(|element| {
            matches!(element, MarkdownElement::DisplayMath(math) if math == "F(R) \\le \\theta F(2R)")
        }));
    }

    #[test]
    fn parses_single_line_dollar_display_math() {
        let elements = parse_markdown("$$x^2 + y^2 = z^2$$");
        assert!(matches!(
            elements.as_slice(),
            [MarkdownElement::DisplayMath(math)] if math == "x^2 + y^2 = z^2"
        ));
    }

    #[test]
    fn math_delimiters_inside_code_blocks_are_not_parsed() {
        let elements = parse_markdown("```latex\n$$x^2$$\n```");
        assert!(elements
            .iter()
            .all(|element| !matches!(element, MarkdownElement::DisplayMath(_))));
    }

    #[test]
    fn test_highlight_code_block_rust() {
        // Should successfully highlight Rust code
        let highlights = highlight_code_block("rust", "let x = 42;");
        assert!(highlights.is_some());
        let hl = highlights.unwrap();
        assert_eq!(hl.len(), 1); // One line
        assert!(!hl[0].is_empty()); // Has some highlights
    }

    #[test]
    fn test_highlight_code_block_unknown_language() {
        // Should return None for unknown language
        let highlights = highlight_code_block("unknownlang12345", "some code");
        assert!(highlights.is_none());
    }

    #[test]
    fn test_render_markdown_with_theme() {
        let elements = parse_markdown("```rust\nlet x = 42;\n```");
        let theme = crate::syntax::Theme::default();
        let lines = render_markdown(&elements, 80, Some(&theme));
        // Should have rendered the code block with syntax highlighting
        assert!(!lines.is_empty());
        // The line should have multiple spans (syntax-highlighted segments)
        assert!(lines[0].spans.len() > 1);
    }

    #[test]
    fn test_render_markdown_without_theme_falls_back() {
        let elements = parse_markdown("```rust\nlet x = 42;\n```");
        let lines = render_markdown(&elements, 80, None);
        // Should still render the code block, just without syntax colors
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_narrow_unicode_code_block_preserves_content() {
        let elements = parse_markdown("```text\nlet greeting = \"hei 👋 verden\";\n```");
        let lines = render_markdown(&elements, 12, None);
        assert!(!lines.is_empty());
        let rendered = lines[0]
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, " let greeting = \"hei 👋 verden\"; ");
    }
}
