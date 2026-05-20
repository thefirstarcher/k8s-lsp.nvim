//! Parser: split multi-doc YAML and extract resource identity per part.
//!
//! tree-sitter integration is deferred until Phase 3 (hover/completion need
//! byte→path lookups). For now we split on document separators and parse
//! each chunk with serde_yaml.

use std::ops::Range;

pub use k8s_lsp_schema::PathSeg;

#[derive(Debug, Clone)]
pub struct DocumentPart {
    pub byte_range: Range<usize>,
    pub api_version: Option<String>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub value: serde_yaml::Value,
}

/// Split a YAML stream into byte ranges, one per document.
///
/// Honors `---` (start-of-document) markers per the YAML spec: a separator
/// is a line containing only `---`, optionally with a trailing comment.
/// `...` (end-of-document) is treated as a terminator for the current doc.
fn split_documents(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0;

    let mut line_start = 0;
    while i <= bytes.len() {
        let at_eof = i == bytes.len();
        let at_nl = !at_eof && bytes[i] == b'\n';
        if at_eof || at_nl {
            let line = &text[line_start..i];
            let trimmed = line.trim_end();
            let is_sep = trimmed == "---" || trimmed.starts_with("--- ") || trimmed.starts_with("---\t");
            let is_term = trimmed == "..." || trimmed.starts_with("... ") || trimmed.starts_with("...\t");
            if is_sep {
                if line_start > start {
                    ranges.push(start..line_start);
                }
                start = i + usize::from(at_nl);
            } else if is_term && at_eof {
                ranges.push(start..i);
                start = i;
            }
            if at_eof {
                if start < bytes.len() {
                    ranges.push(start..bytes.len());
                }
                break;
            }
            line_start = i + 1;
        }
        i += 1;
    }

    ranges
        .into_iter()
        .filter(|r| !text[r.clone()].trim().is_empty())
        .collect()
}

fn as_str<'a>(v: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

/// Parse a YAML stream into one `DocumentPart` per document.
/// Documents that fail to parse fall back to a line-based identity scan so
/// completion/hover still work mid-typing.
pub fn parse(text: &str) -> Vec<DocumentPart> {
    let mut out = Vec::new();
    for range in split_documents(text) {
        let chunk = &text[range.clone()];
        match serde_yaml::from_str::<serde_yaml::Value>(chunk) {
            Ok(value) => {
                let metadata = value.get("metadata");
                out.push(DocumentPart {
                    byte_range: range,
                    api_version: as_str(&value, "apiVersion").map(str::to_string),
                    kind: as_str(&value, "kind").map(str::to_string),
                    name: metadata.and_then(|m| as_str(m, "name").map(str::to_string)),
                    namespace: metadata.and_then(|m| as_str(m, "namespace").map(str::to_string)),
                    value,
                });
            }
            Err(_) => {
                let (api_version, kind, name, namespace) = scan_identity(chunk);
                if api_version.is_none() && kind.is_none() {
                    continue;
                }
                out.push(DocumentPart {
                    byte_range: range,
                    api_version,
                    kind,
                    name,
                    namespace,
                    value: serde_yaml::Value::Null,
                });
            }
        }
    }
    out
}

/// Best-effort line-based scan for `apiVersion`, `kind`, and `metadata.{name,namespace}`.
/// Used when the chunk is unparseable mid-typing.
fn scan_identity(chunk: &str) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let mut api_version = None;
    let mut kind = None;
    let mut name = None;
    let mut namespace = None;
    let mut in_metadata = false;
    for line in chunk.split('\n') {
        let indent = line.bytes().take_while(|b| *b == b' ').count();
        let rest = &line[indent..];
        if rest.is_empty() || rest.starts_with('#') {
            continue;
        }
        if indent == 0 {
            in_metadata = false;
            if let Some(v) = strip_kv(rest, "apiVersion") {
                api_version = Some(v);
            } else if let Some(v) = strip_kv(rest, "kind") {
                kind = Some(v);
            } else if rest.starts_with("metadata:") {
                in_metadata = true;
            }
        } else if in_metadata && indent >= 2 {
            if let Some(v) = strip_kv(rest, "name") {
                name = Some(v);
            } else if let Some(v) = strip_kv(rest, "namespace") {
                namespace = Some(v);
            }
        }
    }
    (api_version, kind, name, namespace)
}

