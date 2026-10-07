//! Frontend-agnostic runtime plumbing shared by every frontend that embeds
//! the editor core (the TUI, the headless loop, and the GUI).
//!
//! `ovim-core` has no ratatui/crossterm dependencies; this module is the
//! analogous boundary inside the `ovim` lib target: everything here is safe
//! for a non-terminal frontend to call directly. Terminal-specific code
//! (crossterm event handling, shell suspend/resume, the two event loops
//! themselves) stays in the binary's `event_loop.rs`.
//!
//! ## The frontend contract
//!
//! A frontend embedding the editor core must:
//!
//! 1. Call [`handle_viewport_resize`] whenever the grid geometry changes
//!    (terminal resize, window resize, split/pane changes).
//! 2. Build one [`TickState`] per `Editor` and call `editor.tick(&mut state)`
//!    on a periodic interval. The tick (in `ovim-core`, see
//!    `ovim_core::tick`) owns all background work and its cadences: LSP,
//!    DAP, syntax, picker loading and result delivery, the external-file
//!    check (500ms) and the debounced full rehighlight (200ms after the
//!    last edit).
//! 3. Handle the returned [`TickReport`]: a [`TerminalRequest`] (`:!cmd`,
//!    `:terminal`) must be run with a real terminal or declined with a
//!    status message.
//! 4. Call [`refresh_after_input`] after dispatching input to the editor,
//!    then call `editor.dispatch_pending_intents().await` right after —
//!    otherwise LSP-triggered work waits for the next tick.
//! 5. On shutdown, call `editor.close_current_file_lsp().await` so the
//!    language server sees a clean `didClose` instead of a dropped socket.

pub mod layout;
mod refresh;
mod viewport;

pub use ovim_core::tick::{process_external_file_change, TerminalRequest, TickReport, TickState};
pub use refresh::{refresh_after_api_mutation, refresh_after_input};
pub use viewport::{compute_text_width, handle_viewport_resize};
