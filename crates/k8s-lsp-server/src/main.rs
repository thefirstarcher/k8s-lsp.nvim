use std::sync::Arc;

use k8s_lsp_cluster::{ClusterRef, ClusterState};
use k8s_lsp_core::{Document, DocumentStore, ResourceRef};
use k8s_lsp_parser::{path_at, position_to_offset, DocumentPart, PathSeg};
use k8s_lsp_schema::{
    fields_at, render_hover, schema_at_path, validate, FieldCandidate, SchemaRegistry, Severity,
};
use serde_json::json;
use tokio::io;
use tower_lsp::jsonrpc::{Error as RpcError, Result as RpcResult};
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};
use tracing_subscriber::EnvFilter;

struct Backend {
    client: Client,
    store: Arc<DocumentStore>,
    schemas: Arc<SchemaRegistry>,
    cluster: Arc<ClusterState>,
    cluster_enabled: std::sync::atomic::AtomicBool,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> RpcResult<InitializeResult> {
        let enabled = cluster_enabled_from(&params.initialization_options);
        self.cluster_enabled
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
        if enabled {
            let cluster = self.cluster.clone();
            tokio::spawn(async move {
                if let Err(e) = cluster.refresh().await {
                    tracing::warn!(error = %e, "initial cluster refresh failed");
                }
            });
        }
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "k8s-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![":".into(), " ".into(), "-".into()]),
                    ..Default::default()
                }),
                execute_command_provider: Some(ExecuteCommandOptions {
                    commands: vec![
                        "k8s-lsp.dumpSnapshot".to_string(),
                        "k8s-lsp.refreshCluster".to_string(),
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "k8s-lsp initialized")
            .await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let doc = params.text_document;
        self.store.upsert(doc.uri.clone(), doc.version, doc.text);
        self.publish_diagnostics(doc.uri, Some(doc.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // We advertise FULL sync, so the first content change carries the entire document.
        let Some(change) = params.content_changes.into_iter().next() else { return };
        let uri = params.text_document.uri;
        let version = params.text_document.version;
        self.store.upsert(uri.clone(), version, change.text);
        self.publish_diagnostics(uri, Some(version)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.store.remove(&uri);
        self.client.publish_diagnostics(uri, Vec::new(), None).await;
    }

    async fn completion(&self, params: CompletionParams) -> RpcResult<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        let snap = self.store.snapshot();
        let Some(doc) = snap.documents.get(&uri) else { return Ok(None) };

        let abs_offset = position_to_offset(&doc.text, pos.line, pos.character);
        let Some(part) = part_at(doc, abs_offset) else { return Ok(None) };
        let part_text = &doc.text[part.byte_range.clone()];
        let rel_offset = abs_offset.saturating_sub(part.byte_range.start);
        let path = path_at(part_text, rel_offset);

        // Value-position name-reference completion (cross-doc + cluster).
        if line_is_value_position(&doc.text, pos.line, pos.character) {
            if let Some(target_kind) = ref_kind_for_path(&path) {
                let mut items = Vec::new();
                let refs = snap.refs_by_kind();
                if let Some(list) = refs.get(target_kind) {
                    items.extend(name_ref_items(list, part.namespace.as_deref(), &uri));
                }
                if self
                    .cluster_enabled
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    let cluster_refs = self.cluster.snapshot().await;
                    if let Some(list) = cluster_refs.get(target_kind) {
                        items.extend(cluster_ref_items(list, part.namespace.as_deref()));
                    }
                }
                if !items.is_empty() {
                    return Ok(Some(CompletionResponse::Array(items)));
                }
            }
            return Ok(None);
        }

        // Field-name completion from the part's schema.
        let (Some(av), Some(kind)) = (part.api_version.as_deref(), part.kind.as_deref()) else {
            return Ok(None);
        };
        let Some(schema) = self.schemas.lookup(av, kind) else { return Ok(None) };
        let candidates = fields_at(&schema, &path);
        if candidates.is_empty() {
            return Ok(None);
        }
        let items: Vec<CompletionItem> = candidates.into_iter().map(field_to_item).collect();
        Ok(Some(CompletionResponse::Array(items)))
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        let snap = self.store.snapshot();
        let Some(doc) = snap.documents.get(&uri) else { return Ok(None) };

        let abs_offset = position_to_offset(&doc.text, pos.line, pos.character);
        let Some(part) = part_at(doc, abs_offset) else { return Ok(None) };
        let (Some(av), Some(kind)) = (part.api_version.as_deref(), part.kind.as_deref()) else {
            return Ok(None);
        };
        let Some(schema) = self.schemas.lookup(av, kind) else { return Ok(None) };

        let part_text = &doc.text[part.byte_range.clone()];
        let rel_offset = abs_offset.saturating_sub(part.byte_range.start);
        let path = path_at(part_text, rel_offset);
        if path.is_empty() {
            return Ok(None);
        }
        let Some(node) = schema_at_path(&schema, &path) else { return Ok(None) };

        let qualified = std::iter::once(kind.to_string())
            .chain(path.iter().map(|seg| match seg {
                k8s_lsp_parser::PathSeg::Key(k) => k.clone(),
                k8s_lsp_parser::PathSeg::Index => "[]".to_string(),
            }))
            .collect::<Vec<_>>()
            .join(".");
        let md = render_hover(&qualified, node);

        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: md,
            }),
            range: None,
        }))
    }

    async fn execute_command(
        &self,
        params: ExecuteCommandParams,
    ) -> RpcResult<Option<serde_json::Value>> {
        match params.command.as_str() {
            "k8s-lsp.dumpSnapshot" => Ok(Some(dump_snapshot(&self.store))),
            "k8s-lsp.refreshCluster" => {
                if !self
                    .cluster_enabled
                    .load(std::sync::atomic::Ordering::SeqCst)
                {
                    return Ok(Some(json!({ "status": "disabled" })));
                }
                let cluster = self.cluster.clone();
                tokio::spawn(async move {
                    if let Err(e) = cluster.refresh().await {
                        tracing::warn!(error = %e, "manual cluster refresh failed");
                    }
                });
                Ok(Some(json!({ "status": "refreshing" })))
            }
            _ => Err(RpcError::method_not_found()),
        }
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }
}

