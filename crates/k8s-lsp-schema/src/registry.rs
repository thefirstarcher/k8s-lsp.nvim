use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

/// Embedded schema bundle: each entry is (apiVersion, kind, raw JSON).
/// Sourced from yannh/kubernetes-json-schema standalone-strict v1.32.0.
const BUNDLE: &[(&str, &str, &str)] = &[
    ("v1", "ConfigMap", include_str!("../../../assets/schemas/configmap-v1.json")),
    ("v1", "Namespace", include_str!("../../../assets/schemas/namespace-v1.json")),
    ("v1", "Pod", include_str!("../../../assets/schemas/pod-v1.json")),
    ("v1", "Secret", include_str!("../../../assets/schemas/secret-v1.json")),
    ("v1", "Service", include_str!("../../../assets/schemas/service-v1.json")),
    ("apps/v1", "DaemonSet", include_str!("../../../assets/schemas/daemonset-apps-v1.json")),
    ("apps/v1", "Deployment", include_str!("../../../assets/schemas/deployment-apps-v1.json")),
    ("apps/v1", "StatefulSet", include_str!("../../../assets/schemas/statefulset-apps-v1.json")),
    ("batch/v1", "CronJob", include_str!("../../../assets/schemas/cronjob-batch-v1.json")),
    ("batch/v1", "Job", include_str!("../../../assets/schemas/job-batch-v1.json")),
    (
        "kustomize.config.k8s.io/v1beta1",
        "Kustomization",
        include_str!("../../../assets/schemas/kustomization-v1beta1.json"),
    ),
];

/// Embedded ObjectMeta JSON schema. Used as a fallback for hover/completion
/// on `metadata.*` paths when a CRD's schema declares `metadata` as an
/// opaque object (typical: `properties.metadata: { type: object }`).
const OBJECT_META: &str = include_str!("../../../assets/schemas/objectmeta-v1.json");

static OBJECT_META_CACHE: std::sync::OnceLock<Arc<Value>> = std::sync::OnceLock::new();

pub fn object_meta() -> Arc<Value> {
    OBJECT_META_CACHE
        .get_or_init(|| {
            let v: Value = serde_json::from_str(OBJECT_META)
                .expect("embedded ObjectMeta schema must parse");
            Arc::new(v)
        })
        .clone()
}

#[derive(Default)]
pub struct SchemaRegistry {
    cache: Mutex<HashMap<(String, String), Arc<Value>>>,
    dynamic: Mutex<HashMap<(String, String), Arc<Value>>>,
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lookup(&self, api_version: &str, kind: &str) -> Option<Arc<Value>> {
        let key = (api_version.to_string(), kind.to_string());
        {
            let g = self.cache.lock().unwrap();
            if let Some(v) = g.get(&key) {
                return Some(v.clone());
            }
        }
        for (av, k, raw) in BUNDLE {
            if *av == api_version && *k == kind {
                let parsed: Value = match serde_json::from_str(raw) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(api_version, kind, error = %e, "schema parse failed");
                        return None;
                    }
                };
                let arc = Arc::new(parsed);
                self.cache.lock().unwrap().insert(key, arc.clone());
                return Some(arc);
            }
        }
        let g = self.dynamic.lock().unwrap();
        g.get(&key).cloned()
    }

    /// True if the registry has a schema for this resource.
    pub fn has(&self, api_version: &str, kind: &str) -> bool {
        if BUNDLE.iter().any(|(av, k, _)| *av == api_version && *k == kind) {
            return true;
        }
        let key = (api_version.to_string(), kind.to_string());
        self.dynamic.lock().unwrap().contains_key(&key)
    }

    /// Insert a schema discovered at runtime (e.g. CRD from live cluster or
    /// offline file). Replaces any previous dynamic entry for (apiVersion, kind).
    /// Built-in BUNDLE entries always win over dynamic.
    pub fn insert_dynamic(&self, api_version: String, kind: String, schema: Value) {
        let key = (api_version, kind);
        self.dynamic.lock().unwrap().insert(key, Arc::new(schema));
    }

    /// Count of dynamic schemas currently registered (for diagnostics/logging).
    pub fn dynamic_len(&self) -> usize {
        self.dynamic.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_known_resource() {
        let r = SchemaRegistry::new();
        let s = r.lookup("apps/v1", "Deployment").expect("Deployment schema");
        assert!(s.get("properties").and_then(|p| p.get("spec")).is_some());
    }

    #[test]
    fn lookup_unknown_returns_none() {
        let r = SchemaRegistry::new();
        assert!(r.lookup("v1", "DoesNotExist").is_none());
    }
}
