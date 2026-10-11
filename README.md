# isle-lsp

A language server for
[ISLE](https://github.com/bytecodealliance/wasmtime/tree/main/cranelift/isle),
Cranelift's instruction selection and rewrite DSL. It is aimed at people writing
optimization rules.

It checks `.isle` files the way the Cranelift build does, across every file that
is compiled together, and offers diagnostics (syntax, types, overlap, shadowed
rules, recursion), go to definition, references, rename, hover, completion and
signature help that know whether a term is being matched or called, inlay hints
for pattern types, symbols, folding, semantic tokens and quick fixes. The full
list is in [docs/features.md](docs/features.md). Remaining work is tracked as
issues in `.beads/`.

## In the editor

Recorded in Neovim with
[tree-sitter-isle](https://github.com/Takashiidobe/tree-sitter-isle) highlighting
on top of the server's semantic tokens and inlay hints. Each feature is
described in [docs/features.md](docs/features.md).

### Diagnostics and quick fixes

An unknown term is replaced with the closest declared one, and once the types
check, two overlapping rules get a priority from a quick fix.

![Diagnostics](docs/images/diagnostics.gif)

### Rule lints

Heuristic warnings on `simplify` rules, and a quick fix that generates a CLIF
filetest for a rule.

![Rule lints](docs/images/lints.gif)

### Go to definition, references and rename

![Navigation](docs/images/navigate.gif)

### Hover

Terms show their `decl`, doc comment and rule counts. Variables show their
inferred type, and variants show their definition.

![Hover](docs/images/hover.gif)

### Completion and signature help

Completion offers only what fits the position and the expected type, and the
signature follows the argument being typed.

![Completion](docs/images/completion.gif)

### Symbols

Document symbols nest each rule under its term, and workspace symbols search the
whole project.

![Symbols](docs/images/symbols.gif)

### Call hierarchy

![Call hierarchy](docs/images/calls.gif)

### Code actions

Inline a term, name a rule and remove an unused declaration.

![Code actions](docs/images/actions.gif)

### Rules for a term

`isle/rulesForTerm` lists the rules of a term by priority, with how many rules
each one wins over.

![Rules for a term](docs/images/rules.gif)

## HTML views

Three views render what `cranelift-isle` builds for your rules as standalone
pages, through the `isle/view` request
([details](docs/features.md#html-views), [Neovim commands](docs/neovim.md)).

### Decision trie

Shows the tests the generated matcher runs for a term, in order, and which rule
each leaf returns.

![Decision trie](docs/images/trie.gif)

### Generated Rust

Shows the function `cranelift-isle` generates for a term, with each rule's result
marked.

![Generated Rust](docs/images/rust.gif)

### Term graph

Draws which terms construct which, tints terms in a call cycle and lists the
overlapping rules of one term.

![Term graph](docs/images/graph.gif)

Every recording is rendered by `docs/demos/render.sh <name>` from
`docs/demos/<name>.tape` and the examples in `docs/demos/isle`.

## Install

You need a Rust toolchain (1.96 or later, which `cranelift-isle` requires). From
a checkout:

```sh
cargo install --path .
```

This puts an `isle-lsp` binary on your `PATH`. It speaks LSP over stdio and
takes no arguments.

## Editor setup

See [docs/neovim.md](docs/neovim.md). Any editor that can start a stdio language
server for `.isle` files works.

## Built against

`cranelift-isle` 0.136.2. Rule semantics follow that version.

## License

MIT or Apache-2.0, at your option.