impl Backend {
    async fn publish_diagnostics(&self, uri: Url, version: Option<i32>) {
        let snap = self.store.snapshot();
        let Some(doc) = snap.documents.get(&uri) else { return };
        let mut diagnostics = Vec::new();
        for part in &doc.parts {
            let (Some(av), Some(kind)) = (part.api_version.as_deref(), part.kind.as_deref())
            else {
                continue;
            };
            let Some(schema) = self.schemas.lookup(av, kind) else { continue };
            let part_line = byte_to_line(&doc.text, part.byte_range.start);
            for issue in validate(&part.value, &schema) {
                let line = locate_path(&doc.text, part, &issue.path).unwrap_or(part_line);
                diagnostics.push(Diagnostic {
                    range: line_range(&doc.text, line),
                    severity: Some(match issue.severity {
                        Severity::Error => DiagnosticSeverity::ERROR,
                        Severity::Warning => DiagnosticSeverity::WARNING,
                    }),
                    source: Some("k8s-lsp".into()),
                    message: format!("{}: {}", issue.path.join("."), issue.message),
                    ..Default::default()
                });
            }
        }
        self.client.publish_diagnostics(uri, diagnostics, version).await;
    }
}

fn part_at(doc: &Document, abs_offset: usize) -> Option<&DocumentPart> {
    doc.parts
        .iter()
        .find(|p| abs_offset >= p.byte_range.start && abs_offset <= p.byte_range.end)
}

fn field_to_item(c: FieldCandidate) -> CompletionItem {
    let mut item = CompletionItem {
        label: c.name.clone(),
        kind: Some(CompletionItemKind::FIELD),
        detail: Some(c.type_label),
        documentation: c.description.map(|d| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: d,
            })
        }),
        insert_text: Some(format!("{}: ", c.name)),
        ..Default::default()
    };
    if c.required {
        item.label_details = Some(CompletionItemLabelDetails {
            detail: Some(" (required)".into()),
            description: None,
        });
        item.sort_text = Some(format!("0_{}", c.name));
    } else {
        item.sort_text = Some(format!("1_{}", c.name));
    }
    item
}

/// True when the cursor on `line` sits after the line's first `:` separator
/// (i.e. typing a value, not a key).
fn line_is_value_position(text: &str, line: u32, character: u32) -> bool {
    let lines: Vec<&str> = text.split('\n').collect();
    let Some(l) = lines.get(line as usize) else { return false };
    let cut = (character as usize).min(l.len());
    let before = &l[..cut];
    if let Some(colon) = before.find(':') {
        matches!(before.as_bytes().get(colon + 1), None | Some(b' ') | Some(b'\t'))
    } else {
        false
    }
}

