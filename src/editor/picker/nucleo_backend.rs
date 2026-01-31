use super::super::nucleo_matcher::NucleoMatcher;

/// State for the FindFiles picker mode using nucleo parallel fuzzy matching.
pub struct NucleoState {
    pub nucleo: NucleoMatcher,
    pub matched_count: usize,
    pub cached_visible_indices: Vec<u32>,
    pub cached_visible_start: usize,
    pub loading: bool,
    pub loading_spawned: bool,
}

impl NucleoState {
    pub fn new() -> Self {
        Self {
            nucleo: NucleoMatcher::new(),
            matched_count: 0,
            cached_visible_indices: Vec::new(),
            cached_visible_start: 0,
            loading: true,
            loading_spawned: false,
        }
    }
}
