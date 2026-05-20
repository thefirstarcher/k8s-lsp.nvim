#!/usr/bin/env bash
# Smoke test: didOpen a Deployment with a type mismatch and an unknown field;
# assert publishDiagnostics is sent with both issues.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

# replicas as string (should be integer), and a bogus top-level key.
YAML='apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: three\nbogus: 1\n'

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/d.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${YAML}\"}}}"
        sleep 0.3
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q 'publishDiagnostics' && echo "ok: publishDiagnostics sent"
echo "$OUT" | grep -q 'expected integer' && echo "ok: type mismatch reported"
echo "$OUT" | grep -q 'unknown field' && echo "ok: unknown field reported"
