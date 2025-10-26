use crate::syntax::{HighlightGroup, Theme};
use ratatui::style::{Color, Modifier, Style};
use std::ops::Range;

/// Adjusts syntax highlight ranges based on tab expansion mapping
pub fn remap_highlights(
    highlights: &[(Range<usize>, HighlightGroup)],
    byte_mapping: &[(usize, usize)],
) -> Vec<(Range<usize>, HighlightGroup)> {
    highlights
        .iter()
        .map(|(range, group)| {
            // Find mapped positions for start and end
            let new_start = byte_mapping
                .iter()
                .find(|(orig, _)| *orig >= range.start)
                .map(|(_, expanded)| *expanded)
                .unwrap_or(0);

            let new_end = byte_mapping
                .iter()
                .find(|(orig, _)| *orig >= range.end)
                .map(|(_, expanded)| *expanded)
                .unwrap_or(new_start);

            (new_start..new_end, *group)
        })
        .collect()
}

/// Returns the style for a character in the buffer based on highlighting priorities
/// Priority: visual selection > search match > syntax > normal
#[allow(dead_code)]
pub fn get_char_style(
    is_selected: bool,
    is_search_match: bool,
    syntax_group: Option<HighlightGroup>,
    theme: &Theme,
) -> Style {
    if is_selected {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else if is_search_match {
        Style::default().bg(Color::Yellow).fg(Color::Black)
    } else if let Some(group) = syntax_group {
        let color = theme.get_color(group);
        Style::default().fg(color)
    } else {
        Style::default()
    }
}

/// Returns the style for line numbers in the gutter
pub fn get_line_number_style(is_current_line: bool) -> Style {
    if is_current_line {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

/// Returns the style and text for git status signs
pub fn get_git_sign_style(status: Option<crate::LineStatus>) -> (&'static str, Color) {
    match status {
        Some(crate::LineStatus::Added) => ("+ ", Color::Green),
        Some(crate::LineStatus::Modified) => ("~ ", Color::Yellow),
        Some(crate::LineStatus::Removed) => ("- ", Color::Red),
        None => ("  ", Color::DarkGray),
    }
}
