use crate::syntax::{Theme, UiGroup};
use ratatui::style::{Color, Modifier, Style};
use std::ops::Range;

/// Adjusts syntax highlight ranges based on tab expansion mapping
pub fn remap_highlights<T: Copy>(
    highlights: &[(Range<usize>, T)],
    byte_mapping: &[(usize, usize)],
) -> Vec<(Range<usize>, T)> {
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

/// Returns the style for line numbers in the gutter
pub fn get_line_number_style(is_current_line: bool, theme: &Theme) -> Style {
    if is_current_line {
        Style::default()
            .fg(crate::key_convert::convert_core_color(
                theme.get_ui_color(UiGroup::LineNumberCurrent),
            ))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(crate::key_convert::convert_core_color(
            theme.get_ui_color(UiGroup::LineNumber),
        ))
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

/// Returns the sign text and color for diagnostic severity in the gutter.
/// Each string MUST be exactly SIGN_WIDTH (2) display columns.
pub fn get_diagnostic_sign_style(
    severity: Option<lsp_types::DiagnosticSeverity>,
) -> (&'static str, Color) {
    use lsp_types::DiagnosticSeverity;
    match severity {
        Some(DiagnosticSeverity::ERROR) => ("E ", Color::Red),
        Some(DiagnosticSeverity::WARNING) => ("W ", Color::Yellow),
        Some(DiagnosticSeverity::INFORMATION) => ("I ", Color::Cyan),
        Some(DiagnosticSeverity::HINT) => ("H ", Color::Gray),
        _ => ("E ", Color::Red), // Default to error style
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
    let idx: usize = hash.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    BLAME_COLORS[idx % BLAME_COLORS.len()]
}

/// A commit-colored band makes groups readable even on rows where the author
/// label is omitted. Adjust foreground contrast for light editor themes.
pub fn blame_style(color: Color, theme: &Theme) -> Style {
    let background =
        crate::key_convert::convert_core_color(theme.get_ui_color(UiGroup::Background));
    let (Color::Rgb(r, g, b), Color::Rgb(br, bg, bb)) = (color, background) else {
        return Style::default().fg(color);
    };
    let tint = |base: u8, accent: u8| ((u16::from(base) * 7 + u16::from(accent)) / 8) as u8;
    let foreground = if u32::from(br) * 299 + u32::from(bg) * 587 + u32::from(bb) * 114 > 140_000 {
        Color::Rgb(r / 2, g / 2, b / 2)
    } else {
        color
    };
    Style::default()
        .fg(foreground)
        .bg(Color::Rgb(tint(br, r), tint(bg, g), tint(bb, b)))
}

#[cfg(test)]
mod blame_tests {
    use super::*;
    use crate::syntax::ColorScheme;

    #[test]
    fn commit_bands_are_stable_distinct_and_adapt_to_light_themes() {
        let first = blame_color_for_hash("abc01");
        let second = blame_color_for_hash("abc02");
        assert_eq!(first, blame_color_for_hash("abc01"));
        assert_ne!(first, second);
        let dark = Theme::from_scheme(ColorScheme::gruvbox_dark());
        let light = Theme::from_scheme(ColorScheme::gruvbox_light());
        assert_ne!(blame_style(first, &dark).bg, blame_style(second, &dark).bg);
        assert_ne!(blame_style(first, &dark).fg, blame_style(first, &light).fg);
        assert_ne!(blame_style(first, &dark).bg, blame_style(first, &light).bg);
    }
}
