use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use k8s_lsp_parser::{parse, DocumentPart};
use url::Url;

#[derive(Debug, Clone)]
pub struct Document {
    pub uri: Url,
    pub version: i32,
    pub text: String,
    pub parts: Vec<DocumentPart>,
}

impl Document {
    pub fn new(uri: Url, version: i32, text: String) -> Self {
        let parts = parse(&text);
        Self { uri, version, text, parts }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub documents: Arc<HashMap<Url, Arc<Document>>>,
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
