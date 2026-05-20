//! Render a JSON Schema node as `kubectl explain`-style Markdown.

use serde_json::Value;

pub fn render_hover(qualified_name: &str, schema: &Value) -> String {
    let mut out = String::new();
    out.push_str("**");
    out.push_str(qualified_name);
    out.push_str("**\n\n");

    let leaf = qualified_name.rsplit('.').next().unwrap_or(qualified_name);
    let secret = is_secret_field(leaf);

    let ty = type_label(schema);
    let format = schema.get("format").and_then(|v| v.as_str());
    let default = schema.get("default");
    let required = is_required(schema);

    out.push('`');
    out.push_str(&ty);
    out.push('`');
    if let Some(f) = format {
        out.push_str(&format!(" · format: `{f}`"));
    }
    if required {
        out.push_str(" · **required**");
    }
    if let Some(d) = default {
        if secret {
            out.push_str(" · default: `<redacted>`");
        } else {
            out.push_str(&format!(" · default: `{}`", serde_json::to_string(d).unwrap_or_default()));
        }
    }
    out.push_str("\n\n");

    if let Some(desc) = schema.get("description").and_then(|v| v.as_str()) {
        out.push_str(desc.trim());
        out.push('\n');
    }

    if let Some(en) = schema.get("enum").and_then(|v| v.as_array()) {
        out.push_str("\n**enum:** ");
        let parts: Vec<String> = en
            .iter()
            .map(|v| format!("`{}`", v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())))
            .collect();
        out.push_str(&parts.join(", "));
        out.push('\n');
    }

    out
}

fn type_label(schema: &Value) -> String {
    if let Some(t) = schema.get("type").and_then(|v| v.as_str()) {
        return t.to_string();
    }
    if let Some(arr) = schema.get("type").and_then(|v| v.as_array()) {
        let v: Vec<String> = arr.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        return v.join(" | ");
    }
    for key in ["allOf", "oneOf", "anyOf"] {
        if schema.get(key).is_some() {
            return key.to_string();
        }
    }
    "any".into()
}

fn is_required(_schema: &Value) -> bool {
    // Required-ness lives on the *parent* schema (the JSON Schema `required`
    // array). Walk callers should set this after locating the leaf; for now
    // we return false and let the caller override by passing context.
    false
}

/// Heuristic match for field names that conventionally hold credentials.
/// Matches case-insensitively against `password`, `token`, `secret`, and
/// `apiKey` / `api_key` / `api-key`.
pub fn is_secret_field(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("password") || n.contains("token") || n.contains("secret") {
        return true;
    }
    // api_key / api-key / apikey
    let stripped: String = n.chars().filter(|c| *c != '_' && *c != '-').collect();
    stripped.contains("apikey")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn renders_basic_field() {
        let s = json!({
            "type": "integer",
            "format": "int32",
            "default": 1,
            "description": "Number of desired pods."
        });
        let md = render_hover("Deployment.spec.replicas", &s);
        assert!(md.contains("**Deployment.spec.replicas**"));
        assert!(md.contains("`integer`"));
        assert!(md.contains("format: `int32`"));
        assert!(md.contains("default: `1`"));
        assert!(md.contains("Number of desired pods."));
    }

    #[test]
    fn redacts_default_for_secret_fields() {
        let s = json!({ "type": "string", "default": "hunter2" });
        let md = render_hover("Backend.spec.password", &s);
        assert!(md.contains("`<redacted>`"));
        assert!(!md.contains("hunter2"));
    }

    #[test]
    fn secret_field_matcher() {
        assert!(is_secret_field("password"));
        assert!(is_secret_field("dbPassword"));
        assert!(is_secret_field("apiKey"));
        assert!(is_secret_field("api_key"));
        assert!(is_secret_field("api-key"));
        assert!(is_secret_field("authToken"));
        assert!(is_secret_field("clientSecret"));
        assert!(!is_secret_field("name"));
        assert!(!is_secret_field("replicas"));
    }

    #[test]
    fn renders_enum() {
        let s = json!({ "type": "string", "enum": ["Always", "IfNotPresent", "Never"] });
        let md = render_hover("imagePullPolicy", &s);
        assert!(md.contains("`Always`"));
        assert!(md.contains("`Never`"));
    }
}
