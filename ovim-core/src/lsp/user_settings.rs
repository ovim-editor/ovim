//! Per-language language-server options set by the user.
//!
//! Two places feed this store, the second winning where both set the same
//! key:
//!
//! 1. `~/.config/ovim/languages.toml`:
//!    ```toml
//!    [[lsp_settings]]
//!    languages = ["java", "kotlin"]
//!    initialization_options = { hyperion = { buildToolClasspath = true } }
//!    settings = { hyperion = { buildToolClasspath = true } }
//!    ```
//! 2. `init.lua`:
//!    ```lua
//!    ovim.lsp.configure({ "java", "kotlin" }, {
//!      initialization_options = { hyperion = { buildToolClasspath = true } },
//!      settings = { hyperion = { buildToolClasspath = true } },
//!    })
//!    ```
//!
//! `initialization_options` go into the `initialize` request;
//! `settings` are sent with `workspace/didChangeConfiguration` right after
//! `initialized` and answer `workspace/configuration` requests. Both are
//! deep-merged over ovim's built-in defaults for the language.
//!
//! Settings only ever come from the user's own config, never from files in
//! the project being opened: a repository must not be able to switch on
//! options (like running the build tool) by shipping a settings file.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// What the user configured for one language.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UserLspSettings {
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
}

/// Two layers so an `init.lua` reload can drop what the script set without
/// losing what `languages.toml` set.
#[derive(Default)]
struct Layers {
    toml: HashMap<String, UserLspSettings>,
    lua: HashMap<String, UserLspSettings>,
}

fn store() -> &'static RwLock<Layers> {
    static STORE: OnceLock<RwLock<Layers>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(Layers::default()))
}

