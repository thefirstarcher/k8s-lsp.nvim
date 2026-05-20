//! Schema registry — embedded yannh standalone-strict JSON schemas for a
//! curated set of built-in Kubernetes resources. Lookup is by (apiVersion,
//! kind). Schemas are parsed lazily on first access.

pub mod completion;
pub mod hover;
pub mod registry;
pub mod validate;
pub mod walk;

pub use completion::{fields_at, FieldCandidate};
pub use hover::{is_secret_field, render_hover};
pub use registry::SchemaRegistry;
pub use validate::{validate, Issue, Severity};
pub use walk::{schema_at_path, PathSeg};
