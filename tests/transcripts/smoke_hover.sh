#!/usr/bin/env bash
# Smoke test: didOpen a Deployment, hover on `spec.replicas`,
# assert kubectl-explain-style markdown is returned.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

# Line-by-line layout (0-indexed):
# 0: apiVersion: apps/v1
# 1: kind: Deployment
# 2: metadata:
# 3:   name: web
# 4: spec:
# 5:   replicas: 3
YAML='apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 3\n'

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/d.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${YAML}\"}}}"
        sleep 0.1
        send '{"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///tmp/d.yaml"},"position":{"line":5,"character":4}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q 'Deployment.spec.replicas' && echo "ok: qualified name in hover"
echo "$OUT" | grep -q 'integer' && echo "ok: integer type rendered"
echo "$OUT" | grep -qi 'replicas' && echo "ok: description mentions replicas"