fn strip_kv(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.strip_prefix(':')?;
    let trimmed = rest.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let val = trimmed.split('#').next()?.trim();
    let val = val.trim_matches('"').trim_matches('\'');
    if val.is_empty() { None } else { Some(val.to_string()) }
}

/// Convert an LSP `(line, character)` (UTF-8 byte index within the line, for now)
/// to a byte offset within `text`.
///
/// LSP technically specifies UTF-16 code units for `character`; we accept that
/// limitation in v0.1 since k8s manifests are overwhelmingly ASCII.
pub fn position_to_offset(text: &str, line: u32, character: u32) -> usize {
    let mut offset = 0usize;
    for (i, line_text) in text.split_inclusive('\n').enumerate() {
        if i as u32 == line {
            let chars_in = (character as usize).min(line_text.trim_end_matches('\n').len());
            return offset + chars_in;
        }
        offset += line_text.len();
    }
    text.len()
}

/// Walk a YAML document's text to determine the path to the node at `offset`.
///
/// Uses indentation/column heuristics — sufficient for hover/completion in v0.1.
/// Returns the path from the document root down to (and including) the key on
/// the target line. If the target line has no `key:`, returns the parent path.
///
/// On the cursor's own line we first pop the stack to the cursor's intended
/// indent (`min(cursor column, leading non-space column)`) before processing
/// it. This stops stale sibling keys from upper lines from polluting the path
/// when the user is on a blank or partially-typed line.
pub fn path_at(text: &str, offset: usize) -> Vec<PathSeg> {
    let scan_end = line_end(text, offset);
    let cursor_line_start = line_start(text, offset);
    let cursor_col = offset.min(text.len()).saturating_sub(cursor_line_start);
    let mut stack: Vec<(usize, PathSeg)> = Vec::new();
    let mut pos = 0usize;
    for line in text[..scan_end].split('\n') {
        if pos == cursor_line_start {
            pop_to(&mut stack, intended_indent(line, cursor_col));
        }
        process_line(line, &mut stack);
        pos += line.len() + 1;
    }
    stack.into_iter().map(|(_, s)| s).collect()
}

fn line_end(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text[offset..].find('\n').map(|n| offset + n).unwrap_or(text.len())
}

fn line_start(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    text[..offset].rfind('\n').map(|n| n + 1).unwrap_or(0)
}

fn intended_indent(line: &str, cursor_col: usize) -> usize {
    let first_nonspace = line
        .bytes()
        .position(|b| b != b' ')
        .unwrap_or(usize::MAX);
    first_nonspace.min(cursor_col)
}

fn process_line(line: &str, stack: &mut Vec<(usize, PathSeg)>) {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    let rest = &line[indent..];
    if rest.is_empty() || rest.starts_with('#') {
        return;
    }

    let mut col = indent;
    let mut s = rest;

    let is_seq_marker = s == "-" || s.starts_with("- ") || s.starts_with("-\t");
    if is_seq_marker {
        pop_to(stack, col);
        stack.push((col, PathSeg::Index));
        if s == "-" {
            return;
        }
        let after = &s[1..];
        let ws = after.bytes().take_while(|b| *b == b' ' || *b == b'\t').count();
        col += 1 + ws;
        s = &after[ws..];
        if s.is_empty() || s.starts_with('#') {
            return;
        }
    }

    if let Some(key) = parse_key(s) {
        pop_to(stack, col);
        stack.push((col, PathSeg::Key(key)));
    }
}

