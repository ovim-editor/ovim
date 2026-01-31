use crate::syntax::ColorSchemeRegistry;

/// Theme and color scheme state.
pub struct ThemeState {
    /// Color scheme registry
    pub color_scheme_registry: ColorSchemeRegistry,
    /// Current color scheme name
    pub current_color_scheme: String,
}

impl Default for ThemeState {
    fn default() -> Self {
        Self {
            color_scheme_registry: ColorSchemeRegistry::new(),
            current_color_scheme: "tokyonight".to_string(),
        }
    }
}
