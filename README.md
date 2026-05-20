# k8s-lsp

A Kubernetes Language Server for YAML manifests — `kubectl explain`-style
hover, schema-driven completion and diagnostics, cross-document name
references, kustomize support, and opt-in live-cluster + CRD discovery via
kube-rs.

> ⚠️ **Vibecoded.** This was built end-to-end with an AI coding agent. It works
> on my machine and the test fixtures, but it has not been audited, hardened,
> or production-tested. Read the code before you trust it.

## Install

Requires a Rust toolchain (stable, ≥ 1.80).

```sh
git clone https://github.com/<you>/k8s-lsp
cd k8s-lsp
cargo build --release
install -m755 target/release/k8s-lsp ~/.local/bin/k8s-lsp
```

Make sure `~/.local/bin` is on your `PATH`.

## Editor setup (Neovim, `nvim-lspconfig`)

```lua
vim.lsp.config('k8s_lsp', {
  cmd = { 'k8s-lsp' },
  filetypes = { 'yaml' },
  root_markers = { '.git' },
  init_options = {
    -- Optional: enable live-cluster completion via current kubeconfig context.
    cluster = false,
    -- Optional: load CRD schemas from local files (apiextensions.k8s.io/v1).
    crdSchemaPaths = { '~/k8s/crds/cnpg-cluster.yaml' },
  },
})
vim.lsp.enable('k8s_lsp')
```

## Features

- Hover with `kubectl explain`-style markdown for built-in resources and CRDs
- Completion for fields, enum values, and cross-document name references
- Schema validation diagnostics
- Kustomization files (`kustomization.yaml`) recognized by filename
- CRD `metadata.*` falls back to embedded ObjectMeta when the CRD declares
  `metadata` as opaque (the common case)
- Opt-in: list CRDs from the live cluster on startup

## License

[MIT](./LICENSE)
