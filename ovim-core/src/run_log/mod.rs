//! Provider-independent, append-only history for agent runs.
//!
//! This module intentionally starts with an in-memory implementation. Durable
//! stores can implement [`RunEventSink`] without changing the event vocabulary
//! consumed by editor projections.

mod artifact;
mod event;
mod identity;
mod layout;
mod local_store;
mod repository;
mod sqlite;
mod store;

pub use artifact::*;
pub use event::*;
pub use identity::*;
pub use layout::*;
pub use local_store::*;
pub use repository::*;
pub use sqlite::*;
pub use store::*;
