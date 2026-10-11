# Fixtures

Each `.isle` file is a fixture. `cargo test --test fixtures [substring]` runs
every fixture whose path contains the substring.

A fixture picks what to run with `; mode: a, b` lines, then checks the output
with `; check:`, `; nextln:`, `; sameln:`, `; not:` and `; unordered:` lines (the
`filecheck` crate). These are ordinary `;` comments to ISLE. The harness prints
`=== mode ===` followed by that mode's output for each mode, in order, and
checks the directives against the whole text.

Lines of the form `;  ^ kind args` are queries. The caret column is the
column in the nearest non-query line above it, and the kind and args go to the
mode. Query lines and directive lines are removed from the text a mode
analyses, so positions in the output count from the first line of real source.

Modes live in `src/modes.rs`.

- `document`: `at` queries print the UTF-16 column and the round-trip byte
  column for a position. `edit L:C-L:C text` queries apply incremental edits in
  order (`\n` in `text` is a newline) and print the final text.
- `tokens`: one `L:C Kind "text"` line per non-whitespace token, then lexer
  errors as `error L:C..L:C message`.
- `syntax-tree`: one `form <keyword> @range closed|unclosed` line per top-level
  form with its nodes indented below, then syntax errors and the comment count.
  Fixtures under `broken/` exercise error recovery.
- `project`: discovers the project of the fixture file and prints `origin:`, the
  `set:` name, the `files:` in order, any `missing:` files and `note:`s, then
  every `decl` name with `file:line` across the whole set (line numbers ignore
  directive and query lines). Paths are relative to `tests/fixtures`, which is
  also the boundary for the search. A query `open <path> <text>` makes `text`
  an open buffer for `path` (`\n` is a newline), overriding the file on disk.
  Fixtures live in directory trees under `project/`; files without a `mode:` line
  are just members of the tree.
- `diagnostics`: analyses the whole project of the fixture file, with `open`
  queries as in `project`, and prints one line per diagnostic as
  `path L:C..L:C severity[code] message`, plus indented `related` lines. Lines
  ignore directive and query lines and columns are UTF-16. Because sibling
  files form a project, each diagnostics fixture lives in its own directory
  under `diagnostics/`.
- `resolve`: builds the symbol index over the whole project and, for each
  `resolve` query, prints what the cursor is on as `kind name @ path L:C`
  (the definition) followed by one indented `role path L:C` line per reference
  to it. Roles are `type`, `call` (constructor use in an expression), `match`
  (extractor use in a pattern), `rule-head`, `impl` (`extern` and `extractor`
  forms), `attr`, `const` and `variable`. Cursors on nothing print `unresolved`.
  Each fixture lives in its own directory, as for `diagnostics`.
- `navigate`: runs navigation queries against the same index. Each query
  echoes as `kind L:C [args]` and is followed by indented `path L:C..L:C`
  results, or `none`. Kinds are `definition`, `references` (with the
  declaration), `uses` (without it), `highlight` (current file only),
  `outline` (document symbol tree), `symbols <query>` (workspace search),
  `folding` (ranges), `semantic` (one token per line as
  `range type modifiers text`), `inlay` (every hint in the file as `path L:C : Type`), `hover` (range then the markdown, each line prefixed with `|`), `complete`
  (one `label Kind detail` line per item), `signature` (label, parameters and
  active index), `prepare` (prepare-rename range) and `rename <new name>`, which lists
  `range => "text"` edits or `error: message`. `view <name> [term]` renders an
  `isle/view` view (`trie`, `rust` or `graph`) for the term under the cursor or
  the given one and prints every section: text and tree lines, tables as
  ` | `-separated rows, and `(html page)` for a page. `page <name> [term]` is
  the same but prints the HTML lines, for checking markup. The views compile
  the project from disk, so line numbers in them are lines of the file
  including directive lines; put the directives at the end of the fixture to
  keep them equal to the source lines. Each fixture lives in its own directory.
- `source`: the text with directives and queries removed.

Every fixture also runs through the invariants in `tests/harness/invariants.rs`:
tokens tile the input with no gaps, every span lies on character boundaries, and
the parser must not panic on any prefix of the file.

`tools/update_filecheck.py [globs]` rewrites the checks of failing fixtures from
their actual output, in place. Globs are relative to this directory and a
directory means every `.isle` below it. `-n` prints the diffs without writing.
It sets `FIXTURES_ACTUAL_DIR` so the harness saves each mismatching fixture's
output, re-runs every file it blessed, and restores the original if the new
checks do not pass. Failures that are not check mismatches are reported, not
blessed.
