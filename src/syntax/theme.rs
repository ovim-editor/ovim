use ratatui::style::Color;
use std::collections::HashMap;

/// Highlight groups representing different syntax elements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightGroup {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Operator,
    Variable,
    Macro,
    Constant,
    Property,
    Parameter,
    Label,
    Punctuation,
    Other,
}

/// Color theme for syntax highlighting
pub struct Theme {
    colors: HashMap<HighlightGroup, Color>,
}

impl Theme {
    /// Creates the default theme with good colors for dark terminals
    pub fn default() -> Self {
        let mut colors = HashMap::new();

        colors.insert(HighlightGroup::Keyword, Color::Magenta);
        colors.insert(HighlightGroup::Function, Color::Blue);
        colors.insert(HighlightGroup::Type, Color::Yellow);
        colors.insert(HighlightGroup::String, Color::Green);
        colors.insert(HighlightGroup::Number, Color::Cyan);
        colors.insert(HighlightGroup::Comment, Color::DarkGray);
        colors.insert(HighlightGroup::Operator, Color::White);
        colors.insert(HighlightGroup::Variable, Color::White);
        colors.insert(HighlightGroup::Macro, Color::Magenta);
        colors.insert(HighlightGroup::Constant, Color::Cyan);
        colors.insert(HighlightGroup::Property, Color::Blue);
        colors.insert(HighlightGroup::Parameter, Color::White);
        colors.insert(HighlightGroup::Label, Color::Yellow);
        colors.insert(HighlightGroup::Punctuation, Color::White);
        colors.insert(HighlightGroup::Other, Color::White);

        Self { colors }
    }

    /// Gets the color for a highlight group
    pub fn get_color(&self, group: HighlightGroup) -> Color {
        self.colors.get(&group).copied().unwrap_or(Color::White)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default()
    }
}
