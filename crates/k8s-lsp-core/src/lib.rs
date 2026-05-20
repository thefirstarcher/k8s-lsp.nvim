//! Core: in-memory document store and snapshot.
//!
//! Salsa-based incremental queries are introduced in later phases. For now
//! we hold the latest `Document` per URI behind a `Mutex<HashMap>`. The
//! `Snapshot` type captures an immutable view used by handlers.

pub mod snapshot;

pub use snapshot::{Document, DocumentStore, Snapshot};
