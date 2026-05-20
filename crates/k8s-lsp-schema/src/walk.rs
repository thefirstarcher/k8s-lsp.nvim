//! Walk a JSON Schema by YAML path, following `properties.<name>` for
//! mappings and `items` for sequences. Handles common combinator wrappers
//! (`allOf`, `oneOf`, `anyOf`) by descending into the first child that
//! advances the walk.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    Key(String),
    Index, // sequence element; we don't track which index, only "into items"
}

pub fn schema_at_path<'a>(root: &'a Value, path: &[PathSeg]) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path {
        cur = step(cur, seg)?;
    }
    Some(cur)
}

fn step<'a>(node: &'a Value, seg: &PathSeg) -> Option<&'a Value> {
    if let Some(next) = direct_step(node, seg) {
        return Some(next);
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(arr) = node.get(key).and_then(|v| v.as_array()) {
            for child in arr {
                if let Some(next) = step(child, seg) {
                    return Some(next);
                }
            }
        }
    }
    None
}

fn direct_step<'a>(node: &'a Value, seg: &PathSeg) -> Option<&'a Value> {
    match seg {
        PathSeg::Key(k) => node.get("properties").and_then(|p| p.get(k)).or_else(|| {
            // additionalProperties as the catch-all schema for unknown keys
            node.get("additionalProperties").and_then(|ap| ap.as_object().map(|_| ap))
        }),
        PathSeg::Index => node.get("items"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn walks_properties() {
        let s = json!({
            "properties": {
                "spec": {
                    "properties": {
                        "replicas": { "type": "integer", "description": "ok" }
                    }
                }
            }
        });
        let leaf = schema_at_path(
            &s,
            &[PathSeg::Key("spec".into()), PathSeg::Key("replicas".into())],
        )
        .unwrap();
        assert_eq!(leaf.get("type").unwrap(), "integer");
    }

    #[test]
    fn walks_items() {
        let s = json!({
            "properties": {
                "containers": {
                    "type": "array",
                    "items": { "properties": { "image": { "type": "string" } } }
                }
            }
        });
        let leaf = schema_at_path(
            &s,
            &[PathSeg::Key("containers".into()), PathSeg::Index, PathSeg::Key("image".into())],
        )
        .unwrap();
        assert_eq!(leaf.get("type").unwrap(), "string");
    }
}
