# Features

This lists everything the server does. Positions and ranges follow LSP.

## Document sync

Incremental text sync. The server keeps each open document in a rope and
converts LSP UTF-16 positions to byte offsets. Every edit re-parses the
document with a tolerant parser that recovers from unbalanced parentheses, so
one error does not hide the rest of the file.

## Diagnostics

Diagnostics update on every edit and cover every file of the project, open or
not. They come in layers, and a later layer only runs when the earlier one is
clean:

1. **Syntax errors** (`syntax`): unclosed or unmatched parentheses, bare tokens
   at the top level, unknown form names, empty forms, bad integer literals, and
   unterminated block comments. The parser recovers at the next line-start
   `(decl`, `(rule` and so on, so one missing parenthesis does not hide the rest
   of the file.
2. **Semantic checks** from `cranelift-isle` 0.136.2, run over the whole project
   with the same code the Cranelift build uses:
   - `parse`: what the upstream parser rejects, such as a rule with no
     right-hand side.
   - `type`: unknown types and terms, wrong argument counts, type mismatches,
     terms used on the wrong side of a rule.
   - `overlap`: two rules that match the same input at the same priority. Every
     rule involved gets a diagnostic covering the whole rule, with the other
     rules attached as related locations, including rules in other files. The
     message names a priority that would resolve it, and a quick fix inserts or
     raises the priority of that rule.
   - `shadowed` and `unreachable`: a rule that can never match.
   - `recursion`: a term that recurses without the `rec` attribute.

If a file the project lists is missing, or the project is a Cranelift mid-end
set without its generated `numerics.isle` and `clif_opt.isle`, semantic checks
are skipped and an `incomplete-project` warning explains why. Checking without
the generated files would report every CLIF term as unknown.

## Navigation

Go to definition, find references, document highlight and rename work on terms,
types, enum variants, constants, macros, named rules and pattern or `let`
variables, across every file of the project. Variables follow ISLE scoping: a
repeated name in a pattern is the same binding, and a `let` binding shadows
earlier ones in its body only.

Terms keep their roles apart in the index: a call in a right-hand side, a match
in a pattern (an extractor use), a rule head, and an `extern` or `extractor`
implementation all resolve to the term's `decl`.

Rename validates the new name. It rejects names that are not valid symbols,
`$`-prefixed names for anything but constants, a variable named `_`, and names
that clash with an existing definition, including one of a different kind.
Renaming a type also rewrites the `Type.` prefix of its variants, and renaming
a variant keeps the type prefix.

## Hover

Hovering a term shows its `decl`, the `;;` doc comment above it, what
implements it (internal constructor from rules, external constructor or
extractor from `extern`, internal extractor from `extractor`) and how many rules
define it, patterns match on it and expressions call it. Types, variants,
constants and macros show their definition. Variables show their inferred type
(from the enclosing term's `decl`, a `let` annotation, or the right-hand side
of an `if-let`), the line they are bound on and their use count.

## Completion and signature help

Completion depends on where the cursor is:

- Top level, after `(`: form keywords.
- Head of a rule's left-hand side: every declared term.
- Head of a nested pattern: only terms that can be matched (extractors, enum
  variants, macros), never constructor-only terms.
- Head of a right-hand side or `if-let` expression: terms that can be called,
  with a snippet tab stop per argument.
- Argument of a call: variables in scope in the current form and constants.
- Type positions in `decl` and `let` bindings: types.

Where the enclosing term's `decl` gives an expected type, terms and values of
other types are left out. Signature help shows `(term ArgType...) -> Ret` for
the innermost call and highlights the current argument.

## Inlay hints

Variables bound in patterns and `if-let` clauses get a `: Type` hint after the
name, using the same type inference as hover. `let` bindings already spell out
their type, so they get no hint.

## Symbols, folding and semantic tokens

- **Document symbols** give an outline: types with their variants, constants,
  macros and declared terms. Each rule is nested under the term it defines,
  named by its rule name or its left-hand side and annotated with its priority.
  Rules whose term is declared in another file are grouped under that term's
  name.
- **Workspace symbols** search types, variants, terms, constants, macros and
  named rules across the project by case-insensitive subsequence.
- **Folding** covers every multi-line top-level form and block comment.
- **Semantic tokens** classify form keywords and `decl` flags as keywords,
  terms as functions, types, enum variants, constants (readonly variables),
  macros and variables (binding sites carry the `declaration` modifier), and
  integers, with the `priority` modifier on rule priorities.

## Code actions

- **Overlap**: on an `overlap` diagnostic, give the rule a priority (insert
  `1`, or raise an existing priority by one).
- **Unknown term**: on an `Unknown term` error, replace it with the closest
  declared term, or insert a `(decl ...)` for it above the rule, with one type
  placeholder per argument.
- **Unused declaration**: with the cursor in a `decl` that nothing references
  (no rule, call, match or `extern`), remove it.

## Custom requests

- **`isle/rulesForTerm`** takes `{ textDocument, position }` with the cursor on
  a term and returns every rule whose head is that term, highest priority
  first. Each entry has `location`, `priority` and `winsOver`, the locations of
  lower-priority rules whose patterns can overlap it. Overlap is judged by
  comparing pattern shapes, so it is a heuristic.
- **`isle/rulesForClif`** takes `{ textDocument, snippet }` where `snippet` is
  CLIF text such as `v1 = iconst.i32 0` then `v2 = iadd v0, v1`, and returns the
  rules whose left-hand side could match the instruction that produces the
  returned (or last) value, in the same shape. Operands defined in the snippet
  are matched structurally; other values match anything.
- A **code lens** above each `decl` shows how many rules define the term and
  runs the client command `isle-lsp.rulesForTerm` with the document URI and the
  position of the name, for clients that want to forward it to the request
  above.

## Project files

ISLE files only make sense together, so the server groups them into a project
when a document opens. It looks in this order:

1. **`isle-lsp.toml`**, found by walking up from the document. Each `[[set]]`
   lists the files that are compiled together. The first set that lists the
   document is its project. A path may use `*` in its last component, and
   relative paths are relative to the config file.

   ```toml
   [[set]]
   name = "opt"
   files = [
     "src/prelude.isle",
     "src/prelude_opt.isle",
     "src/opts/*.isle",
     "target/debug/build/cranelift-codegen/HASH/out/numerics.isle",
     "target/debug/build/cranelift-codegen/HASH/out/clif_opt.isle",
   ]
   ```

   If the nearest config does not list the document, the document is its own
   project. A config that does not parse is ignored.
2. **Cranelift's mid-end layout**: a document in `src/opts/` next to
   `prelude.isle` and `prelude_opt.isle`. The project is those two preludes,
   every file in `opts/`, and the generated `numerics.isle` and `clif_opt.isle`
   from the newest `cranelift-codegen` build output under an enclosing
   `target/`. Without a build, they are generated by the pinned
   `cranelift-codegen-meta` 0.136.2 into the per-user cache directory and
   reused from there.
   The lowering layout works the same way for a document in `src/isa/<arch>/`
   next to `prelude.isle` and `prelude_lower.isle`: the project is the two
   preludes, the architecture's `.isle` files (`inst*` first), and the
   generated `numerics.isle` and `clif_lower.isle`.
3. **Siblings**: every `.isle` file in the document's directory.

Open buffers replace the files on disk, so unsaved edits in one file are visible
to the others. A file the project lists but that is missing on disk is
reported, not fatal.
