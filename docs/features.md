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

If a file the project lists is missing, or the project is a Cranelift set whose
generated files cannot be found or produced, semantic checks are skipped and an
`incomplete-project` warning explains why. Checking without the generated files
would report every CLIF term as unknown.

Two more groups run alongside the semantic layer on a complete project:

- **Unused declarations** (`unused`, hint, tagged Unnecessary): a `decl` that
  nothing references.
- **Rule lints**, all heuristics and all labelled `heuristic:` in the message.
  They look at `simplify` rules by comparing pattern shapes, not by running
  them, so they can be wrong in both directions:
  - `costlier-rhs`: the right-hand side has more operations than the
    left-hand side. Constants (`iconst...`) and `subsume` cost nothing.
  - `no-op-rewrite`: the right-hand side is the left-hand side.
  - `shadowed-by-priority`: a rule with a higher priority and no `if-let`
    clauses matches everything this rule matches. Related location: that rule.
  - `rewrite-loop`: the result of a rule is matched by its own left-hand side,
    or by the left-hand side of a rule whose result this rule matches in turn.
    Rules with `if-let` clauses are skipped. Related location: the other rule.
  - `missing-commutative-twin`: a rule on a commutative operation with one
    constant operand and no rule for the operands in the other order. A quick
    fix adds the flipped rule right after it, one priority higher so that it
    does not overlap, and renames a named rule to `<name>_flipped`.
  - `partial-but-total`: a `decl partial` without an `extern` implementation
    has a rule whose patterns are all variables or `_`, with no clauses and a
    result that calls no partial term, so the term cannot fail. Related
    location: that rule.
  - `multi-single-result`: a `decl multi` without an `extern` implementation
    has exactly one rule, and nothing in that rule calls a multi term. Related
    location: that rule.

  - `priority-gap` and `unneeded-priority`: a rule's priority is higher than
    the project needs. They compile the project and, for each term, take the
    pairs of rules that may overlap at different priorities, the only pairs
    whose order matters. Every rule then gets the smallest priority above those
    of the overlapping rules it must beat (0 if there are none). A rule with a
    higher explicit priority than that gets `priority-gap` when it takes part in
    such a pair and `unneeded-priority` when it does not. They are skipped when
    the project does not compile or a term has more than 3000 rules. The quick
    fix renumbers every rule of the term that differs, in any file, as one
    edit: it replaces a priority, removes one that becomes 0 and adds one to a
    rule that has none. The ordering of overlapping rules, and so the decision
    trie, is unchanged.

  Rules that repeat each other under variable renaming need no lint of their
  own, because Cranelift already reports them as `shadowed` or `overlap`
  errors. Run over Cranelift 0.136.2's `src/opts` (46 `partial` and 7 `multi`
  declarations), `partial-but-total` and `multi-single-result` report nothing,
  and `priority-gap` reports 28 rules of four terms (`all_zero`,
  `iadd_uextend`, `isub_uextend` and one in `prelude_opt.isle`), which use
  priorities as explicit tie-breakers. Disable it there if you prefer that style.

  Cranelift's own mid-end moves constants to the right before `simplify` runs,
  so it deliberately has no twin rules and `missing-commutative-twin` fires on
  its rules (about a dozen in `src/opts`). Disable it there.

  Disable any of them for a project in `isle-lsp.toml`:

  ```toml
  [lints]
  disable = ["costlier-rhs", "missing-commutative-twin"]
  ```

## Navigation

Go to definition, find references, document highlight and rename work on terms,
types, enum variants, constants, macros, named rules and pattern or `let`
variables, across every file of the project. Variables follow ISLE scoping: a
repeated name in a pattern is the same binding, and a `let` binding shadows
earlier ones in its body only.

Terms keep their roles apart in the index: a call in a right-hand side, a match
in a pattern (an extractor use), a rule head, and an `extern` or `extractor`
implementation all resolve to the term's `decl`.

Go to definition on the type part of a variant such as `Kind.A` jumps to the
type, and on the rest of it to the variant. The heads of `spec` forms, the term
of `instantiate` and the type of `model` resolve to their declarations, and take
part in references and rename.

Rename validates the new name. It rejects names that are not valid symbols,
`$`-prefixed names for anything but constants, a variable named `_`, and names
that clash with an existing definition, including one of a different kind.
Renaming a type also rewrites the `Type.` prefix of its variants, and renaming
a variant keeps the type prefix.

### Call hierarchy

`textDocument/prepareCallHierarchy` works on terms and macros. A call is a use in
a right-hand side, an `if` or `if-let` clause or a macro body, and its caller is
the term of the enclosing rule or the enclosing macro. Incoming calls list the
callers across the project with every call site, outgoing calls list the terms
and macros a term's rules call. Recursion shows a term as its own caller and
callee, and mutual recursion shows up on both sides. Terms without rules, such as
`extern` constructors, have no outgoing calls. Pattern matches (extractor uses)
are not calls. It is built from the symbol index, so it keeps working while the
project has errors.

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
- **Commutative twin**: with the cursor in a `simplify` rule on a commutative
  operation with one constant operand and no flipped counterpart, add the
  flipped rule after it. This works even when the `missing-commutative-twin`
  lint is disabled.
