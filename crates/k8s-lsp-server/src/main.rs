use std::sync::Arc;

use k8s_lsp_core::{Document, DocumentStore};
use k8s_lsp_parser::{path_at, position_to_offset, DocumentPart};
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
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> RpcResult<InitializeResult> {
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
                    commands: vec!["k8s-lsp.dumpSnapshot".to_string()],
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
        let (Some(av), Some(kind)) = (part.api_version.as_deref(), part.kind.as_deref()) else {
            return Ok(None);
        };
        let Some(schema) = self.schemas.lookup(av, kind) else { return Ok(None) };

        let part_text = &doc.text[part.byte_range.clone()];
        let rel_offset = abs_offset.saturating_sub(part.byte_range.start);
        let path = path_at(part_text, rel_offset);
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
    let (service, socket) = LspService::new(|client| Backend {
        client,
        store: store.clone(),
        schemas: schemas.clone(),
    });
    Server::new(io::stdin(), io::stdout(), socket)
        .serve(service)
        .await;
}
