use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier, Style};

use super::renderer::line_cache::LineRenderCache;
use crate::editor::Editor;

/// Renders the editor to an in-memory buffer and returns ANSI output.
/// Used for headless mode to get pixel-perfect terminal representation.
pub fn render_editor_to_ansi(editor: &mut Editor, width: u16, height: u16) -> Result<String> {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;

    let mut line_cache = LineRenderCache::new();
    terminal.draw(|f| {
        super::Renderer::render_to_frame(f, editor, &mut line_cache);
    })?;

    let buffer = terminal.backend().buffer();
    Ok(buffer_to_ansi(buffer))
}

/// Converts a ratatui Buffer to an ANSI-escaped string
/// This allows headless mode to export pixel-perfect terminal output
pub fn buffer_to_ansi(buffer: &Buffer) -> String {
    let mut result = String::new();
    let mut last_style = Style::default();

    // Clear screen and reset cursor
    result.push_str("\x1b[2J\x1b[H");

    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];

            // Only emit style change if different from last
            if cell.style() != last_style {
                result.push_str(&style_to_ansi(&cell.style()));
                last_style = cell.style();
            }

            result.push_str(cell.symbol());
        }

        // Don't add newline after last line
        if y < buffer.area.height - 1 {
            result.push('\n');
        }
    }

    // Reset all attributes at end
    result.push_str("\x1b[0m");

    result
}

/// Converts ratatui Style to ANSI escape sequence
fn style_to_ansi(style: &Style) -> String {
    let mut codes = Vec::new();

    // Reset first
    codes.push("0".to_string());

    // Foreground color
    if let Some(fg) = style.fg {
        codes.push(color_to_ansi_fg(fg));
    }

    // Background color
    if let Some(bg) = style.bg {
        codes.push(color_to_ansi_bg(bg));
    }

    // Modifiers
    if style.add_modifier.contains(Modifier::BOLD) {
        codes.push("1".to_string());
    }
    if style.add_modifier.contains(Modifier::DIM) {
        codes.push("2".to_string());
    }
    if style.add_modifier.contains(Modifier::ITALIC) {
        codes.push("3".to_string());
    }
    if style.add_modifier.contains(Modifier::UNDERLINED) {
        codes.push("4".to_string());
    }
    if style.add_modifier.contains(Modifier::SLOW_BLINK) {
        codes.push("5".to_string());
    }
    if style.add_modifier.contains(Modifier::RAPID_BLINK) {
        codes.push("6".to_string());
    }
    if style.add_modifier.contains(Modifier::REVERSED) {
        codes.push("7".to_string());
    }
    if style.add_modifier.contains(Modifier::HIDDEN) {
        codes.push("8".to_string());
    }
    if style.add_modifier.contains(Modifier::CROSSED_OUT) {
        codes.push("9".to_string());
    }

    format!("\x1b[{}m", codes.join(";"))
}

/// Converts ratatui Color to ANSI foreground code
fn color_to_ansi_fg(color: Color) -> String {
    match color {
        Color::Reset => "39".to_string(),
        Color::Black => "30".to_string(),
        Color::Red => "31".to_string(),
        Color::Green => "32".to_string(),
        Color::Yellow => "33".to_string(),
        Color::Blue => "34".to_string(),
        Color::Magenta => "35".to_string(),
        Color::Cyan => "36".to_string(),
        Color::Gray => "37".to_string(),
        Color::DarkGray => "90".to_string(),
        Color::LightRed => "91".to_string(),
        Color::LightGreen => "92".to_string(),
        Color::LightYellow => "93".to_string(),
        Color::LightBlue => "94".to_string(),
        Color::LightMagenta => "95".to_string(),
        Color::LightCyan => "96".to_string(),
        Color::White => "97".to_string(),
        Color::Rgb(r, g, b) => format!("38;2;{};{};{}", r, g, b),
        Color::Indexed(i) => format!("38;5;{}", i),
    }
}

/// Strips ANSI escape sequences, returning the plain character grid.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequences: ESC [ ... final_byte
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                              // Consume parameter bytes (0x30–0x3F), intermediate (0x20–0x2F),
                              // then the final byte (0x40–0x7E).
                loop {
                    match chars.peek() {
                        Some(&c) if ('@'..='~').contains(&c) => {
                            chars.next();
                            break;
                        }
                        Some(_) => {
                            chars.next();
                        }
                        None => break,
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Converts ratatui Color to ANSI background code
fn color_to_ansi_bg(color: Color) -> String {
    match color {
        Color::Reset => "49".to_string(),
        Color::Black => "40".to_string(),
        Color::Red => "41".to_string(),
        Color::Green => "42".to_string(),
        Color::Yellow => "43".to_string(),
        Color::Blue => "44".to_string(),
        Color::Magenta => "45".to_string(),
        Color::Cyan => "46".to_string(),
        Color::Gray => "47".to_string(),
        Color::DarkGray => "100".to_string(),
        Color::LightRed => "101".to_string(),
        Color::LightGreen => "102".to_string(),
        Color::LightYellow => "103".to_string(),
        Color::LightBlue => "104".to_string(),
        Color::LightMagenta => "105".to_string(),
        Color::LightCyan => "106".to_string(),
        Color::White => "107".to_string(),
        Color::Rgb(r, g, b) => format!("48;2;{};{};{}", r, g, b),
        Color::Indexed(i) => format!("48;5;{}", i),
    }
}
