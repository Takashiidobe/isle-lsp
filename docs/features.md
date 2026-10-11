# Features

This lists everything the server does. Positions and ranges follow LSP.

## Document sync

Incremental text sync. The server keeps each open document in a rope and
converts LSP UTF-16 positions to byte offsets. Every edit re-parses the
document with a tolerant parser that recovers from unbalanced parentheses, so
one error does not hide the rest of the file.

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
   `target/`. Without a build, the generated files are left out.
3. **Siblings**: every `.isle` file in the document's directory.

Open buffers replace the files on disk, so unsaved edits in one file are visible
to the others. A file the project lists but that is missing on disk is
reported, not fatal.
