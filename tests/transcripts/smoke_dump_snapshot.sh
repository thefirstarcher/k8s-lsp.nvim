#!/usr/bin/env bash
# Smoke test: didOpen a multi-doc YAML, then workspace/executeCommand
# k8s-lsp.dumpSnapshot, and assert apiVersion/kind/name appear.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

YAML='apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: foo\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: bar\n  namespace: prod\n'

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/k.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${YAML}\"}}}"
        sleep 0.1
        send '{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"k8s-lsp.dumpSnapshot"}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q '"kind":"ConfigMap"' && echo "ok: ConfigMap part present"
echo "$OUT" | grep -q '"kind":"Deployment"' && echo "ok: Deployment part present"
echo "$OUT" | grep -q '"namespace":"prod"' && echo "ok: namespace extracted"
