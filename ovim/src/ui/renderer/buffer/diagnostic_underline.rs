//! How a diagnostic is drawn on the text of a line: an underline in the
//! severity colour, shared by both line pipelines.

use ratatui::style::{Color, Modifier, Style};

/// A diagnostic range remapped to expanded char indices for rendering
pub(super) struct RemappedDiagnostic {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) color: Color,
    /// 1 = error … 4 = hint, the LSP severity order.
    rank: u8,
}

impl RemappedDiagnostic {
    pub(super) fn new(
        range: std::ops::Range<usize>,
        severity: Option<lsp_types::DiagnosticSeverity>,
    ) -> Self {
        use lsp_types::DiagnosticSeverity as Severity;
        let (color, rank) = match severity {
            Some(Severity::WARNING) => (Color::Yellow, 2),
            Some(Severity::INFORMATION) => (Color::Cyan, 3),
            Some(Severity::HINT) => (Color::Gray, 4),
            // A missing severity is the client's to interpret; treat it as
            // an error, like the gutter and the counts do.
            _ => (Color::Red, 1),
        };
        Self {
            start: range.start,
            end: range.end,
            color,
            rank,
        }
    }
}

/// Where underlines overlap, the first one wins the cell. Put the most
/// severe first so an error is never hidden under a hint.
pub(super) fn sort_most_severe_first(diagnostics: &mut [RemappedDiagnostic]) {
    diagnostics.sort_by_key(|diagnostic| diagnostic.rank);
}

/// Whether to colour the underline itself (SGR 58) rather than the text.
///
/// The sequence is sent in its semicolon form, which a terminal that does
/// not know SGR 58 can misread as other attributes (`58;5;3` as blink and
/// italic). So it is used only where support is known: `OVIM_UNDERLINE_COLOR`
/// set to `1`, a terminal that advertises 24-bit colour (`COLORTERM`), or a
/// `TERM` naming one that has it. `OVIM_UNDERLINE_COLOR=0` turns it off.
/// Two are left out because their handling has not been checked: Apple
/// Terminal, and GNU screen, which inherits `COLORTERM` from the terminal it
/// runs in (tmux, which also uses a `screen` `TERM`, is recognised by `TMUX`).
pub(super) fn detect_underline_color(var: impl Fn(&str) -> Option<String>) -> bool {
    match var("OVIM_UNDERLINE_COLOR").as_deref() {
        Some("1" | "true" | "on") => return true,
        Some("0" | "false" | "off") => return false,
        _ => {}
    }
    if var("TERM_PROGRAM").as_deref() == Some("Apple_Terminal") {
        return false;
    }
    let term = var("TERM").unwrap_or_default();
    let mut names = term.split('-');
    if names.clone().next() == Some("screen") && var("TMUX").is_none() {
        return false;
    }
    matches!(var("COLORTERM").as_deref(), Some("truecolor" | "24bit"))
        || names.any(|name| {
            matches!(
                name,
                "kitty" | "ghostty" | "alacritty" | "wezterm" | "foot" | "contour"
            )
        })
}

/// 0 = detect from the environment, 1 = on, 2 = off.
static UNDERLINE_COLOR_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Force coloured underlines on or off, or back to detection with `None`.
/// For tests and for frontends that know their own terminal.
pub fn set_underline_color_support(supported: Option<bool>) {
    let value = match supported {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    UNDERLINE_COLOR_OVERRIDE.store(value, std::sync::atomic::Ordering::Relaxed);
}

fn underline_color_supported() -> bool {
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    match UNDERLINE_COLOR_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => *DETECTED.get_or_init(|| detect_underline_color(|name| std::env::var(name).ok())),
    }
}

/// Underline a cell for a diagnostic. The text keeps its own colour and only
/// the underline takes the severity colour, so a diagnostic never hides the
/// syntax highlighting of the code it is about. Where the terminal cannot
/// colour an underline, the text takes the severity colour instead.
pub(super) fn diagnostic_underline(style: Style, color: Color) -> Style {
    diagnostic_underline_for(style, color, underline_color_supported())
}

pub(super) fn diagnostic_underline_for(style: Style, color: Color, colored: bool) -> Style {
    let style = style.add_modifier(Modifier::UNDERLINED);
    if colored {
        style.underline_color(color)
    } else {
        style.fg(color)
    }
}
