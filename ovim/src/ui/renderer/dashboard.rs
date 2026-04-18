//! Dashboard screen renderer (LunarVim-style startup screen)
//!
//! Renders a welcome screen with ASCII logo and interactive menu.

use super::CatAnimation;
use crate::editor::Editor;
pub use ovim_core::dashboard::MENU_ITEMS;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// ASCII art logo (Block Art style)
const LOGO: &[&str] = &[
    "   ██████  ██    ██ ██ ███    ███",
    "  ██    ██ ██    ██ ██ ████  ████",
    "  ██    ██ ██    ██ ██ ██ ████ ██",
    "  ██    ██  ██  ██  ██ ██  ██  ██",
    "   ██████    ████   ██ ██      ██",
];

/// Tagline under the logo
const TAGLINE: &str = "It's.. oxidized";

/// Colors for the dashboard (Catppuccin-inspired)
mod colors {
    use ratatui::style::Color;

    pub const LOGO: Color = Color::Rgb(137, 180, 250); // Blue
    pub const TAGLINE: Color = Color::Rgb(166, 176, 207); // Subtext
    pub const MENU_KEY: Color = Color::White; // Bold white pairs with the blue logo / muted prefix
    pub const MENU_LABEL: Color = Color::Rgb(205, 214, 244); // Text
    pub const KEY_MUTED: Color = Color::Rgb(108, 112, 134); // Overlay0 — for `<Space>` prefix
    pub const SEPARATOR: Color = Color::Rgb(88, 91, 112); // Surface2
    pub const VERSION: Color = Color::Rgb(127, 132, 156); // Overlay
}


/// Renders the dashboard screen
pub fn render_dashboard(frame: &mut Frame, editor: &mut Editor, area: Rect) {
    // Calculate vertical centering
    // Logo (5) + spacing (1) + tagline (1) + spacing (1) + separator (1)
    // + spacing (1) + rows + spacing (2) + version (1)
    let total_height = 5 + 1 + 1 + 1 + 1 + 1 + MENU_ITEMS.len() + 2 + 1;
    let vertical_offset = if area.height as usize > total_height {
        (area.height as usize - total_height) / 2
    } else {
        1
    };

    // Calculate horizontal centering (use chars().count() for proper Unicode width)
    let logo_width = LOGO.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let horizontal_offset = if area.width as usize > logo_width {
        (area.width as usize - logo_width) / 2
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::new();

    // Add vertical padding
    for _ in 0..vertical_offset {
        lines.push(Line::from(""));
    }

    // Render logo
    for logo_line in LOGO {
        let padding = " ".repeat(horizontal_offset);
        lines.push(Line::from(vec![
            Span::raw(padding),
            Span::styled(*logo_line, Style::default().fg(colors::LOGO)),
        ]));
    }

    // Spacing after logo
    lines.push(Line::from(""));

    // Tagline (centered)
    let tagline_padding = if area.width as usize > TAGLINE.len() {
        " ".repeat((area.width as usize - TAGLINE.len()) / 2)
    } else {
        String::new()
    };
    lines.push(Line::from(vec![
        Span::raw(tagline_padding),
        Span::styled(
            TAGLINE,
            Style::default()
                .fg(colors::TAGLINE)
                .add_modifier(Modifier::ITALIC),
        ),
    ]));

    // Spacing before separator
    lines.push(Line::from(""));

    // Separator line
    let separator_width = 40.min(area.width as usize - 4);
    let separator_padding = if area.width as usize > separator_width {
        " ".repeat((area.width as usize - separator_width) / 2)
    } else {
        String::new()
    };
    lines.push(Line::from(vec![
        Span::raw(separator_padding.clone()),
        Span::styled(
            "─".repeat(separator_width),
            Style::default().fg(colors::SEPARATOR),
        ),
    ]));

    // Spacing
    lines.push(Line::from(""));

    // Shortcut rows: keybind on the left (the user is hunting for a key),
    // single label phrase on the right. Keys use Vim's `<Space>` notation
    // verbatim — self-documenting, no glyph legend needed.
    const KEY_GAP: usize = 4;

    let key_col_width = MENU_ITEMS
        .iter()
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or(0);
    let label_col_width = MENU_ITEMS
        .iter()
        .map(|(_, label)| label.chars().count())
        .max()
        .unwrap_or(0);
    let menu_width = key_col_width + KEY_GAP + label_col_width;
    let menu_padding = if area.width as usize > menu_width {
        " ".repeat((area.width as usize - menu_width) / 2)
    } else {
        String::new()
    };

    let bold_key = Style::default()
        .fg(colors::MENU_KEY)
        .add_modifier(Modifier::BOLD);
    let muted_key = Style::default()
        .fg(colors::KEY_MUTED)
        .add_modifier(Modifier::BOLD);

    for (keys, label) in MENU_ITEMS.iter().copied() {
        let key_pad = " ".repeat(key_col_width - keys.chars().count() + KEY_GAP);
        let mut spans = vec![Span::raw(menu_padding.clone())];

        // Render `<Space>` tokens muted, real command keys in bold white.
        // The eye scans the white column for the unique letter to press;
        // `<Space>` itself is the predictable leader and recedes.
        let mut rest = keys;
        while let Some(after) = rest.strip_prefix("<Space>") {
            spans.push(Span::styled("<Space>", muted_key));
            rest = after;
        }
        if !rest.is_empty() {
            spans.push(Span::styled(rest, bold_key));
        }

        spans.push(Span::raw(key_pad));
        spans.push(Span::styled(label, Style::default().fg(colors::MENU_LABEL)));
        lines.push(Line::from(spans));
    }

    // Spacing before version
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    // Version info
    let version = env!("CARGO_PKG_VERSION");
    let version_text = format!("v{}", version);
    let version_padding = if area.width as usize > version_text.len() {
        " ".repeat((area.width as usize - version_text.len()) / 2)
    } else {
        String::new()
    };
    lines.push(Line::from(vec![
        Span::raw(version_padding),
        Span::styled(version_text, Style::default().fg(colors::VERSION)),
    ]));

    // Fill remaining space
    while lines.len() < area.height as usize {
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, area);

    // Render cat animation overlay (if active)
    let logo_y = area.y + vertical_offset as u16;
    let logo_center_x = area.x + (area.width / 2);
    if let Some(anim) = editor.cat_animation_mut() {
        if let Some(cat) = anim.as_any_mut().downcast_mut::<CatAnimation>() {
            cat.set_layout(logo_y, logo_center_x, area.width, area.height);
            cat.render(frame, area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_items_count() {
        // Tight, focused list — leader-key cheat sheet, not a feature catalog.
        assert!(MENU_ITEMS.len() >= 4 && MENU_ITEMS.len() <= 8);
    }

    #[test]
    fn test_logo_lines() {
        assert_eq!(LOGO.len(), 5);
    }

}
