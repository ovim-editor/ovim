//! # TUI Safety: No stdout/stderr output!
//!
//! This is a TUI application. Any output to stdout/stderr will corrupt the terminal display.
//! Use the logging system instead: log_info!, log_warn!, log_error!, log_debug!
//!
//! The following macros are BANNED in source code (tests are OK):
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod api;
pub mod key_convert;
pub mod buffer;
pub mod cli;
pub mod client;
pub mod commands;
pub mod daemon;
pub mod editor;
pub mod lsp;
pub mod lua;
pub mod mcp_stdio_server;
pub mod subcommands;
pub mod syntax;
pub mod ui;

// Re-export modules that moved to ovim-core
pub use ovim_core::display;
pub use ovim_core::git;
pub use ovim_core::language_config;
pub use ovim_core::log;
pub use ovim_core::metrics;
pub use ovim_core::mode;
pub use ovim_core::modeline;
pub use ovim_core::session;
pub use ovim_core::unicode;

pub use ovim_core::git::{GitStatus, LineStatus};
