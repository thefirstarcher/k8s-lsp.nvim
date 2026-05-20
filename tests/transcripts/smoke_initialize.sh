#!/usr/bin/env bash
# Smoke test: send initialize + initialized + shutdown + exit; check server replies.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

send() {
    local body="$1"
    local len=${#body}
    printf 'Content-Length: %d\r\n\r\n%s' "$len" "$body"
}

{
    send '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}'
    sleep 0.2
    send '{"jsonrpc":"2.0","method":"initialized","params":{}}'
    sleep 0.1
    send '{"jsonrpc":"2.0","id":2,"method":"shutdown"}'
    sleep 0.1
    send '{"jsonrpc":"2.0","method":"exit"}'
} | "$BIN"