- **Generate filetest**: with the cursor in a `simplify` rule, create
  `<rule name>.clif` (or `<file>_rule<N>.clif`) next to the source with a
  `test optimize` filetest. The left-hand side becomes an `i32` function, with
  one parameter per variable and distinct constants for `iconst` variables, and
  the right-hand side becomes `; check:` lines. It is offered only for rules
  without `if-let` clauses that use plain integer opcodes (`iadd`, `isub`,
  `band`, `ishl`, ... ) and whose result is a variable, a constant, or one
  such operation. Operand order of commutative operations is not checked,
  since Cranelift canonicalizes it. Run against Cranelift 0.136.2, all 364 tests
  generated from `src/opts` pass under `optimize`. A rule whose right-hand side
  is costlier than its left-hand side and lacks `subsume` can still fail,
  because the egraph keeps the cheaper form.
- **Name or unname a rule**: with the cursor in a rule, give an unnamed rule the
  name `<term>_<n>` (the first unused), or remove the name of a named one.
- **Inline a term**: with the cursor on a call to a term that has exactly one
  rule, no `extern` implementation, only distinct variables as patterns, no
  clauses and no `let` in its result, replace the call with that result with the
  arguments substituted. The `decl` stays; remove it with the unused-declaration
  action once nothing calls it. ISLE macros are not used, because `macro` only
  defines spec macros for verification annotations.
- **Extract into a term**: with a selection that is exactly a call inside a
  rule's result, replace it with a call to a new term `extracted_<n>` and add
  its `decl` and single rule above the rule. The parameters are the variables
  the selection uses from outside, so every one needs a known type, and the
  result type is the called term's. A selection starting with `let` is not
  offered.
- **Insert simplify rule**: with the cursor on an opcode declared in
  `clif_opt.isle` and a `simplify` term in the project, append
  `(rule (simplify (<opcode> ty x y)) x)` to the file. Variable names come from
  the argument types (`ty` for `Type`, `x`, `y`, `z` for `Value`), and the
  result is the first `Value` operand as a placeholder to replace. It is not
  offered for opcodes without a `Value` operand.

Edits that touch other files are sent as one workspace edit.

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

## HTML views

**`isle/view`** returns a named view as a self-describing document, in the
same shape as clif-lsp's `clif/view`. Params are `{ textDocument, view, term?,
position? }`. `term` names the term to show. Without it, the term under
`position` is used (a `decl`, a rule head, a call or a match). The result is
`{ schema, view, term, title, sections }`. `schema` is 1 and changes only for
incompatible changes, so clients must skip unknown fields and unknown section
kinds. Section kinds:

- `text`: `{ text }`, preformatted plain text.
- `html`: `{ html }`, a standalone page with inline CSS and script and no
  external requests. It follows the system light or dark theme. Save it to a
  file and open it in a browser.
- `table`: `{ columns, rows }`, rows of strings.
- `tree`: `{ nodes }`, each node `{ label, children }`.

Every view compiles the project with `cranelift-isle` 0.136.2, the way the
Cranelift build does, so the views describe what the generator really emits. A
project with errors has no view: the request fails with the compiler's error
text, which is also how a rule that can never match is explained (the
`shadowed` error names the rule that masks it).

- **`trie`** (needs a term): the decision tree that `cranelift-isle` builds for
  the term's rules. Each node is a value binding (`v3 = iadd_extract(v0)`, in
  grey), a `match` on a binding with one arm per pattern (an enum variant, an
  integer, `Some`), an equality test between two bindings (`v1 == v2`, for a
  variable that appears twice), a loop over a multi-term, or the result of a
  rule, labelled with the rule name, priority and location. Higher-priority
  rules are tested first, which is why the order differs from the file order.
  The page folds every node and has expand and collapse buttons. Other
  sections: the same tree as `tree`, and a `table` of the rules in the order
  they are reached.
- **`rust`** (needs a term): the function `cranelift-isle` generates for the
  term, extracted from the project's generated Rust. Each `// Rule at ...`
  comment marks the result of the rule at that location and is highlighted on
  the page. Sections: `text` with the code, the page, and a `table` of the
  rules the function contains. Line numbers are those of `cranelift-isle`.
- **`graph`** (term optional): the terms that have rules, with an edge from a
  term to each term its rules construct. Terms in a call cycle, including
  mutual recursion, are tinted and their cycle edges are dashed, and a `rec`
  declaration is marked on the node. Terms without rules (external
  constructors) are left out. With a term, the graph shows that term, every
  term it reaches and its direct callers, and the `table` section adds the
  rules of that term that overlap at different priorities (the lower-priority
  rule is still reachable for some inputs), saying whether one pattern covers
  the other or the overlap is partial. Sections: a `table` of terms with their
  rule counts, the terms they call and whether they are in a cycle, the page,
  and the overlap table.

An unknown view, a term without rules, or a missing term where one is needed
fails with an invalid-params error.

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
