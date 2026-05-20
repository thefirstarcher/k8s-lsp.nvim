# k8s-lsp

Kubernetes Language Server — Rust, local-only build.

Hover (`kubectl explain`-style), schema-driven completion + diagnostics,
cross-document name refs, and opt-in live-cluster completion via kube-rs.

Local-only; not published to any registry. Install path:

```
cargo build --release
install -m 0755 target/release/k8s-lsp ~/.local/bin/
```

See plan: `~/.config/claude-wing/plans/analyse-enterprise-level-lsp-transient-lagoon.md`.
