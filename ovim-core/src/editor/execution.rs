//! Boundaries for synchronous command execution and external effects.
//!
//! A scope batches clipboard synchronization, not edits or undo history.
//! Nested dispatch (mappings, macros, Ex commands) joins the outer scope.
//! Returning an error keeps completed edits and publishes their clipboard value.
//!
//! Physical input opens a scope per dispatch; a mapping RHS or macro runs inside
//! that dispatch. Explicit API SendKeys batches can open a wider scope. Direct
//! Ex entry points also open scopes, including when invoked without a frontend.
//! Merely collecting terminal events for rendering does not define a scope.
//!
//! Clipboard register names (+/*) select content, not synchronization timing.
//! Synchronous shell/Lua execution is an external boundary; asynchronous work and
//! queued interactive :! commands retain their existing scheduling. The scope
//! does not defer parsing, redraw policy, or undo recording.

use super::Editor;

/// Nested ex command lines (`:source` of itself, `:cdo`, `:g`, Lua) fail
/// with E169 past this depth. vim allows 'maxmapdepth' (1000), but every
/// level here is a stack of native frames: a debug build overflows a 2 MiB
/// thread a few hundred levels down, and real use never nests this far.
const MAX_EX_LINE_DEPTH: usize = 100;

/// `:normal` nested in itself (through mappings or `:g`) fails with E192
/// past this depth.
const MAX_NORMAL_DEPTH: usize = 50;

/// The kinds of dispatch that can re-enter themselves without bound.
#[derive(Clone, Copy)]
pub(crate) enum Nesting {
    /// A command line run by [`crate::commands::run_line`].
    ExLine,
    /// Keys typed by `:normal`.
    Normal,
}

impl Editor {
    /// Run `run` one level deeper in `kind`. Returns `None` without running
    /// it when that nesting is already too deep: a mapping that invokes
    /// itself through `:normal` would otherwise overflow the stack and take
    /// every unsaved buffer with it.
    pub(crate) fn nested<R>(
        &mut self,
        kind: Nesting,
        run: impl FnOnce(&mut Self) -> R,
    ) -> Option<R> {
        let (depth, max) = match kind {
            Nesting::ExLine => (&mut self.input.ex_line_depth, MAX_EX_LINE_DEPTH),
            Nesting::Normal => (&mut self.input.normal_depth, MAX_NORMAL_DEPTH),
        };
        if *depth >= max {
            return None;
        }
        *depth += 1;
        let result = run(self);
        match kind {
            Nesting::ExLine => self.input.ex_line_depth -= 1,
            Nesting::Normal => self.input.normal_depth -= 1,
        }
        Some(result)
    }

    /// Run synchronous commands as one unit of clipboard synchronization.
    /// Frontends may use this for an explicit programmatic command batch;
    /// unrelated physical events should remain separate scopes.
    pub fn with_execution_scope<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        let _scope = self.registers.execution_scope();
        run(self)
    }

    /// Publish clipboard effects before handing control to synchronous external
    /// code, then discard the external snapshot so subsequent reads see changes.
    /// This does not change scheduling of commands queued for a frontend.
    pub(crate) fn with_external_effects<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        let _boundary = self.registers.external_clipboard_scope();
        run(self)
    }
}
