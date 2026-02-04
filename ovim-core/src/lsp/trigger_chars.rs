//! Completion trigger character fallback table.
//!
//! Prefer using `completionProvider.triggerCharacters` from the LSP server when available.
//! This table is a best-effort fallback for when capabilities aren't ready yet.

pub fn fallback_completion_trigger_characters(language_id: &str) -> &'static [char] {
    match language_id {
        // Rust: method access (`.`) and module paths (`::`) commonly drive completion.
        "rust" => &['.', ':'],

        // JVM languages
        "java" | "kotlin" => &['.'],

        // JS/TS
        "javascript" | "typescript" => &['.', '\'', '"', '/', '@', '<'],

        // Web
        "css" | "scss" | "less" => &['.', ':', '#', '-'],
        "html" => &['<', ' ', '"', '\'', '/', '='],

        // Python / Go / Ruby
        "python" => &['.'],
        "go" => &['.'],
        "ruby" => &['.', ':'],

        // C-family
        "c" | "cpp" | "objective-c" | "objective-cpp" => &['.', '>', ':'],
        "csharp" => &['.'],

        _ => &['.'],
    }
}

