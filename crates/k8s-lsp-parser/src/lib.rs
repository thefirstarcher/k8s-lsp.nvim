//! Parser: split multi-doc YAML and extract resource identity per part.
//!
//! tree-sitter integration is deferred until Phase 3 (hover/completion need
//! byte→path lookups). For now we split on document separators and parse
//! each chunk with serde_yaml.

use std::ops::Range;

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
/// Documents that fail to parse are skipped (mid-typing tolerance).
pub fn parse(text: &str) -> Vec<DocumentPart> {
    let mut out = Vec::new();
    for range in split_documents(text) {
        let chunk = &text[range.clone()];
        let value: serde_yaml::Value = match serde_yaml::from_str(chunk) {
            Ok(v) => v,
            Err(_) => continue,
        };
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
    out
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
}