/// Recursively merges `overlay` into `base`: objects merge key by key,
/// everything else is replaced.
pub fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(key) {
                    Some(existing) => merge_json(existing, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}

fn merge_into(
    map: &mut HashMap<String, UserLspSettings>,
    languages: &[String],
    settings: &UserLspSettings,
) {
    for language in languages {
        let entry = map.entry(language.clone()).or_default();
        for (target, overlay) in [
            (
                &mut entry.initialization_options,
                &settings.initialization_options,
            ),
            (&mut entry.settings, &settings.settings),
        ] {
            if let Some(overlay) = overlay {
                match target {
                    Some(existing) => merge_json(existing, overlay),
                    None => *target = Some(overlay.clone()),
                }
            }
        }
    }
}

/// `ovim.lsp.configure` from Lua: merged over `languages.toml`.
pub fn configure(languages: &[String], settings: UserLspSettings) {
    let mut store = store().write().unwrap_or_else(|e| e.into_inner());
    merge_into(&mut store.lua, languages, &settings);
}

/// Forgets what Lua scripts configured (an `init.lua` reload starts over).
pub fn clear_lua() {
    store()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .lua
        .clear();
}

pub fn for_language(language: &str) -> Option<UserLspSettings> {
    let store = store().read().unwrap_or_else(|e| e.into_inner());
    let one = |map: &HashMap<String, UserLspSettings>| map.get(language).cloned();
    match (one(&store.toml), one(&store.lua)) {
        (Some(mut base), Some(lua)) => {
            for (target, overlay) in [
                (&mut base.initialization_options, lua.initialization_options),
                (&mut base.settings, lua.settings),
            ] {
                if let Some(overlay) = overlay {
                    match target {
                        Some(existing) => merge_json(existing, &overlay),
                        None => *target = Some(overlay),
                    }
                }
            }
            Some(base)
        }
        (a, b) => a.or(b),
    }
}

/// `base` (ovim's built-in value, if any) with the user's initialization
/// options merged over it.
pub fn initialization_options(language: &str, base: Option<Value>) -> Option<Value> {
    overlay(
        base,
        for_language(language).and_then(|s| s.initialization_options),
    )
}

/// `base` (ovim's built-in workspace settings, if any) with the user's
/// `settings` merged over it.
pub fn workspace_settings(language: &str, base: Option<Value>) -> Option<Value> {
    overlay(base, for_language(language).and_then(|s| s.settings))
}

fn overlay(base: Option<Value>, user: Option<Value>) -> Option<Value> {
    match (base, user) {
        (Some(mut base), Some(user)) => {
            merge_json(&mut base, &user);
            Some(base)
        }
        (base, None) => base,
        (None, user) => user,
    }
}

/// One `[[lsp_settings]]` entry of `languages.toml`.
#[derive(Debug, Deserialize)]
struct TomlEntry {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    initialization_options: Option<toml::Value>,
    #[serde(default)]
    settings: Option<toml::Value>,
}

/// The `[[lsp_settings]]` entries of a user `languages.toml` that could be
/// read, and what was wrong with the others.
#[derive(Debug, Default)]
pub struct ParsedToml {
    pub entries: Vec<(Vec<String>, UserLspSettings)>,
    /// One message per entry (or per file) that was skipped, naming it so a
    /// typo is not silently ignored.
    pub problems: Vec<String>,
}

/// Parses the `[[lsp_settings]]` entries of a user `languages.toml`. A bad
/// entry is skipped and reported, the rest still apply.
pub fn parse_toml(text: &str) -> ParsedToml {
    let mut parsed = ParsedToml::default();
    let file: toml::Table = match toml::from_str(text) {
        Ok(file) => file,
        Err(e) => {
            parsed.problems.push(format!("invalid languages.toml: {e}"));
            return parsed;
        }
    };
    let entries = match file.get("lsp_settings") {
        None => return parsed,
        Some(toml::Value::Array(entries)) => entries,
        Some(_) => {
            parsed
                .problems
                .push("invalid [[lsp_settings]]: expected an array of tables".to_string());
            return parsed;
        }
    };
    for (index, entry) in entries.iter().enumerate() {
        let number = index + 1;
        let entry: TomlEntry = match entry.clone().try_into() {
            Ok(entry) => entry,
            Err(e) => {
                parsed
                    .problems
                    .push(format!("[[lsp_settings]] entry {number}: {e}"));
                continue;
            }
        };
        let mut languages = entry.languages;
        languages.extend(entry.language);
        if languages.is_empty() {
            parsed.problems.push(format!(
                "[[lsp_settings]] entry {number} needs `language = \"...\"` or `languages = [...]`"
            ));
            continue;
        }
        let to_json = |value: Option<toml::Value>| -> Result<Option<Value>, String> {
            value
                .map(|v| {
                    serde_json::to_value(v)
                        .map_err(|e| format!("[[lsp_settings]] entry {number} value: {e}"))
                })
                .transpose()
        };
        match (
            to_json(entry.initialization_options),
            to_json(entry.settings),
        ) {
            (Ok(initialization_options), Ok(settings)) => parsed.entries.push((
                languages,
                UserLspSettings {
                    initialization_options,
                    settings,
                },
            )),
            (Err(e), _) | (_, Err(e)) => parsed.problems.push(e),
        }
    }
    parsed
}

/// Loads the `[[lsp_settings]]` of a user `languages.toml` into the store.
/// Returns what was wrong with the entries that were skipped.
pub fn load_toml(text: &str) -> Vec<String> {
    let parsed = parse_toml(text);
    let mut store = store().write().unwrap_or_else(|e| e.into_inner());
    for (languages, settings) in &parsed.entries {
        merge_into(&mut store.toml, languages, settings);
    }
    parsed.problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn toml_entries_parse_for_one_or_many_languages() {
        let parsed = parse_toml(
            r#"
            [[lsp_settings]]
            languages = ["java", "kotlin"]
            initialization_options = { hyperion = { buildToolClasspath = true } }

            [[lsp_settings]]
            language = "rust"
            settings = { "rust-analyzer" = { check = { command = "check" } } }
            "#,
        );
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        let entries = parsed.entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, ["java", "kotlin"]);
        assert_eq!(
            entries[0].1.initialization_options,
            Some(json!({"hyperion": {"buildToolClasspath": true}}))
        );
        assert_eq!(entries[1].0, ["rust"]);
        assert_eq!(entries[1].1.initialization_options, None);
    }

    #[test]
    fn a_bad_entry_is_skipped_and_reported_without_dropping_the_others() {
        let parsed = parse_toml(
            r#"
            [[lsp_settings]]
            settings = {}

            [[lsp_settings]]
            languages = "java"

            [[lsp_settings]]
            language = "rust"
            settings = { good = true }
            "#,
        );
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].0, ["rust"]);
        assert_eq!(parsed.problems.len(), 2, "{:?}", parsed.problems);
        assert!(parsed.problems[0].contains("entry 1 needs"));
        assert!(parsed.problems[1].contains("entry 2"));

        let broken = parse_toml("[[lsp_settings]\nlanguage = ");
        assert!(broken.entries.is_empty());
        assert_eq!(broken.problems.len(), 1);
    }

    #[test]
    fn user_values_are_merged_over_defaults_key_by_key() {
        let mut base = json!({"a": {"x": 1, "y": 2}, "b": 1});
        merge_json(&mut base, &json!({"a": {"y": 3, "z": 4}, "c": [1]}));
        assert_eq!(
            base,
            json!({"a": {"x": 1, "y": 3, "z": 4}, "b": 1, "c": [1]})
        );
        assert_eq!(overlay(None, Some(json!(1))), Some(json!(1)));
        assert_eq!(overlay(Some(json!(1)), None), Some(json!(1)));
    }

    #[test]
    fn configure_merges_and_lookups_are_per_language() {
        let lang = "user-settings-test-language";
        configure(
            &[lang.to_string()],
            UserLspSettings {
                initialization_options: Some(json!({"a": 1})),
                settings: None,
            },
        );
        configure(
            &[lang.to_string()],
            UserLspSettings {
                initialization_options: Some(json!({"b": 2})),
                settings: Some(json!({"s": true})),
            },
        );
        assert_eq!(
            initialization_options(lang, Some(json!({"base": 0}))),
            Some(json!({"base": 0, "a": 1, "b": 2}))
        );
        assert_eq!(workspace_settings(lang, None), Some(json!({"s": true})));
        assert_eq!(workspace_settings("another-language", None), None);
    }
}
