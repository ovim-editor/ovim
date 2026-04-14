//! LSP integration modules
//!
//! This module organizes LSP functionality into focused submodules:
//! - `hover`: Hover information display (K command)
//! - `goto`: Go-to-definition, implementation, type
//! - `diagnostics`: Error/warning diagnostics
//! - `completion`: Code completion
//! - `actions`: Code actions, formatting, refactoring

// Submodules extend Editor with LSP functionality
pub(in crate::editor) mod actions;
mod completion;
mod diagnostics;
mod goto;
mod hover;
mod inlay_hints;
pub(in crate::editor) mod navigation;
mod references;
pub(in crate::editor) mod workspace_edits;
