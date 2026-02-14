use std::collections::HashMap;

/// Interpolate `{{variable}}` placeholders in a template string.
pub fn interpolate(template: &str, vars: &HashMap<&str, &str>) -> String {
    let mut result = template.to_string();
    for (key, value) in vars {
        result = result.replace(&format!("{{{{{}}}}}", key), value);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolate_basic() {
        let vars = HashMap::from([("name", "World")]);
        assert_eq!(interpolate("Hello {{name}}", &vars), "Hello World");
    }

    #[test]
    fn interpolate_missing_var() {
        let vars = HashMap::from([("name", "World")]);
        assert_eq!(
            interpolate("Hello {{name}}, {{unknown}}", &vars),
            "Hello World, {{unknown}}"
        );
    }

    #[test]
    fn interpolate_multiple() {
        let vars = HashMap::from([("file", "main.rs"), ("language", "rust")]);
        assert_eq!(
            interpolate("Editing {{file}} ({{language}})", &vars),
            "Editing main.rs (rust)"
        );
    }

    #[test]
    fn interpolate_empty_value() {
        let vars = HashMap::from([("selection", "")]);
        assert_eq!(
            interpolate("Selected: [{{selection}}]", &vars),
            "Selected: []"
        );
    }
}
