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
];

#[derive(Default)]
pub struct SchemaRegistry {
    cache: Mutex<HashMap<(String, String), Arc<Value>>>,
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
        None
    }

    /// True if the registry has a schema for this resource.
    pub fn has(&self, api_version: &str, kind: &str) -> bool {
        BUNDLE.iter().any(|(av, k, _)| *av == api_version && *k == kind)
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
