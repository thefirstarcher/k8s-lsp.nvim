#!/usr/bin/env bash
# Smoke test: cross-document name reference completion.
# Open a ServiceAccount in a.yaml; open a Deployment in b.yaml whose template
# has `serviceAccountName: ` with cursor in value position. Completion should
# return the ServiceAccount name.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

A_YAML='apiVersion: v1\nkind: ServiceAccount\nmetadata:\n  name: sa-1\n'

# Deployment with cursor in value position on serviceAccountName.
# Lines (0-indexed):
# 0: apiVersion: apps/v1
# 1: kind: Deployment
# 2: metadata:
# 3:   name: web
# 4: spec:
# 5:   template:
# 6:     spec:
# 7:       serviceAccountName: ..   <-- cursor at char 26 (after "serviceAccountName: ")
B_YAML='apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  template:\n    spec:\n      serviceAccountName: \n'

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/a.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${A_YAML}\"}}}"
        sleep 0.1
        send "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///tmp/b.yaml\",\"languageId\":\"yaml\",\"version\":1,\"text\":\"${B_YAML}\"}}}"
        sleep 0.1
        send '{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///tmp/b.yaml"},"position":{"line":7,"character":27}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q '"label":"sa-1"' && echo "ok: sa-1 suggested"
echo "$OUT" | grep -q '"kind":18' && echo "ok: REFERENCE kind (18) emitted"
echo "$OUT" | grep -q 'Defined in' && echo "ok: source-file documentation present"
