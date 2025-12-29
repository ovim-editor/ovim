use std::path::Path;

/// Supported programming languages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    JavaScript,
    TypeScript,
    Python,
    Java,
    Go,
    C,
    Cpp,
    Ruby,
    Bash,
    Dockerfile,
    Json,
    Yaml,
    Html,
    Css,
    Toml,
    Markdown,
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

            // JavaScript
            "js" | "jsx" | "mjs" | "cjs" | "es" | "es6" | "es7" => Some(Language::JavaScript),

            // TypeScript
            "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),

            // Python
            "py" | "pyw" | "pyi" | "pyx" | "pxd" | "pxi" | "pyc" | "pyd" | "pyo" | "pyz"
            | "pywz" | "py3" | "pyde" | "pyt" | "snakefile" | "smk" => Some(Language::Python),

            // Java
            "java" => Some(Language::Java),

            // Go
            "go" => Some(Language::Go),

            // C
            "c" | "h" => Some(Language::C),

            // C++
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" => Some(Language::Cpp),

            // Ruby
            "rb" | "rake" | "rbw" | "gemspec" => Some(Language::Ruby),

            // Bash
            "sh" | "bash" | "zsh" | "fish" | "ksh" | "csh" | "tcsh" => Some(Language::Bash),

            // JSON
            "json" | "jsonc" | "json5" => Some(Language::Json),

            // YAML
            "yaml" | "yml" => Some(Language::Yaml),

            // HTML
            "html" | "htm" | "xhtml" => Some(Language::Html),

            // CSS
            "css" | "scss" | "sass" | "less" => Some(Language::Css),

            // TOML
            "toml" => Some(Language::Toml),

            // Markdown
            "md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdx" => Some(Language::Markdown),

            _ => None,
        }
    }

    /// Detects language from full filename (for files without extensions)
    fn detect_from_filename(filename: &str) -> Option<Language> {
        let lower = filename.to_lowercase();

        match lower.as_str() {
            // Python special files
            "pipfile" | "pipfile.lock" | "snakefile" | "wscript" | "sconstruct"
            | ".pythonstartup" | ".pythonrc" => Some(Language::Python),

            // JavaScript/Node special files
            "jakefile" | "gulpfile.js" | "gruntfile.js" | "webpack.config.js"
            | "rollup.config.js" => Some(Language::JavaScript),

            // TypeScript special files
            ".eslintrc.ts" | ".prettierrc.ts" => Some(Language::TypeScript),

            // Bash special files
            ".bashrc" | ".bash_profile" | ".bash_login" | ".bash_logout" | ".zshrc"
            | ".zprofile" | ".zshenv" | ".zlogin" | ".zlogout" | "bashrc" | "zshrc" => {
                Some(Language::Bash)
            }

            // Dockerfile special files
            "dockerfile" | "dockerfile.prod" | "dockerfile.dev" => Some(Language::Dockerfile),

            // Ruby special files
            "rakefile" | "gemfile" | "gemfile.lock" | "guardfile" | "capfile" | "vagrantfile" => {
                Some(Language::Ruby)
            }

            // Go special files
            "go.mod" | "go.sum" => Some(Language::Go),

            // JSON special files
            ".eslintrc" | ".prettierrc" | ".babelrc" | "package.json" | "tsconfig.json"
            | "composer.json" => Some(Language::Json),

            // YAML special files
            ".travis.yml" | ".gitlab-ci.yml" | "docker-compose.yml" | ".clang-format"
            | ".clang-tidy" => Some(Language::Yaml),

            // Markdown special files
            "readme" | "changelog" | "contributing" | "license" => Some(Language::Markdown),

            // TOML special files
            "cargo.toml" | "cargo.lock" | "pyproject.toml" => Some(Language::Toml),

            _ => {
                // Check for common patterns
                if lower.starts_with(".python") {
                    Some(Language::Python)
                } else if lower.ends_with(".js") {
                    Some(Language::JavaScript)
                } else if lower.ends_with(".ts") {
                    Some(Language::TypeScript)
                } else if lower.starts_with(".bash") || lower.starts_with(".zsh") {
                    Some(Language::Bash)
                } else if lower.contains("dockerfile") {
                    // Resolved: Added tree-sitter-dockerfile dependency
                    Some(Language::Dockerfile)
                } else if lower.ends_with("makefile") {
                    Some(Language::Bash)
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
            Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Language::Python => tree_sitter_python::LANGUAGE.into(),
            Language::Java => tree_sitter_java::LANGUAGE.into(),
            Language::Go => tree_sitter_go::LANGUAGE.into(),
            Language::C => tree_sitter_c::LANGUAGE.into(),
            Language::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Language::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Language::Bash => tree_sitter_bash::LANGUAGE.into(),
            // Dockerfile uses Bash syntax highlighting to avoid tree-sitter version conflicts
            // tree-sitter-dockerfile depends on tree-sitter ^0.20, incompatible with our 0.23
            Language::Dockerfile => tree_sitter_bash::LANGUAGE.into(),
            Language::Json => tree_sitter_json::LANGUAGE.into(),
            Language::Yaml => tree_sitter_yaml::language(),
            Language::Html => tree_sitter_html::LANGUAGE.into(),
            Language::Css => tree_sitter_css::LANGUAGE.into(),
            // TOML: tree-sitter-toml depends on tree-sitter 0.20, incompatible with our 0.23
            // Use JSON for highlighting fallback (similar key-value structure)
            Language::Toml => tree_sitter_json::LANGUAGE.into(),
            Language::Markdown => tree_sitter_md::LANGUAGE.into(),
        }
    }

    /// Gets the highlight query for a language
    pub fn get_highlight_query(lang: Language) -> &'static str {
        match lang {
            Language::Rust => include_str!("queries/rust.scm"),
            Language::JavaScript => include_str!("queries/javascript.scm"),
            Language::TypeScript => include_str!("queries/typescript.scm"),
            Language::Python => include_str!("queries/python.scm"),
            Language::Java => include_str!("queries/java.scm"),
            Language::Go => include_str!("queries/go.scm"),
            Language::C => include_str!("queries/c.scm"),
            Language::Cpp => include_str!("queries/cpp.scm"),
            Language::Ruby => include_str!("queries/ruby.scm"),
            Language::Bash => include_str!("queries/bash.scm"),
            Language::Dockerfile => include_str!("queries/bash.scm"), // Use Bash syntax for now
            Language::Json => include_str!("queries/json.scm"),
            Language::Yaml => include_str!("queries/yaml.scm"),
            Language::Html => include_str!("queries/html.scm"),
            Language::Css => include_str!("queries/css.scm"),
            // TOML uses JSON highlighting as fallback (similar key-value structure)
            Language::Toml => include_str!("queries/json.scm"),
            Language::Markdown => include_str!("queries/markdown.scm"),
        }
    }

    /// Get LSP language identifier from file path
    /// Returns None if language is not supported by LSP
    pub fn get_lsp_language_id(file_path: &str) -> Option<&'static str> {
        Self::detect_from_path(file_path).map(|lang| match lang {
            Language::Rust => "rust",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Python => "python",
            Language::Java => "java",
            Language::Go => "go",
            Language::C => "c",
            Language::Cpp => "cpp",
            Language::Ruby => "ruby",
            Language::Bash => "bash",
            Language::Dockerfile => "dockerfile",
            Language::Json => "json",
            Language::Yaml => "yaml",
            Language::Html => "html",
            Language::Css => "css",
            Language::Toml => "toml",
            Language::Markdown => "markdown",
        })
    }

    /// Check if a file path has LSP support
    pub fn has_lsp_support(file_path: &str) -> bool {
        Self::get_lsp_language_id(file_path).is_some()
    }
}
