# isle-lsp

A language server for
[ISLE](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift/isle),
Cranelift's instruction selection and rewrite DSL. It is aimed at people writing
optimization rules: diagnostics, navigation, and completion for `.isle` files.

This is early work. The server starts and tracks open documents; the features
are tracked as issues in `.beads/` and listed in [docs/features.md](docs/features.md).

## Install

You need a Rust toolchain. From a checkout:

```sh
cargo install --path .
```

This puts an `isle-lsp` binary on your `PATH`. It speaks LSP over stdio and
takes no arguments.

## Built against

`cranelift-isle` 0.136.2. Rule semantics follow that version.

## License

MIT or Apache-2.0, at your option.
