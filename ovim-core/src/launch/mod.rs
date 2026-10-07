//! Running and debugging JVM programs.
//!
//! This module holds the pure, testable pieces; the editor-side state machine
//! that strings them together lives in `editor/launch_flow.rs`.
//!
//! - [`plan`]: one normalised [`plan::LaunchPlan`] for every source of
//!   launch information (Hyperion's `hyperion.resolveLaunch`, `.ovim/debug.toml`,
//!   `hyperion.runConfigurations`).
//! - [`lsp`]: asking the language server that owns a document.
//! - [`process`]: non-blocking child processes with streamed output.
//! - [`process_groups`]: keeping started programs from outliving the editor.
//! - [`diagnostics`]: javac / kotlinc / Gradle / Maven output -> quickfix.
//! - [`stacktrace`]: jumpable `at com.foo.Bar.baz(Bar.java:42)` lines.
//! - [`console`]: the persistent run console model.
//! - [`junit`]: JUnit XML results.
//! - [`test_report`]: a finished test run's results, prepared off the editor thread.

pub mod console;
pub mod diagnostics;
pub mod junit;
pub mod lsp;
pub mod plan;
pub mod process;
pub mod process_groups;
pub mod stacktrace;
pub mod test_report;

pub use console::{
    ConsoleLine, LineKind, RunConsoleState, RunOutcome, RunPhase, RunRecord, RunStatus,
};
pub use plan::{LaunchMode, LaunchPlan, PlanKind};
pub use process_groups::kill_all_launch_groups;
