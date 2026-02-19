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
    pub const MENU_KEY: Color = Color::Rgb(166, 227, 161); // Green
    pub const MENU_LABEL: Color = Color::Rgb(205, 214, 244); // Text
    pub const MENU_HINT: Color = Color::Rgb(127, 132, 156); // Overlay
    pub const MENU_DESC: Color = Color::Rgb(148, 156, 187); // Subtext
    pub const SEPARATOR: Color = Color::Rgb(88, 91, 112); // Surface2
    pub const VERSION: Color = Color::Rgb(127, 132, 156); // Overlay
}

fn prettify_keys(keys: &str) -> String {
    keys.replace("<Space>", "␣")
}

/// Renders the dashboard screen
pub fn render_dashboard(frame: &mut Frame, editor: &mut Editor, area: Rect) {
    // Calculate vertical centering
    // Logo (5) + spacing (1) + tagline (1) + spacing (1) + tips legend (1)
    // + spacing (1) + separator (1) + spacing (1) + rows + spacing (2) + version (1)
    let total_height = 5 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + MENU_ITEMS.len() + 2 + 1;
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

    // Spacing + legend
    lines.push(Line::from(""));
    let legend = "Press normal keys.  ␣ means <Space>";
    let legend_padding = if area.width as usize > legend.len() {
        " ".repeat((area.width as usize - legend.len()) / 2)
    } else {
        String::new()
    };
    lines.push(Line::from(vec![
        Span::raw(legend_padding),
        Span::styled(legend, Style::default().fg(colors::MENU_HINT)),
    ]));
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

    // Shortcut rows
    let menu_width = 74usize;
    let menu_padding = if area.width as usize > menu_width {
        " ".repeat((area.width as usize - menu_width) / 2)
    } else {
        String::new()
    };

    let action_width = MENU_ITEMS
        .iter()
        .map(|(action, _, _)| action.len())
        .max()
        .unwrap_or(0)
        + 2;
    let desc_width = MENU_ITEMS
        .iter()
        .map(|(_, desc, _)| desc.len())
        .max()
        .unwrap_or(0)
        + 2;

    for (action, desc, keys) in MENU_ITEMS.iter().copied() {
        let key_text = prettify_keys(keys);
        let mut spans = vec![Span::raw(menu_padding.clone())];
        spans.push(Span::styled(
            format!(
                "{action:<action_width$}",
                action = action,
                action_width = action_width
            ),
            Style::default()
                .fg(colors::MENU_LABEL)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!("{desc:<desc_width$}", desc = desc, desc_width = desc_width),
            Style::default().fg(colors::MENU_DESC),
        ));
        spans.push(Span::styled(
            key_text,
            Style::default()
                .fg(colors::MENU_KEY)
                .add_modifier(Modifier::BOLD),
        ));
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
        assert!(MENU_ITEMS.len() >= 8);
    }

    #[test]
    fn test_logo_lines() {
        assert_eq!(LOGO.len(), 5);
    }

    #[test]
    fn test_prettify_keys() {
        assert_eq!(prettify_keys("<Space>sf"), "␣sf");
    }
}