/// Map a YAML path that ends at a known cross-reference field to the Kind it
/// points at. Returns `None` for non-reference paths.
fn ref_kind_for_path(path: &[PathSeg]) -> Option<&'static str> {
    let last = match path.last()? {
        PathSeg::Key(k) => k.as_str(),
        PathSeg::Index => return None,
    };
    match last {
        "serviceAccountName" => Some("ServiceAccount"),
        "claimName" => Some("PersistentVolumeClaim"),
        "priorityClassName" => Some("PriorityClass"),
        "storageClassName" => Some("StorageClass"),
        "runtimeClassName" => Some("RuntimeClass"),
        "secretName" => Some("Secret"),
        "name" => {
            let parent = path.get(path.len().checked_sub(2)?)?;
            let PathSeg::Key(p) = parent else { return None };
            match p.as_str() {
                "secretRef" | "secretKeyRef" | "secret" => Some("Secret"),
                "configMapRef" | "configMapKeyRef" | "configMap" => Some("ConfigMap"),
                "persistentVolumeClaim" => Some("PersistentVolumeClaim"),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Parse `initializationOptions = { "cluster": { "enabled": true } }`.
fn cluster_enabled_from(opts: &Option<serde_json::Value>) -> bool {
    opts.as_ref()
        .and_then(|v| v.get("cluster"))
        .and_then(|c| c.get("enabled"))
        .and_then(|e| e.as_bool())
        .unwrap_or(false)
}

fn cluster_ref_items(refs: &[ClusterRef], part_ns: Option<&str>) -> Vec<CompletionItem> {
    refs.iter()
        .filter(|r| match (part_ns, r.namespace.as_deref()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        })
        .map(|r| {
            let detail = match &r.namespace {
                Some(ns) => format!("cluster · namespace: {ns}"),
                None => "cluster · cluster-scoped".to_string(),
            };
            CompletionItem {
                label: r.name.clone(),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some(detail),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: "From live cluster".to_string(),
                })),
                sort_text: Some(format!("2_{}", r.name)),
                ..Default::default()
            }
        })
        .collect()
}

fn name_ref_items(refs: &[ResourceRef], part_ns: Option<&str>, self_uri: &Url) -> Vec<CompletionItem> {
    refs.iter()
        .filter(|r| match (part_ns, r.namespace.as_deref()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        })
        .map(|r| {
            let detail = match &r.namespace {
                Some(ns) => format!("namespace: {ns}"),
                None => "cluster-scoped or no namespace".to_string(),
            };
            let file = r.uri.path().rsplit('/').next().unwrap_or("");
            let source = if r.uri == *self_uri { "this file".into() } else { file.to_string() };
            CompletionItem {
                label: r.name.clone(),
                kind: Some(CompletionItemKind::REFERENCE),
                detail: Some(detail),
                documentation: Some(Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: format!("Defined in **{source}**"),
                })),
                ..Default::default()
            }
        })
        .collect()
}

fn byte_to_line(text: &str, byte: usize) -> u32 {
    text[..byte.min(text.len())].bytes().filter(|b| *b == b'\n').count() as u32
}

fn line_range(text: &str, line: u32) -> Range {
    let lines: Vec<&str> = text.split('\n').collect();
    let len = lines.get(line as usize).map(|l| l.len()).unwrap_or(0);
    Range {
        start: Position { line, character: 0 },
        end: Position { line, character: len as u32 },
    }
}

fn locate_path(text: &str, part: &DocumentPart, path: &[String]) -> Option<u32> {
    let last = path.last()?;
    let part_start_line = byte_to_line(text, part.byte_range.start);
    let part_text = &text[part.byte_range.clone()];
    for (i, line) in part_text.split('\n').enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(last.as_str())
            && trimmed[last.len()..].starts_with(':')
        {
            return Some(part_start_line + i as u32);
        }
    }
    None
}

fn dump_snapshot(store: &DocumentStore) -> serde_json::Value {
    let snap = store.snapshot();
    let docs: Vec<_> = snap
        .documents
        .iter()
        .map(|(uri, doc)| {
            let parts: Vec<_> = doc
                .parts
                .iter()
                .map(|p| {
                    json!({
                        "byteRange": [p.byte_range.start, p.byte_range.end],
                        "apiVersion": p.api_version,
                        "kind": p.kind,
                        "name": p.name,
                        "namespace": p.namespace,
                    })
                })
                .collect();
            json!({ "uri": uri.as_str(), "version": doc.version, "parts": parts })
        })
        .collect();
    json!({ "revision": snap.revision, "documents": docs })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("K8S_LSP_LOG").unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let store = Arc::new(DocumentStore::new());
    let schemas = Arc::new(SchemaRegistry::new());
    let cluster = Arc::new(ClusterState::new());
    let (service, socket) = LspService::new(|client| Backend {
        client,
        store: store.clone(),
        schemas: schemas.clone(),
        cluster: cluster.clone(),
        cluster_enabled: std::sync::atomic::AtomicBool::new(false),
    });
    Server::new(io::stdin(), io::stdout(), socket)
        .serve(service)
        .await;
}
