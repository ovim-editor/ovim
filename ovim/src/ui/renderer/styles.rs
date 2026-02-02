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
        let color = crate::key_convert::convert_core_color(theme.get_color(group));
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

/// Returns the sign text and color for diagnostic severity in the gutter (nerd font icons)
pub fn get_diagnostic_sign_style(
    severity: Option<lsp_types::DiagnosticSeverity>,
) -> (&'static str, Color) {
    use lsp_types::DiagnosticSeverity;
    match severity {
        Some(DiagnosticSeverity::ERROR) => (" ", Color::Red),
        Some(DiagnosticSeverity::WARNING) => (" ", Color::Yellow),
        Some(DiagnosticSeverity::INFORMATION) => (" ", Color::Cyan),
        Some(DiagnosticSeverity::HINT) => (" ", Color::Gray),
        _ => (" ", Color::Red), // Default to error style
    }
}

/// Muted color palette for blame gutter (12 distinct colors)
const BLAME_COLORS: [Color; 12] = [
    Color::Rgb(130, 170, 200), // steel blue
    Color::Rgb(180, 140, 180), // muted purple
    Color::Rgb(140, 180, 140), // sage green
    Color::Rgb(200, 160, 120), // sandy brown
    Color::Rgb(160, 160, 200), // lavender
    Color::Rgb(180, 180, 130), // olive
    Color::Rgb(170, 140, 140), // dusty rose
    Color::Rgb(130, 180, 170), // teal
    Color::Rgb(190, 150, 150), // mauve
    Color::Rgb(150, 170, 130), // fern
    Color::Rgb(170, 160, 180), // wisteria
    Color::Rgb(180, 170, 140), // khaki
];

/// Returns a deterministic color for a blame commit hash
pub fn blame_color_for_hash(hash: &str) -> Color {
    let idx: usize = hash.bytes().fold(0usize, |acc, b| acc.wrapping_mul(31).wrapping_add(b as usize));
    BLAME_COLORS[idx % BLAME_COLORS.len()]
}

/// Returns the (icon, foreground color, background color) for diagnostic virtual text
pub fn get_diagnostic_virtual_text_style(
    severity: Option<lsp_types::DiagnosticSeverity>,
) -> (&'static str, Color, Color) {
    use lsp_types::DiagnosticSeverity;
    match severity {
        Some(DiagnosticSeverity::ERROR) => ("", Color::Red, Color::Rgb(60, 20, 20)),
        Some(DiagnosticSeverity::WARNING) => ("", Color::Yellow, Color::Rgb(60, 50, 20)),
        Some(DiagnosticSeverity::INFORMATION) => ("", Color::Cyan, Color::Rgb(20, 40, 60)),
        Some(DiagnosticSeverity::HINT) => ("", Color::Gray, Color::Rgb(40, 40, 40)),
        _ => ("", Color::Gray, Color::Rgb(40, 40, 40)),
    }
}
