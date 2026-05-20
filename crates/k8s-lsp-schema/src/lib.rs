//! Schema registry — embedded yannh standalone-strict JSON schemas for a
//! curated set of built-in Kubernetes resources. Lookup is by (apiVersion,
//! kind). Schemas are parsed lazily on first access.

pub mod hover;
pub mod registry;
pub mod walk;

pub use hover::render_hover;
pub use registry::SchemaRegistry;
pub use walk::{schema_at_path, PathSeg};
