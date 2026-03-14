/// Grouped state for the build/test subsystem (`:make`, `<Space>t` test runs).
pub(crate) struct BuildState {
    /// Pending `:make` result from background thread
    pub(crate) pending_make: Option<super::PendingMake>,
    /// Last test command run via `<Space>t` keybindings (for `<Space>tl` repeat)
    pub(crate) last_test_command: Option<String>,
    /// Raw output from last `:make` / test run
    pub(crate) last_make_output: Option<String>,
}

impl Default for BuildState {
    fn default() -> Self {
        Self {
            pending_make: None,
            last_test_command: None,
            last_make_output: None,
        }
    }
}
