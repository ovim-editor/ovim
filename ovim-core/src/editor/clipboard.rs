//! Clipboard synchronization at command execution boundaries.
//!
//! Registers own editing state; this integration owns external I/O. A scope
//! observes one lazy external snapshot, then its own writes. Only its final
//! write is published, unless execution explicitly crosses an external boundary.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, MutexGuard};

/// Process-wide serialization is required even across independent editors:
/// NSPasteboard is a singleton and concurrent arboard access can corrupt the
/// Objective-C runtime. Lazily initialize for headless/SSH sessions.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with_clipboard<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Option<T> {
    let mut guard = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = arboard::Clipboard::new().ok();
    }
    f(guard.as_mut()?).ok()
}

/// Fallible external clipboard access, isolated for deterministic testing.
/// Failure is best-effort: the integration retains its local fallback.
pub(crate) trait ClipboardBackend: Debug + Send + Sync {
    fn read(&self) -> Option<String>;
    fn write(&self, text: &str) -> bool;
}

#[derive(Debug)]
struct SystemClipboard;

impl ClipboardBackend for SystemClipboard {
    fn read(&self) -> Option<String> {
        with_clipboard(|cb| cb.get_text())
    }

    fn write(&self, text: &str) -> bool {
        with_clipboard(|cb| cb.set_text(text.to_owned())).is_some()
    }
}

/// Process-local clipboard for embedders and tests that need `clipboard=unnamedplus`
/// semantics without touching (or being disturbed by) the shared OS clipboard.
#[derive(Debug, Default)]
pub(crate) struct MemoryClipboard {
    text: Mutex<Option<String>>,
}

impl ClipboardBackend for MemoryClipboard {
    fn read(&self) -> Option<String> {
        self.text.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn write(&self, text: &str) -> bool {
        *self.text.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        true
    }
}

#[derive(Debug, Default)]
struct ClipboardState {
    /// Latest known text, also used when the OS clipboard is unavailable.
    fallback: String,
    /// Whether fallback also represents this execution scope's snapshot.
    /// Keeping one owned string avoids copying every intermediate macro write.
    snapshot_valid: bool,
    pending_write: bool,
    execution_depth: usize,
}

#[derive(Debug)]
pub(crate) struct Clipboard {
    backend: Arc<dyn ClipboardBackend>,
    // Register inspection historically takes &self. Synchronization state is
    // interior mutable while register contents remain ordinary owned values.
    state: Arc<Mutex<ClipboardState>>,
}

impl Clone for Clipboard {
    fn clone(&self) -> Self {
        // Cloning editing state must not duplicate responsibility for publishing
        // an in-flight command, or leave the clone inside an unfinishable scope.
        Self {
            backend: Arc::clone(&self.backend),
            state: Arc::new(Mutex::new(ClipboardState {
                fallback: self.state().fallback.clone(),
                ..ClipboardState::default()
            })),
        }
    }
}

/// Owns completion of precisely the state that opened an execution scope.
/// It remains valid if the editor replaces or drops its register manager.
#[must_use = "keep the execution guard alive until the command completes"]
pub(crate) struct ClipboardExecutionScope {
    clipboard: Clipboard,
}

impl Drop for ClipboardExecutionScope {
    fn drop(&mut self) {
        self.clipboard.end_execution();
    }
}

/// Refreshes the original clipboard snapshot after external code returns,
/// including early returns and unwinding.
#[must_use = "keep the external guard alive until external execution completes"]
pub(crate) struct ExternalClipboardScope {
    clipboard: Clipboard,
}

impl Drop for ExternalClipboardScope {
    fn drop(&mut self) {
        self.clipboard.invalidate_snapshot();
    }
}

impl Clipboard {
    pub(crate) fn new() -> Self {
        Self::with_backend(Arc::new(SystemClipboard))
    }

    pub(crate) fn in_memory() -> Self {
        Self::with_backend(Arc::new(MemoryClipboard::default()))
    }

    pub(crate) fn with_backend(backend: Arc<dyn ClipboardBackend>) -> Self {
        Self {
            backend,
            state: Arc::new(Mutex::new(ClipboardState::default())),
        }
    }

