#!/usr/bin/env bash
# Smoke test: oversized documents are rejected with a single diagnostic and
# never reach the parser. We send a didOpen carrying ~11 MiB of YAML; the
# server should reply with a publishDiagnostics whose message references the
# size limit. Driver written in Python because bash parameter expansion on
# multi-MiB strings is unusably slow.
set -euo pipefail

BIN="${1:-$(dirname "$0")/../../target/debug/k8s-lsp}"

OUT="$(python3 - "$BIN" <<'PY'
import json, subprocess, sys, time

bin_path = sys.argv[1]
proc = subprocess.Popen(
    [bin_path],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
)

def send(msg):
    body = json.dumps(msg).encode("utf-8")
    proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body))
    proc.stdin.write(body)
    proc.stdin.flush()

big = "k: v\n" * (11 * 1024 * 1024 // 5)  # ~11 MiB

send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"capabilities": {}}})
send({"jsonrpc": "2.0", "method": "initialized", "params": {}})
send({
    "jsonrpc": "2.0",
    "method": "textDocument/didOpen",
    "params": {"textDocument": {
        "uri": "file:///tmp/big.yaml",
        "languageId": "yaml",
        "version": 1,
        "text": big,
    }},
})
time.sleep(0.5)
send({"jsonrpc": "2.0", "id": 2, "method": "shutdown"})
send({"jsonrpc": "2.0", "method": "exit"})
proc.stdin.close()
out, _ = proc.communicate(timeout=10)
sys.stdout.write(out.decode("utf-8", errors="replace"))
PY
)"

echo "$OUT" | head -c 2000
echo
echo "---"
echo "$OUT" | grep -q 'k8s-lsp skips files larger than' && echo "ok: oversize diagnostic emitted"
echo "$OUT" | grep -q '"severity":1' && echo "ok: severity ERROR"
