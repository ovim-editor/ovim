//! # TUI Safety: No stdout/stderr output!
//!
//! This is a TUI application. Any output to stdout/stderr will corrupt the terminal display.
//! Use the logging system instead: log_info!, log_warn!, log_error!, log_debug!
//!
//! The following macros are BANNED in source code (tests are OK):
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod api;
pub mod buffer;
pub mod cli;
pub mod client;
pub mod commands;
pub mod daemon;
pub mod display;
pub mod editor;
pub mod git;
pub mod language_config;
pub mod log;
pub mod lsp;
pub mod lua;
pub mod mcp_stdio_server;
pub mod metrics;
pub mod mode;
pub mod modeline;
pub mod session;
pub mod subcommands;
pub mod syntax;
pub mod ui;
pub mod unicode;

pub use git::{GitStatus, LineStatus};
