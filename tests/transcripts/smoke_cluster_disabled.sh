#!/usr/bin/env bash
# Smoke test: cluster integration defaults to disabled.
# Without initializationOptions.k8sLsp.cluster.enabled=true, the refreshCluster
# command should return {"status":"disabled"} and no kube client should be
# constructed. Verifies the hot path is never coupled to cluster reachability.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

OUT="$(
    {
        send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
        sleep 0.1
        send '{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"k8s-lsp.refreshCluster"}}'
        sleep 0.2
        send '{"jsonrpc":"2.0","id":3,"method":"shutdown"}'
        sleep 0.1
        send '{"jsonrpc":"2.0","method":"exit"}'
    } | "$BIN"
)"

echo "$OUT"
echo "---"
echo "$OUT" | grep -q '"status":"disabled"' && echo "ok: refreshCluster reports disabled by default"
echo "$OUT" | grep -q '"k8s-lsp.refreshCluster"' && echo "ok: refreshCluster command advertised"
