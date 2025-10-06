mod highlighter;
mod languages;
mod theme;

pub use highlighter::SyntaxHighlighter;
pub use languages::{Language, LanguageRegistry};
pub use theme::{ColorScheme, ColorSchemeRegistry, HighlightGroup, Theme};
