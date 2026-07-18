//! Transient orchestration state projected onto the append-only run log.

mod dispatch;
pub mod fake_provider;
mod service;

pub use dispatch::*;
pub use service::*;
