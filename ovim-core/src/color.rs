/// Terminal color representation, independent of any UI framework.
///
/// This mirrors ratatui::style::Color but lives in ovim-core to keep the
/// syntax highlighting system free of ratatui dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    Gray,
    Rgb(u8, u8, u8),
    Indexed(u8),
    Reset,
}
