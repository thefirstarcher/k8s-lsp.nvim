//! Minimal JSON-Schema-style validator.
//!
//! Covers the high-value subset for k8s manifests: required fields, unknown
//! fields under `additionalProperties: false`, and primitive type mismatches.
//! Walks descendants transparently through `allOf`/`oneOf`/`anyOf`.

use serde_json::Value as Json;
use serde_yaml::Value as Yaml;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub path: Vec<String>,
    pub message: String,
    pub severity: Severity,
}

pub fn validate(value: &Yaml, schema: &Json) -> Vec<Issue> {
    let mut issues = Vec::new();
    walk(value, schema, &mut Vec::new(), &mut issues);
    issues
}

fn walk(value: &Yaml, schema: &Json, path: &mut Vec<String>, issues: &mut Vec<Issue>) {
    let schema = resolve(schema);
    check_type(value, schema, path, issues);

    match value {
        Yaml::Mapping(map) => {
            let props = effective_properties(schema);
            let additional = additional_properties_allowed(schema);
            for (k, v) in map {
                let Some(k) = k.as_str() else { continue };
                path.push(k.to_string());
                if let Some(child_schema) = props.iter().find(|(name, _)| *name == k).map(|(_, s)| s) {
                    walk(v, child_schema, path, issues);
                } else if !additional {
                    issues.push(Issue {
                        path: path.clone(),
                        message: format!("unknown field `{k}`"),
                        severity: Severity::Warning,
                    });
                }
                path.pop();
            }
            for req in required_fields(schema) {
                if !map.iter().any(|(k, _)| k.as_str() == Some(req.as_str())) {
                    path.push(req.clone());
                    issues.push(Issue {
                        path: path.clone(),
                        message: format!("missing required field `{req}`"),
                        severity: Severity::Error,
                    });
                    path.pop();
                }
            }
        }
        Yaml::Sequence(seq) => {
            if let Some(items) = schema.get("items") {
                for (i, v) in seq.iter().enumerate() {
                    path.push(format!("[{i}]"));
                    walk(v, items, path, issues);
                    path.pop();
                }
            }
        }
        _ => {}
    }
}

fn resolve(schema: &Json) -> &Json {
    // For combinators, prefer the first child that has `properties` (mapping schema).
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(arr) = schema.get(key).and_then(|v| v.as_array()) {
            if let Some(child) = arr.iter().find(|c| c.get("properties").is_some()) {
                return child;
            }
        }
    }
    schema
}

fn effective_properties(schema: &Json) -> Vec<(&str, &Json)> {
    let mut out = Vec::new();
    if let Some(obj) = schema.get("properties").and_then(|v| v.as_object()) {
        out.extend(obj.iter().map(|(k, v)| (k.as_str(), v)));
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(arr) = schema.get(key).and_then(|v| v.as_array()) {
            for child in arr {
                if let Some(obj) = child.get("properties").and_then(|v| v.as_object()) {
                    out.extend(obj.iter().map(|(k, v)| (k.as_str(), v)));
                }
            }
        }
    }
    out
}

fn additional_properties_allowed(schema: &Json) -> bool {
    match schema.get("additionalProperties") {
        Some(Json::Bool(b)) => *b,
        Some(_) => true,
        None => true,
    }
}

fn required_fields(schema: &Json) -> Vec<String> {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn check_type(value: &Yaml, schema: &Json, path: &[String], issues: &mut Vec<Issue>) {
    let Some(expected) = type_strings(schema) else { return };
    let actual = yaml_type(value);
    let ok = expected.iter().any(|t| t == actual || (t == "number" && actual == "integer"));
    if !ok && actual != "null" {
        issues.push(Issue {
            path: path.to_vec(),
            message: format!("expected {}, got {}", expected.join(" | "), actual),
            severity: Severity::Error,
        });
    }
}

fn type_strings(schema: &Json) -> Option<Vec<String>> {
    if let Some(s) = schema.get("type").and_then(|v| v.as_str()) {
        return Some(vec![s.to_string()]);
    }
    if let Some(arr) = schema.get("type").and_then(|v| v.as_array()) {
        let v: Vec<String> = arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

fn yaml_type(v: &Yaml) -> &'static str {
    match v {
        Yaml::Null => "null",
        Yaml::Bool(_) => "boolean",
        Yaml::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Yaml::Number(_) => "number",
        Yaml::String(_) => "string",
        Yaml::Sequence(_) => "array",
        Yaml::Mapping(_) => "object",
        Yaml::Tagged(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_yaml::from_str;

    #[test]
    fn flags_unknown_field() {
        let schema = json!({
            "properties": { "replicas": { "type": "integer" } },
            "additionalProperties": false,
        });
        let value: Yaml = from_str("replicas: 3\nbogus: 1\n").unwrap();
        let issues = validate(&value, &schema);
        assert!(issues.iter().any(|i| i.message.contains("bogus")));
    }

    #[test]
    fn flags_type_mismatch() {
        let schema = json!({ "properties": { "replicas": { "type": "integer" } } });
        let value: Yaml = from_str("replicas: three\n").unwrap();
        let issues = validate(&value, &schema);
        assert!(issues.iter().any(|i| i.message.contains("expected integer")));
    }

    #[test]
    fn flags_missing_required() {
        let schema = json!({
            "properties": { "name": { "type": "string" } },
            "required": ["name"],
        });
        let value: Yaml = from_str("{}\n").unwrap();
        let issues = validate(&value, &schema);
        assert!(issues.iter().any(|i| i.message.contains("missing required")));
    }

    #[test]
    fn ignores_when_additional_properties_true() {
        let schema = json!({ "properties": {}, "additionalProperties": true });
        let value: Yaml = from_str("anything: 1\n").unwrap();
        assert!(validate(&value, &schema).is_empty());
    }
}
