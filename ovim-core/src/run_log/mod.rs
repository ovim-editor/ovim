//! Provider-independent, append-only history for agent runs.
//!
//! This module intentionally starts with an in-memory implementation. Durable
//! stores can implement [`RunEventSink`] without changing the event vocabulary
//! consumed by editor projections.

mod event;
mod identity;
mod sqlite;
mod store;

pub use event::*;
pub use identity::*;
pub use sqlite::*;
pub use store::*;
