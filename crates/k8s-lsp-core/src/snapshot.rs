use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use k8s_lsp_parser::{parse, DocumentPart};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceRef {
    pub uri: Url,
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub version: i32,
    pub text: String,
    pub parts: Vec<DocumentPart>,
}

impl Document {
    pub fn new(uri: Url, version: i32, text: String) -> Self {
        let mut parts = parse(&text);
        apply_kustomization_fallback(&uri, &mut parts);
        Self { uri, version, text, parts }
    }
}

/// Kustomization files conventionally omit `apiVersion`/`kind`. Detect by
/// filename and inject the canonical identity so schema lookup works.
fn apply_kustomization_fallback(uri: &Url, parts: &mut [DocumentPart]) {
    let Some(seg) = uri.path_segments().and_then(|s| s.last()) else { return };
    let name = seg.to_ascii_lowercase();
    let is_kustomization = matches!(
        name.as_str(),
        "kustomization.yaml" | "kustomization.yml" | "kustomization"
    );
    if !is_kustomization { return; }
    if parts.len() != 1 { return; }
    let part = &mut parts[0];
    if part.api_version.is_none() && part.kind.is_none() {
        part.api_version = Some("kustomize.config.k8s.io/v1beta1".into());
        part.kind = Some("Kustomization".into());
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub documents: Arc<HashMap<Url, Arc<Document>>>,
}

impl Snapshot {
    /// Index of named resources across all open documents, keyed by `kind`.
    /// Used by completion to suggest sibling-manifest names.
    pub fn refs_by_kind(&self) -> BTreeMap<String, Vec<ResourceRef>> {
        let mut out: BTreeMap<String, Vec<ResourceRef>> = BTreeMap::new();
        for (uri, doc) in self.documents.iter() {
            for part in &doc.parts {
                let (Some(kind), Some(name)) = (part.kind.as_deref(), part.name.as_deref()) else {
                    continue;
                };
                out.entry(kind.to_string()).or_default().push(ResourceRef {
                    uri: uri.clone(),
                    namespace: part.namespace.clone(),
                    name: name.to_string(),
                });
            }
        }
        for v in out.values_mut() {
            v.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)));
            v.dedup();
        }
        out
    }
}

#[derive(Default)]
pub struct DocumentStore {
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    revision: u64,
    documents: HashMap<Url, Arc<Document>>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&self, uri: Url, version: i32, text: String) {
        let doc = Arc::new(Document::new(uri.clone(), version, text));
        let mut g = self.inner.lock().unwrap();
        g.revision = g.revision.wrapping_add(1);
        g.documents.insert(uri, doc);
    }

    pub fn remove(&self, uri: &Url) {
        let mut g = self.inner.lock().unwrap();
        g.revision = g.revision.wrapping_add(1);
        g.documents.remove(uri);
    }

    pub fn snapshot(&self) -> Snapshot {
        let g = self.inner.lock().unwrap();
        Snapshot {
            revision: g.revision,
            documents: Arc::new(g.documents.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_snapshot() {
        let s = DocumentStore::new();
        let uri = Url::parse("file:///tmp/a.yaml").unwrap();
        s.upsert(uri.clone(), 1, "kind: ConfigMap\nmetadata:\n  name: foo\n".into());
        let snap = s.snapshot();
        assert_eq!(snap.documents.len(), 1);
        let doc = snap.documents.get(&uri).unwrap();
        assert_eq!(doc.parts.len(), 1);
        assert_eq!(doc.parts[0].kind.as_deref(), Some("ConfigMap"));
    }

    #[test]
    fn refs_by_kind_collects_across_documents() {
        let s = DocumentStore::new();
        let a = Url::parse("file:///tmp/a.yaml").unwrap();
        let b = Url::parse("file:///tmp/b.yaml").unwrap();
        s.upsert(a, 1, "kind: ConfigMap\nmetadata:\n  name: cm-a\n".into());
        s.upsert(
            b,
            1,
            "kind: ConfigMap\nmetadata:\n  name: cm-b\n  namespace: ns1\n---\nkind: ServiceAccount\nmetadata:\n  name: sa-1\n".into(),
        );
        let refs = s.snapshot().refs_by_kind();
        let cms = refs.get("ConfigMap").unwrap();
        assert_eq!(cms.len(), 2);
        assert!(cms.iter().any(|r| r.name == "cm-a" && r.namespace.is_none()));
        assert!(cms.iter().any(|r| r.name == "cm-b" && r.namespace.as_deref() == Some("ns1")));
        let sas = refs.get("ServiceAccount").unwrap();
        assert_eq!(sas.len(), 1);
        assert_eq!(sas[0].name, "sa-1");
    }

    #[test]
    fn revision_advances() {
        let s = DocumentStore::new();
        let uri = Url::parse("file:///tmp/a.yaml").unwrap();
        s.upsert(uri.clone(), 1, "kind: A".into());
        let r1 = s.snapshot().revision;
        s.upsert(uri, 2, "kind: B".into());
        let r2 = s.snapshot().revision;
        assert_ne!(r1, r2);
    }
}
