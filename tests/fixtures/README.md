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
- `source`: the text with directives and queries removed.

`tools/update_filecheck.py [globs]` rewrites the checks of failing fixtures from
their actual output, in place. Globs are relative to this directory and a
directory means every `.isle` below it. `-n` prints the diffs without writing.
It sets `FIXTURES_ACTUAL_DIR` so the harness saves each mismatching fixture's
output, re-runs every file it blessed, and restores the original if the new
checks do not pass. Failures that are not check mismatches are reported, not
blessed.
