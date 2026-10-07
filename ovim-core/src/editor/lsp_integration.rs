//! LSP Integration for Editor
//!
//! This module contains all LSP-related functionality extracted from the main editor module.
//! It provides LSP initialization, document synchronization, LSP actions, and workspace editing.
//!
//! The editor-wide plumbing is split by responsibility:
//! - `lifecycle`: initialization, install consent, server registry, crash recovery
//! - `status`: status line text and toast classification
//! - `document_sync`: per-document sync bookkeeping
//! - `document_notifications`: `didOpen`/`didChange`/`didSave`/`didClose` senders
//! - `diagnostics_refresh`: keeping the diagnostics slot in step with sync
//! - `intents`: request flags and their dispatcher
//! - `request_context`: request preparation and UTF-16 position conversion
//! - `response_polling`: polling response slots and applying their results
//! - `goto_results`: applying goto-definition/implementation/type results
//! - `server_messages`: file-watch forwarding and `window/showMessage` handling

// Submodules for focused functionality
#[path = "lsp_modules/mod.rs"]
pub(in crate::editor) mod lsp_modules;

mod diagnostics_refresh;
mod document_notifications;
mod document_sync;
mod goto_results;
mod intents;
mod lifecycle;
mod request_context;
mod response_polling;
mod server_messages;
mod status;
#[cfg(test)]
mod tests;

use super::*;

pub(in crate::editor) use request_context::LspRequestContext;
