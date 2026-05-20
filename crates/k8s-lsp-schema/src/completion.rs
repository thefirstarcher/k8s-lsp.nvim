//! Schema-driven completion candidates.

use serde_json::Value;

use crate::walk::{resolve_ref, schema_at_path, PathSeg};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldCandidate {
    pub name: String,
    pub type_label: String,
    pub description: Option<String>,
    pub required: bool,
}

/// Field-name candidates for the mapping at `path` (relative to `root`).
///
/// If `path` resolves to a mapping schema, returns its `properties` (including
/// those merged in via `allOf`/`oneOf`/`anyOf`). Required fields are marked.
pub fn fields_at(root: &Value, path: &[PathSeg]) -> Vec<FieldCandidate> {
    let Some(node) = schema_at_path(root, path) else { return Vec::new() };
    let required = required_set(node);
    let mut out = Vec::new();
    collect_properties(root, node, &required, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn collect_properties(root: &Value, node: &Value, required: &[String], out: &mut Vec<FieldCandidate>) {
    let node = resolve_ref(root, node);
    if let Some(obj) = node.get("properties").and_then(|v| v.as_object()) {
        for (name, child) in obj {
            let child = resolve_ref(root, child);
            out.push(FieldCandidate {
                name: name.clone(),
                type_label: type_label(child),
                description: child
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string()),
                required: required.iter().any(|r| r == name),
            });
        }
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(arr) = node.get(key).and_then(|v| v.as_array()) {
            for child in arr {
                collect_properties(root, child, required, out);
            }
        }
    }
}

fn required_set(node: &Value) -> Vec<String> {
    node.get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn type_label(schema: &Value) -> String {
    if let Some(s) = schema.get("type").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = schema.get("type").and_then(|v| v.as_array()) {
        let v: Vec<String> = arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        if !v.is_empty() {
            return v.join(" | ");
        }
    }
    "any".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lists_top_level_fields() {
        let schema = json!({
            "properties": {
                "spec": { "type": "object" },
                "metadata": { "type": "object" },
            },
            "required": ["spec"],
        });
        let cands = fields_at(&schema, &[]);
        assert_eq!(cands.len(), 2);
        assert!(cands.iter().find(|c| c.name == "spec").unwrap().required);
        assert!(!cands.iter().find(|c| c.name == "metadata").unwrap().required);
    }

    #[test]
    fn lists_nested_fields() {
        let schema = json!({
            "properties": {
                "spec": {
                    "properties": {
                        "replicas": { "type": "integer", "description": "pod count" }
                    }
                }
            }
        });
        let cands = fields_at(&schema, &[PathSeg::Key("spec".into())]);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].name, "replicas");
        assert_eq!(cands[0].type_label, "integer");
        assert_eq!(cands[0].description.as_deref(), Some("pod count"));
    }
}
