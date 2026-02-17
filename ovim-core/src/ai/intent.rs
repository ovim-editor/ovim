/// Classified intent of a user prompt, used to append task-specific hints
/// to the AI request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// Bug fix or error resolution.
    Fix,
    /// Code restructuring without behavior change.
    Refactor,
    /// New code generation.
    Generate,
    /// Explanation request (no edit expected).
    Explain,
    /// Catch-all for prompts that don't match other categories.
    General,
}

impl std::fmt::Display for Intent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Intent::Fix => write!(f, "fix"),
            Intent::Refactor => write!(f, "refactor"),
            Intent::Generate => write!(f, "generate"),
            Intent::Explain => write!(f, "explain"),
            Intent::General => write!(f, "general"),
        }
    }
}

/// Classify the intent of a user prompt using keyword heuristics.
///
/// This is a pure heuristic classifier (no LLM call). It examines the prompt
/// text, presence of a selection, and diagnostic context to determine what
/// kind of task the user is requesting.
pub fn classify_intent(prompt: &str, selection_lines: usize, diagnostic_count: usize) -> Intent {
    let lower = prompt.to_lowercase();

    // Explain: user is asking for information, not an edit
    if starts_with_any(
        &lower,
        &[
            "explain",
            "what does",
            "what is",
            "how does",
            "why does",
            "describe",
        ],
    ) || lower.contains("explain this")
        || lower.contains("what's going on")
    {
        return Intent::Explain;
    }

    // Generate: check prefix-based generation first (before Fix contains-checks,
    // since "add error handling" starts with "add" but contains "error").
    // "make" is excluded — too ambiguous ("make it faster" is not generation).
    if starts_with_any(&lower, &["add", "create", "implement", "write", "generate"])
        || lower.contains("new function")
        || lower.contains("new method")
        || lower.contains("new struct")
        || lower.contains("new type")
        || lower.contains("new test")
    {
        return Intent::Generate;
    }

    // Fix: error-related keywords or diagnostics present
    if starts_with_any(&lower, &["fix", "debug", "resolve", "repair"])
        || lower.contains("bug")
        || lower.contains("error")
        || lower.contains("broken")
        || lower.contains("wrong")
        || lower.contains("not working")
        || lower.contains("doesn't work")
        || lower.contains("doesn't compile")
        || lower.contains("fails")
    {
        return Intent::Fix;
    }

    // If there are diagnostics and user gives a short prompt, likely a fix
    if diagnostic_count > 0 && lower.split_whitespace().count() <= 3 {
        return Intent::Fix;
    }

    // Refactor: restructuring keywords
    if starts_with_any(
        &lower,
        &[
            "refactor", "rename", "extract", "inline", "simplify", "clean up", "cleanup",
        ],
    ) || lower.contains("refactor")
        || lower.contains("restructure")
        || lower.contains("reorganize")
        || lower.contains("simplify")
        || lower.contains("make more readable")
        || lower.contains("clean up")
    {
        return Intent::Refactor;
    }

    // If there's a multi-line selection, it's likely a targeted edit (general)
    if selection_lines > 1 {
        return Intent::General;
    }

    Intent::General
}

fn starts_with_any(text: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| text.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_fix_prompts() {
        assert_eq!(classify_intent("fix the bug", 5, 0), Intent::Fix);
        assert_eq!(classify_intent("Fix this error", 3, 0), Intent::Fix);
        assert_eq!(classify_intent("debug this", 1, 0), Intent::Fix);
        assert_eq!(classify_intent("this is broken", 1, 0), Intent::Fix);
        assert_eq!(classify_intent("doesn't work", 1, 0), Intent::Fix);
    }

    #[test]
    fn classify_fix_from_diagnostics() {
        // Short prompt + diagnostics → fix
        assert_eq!(classify_intent("help", 1, 3), Intent::Fix);
    }

    #[test]
    fn classify_refactor_prompts() {
        assert_eq!(classify_intent("refactor this", 10, 0), Intent::Refactor);
        assert_eq!(
            classify_intent("rename to snake_case", 1, 0),
            Intent::Refactor
        );
        assert_eq!(classify_intent("simplify", 5, 0), Intent::Refactor);
        assert_eq!(
            classify_intent("clean up this function", 20, 0),
            Intent::Refactor
        );
    }

    #[test]
    fn classify_generate_prompts() {
        assert_eq!(
            classify_intent("add error handling", 5, 0),
            Intent::Generate
        );
        assert_eq!(
            classify_intent("implement Display trait", 1, 0),
            Intent::Generate
        );
        assert_eq!(
            classify_intent("write a test for this", 1, 0),
            Intent::Generate
        );
        assert_eq!(
            classify_intent("create a new struct", 0, 0),
            Intent::Generate
        );
    }

    #[test]
    fn classify_explain_prompts() {
        assert_eq!(classify_intent("explain this code", 10, 0), Intent::Explain);
        assert_eq!(classify_intent("what does this do", 5, 0), Intent::Explain);
        assert_eq!(classify_intent("how does this work", 3, 0), Intent::Explain);
    }

    #[test]
    fn classify_general_prompts() {
        assert_eq!(classify_intent("make it faster", 10, 0), Intent::General);
        assert_eq!(
            classify_intent("use iterators instead", 5, 0),
            Intent::General
        );
    }

    #[test]
    fn intent_display() {
        assert_eq!(format!("{}", Intent::Fix), "fix");
        assert_eq!(format!("{}", Intent::Refactor), "refactor");
        assert_eq!(format!("{}", Intent::Generate), "generate");
        assert_eq!(format!("{}", Intent::Explain), "explain");
        assert_eq!(format!("{}", Intent::General), "general");
    }
}
