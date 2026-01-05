use super::Editor;
use crate::syntax::ColorScheme;
use anyhow::Result;

impl Editor {
    /// Gets the current color scheme
    pub fn get_color_scheme(&self) -> Option<&ColorScheme> {
        self.color_scheme_registry.get(&self.current_color_scheme)
    }

    /// Sets the color scheme by name
    pub fn set_color_scheme(&mut self, name: &str) -> Result<()> {
        if self.color_scheme_registry.get(name).is_some() {
            self.current_color_scheme = name.to_string();
            Ok(())
        } else {
            Err(anyhow::anyhow!("Color scheme '{}' not found", name))
        }
    }

    /// Lists all available color scheme names
    pub fn list_color_schemes(&self) -> Vec<&str> {
        self.color_scheme_registry.list_names()
    }

    /// Gets the current color scheme name
    pub fn current_color_scheme_name(&self) -> &str {
        &self.current_color_scheme
    }
}
