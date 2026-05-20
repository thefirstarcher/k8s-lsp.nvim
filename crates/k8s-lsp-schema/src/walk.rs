//! Walk a JSON Schema by YAML path, following `properties.<name>` for
//! mappings and `items` for sequences. Handles common combinator wrappers
//! (`allOf`, `oneOf`, `anyOf`) and local `$ref` pointers (`#/definitions/...`).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    Key(String),
    Index, // sequence element; we don't track which index, only "into items"
}

pub fn schema_at_path<'a>(root: &'a Value, path: &[PathSeg]) -> Option<&'a Value> {
    let mut cur = resolve_ref(root, root);
    for seg in path {
        cur = step(root, cur, seg)?;
    }
    Some(cur)
}

/// Resolve a chain of local `$ref` pointers, capped to avoid cycles.
/// Non-`#/` refs and missing targets short-circuit to the current node.
pub fn resolve_ref<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    let mut cur = node;
    for _ in 0..8 {
        let Some(r) = cur.get("$ref").and_then(|v| v.as_str()) else { return cur };
        let Some(rest) = r.strip_prefix("#/") else { return cur };
        let mut next = root;
        let mut ok = true;
        for part in rest.split('/') {
            let part = part.replace("~1", "/").replace("~0", "~");
            match next.get(&part) {
                Some(v) => next = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok { return cur; }
        cur = next;
    }
    cur
}

fn step<'a>(root: &'a Value, node: &'a Value, seg: &PathSeg) -> Option<&'a Value> {
    let node = resolve_ref(root, node);
    if let Some(next) = direct_step(node, seg) {
        return Some(resolve_ref(root, next));
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(arr) = node.get(key).and_then(|v| v.as_array()) {
            for child in arr {
                let child = resolve_ref(root, child);
                if let Some(next) = step(root, child, seg) {
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

    #[test]
    fn follows_root_and_nested_refs() {
        let s = json!({
            "$ref": "#/definitions/Root",
            "definitions": {
                "Root": {
                    "properties": {
                        "child": { "$ref": "#/definitions/Child" }
                    }
                },
                "Child": {
                    "properties": {
                        "name": { "type": "string", "description": "the name" }
                    }
                }
            }
        });
        let leaf = schema_at_path(
            &s,
            &[PathSeg::Key("child".into()), PathSeg::Key("name".into())],
        )
        .unwrap();
        assert_eq!(leaf.get("type").unwrap(), "string");
        assert_eq!(leaf.get("description").unwrap(), "the name");
    }
}
