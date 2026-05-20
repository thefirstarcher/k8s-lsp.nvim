#!/usr/bin/env bash
# Smoke test: completion of Deployment.spec fields at indent under `spec:`.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

# Cursor at line 5 (0-indexed), column 2 — under `spec:` after two spaces of indent.
# 0: apiVersion: apps/v1
# 1: kind: Deployment
# 2: metadata:
# 3:   name: web
# 4: spec:
# 5: ..  (two spaces, partial completion location)
YAML='apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  '

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/d.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${YAML}\"}}}"
        sleep 0.1
        send '{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///tmp/d.yaml"},"position":{"line":5,"character":2}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q '"label":"replicas"' && echo "ok: replicas field suggested"
echo "$OUT" | grep -q '"label":"selector"' && echo "ok: selector field suggested"
echo "$OUT" | grep -q '"label":"template"' && echo "ok: template field suggested"
echo "$OUT" | grep -q '(required)' && echo "ok: required marker present"
