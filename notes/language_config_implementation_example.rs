// Example implementation of language_config.rs
// This is a reference implementation showing the proposed architecture
// Location: src/language_config.rs (to be created in Phase 1)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

// ============================================================================
// Core Data Structures
// ============================================================================

/// Complete language configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LanguageConfig {
    /// Language ID (e.g., "rust", "typescript")
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// File extensions (e.g., ["rs"], ["ts", "tsx"])
    #[serde(default)]
    pub extensions: Vec<String>,

    /// Filenames without extensions (e.g., ["Dockerfile", "Makefile"])
    #[serde(default)]
    pub filenames: Vec<String>,

    /// Tree-sitter syntax highlighting config (optional)
    pub syntax: Option<SyntaxConfig>,

    /// LSP server configuration (optional)
    pub lsp: Option<LspConfig>,
}

/// Syntax highlighting configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyntaxConfig {
    /// Tree-sitter grammar name (matches crate name)
    pub grammar: String,

    /// Highlight query source (embedded or file path)
    #[serde(flatten)]
    pub query: QuerySource,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QuerySource {
    /// Use official query from tree-sitter crate
    Official {
        #[serde(rename = "crate")]
        crate_name: String,
        constant: String,
    },

    /// Load from file (e.g., "queries/markdown.scm")
    File { path: String },

    /// Inline query string
    Inline { content: String },
}

