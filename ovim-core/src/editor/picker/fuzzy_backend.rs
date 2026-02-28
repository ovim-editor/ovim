/// Variant for FuzzyList-based picker modes (Custom, Completion, LspLocations, DebugConfig).
/// These all use the same fuzzy filtering logic on `all_results`/`filtered_results`.
pub enum FuzzyListKind {
    Custom,
    Completion,
    LspLocations,
    DebugConfig,
}