    /// Scope guards share synchronization ownership. This is deliberately
    /// distinct from Clone, which copies editing state into a fresh lifecycle.
    fn share_state(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            state: Arc::clone(&self.state),
        }
    }

    pub(crate) fn execution_scope(&mut self) -> ClipboardExecutionScope {
        self.begin_execution();
        ClipboardExecutionScope {
            clipboard: self.share_state(),
        }
    }

    pub(crate) fn external_scope(&mut self) -> ExternalClipboardScope {
        self.flush();
        ExternalClipboardScope {
            clipboard: self.share_state(),
        }
    }

    fn state(&self) -> MutexGuard<'_, ClipboardState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin_execution(&mut self) {
        let mut state = self.state();
        if state.execution_depth == 0 {
            state.snapshot_valid = false;
        }
        state.execution_depth += 1;
    }

    fn end_execution(&mut self) {
        let mut state = self.state();
        assert!(
            state.execution_depth > 0,
            "unbalanced clipboard execution scope"
        );
        state.execution_depth -= 1;
        if state.execution_depth == 0 {
            self.flush_pending(&mut state);
            state.snapshot_valid = false;
        }
    }

    pub(crate) fn read(&self) -> String {
        let mut state = self.state();
        if state.execution_depth > 0 {
            // Pending local writes remain authoritative even if a caller
            // invalidates a snapshot before synchronizing external effects.
            if state.pending_write || state.snapshot_valid {
                return state.fallback.clone();
            }
        }
        let text = self
            .backend
            .read()
            .unwrap_or_else(|| state.fallback.clone());
        state.fallback.clone_from(&text);
        if state.execution_depth > 0 {
            state.snapshot_valid = true;
        }
        text
    }

    pub(crate) fn write(&mut self, text: String) {
        let mut state = self.state();
        state.fallback = text;
        if state.execution_depth > 0 {
            state.snapshot_valid = true;
            state.pending_write = true;
        } else {
            self.backend.write(&state.fallback);
        }
    }

    fn flush_pending(&self, state: &mut ClipboardState) {
        if state.pending_write {
            self.backend.write(&state.fallback);
            // An unavailable clipboard must not cause unrelated future commands
            // to publish stale effects. Keep the local fallback, but finish this
            // best-effort synchronization attempt.
            state.pending_write = false;
        }
    }

    pub(super) fn flush(&mut self) {
        self.flush_pending(&mut self.state());
    }

    fn invalidate_snapshot(&mut self) {
        self.state().snapshot_valid = false;
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    #[derive(Debug, Default)]
    pub(crate) struct FakeClipboard {
        pub(crate) state: Mutex<FakeClipboardState>,
    }

    #[derive(Debug, Default)]
    pub(crate) struct FakeClipboardState {
        pub(crate) text: String,
        pub(crate) reads: usize,
        pub(crate) writes: Vec<String>,
        pub(crate) unavailable: bool,
    }

    impl FakeClipboard {
        pub(crate) fn new(text: &str) -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(FakeClipboardState {
                    text: text.to_owned(),
                    ..FakeClipboardState::default()
                }),
            })
        }
    }

    impl ClipboardBackend for FakeClipboard {
        fn read(&self) -> Option<String> {
            let mut state = self.state.lock().unwrap();
            state.reads += 1;
            (!state.unavailable).then(|| state.text.clone())
        }

        fn write(&self, text: &str) -> bool {
            let mut state = self.state.lock().unwrap();
            state.writes.push(text.to_owned());
            if state.unavailable {
                false
            } else {
                state.text = text.to_owned();
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::FakeClipboard;
    use super::*;

    #[test]
    fn standalone_operations_synchronize_immediately() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        assert_eq!(clipboard.read(), "external");
        clipboard.write("local".into());
        assert_eq!(backend.state.lock().unwrap().text, "local");
        backend.state.lock().unwrap().text = "changed".into();
        assert_eq!(clipboard.read(), "changed");
        assert_eq!(backend.state.lock().unwrap().reads, 2);
    }

    #[test]
    fn empty_execution_does_no_io() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        clipboard.end_execution();
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert!(state.writes.is_empty());
    }

    #[test]
    fn writes_are_local_until_outer_execution_completes() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        clipboard.write("first".into());
        clipboard.begin_execution();
        assert_eq!(clipboard.read(), "first");
        clipboard.write("final".into());
        clipboard.end_execution();
        assert!(backend.state.lock().unwrap().writes.is_empty());
        assert_eq!(clipboard.read(), "final");
        clipboard.end_execution();
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 0);
        assert_eq!(state.writes, ["final"]);
    }

    #[test]
    fn reads_share_lazy_snapshot_then_refresh_in_next_execution() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        assert_eq!(clipboard.read(), "external");
        backend.state.lock().unwrap().text = "changed".into();
        assert_eq!(clipboard.read(), "external");
        clipboard.end_execution();
        clipboard.begin_execution();
        assert_eq!(clipboard.read(), "changed");
        clipboard.end_execution();
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 2);
        assert!(state.writes.is_empty());
    }

    #[test]
    fn external_boundary_publishes_prior_writes_and_refreshes_reads() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        clipboard.write("before command".into());
        clipboard.flush();
        assert_eq!(backend.state.lock().unwrap().text, "before command");
        backend.state.lock().unwrap().text = "from command".into();
        clipboard.invalidate_snapshot();
        assert_eq!(clipboard.read(), "from command");
        clipboard.write("after command".into());
        clipboard.end_execution();
        let state = backend.state.lock().unwrap();
        assert_eq!(state.reads, 1);
        assert_eq!(state.writes, ["before command", "after command"]);
    }

    #[test]
    fn unavailable_backend_uses_fallback_without_retrying_stale_writes() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        // Successful reads also seed fallback for subsequent unavailability.
        assert_eq!(clipboard.read(), "external");
        backend.state.lock().unwrap().unavailable = true;
        clipboard.begin_execution();
        assert_eq!(clipboard.read(), "external");
        clipboard.write("local".into());
        clipboard.end_execution();
        clipboard.begin_execution();
        assert_eq!(clipboard.read(), "local");
        clipboard.end_execution();
        assert_eq!(backend.state.lock().unwrap().writes, ["local"]);
    }

    #[test]
    fn invalidation_cannot_discard_pending_local_content() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        clipboard.write("local".into());
        clipboard.invalidate_snapshot();
        assert_eq!(clipboard.read(), "local");
        clipboard.end_execution();
        assert_eq!(backend.state.lock().unwrap().text, "local");
    }

    #[test]
    fn clone_does_not_inherit_pending_effects_or_execution_depth() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        clipboard.begin_execution();
        clipboard.write("original".into());
        let mut cloned = clipboard.clone();
        cloned.begin_execution();
        cloned.end_execution();
        assert!(backend.state.lock().unwrap().writes.is_empty());
        clipboard.end_execution();
        assert_eq!(backend.state.lock().unwrap().writes, ["original"]);
        backend.state.lock().unwrap().unavailable = true;
        assert_eq!(cloned.read(), "original");
    }

    #[test]
    fn owned_guards_complete_original_state_after_clipboard_is_dropped() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        let outer = clipboard.execution_scope();
        clipboard.write("first".into());
        let inner = clipboard.execution_scope();
        clipboard.write("final".into());
        drop(clipboard);
        drop(inner);
        assert!(backend.state.lock().unwrap().writes.is_empty());
        drop(outer);
        assert_eq!(backend.state.lock().unwrap().writes, ["final"]);
    }

    #[test]
    fn external_guard_invalidates_snapshot_during_unwind() {
        let backend = FakeClipboard::new("external");
        let mut clipboard = Clipboard::with_backend(backend.clone());
        let execution = clipboard.execution_scope();
        clipboard.write("before external".into());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _external = clipboard.external_scope();
            assert_eq!(backend.state.lock().unwrap().text, "before external");
            backend.state.lock().unwrap().text = "changed externally".into();
            panic!("external operation failed");
        }));
        assert!(result.is_err());
        assert_eq!(clipboard.read(), "changed externally");
        drop(execution);
        assert_eq!(backend.state.lock().unwrap().writes, ["before external"]);
    }

    #[test]
    fn clipboard_remains_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Clipboard>();
        assert_send_sync::<crate::editor::register::RegisterManager>();
    }
}
