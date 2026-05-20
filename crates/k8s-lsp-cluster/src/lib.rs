//! Cluster: opt-in kube-rs wrapper.
//!
//! Listing a small set of namespaced + cluster-scoped resource names to feed
//! value-position completion. Strictly off the hot path: every request has a
//! short timeout and failures degrade to "empty cluster state" so hover/
//! completion never block on the network.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace, PersistentVolumeClaim, Secret, ServiceAccount};
use k8s_openapi::api::scheduling::v1::PriorityClass;
use k8s_openapi::api::storage::v1::StorageClass;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{Api, ListParams, ObjectList, ResourceExt};
use kube::Client;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

/// A schema discovered at runtime (live CRD or offline file).
#[derive(Debug, Clone)]
pub struct CrdSchema {
    /// "group/version" — e.g. "postgresql.cnpg.io/v1".
    pub api_version: String,
    pub kind: String,
    /// JSON-schema body matching the shape of bundled schemas: an object with
    /// `properties.spec` etc. Sourced from `spec.versions[i].schema.openAPIV3Schema`.
    pub schema: Value,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClusterRef {
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Default)]
struct Inner {
    resources: BTreeMap<String, Vec<ClusterRef>>,
    last_refresh: Option<Instant>,
    last_error: Option<String>,
}

/// Lazily-populated, periodically refreshed view of the bound cluster.
#[derive(Default, Clone)]
pub struct ClusterState {
    inner: Arc<RwLock<Inner>>,
}

impl ClusterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current per-Kind name map. Cheap; clones the small index.
    pub async fn snapshot(&self) -> BTreeMap<String, Vec<ClusterRef>> {
        self.inner.read().await.resources.clone()
    }

    pub async fn last_error(&self) -> Option<String> {
        self.inner.read().await.last_error.clone()
    }

    /// Refresh the cluster cache by listing a curated set of resources.
    /// Soft failures are logged and stored in `last_error` rather than
    /// bubbling up: completion must keep working even when the cluster is
    /// unreachable.
    pub async fn refresh(&self) -> Result<()> {
        let client = match tokio::time::timeout(REQUEST_TIMEOUT, Client::try_default()).await {
            Ok(Ok(c)) => c,
            Ok(Err(e)) => {
                self.set_error(format!("kube client init failed: {e}")).await;
                return Ok(());
            }
            Err(_) => {
                self.set_error("kube client init timed out".into()).await;
                return Ok(());
            }
        };

        let mut resources: BTreeMap<String, Vec<ClusterRef>> = BTreeMap::new();
        list_cluster_scoped::<Namespace>(&client, "Namespace", &mut resources).await;
        list_cluster_scoped::<StorageClass>(&client, "StorageClass", &mut resources).await;
        list_cluster_scoped::<PriorityClass>(&client, "PriorityClass", &mut resources).await;
        list_namespaced::<ServiceAccount>(&client, "ServiceAccount", &mut resources).await;
        list_namespaced::<ConfigMap>(&client, "ConfigMap", &mut resources).await;
        list_namespaced::<Secret>(&client, "Secret", &mut resources).await;
        list_namespaced::<PersistentVolumeClaim>(&client, "PersistentVolumeClaim", &mut resources).await;

        let mut g = self.inner.write().await;
        g.resources = resources;
        g.last_refresh = Some(Instant::now());
        g.last_error = None;
        Ok(())
    }

    async fn set_error(&self, msg: String) {
        tracing::warn!(error = %msg, "cluster refresh degraded");
        let mut g = self.inner.write().await;
        g.last_error = Some(msg);
    }
}

async fn list_cluster_scoped<K>(client: &Client, kind: &str, out: &mut BTreeMap<String, Vec<ClusterRef>>)
where
    K: kube::Resource<DynamicType = (), Scope = k8s_openapi::ClusterResourceScope>
        + Clone
        + std::fmt::Debug
        + for<'de> serde::Deserialize<'de>
        + 'static,
{
    let api: Api<K> = Api::all(client.clone());
    let items = match timed_list(api.list(&ListParams::default())).await {
        Ok(list) => list,
        Err(e) => {
            tracing::debug!(kind, error = %e, "cluster-scoped list failed");
            return;
        }
    };
    let refs: Vec<ClusterRef> = items
        .into_iter()
        .map(|o| ClusterRef { namespace: None, name: o.name_any() })
        .collect();
    if !refs.is_empty() {
        out.insert(kind.to_string(), refs);
    }
}

