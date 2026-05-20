#![no_main]
//! Fuzz target: arbitrary bytes through the YAML parser must never panic.
//! Tree-sitter is error-tolerant by design and `path_at` walks string slices —
//! both should handle malformed input gracefully. Run with:
//!   cargo +nightly fuzz run parse_yaml

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let parts = k8s_lsp_parser::parse(text);
    // Exercise path resolution at every byte boundary we touched.
    for p in &parts {
        let mid = (p.byte_range.start + p.byte_range.end) / 2;
        if mid <= text.len() {
            let _ = k8s_lsp_parser::path_at(&text[p.byte_range.clone()], 0);
            let _ = k8s_lsp_parser::path_at(text, mid);
        }
    }
});
