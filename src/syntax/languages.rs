use std::path::Path;

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    JavaScript,
    Python,
    Java,
}

/// Registry for language detection and grammar access
pub struct LanguageRegistry;

impl LanguageRegistry {
    /// Detects language from file path/extension
    /// Supports a comprehensive list of file extensions for each language
    pub fn detect_from_path<P: AsRef<Path>>(path: P) -> Option<Language> {
        let path = path.as_ref();

        // Try extension first
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            if let Some(lang) = Self::detect_from_extension(extension) {
                return Some(lang);
            }
        }

        // Try full filename for special cases (e.g., .bashrc, Dockerfile)
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            return Self::detect_from_filename(filename);
        }

        None
    }

    /// Detects language from file extension only
    fn detect_from_extension(extension: &str) -> Option<Language> {
        match extension.to_lowercase().as_str() {
            // Rust
            "rs" => Some(Language::Rust),

            // JavaScript / TypeScript / JSX / TSX
            "js" | "jsx" | "mjs" | "cjs" | "es" | "es6" | "es7" |
            "ts" | "tsx" | "mts" | "cts" => Some(Language::JavaScript),

            // Python
            "py" | "pyw" | "pyi" | "pyx" | "pxd" | "pxi" |
            "pyc" | "pyd" | "pyo" | "pyz" | "pywz" |
            "py3" | "pyde" | "pyt" | "snakefile" | "smk" => Some(Language::Python),

            // Java
            "java" => Some(Language::Java),

            _ => None,
        }
    }

    /// Detects language from full filename (for files without extensions)
    fn detect_from_filename(filename: &str) -> Option<Language> {
        let lower = filename.to_lowercase();

        match lower.as_str() {
            // Python special files
            "pipfile" | "pipfile.lock" |
            "snakefile" | "wscript" | "sconstruct" |
            ".pythonstartup" | ".pythonrc" => Some(Language::Python),

            // JavaScript/Node special files
            "jakefile" | "gulpfile.js" | "gruntfile.js" |
            "webpack.config.js" | "rollup.config.js" |
            ".eslintrc.js" | ".prettierrc.js" => Some(Language::JavaScript),

            _ => {
                // Check for common patterns
                if lower.starts_with(".python") {
                    Some(Language::Python)
                } else if lower.ends_with(".js") || lower.ends_with(".ts") {
                    Some(Language::JavaScript)
                } else {
                    None
                }
            }
        }
    }

    /// Gets the tree-sitter language grammar
    pub fn get_tree_sitter_language(lang: Language) -> tree_sitter::Language {
        match lang {
            Language::Rust => tree_sitter_rust::LANGUAGE.into(),
            Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
        }
    }

    /// Gets the highlight query for a language
    pub fn get_highlight_query(lang: Language) -> &'static str {
        match lang {
            Language::Rust => include_str!("queries/rust.scm"),
            Language::JavaScript => include_str!("queries/javascript.scm"),
            Language::Python => include_str!("queries/python.scm"),
            Language::Java => include_str!("queries/java.scm"),
        }
    }

    /// Get LSP language identifier from file path
    /// Returns None if language is not supported by LSP
    pub fn get_lsp_language_id(file_path: &str) -> Option<&'static str> {
        Self::detect_from_path(file_path).and_then(|lang| match lang {
            Language::Rust => Some("rust"),
            Language::JavaScript => Some("javascript"),
            Language::Python => Some("python"),
            Language::Java => Some("java"),
        })
    }

    /// Check if a file path has LSP support
    pub fn has_lsp_support(file_path: &str) -> bool {
        Self::get_lsp_language_id(file_path).is_some()
    }
}