async fn list_namespaced<K>(client: &Client, kind: &str, out: &mut BTreeMap<String, Vec<ClusterRef>>)
where
    K: kube::Resource<DynamicType = (), Scope = k8s_openapi::NamespaceResourceScope>
        + Clone
        + std::fmt::Debug
        + for<'de> serde::Deserialize<'de>
        + 'static,
{
    let api: Api<K> = Api::all(client.clone());
    let items = match timed_list(api.list(&ListParams::default())).await {
        Ok(list) => list,
        Err(e) => {
            tracing::debug!(kind, error = %e, "namespaced list failed");
            return;
        }
    };
    let refs: Vec<ClusterRef> = items
        .into_iter()
        .map(|o| ClusterRef { namespace: o.namespace(), name: o.name_any() })
        .collect();
    if !refs.is_empty() {
        out.insert(kind.to_string(), refs);
    }
}

async fn timed_list<K>(fut: impl std::future::Future<Output = kube::Result<ObjectList<K>>>) -> Result<Vec<K>>
where
    K: Clone + std::fmt::Debug + for<'de> serde::Deserialize<'de>,
{
    let list = tokio::time::timeout(REQUEST_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow::anyhow!("list timed out"))??;
    Ok(list.items)
}

/// List every CRD in the bound cluster and flatten to one entry per
/// (group/version, kind). Soft-fails like the rest of this module: any
/// failure returns an empty Vec so the server keeps serving built-in
/// schemas.
pub async fn list_crds() -> Vec<CrdSchema> {
    let client = match tokio::time::timeout(REQUEST_TIMEOUT, Client::try_default()).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "CRD fetch: kube client init failed");
            return Vec::new();
        }
        Err(_) => {
            tracing::warn!("CRD fetch: kube client init timed out");
            return Vec::new();
        }
    };

    let api: Api<CustomResourceDefinition> = Api::all(client);
    let items = match timed_list(api.list(&ListParams::default())).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "CRD list failed");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for crd in items {
        let group = crd.spec.group.clone();
        let kind = crd.spec.names.kind.clone();
        for v in crd.spec.versions {
            let Some(validation) = v.schema else { continue };
            let Some(schema) = validation.open_api_v3_schema else { continue };
            let api_version = if group.is_empty() {
                v.name.clone()
            } else {
                format!("{group}/{}", v.name)
            };
            // serde_json::Value via re-serialize.
            let raw = match serde_json::to_value(&schema) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(api_version, kind, error = %e, "CRD schema serialize failed");
                    continue;
                }
            };
            out.push(CrdSchema {
                api_version,
                kind: kind.clone(),
                schema: raw,
            });
        }
    }
    tracing::info!(count = out.len(), "CRD schemas loaded from cluster");
    out
}

/// Load CRD schemas from local YAML/JSON files. Each file may contain one or
/// more CustomResourceDefinition documents (YAML `---`-separated). Bad files
/// are logged and skipped; one broken file never poisons the rest.
pub fn load_crds_from_paths<P: AsRef<std::path::Path>>(paths: &[P]) -> Vec<CrdSchema> {
    let mut out = Vec::new();
    for p in paths {
        let path = p.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "CRD file read failed");
                continue;
            }
        };
        for doc in serde_yaml::Deserializer::from_str(&text) {
            let val: Value = match Value::deserialize(doc) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "CRD yaml parse failed");
                    continue;
                }
            };
            let crd: CustomResourceDefinition = match serde_json::from_value(val) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "CRD shape mismatch");
                    continue;
                }
            };
            let group = crd.spec.group.clone();
            let kind = crd.spec.names.kind.clone();
            for v in crd.spec.versions {
                let Some(validation) = v.schema else { continue };
                let Some(schema) = validation.open_api_v3_schema else { continue };
                let api_version = if group.is_empty() {
                    v.name.clone()
                } else {
                    format!("{group}/{}", v.name)
                };
                let raw = match serde_json::to_value(&schema) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(api_version, kind, error = %e, "CRD schema serialize failed");
                        continue;
                    }
                };
                out.push(CrdSchema {
                    api_version,
                    kind: kind.clone(),
                    schema: raw,
                });
            }
        }
    }
    tracing::info!(count = out.len(), "CRD schemas loaded from files");
    out
}