fn pop_to(stack: &mut Vec<(usize, PathSeg)>, col: usize) {
    while stack.last().map_or(false, |(c, _)| *c >= col) {
        stack.pop();
    }
}

fn parse_key(s: &str) -> Option<String> {
    let end = s
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'))
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let rest = &s[end..];
    if !rest.starts_with(':') {
        return None;
    }
    let after_colon = &rest[1..];
    if after_colon.is_empty() || matches!(after_colon.as_bytes()[0], b' ' | b'\t' | b'\n' | b'\r') {
        Some(s[..end].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_doc() {
        let text = "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: foo\n";
        let parts = parse(text);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].api_version.as_deref(), Some("v1"));
        assert_eq!(parts[0].kind.as_deref(), Some("ConfigMap"));
        assert_eq!(parts[0].name.as_deref(), Some("foo"));
    }

    #[test]
    fn multi_doc_split() {
        let text = "kind: A\nmetadata:\n  name: a\n---\nkind: B\nmetadata:\n  name: b\n";
        let parts = parse(text);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind.as_deref(), Some("A"));
        assert_eq!(parts[1].kind.as_deref(), Some("B"));
        // Byte ranges must be non-overlapping and within bounds.
        assert!(parts[0].byte_range.end <= parts[1].byte_range.start);
        assert!(parts[1].byte_range.end <= text.len());
    }

    #[test]
    fn leading_separator() {
        let text = "---\nkind: A\nmetadata:\n  name: a\n";
        let parts = parse(text);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].kind.as_deref(), Some("A"));
    }

    #[test]
    fn broken_yaml_skipped() {
        let text = "kind: A\nmetadata:\n  name: a\n---\n: : :\n---\nkind: B\nmetadata:\n  name: b\n";
        let parts = parse(text);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].kind.as_deref(), Some("A"));
        assert_eq!(parts[1].kind.as_deref(), Some("B"));
    }

    #[test]
    fn namespace_extracted() {
        let text = "kind: Pod\nmetadata:\n  name: p\n  namespace: ns1\n";
        let parts = parse(text);
        assert_eq!(parts[0].namespace.as_deref(), Some("ns1"));
    }

    #[test]
    fn path_at_top_level_key() {
        let text = "apiVersion: apps/v1\nkind: Deployment\n";
        let off = position_to_offset(text, 1, 0); // on "kind:"
        let path = path_at(text, off);
        assert_eq!(path, vec![PathSeg::Key("kind".into())]);
    }

    #[test]
    fn path_at_nested_mapping() {
        let text = "spec:\n  replicas: 3\n";
        let off = position_to_offset(text, 1, 2); // on "replicas"
        let path = path_at(text, off);
        assert_eq!(
            path,
            vec![PathSeg::Key("spec".into()), PathSeg::Key("replicas".into())]
        );
    }

    #[test]
    fn path_at_sequence_item_field() {
        let text = "spec:\n  containers:\n    - name: foo\n      image: bar\n";
        let off = position_to_offset(text, 3, 6); // on "image"
        let path = path_at(text, off);
        assert_eq!(
            path,
            vec![
                PathSeg::Key("spec".into()),
                PathSeg::Key("containers".into()),
                PathSeg::Index,
                PathSeg::Key("image".into()),
            ]
        );
    }

    #[test]
    fn path_at_second_sequence_item() {
        let text = "spec:\n  containers:\n    - name: foo\n    - name: bar\n";
        let off = position_to_offset(text, 3, 6); // on second "name"
        let path = path_at(text, off);
        assert_eq!(
            path,
            vec![
                PathSeg::Key("spec".into()),
                PathSeg::Key("containers".into()),
                PathSeg::Index,
                PathSeg::Key("name".into()),
            ]
        );
    }

    #[test]
    fn path_at_partial_line_returns_parent() {
        let text = "spec:\n  repl"; // user typing
        let off = text.len();
        let path = path_at(text, off);
        assert_eq!(path, vec![PathSeg::Key("spec".into())]);
    }
}