/// LSP server configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LspConfig {
    /// Primary server command (e.g., "rust-analyzer")
    pub command: String,

    /// Command-line arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// Alternative commands to try if primary fails
    #[serde(default)]
    pub fallback_commands: Vec<String>,

    /// Project root markers (searched in order)
    #[serde(default)]
    pub root_markers: Vec<String>,

    /// Installation instructions (shown on failure)
    pub install_hint: Option<String>,

    /// Auto-install configuration (optional)
    pub auto_install: Option<AutoInstallConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AutoInstallConfig {
    /// Installation method
    pub method: InstallMethod,

    /// Version constraint (e.g., ">=1.0.0")
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum InstallMethod {
    /// Download from GitHub release
    Github {
        repo: String,
        asset_pattern: String,
        install_path: String,
    },

    /// Install via npm
    Npm {
        package: String,
        #[serde(default)]
        global: bool,
    },

    /// Install via cargo
    Cargo { package: String },

    /// Custom shell command
    Shell { command: String },
}

// ============================================================================
// Language Registry (Singleton)
// ============================================================================

/// Global language registry
static LANGUAGE_REGISTRY: OnceLock<LanguageRegistry> = OnceLock::new();

pub struct LanguageRegistry {
    /// All configured languages
    languages: Vec<LanguageConfig>,

    /// Extension → Language index lookup
    by_extension: HashMap<String, usize>,

    /// Filename → Language index lookup (for extensionless files)
    by_filename: HashMap<String, usize>,

    /// Language ID → Language index lookup
    by_id: HashMap<String, usize>,
}

impl LanguageRegistry {
    /// Initialize the global registry from embedded + user configs
    pub fn init() -> Result<(), String> {
        // Load embedded config
        let embedded = include_str!("../languages.toml");

        // Load user config (if exists)
        let user_config = Self::load_user_config();

        // Parse and merge
        let languages = Self::parse_configs(embedded, user_config)?;

        // Build indices
        let registry = Self::build_indices(languages);

        // Set global singleton
        LANGUAGE_REGISTRY
            .set(registry)
            .map_err(|_| "Registry already initialized".to_string())?;

        Ok(())
    }

    /// Get the global registry (panics if not initialized)
    pub fn get() -> &'static LanguageRegistry {
        LANGUAGE_REGISTRY
            .get()
            .expect("LanguageRegistry not initialized - call init() first")
    }

    /// Get the global registry (returns None if not initialized)
    pub fn try_get() -> Option<&'static LanguageRegistry> {
        LANGUAGE_REGISTRY.get()
    }

    /// Detect language from file path
    pub fn detect<P: AsRef<Path>>(&self, path: P) -> Option<&LanguageConfig> {
        let path = path.as_ref();

        // Try extension first
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(&idx) = self.by_extension.get(ext) {
                return Some(&self.languages[idx]);
            }
        }

        // Try filename (for extensionless files)
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Try exact match
            if let Some(&idx) = self.by_filename.get(name) {
                return Some(&self.languages[idx]);
            }

            // Try lowercase match (case-insensitive)
            let lower = name.to_lowercase();
            if let Some(&idx) = self.by_filename.get(&lower) {
                return Some(&self.languages[idx]);
            }
        }

        None
    }

    /// Get language by ID
    pub fn get_by_id(&self, id: &str) -> Option<&LanguageConfig> {
        self.by_id.get(id).map(|&idx| &self.languages[idx])
    }

    /// List all languages (for debugging/introspection)
    pub fn all(&self) -> &[LanguageConfig] {
        &self.languages
    }

    /// Load user config from ~/.config/ovim/languages.toml
    fn load_user_config() -> Option<String> {
        let config_path = dirs::config_dir()?.join("ovim/languages.toml");
        std::fs::read_to_string(config_path).ok()
    }

    /// Parse embedded and user configs, merging them
    fn parse_configs(
        embedded: &str,
        user: Option<String>,
    ) -> Result<Vec<LanguageConfig>, String> {
        // Parse embedded config
        #[derive(Deserialize)]
        struct ConfigFile {
            language: Vec<LanguageConfig>,
        }

        let embedded_config: ConfigFile = toml::from_str(embedded)
            .map_err(|e| format!("Failed to parse embedded config: {}", e))?;

        let mut languages = embedded_config.language;

        // Parse and merge user config
        if let Some(user_toml) = user {
            let user_config: ConfigFile = toml::from_str(&user_toml)
                .map_err(|e| format!("Failed to parse user config: {}", e))?;

            // User config overrides embedded (by language ID)
            for user_lang in user_config.language {
                if let Some(pos) = languages.iter().position(|l| l.id == user_lang.id) {
                    // Override existing
                    languages[pos] = user_lang;
                } else {
                    // Add new language
                    languages.push(user_lang);
                }
            }
        }

        Ok(languages)
    }

    /// Build lookup indices for fast detection
    fn build_indices(languages: Vec<LanguageConfig>) -> Self {
        let mut by_extension = HashMap::new();
        let mut by_filename = HashMap::new();
        let mut by_id = HashMap::new();

        for (idx, lang) in languages.iter().enumerate() {
            // Index extensions
            for ext in &lang.extensions {
                by_extension.insert(ext.clone(), idx);
            }

            // Index filenames (store lowercase for case-insensitive matching)
            for name in &lang.filenames {
                by_filename.insert(name.to_lowercase(), idx);
            }

            // Index ID
            by_id.insert(lang.id.clone(), idx);
        }

        Self {
            languages,
            by_extension,
            by_filename,
            by_id,
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Find LSP command by checking primary + fallbacks
pub fn find_lsp_command(config: &LspConfig) -> Option<String> {
    // Try primary command in PATH
    if which::which(&config.command).is_ok() {
        return Some(config.command.clone());
    }

    // Try fallback commands
    for fallback in &config.fallback_commands {
        // Expand ~ to home directory
        let expanded = shellexpand::tilde(fallback).to_string();

        // Check if path exists
        if std::path::Path::new(&expanded).exists() {
            return Some(expanded);
        }

        // Check if in PATH
        if which::which(&expanded).is_ok() {
            return Some(expanded);
        }
    }

    None
}

/// Find project root by walking up and checking markers
pub fn find_project_root(file_path: &Path, markers: &[String]) -> PathBuf {
    let mut current = file_path.parent();

    while let Some(dir) = current {
        // Check each marker in order
        for marker in markers {
            if dir.join(marker).exists() {
                return dir.to_path_buf();
            }
        }

        // Move up one directory
        current = dir.parent();
    }

    // Fallback to file's directory
    file_path
        .parent()
        .unwrap_or_else(|| Path::new("/"))
        .to_path_buf()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sample_config() {
        let toml = r#"
            [[language]]
            id = "rust"
            name = "Rust"
            extensions = ["rs"]

            [language.syntax]
            grammar = "tree-sitter-rust"
            official = { crate = "tree_sitter_rust", constant = "HIGHLIGHTS_QUERY" }

            [language.lsp]
            command = "rust-analyzer"
            root_markers = ["Cargo.toml"]
        "#;

        #[derive(Deserialize)]
        struct ConfigFile {
            language: Vec<LanguageConfig>,
        }

        let config: ConfigFile = toml::from_str(toml).unwrap();
        assert_eq!(config.language.len(), 1);
        assert_eq!(config.language[0].id, "rust");
        assert_eq!(config.language[0].extensions, vec!["rs"]);
    }

    #[test]
    fn test_registry_building() {
        let languages = vec![
            LanguageConfig {
                id: "rust".to_string(),
                name: "Rust".to_string(),
                extensions: vec!["rs".to_string()],
                filenames: vec![],
                syntax: None,
                lsp: None,
            },
            LanguageConfig {
                id: "markdown".to_string(),
                name: "Markdown".to_string(),
                extensions: vec!["md".to_string(), "markdown".to_string()],
                filenames: vec!["README".to_string()],
                syntax: None,
                lsp: None,
            },
        ];

        let registry = LanguageRegistry::build_indices(languages);

        // Test extension lookup
        assert!(registry.by_extension.contains_key("rs"));
        assert!(registry.by_extension.contains_key("md"));
        assert!(registry.by_extension.contains_key("markdown"));

        // Test filename lookup
        assert!(registry.by_filename.contains_key("readme")); // lowercase

        // Test ID lookup
        assert!(registry.by_id.contains_key("rust"));
        assert!(registry.by_id.contains_key("markdown"));
    }

    #[test]
    fn test_language_detection() {
        let languages = vec![
            LanguageConfig {
                id: "rust".to_string(),
                name: "Rust".to_string(),
                extensions: vec!["rs".to_string()],
                filenames: vec![],
                syntax: None,
                lsp: None,
            },
            LanguageConfig {
                id: "markdown".to_string(),
                name: "Markdown".to_string(),
                extensions: vec!["md".to_string()],
                filenames: vec!["readme".to_string()],
                syntax: None,
                lsp: None,
            },
        ];

        let registry = LanguageRegistry::build_indices(languages);

        // Test extension detection
        assert_eq!(
            registry.detect("src/main.rs").unwrap().id,
            "rust"
        );
        assert_eq!(
            registry.detect("docs/guide.md").unwrap().id,
            "markdown"
        );

        // Test filename detection
        assert_eq!(
            registry.detect("README").unwrap().id,
            "markdown"
        );

        // Test no match
        assert!(registry.detect("unknown.xyz").is_none());
    }

    #[test]
    fn test_find_project_root() {
        use std::fs;
        use tempfile::tempdir;

        // Create temp directory structure:
        // temp/
        //   Cargo.toml
        //   src/
        //     lib.rs
        //     subdir/
        //       mod.rs

        let temp = tempdir().unwrap();
        let root = temp.path();

        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::create_dir(root.join("src/subdir")).unwrap();
        let file = root.join("src/subdir/mod.rs");
        fs::write(&file, "").unwrap();

        // Find Cargo.toml from nested file
        let markers = vec!["Cargo.toml".to_string()];
        let found_root = find_project_root(&file, &markers);

        assert_eq!(found_root, root);
    }

    #[test]
    fn test_config_merging() {
        let embedded = r#"
            [[language]]
            id = "rust"
            name = "Rust"
            extensions = ["rs"]
        "#;

        let user = Some(
            r#"
            [[language]]
            id = "rust"
            name = "Rust (Custom)"
            extensions = ["rs"]

            [[language]]
            id = "custom"
            name = "Custom Language"
            extensions = ["custom"]
        "#
            .to_string(),
        );

        let languages = LanguageRegistry::parse_configs(embedded, user).unwrap();

        // Should have 2 languages (rust overridden, custom added)
        assert_eq!(languages.len(), 2);

        // Rust should be overridden
        let rust = languages.iter().find(|l| l.id == "rust").unwrap();
        assert_eq!(rust.name, "Rust (Custom)");

        // Custom should be added
        let custom = languages.iter().find(|l| l.id == "custom").unwrap();
        assert_eq!(custom.name, "Custom Language");
    }
}
