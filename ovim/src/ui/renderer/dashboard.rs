//! Dashboard screen renderer (LunarVim-style startup screen)
//!
//! Renders a welcome screen with ASCII logo and interactive menu.

use super::CatAnimation;
use crate::editor::Editor;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// Menu items for the dashboard
pub const MENU_ITEMS: &[(&str, &str, &str)] = &[
    ("e", "New File", "Open empty buffer"),
    ("f", "Find File", "<Space>sf"),
    ("r", "Recent Files", "<Space>sr"),
    ("g", "Find Word", "<Space>sg"),
    ("c", "Configuration", ":e ~/.config/ovim"),
    ("q", "Quit", ":q"),
];

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
    pub const MENU_KEY: Color = Color::Rgb(166, 227, 161); // Green
    pub const MENU_LABEL: Color = Color::Rgb(205, 214, 244); // Text
    pub const MENU_HINT: Color = Color::Rgb(127, 132, 156); // Overlay
    pub const MENU_SELECTED_BG: Color = Color::Rgb(49, 50, 68); // Surface0
    pub const SEPARATOR: Color = Color::Rgb(88, 91, 112); // Surface2
    pub const VERSION: Color = Color::Rgb(127, 132, 156); // Overlay
}

/// Renders the dashboard screen
pub fn render_dashboard(frame: &mut Frame, editor: &mut Editor, area: Rect) {
    let selected = editor.dashboard_selected();

    // Calculate vertical centering
    // Logo (5 lines) + spacing (1) + tagline (1) + spacing (2) + separator (1) + spacing (1) + menu (6) + spacing (2) + version (1)
    let total_height = 5 + 1 + 1 + 2 + 1 + 1 + MENU_ITEMS.len() + 2 + 1;
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

    // Spacing
    lines.push(Line::from(""));
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

    // Menu items
    let menu_width = 45; // Fixed width for menu items
    let menu_padding = if area.width as usize > menu_width {
        " ".repeat((area.width as usize - menu_width) / 2)
    } else {
        String::new()
    };

    for (idx, (key, label, hint)) in MENU_ITEMS.iter().enumerate() {
        let is_selected = idx == selected;

        let key_style = if is_selected {
            Style::default()
                .fg(colors::MENU_KEY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::MENU_KEY)
        };

        let label_style = if is_selected {
            Style::default()
                .fg(colors::MENU_LABEL)
                .bg(colors::MENU_SELECTED_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::MENU_LABEL)
        };

        let hint_style = Style::default().fg(colors::MENU_HINT);

        // Format: [key]  Label                    hint
        let key_part = format!("  {}  ", key);
        let label_len = label.len();
        let hint_len = hint.len();
        let spacing = menu_width.saturating_sub(4 + label_len + hint_len + 4);

        let mut spans = vec![Span::raw(menu_padding.clone())];

        if is_selected {
            // Highlight the entire line for selected item
            spans.push(Span::styled(
                key_part,
                key_style.bg(colors::MENU_SELECTED_BG),
            ));
            spans.push(Span::styled(*label, label_style));
            spans.push(Span::styled(
                " ".repeat(spacing),
                Style::default().bg(colors::MENU_SELECTED_BG),
            ));
            spans.push(Span::styled(*hint, hint_style.bg(colors::MENU_SELECTED_BG)));
            spans.push(Span::styled(
                "  ",
                Style::default().bg(colors::MENU_SELECTED_BG),
            ));
        } else {
            spans.push(Span::styled(key_part, key_style));
            spans.push(Span::styled(*label, label_style));
            spans.push(Span::raw(" ".repeat(spacing)));
            spans.push(Span::styled(*hint, hint_style));
            spans.push(Span::raw("  "));
        }

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
        assert_eq!(MENU_ITEMS.len(), 6);
    }

    #[test]
    fn test_logo_lines() {
        assert_eq!(LOGO.len(), 5);
    }
}
