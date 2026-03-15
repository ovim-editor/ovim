/// A shell command queued by `:!cmd` for the event loop to execute
/// with full terminal access (outside the alternate screen).
pub struct PendingShellCommand {
    /// The expanded shell command string
    pub command: String,
}

/// Grouped state for the build/test subsystem (`:make`, `<Space>t` test runs).
pub(crate) struct BuildState {
    /// Pending `:make` result from background thread
    pub(crate) pending_make: Option<super::PendingMake>,
    /// Last test command run via `<Space>t` keybindings (for `<Space>tl` repeat)
    pub(crate) last_test_command: Option<String>,
    /// Raw output from last `:make` / test run
    pub(crate) last_make_output: Option<String>,
    /// Shell command waiting for the event loop to execute with terminal access
    pub(crate) pending_shell_command: Option<PendingShellCommand>,
    /// Last `:!` command (for bare `:!` repeat)
    pub(crate) last_shell_command: Option<String>,
}

impl Default for BuildState {
    fn default() -> Self {
        Self {
            pending_make: None,
            last_test_command: None,
            last_make_output: None,
            pending_shell_command: None,
            last_shell_command: None,
        }
    }
}
