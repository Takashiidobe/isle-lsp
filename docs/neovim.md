# Neovim

This setup needs Neovim 0.11 or later and `isle-lsp` on your `PATH` (see the
[README](../README.md#install)).

## Setup

Add this to your `init.lua`:

```lua
vim.filetype.add({ extension = { isle = 'isle' } })

vim.lsp.config('isle_lsp', {
  cmd = { 'isle-lsp' },
  filetypes = { 'isle' },
  root_markers = { 'isle-lsp.toml', '.git' },
})
vim.lsp.enable('isle_lsp')
```

Open a `.isle` file and run `:checkhealth vim.lsp` to confirm the client
attached. Neovim does not set an `isle` filetype by default, so the first line is
required.

ISLE files only make sense together. If your files are not in a layout the
server recognizes, add an `isle-lsp.toml` at the project root; see
[Project files](features.md#project-files).

## Using the features

Neovim 0.11 maps these by default:

- `K` shows hover.
- `grr` lists references, `grn` renames, `gra` opens code actions.
- `gO` lists document symbols.
- `CTRL-S` in insert mode shows signature help.

Go to definition is `vim.lsp.buf.definition()`. Map it if you want a key:

```lua
vim.keymap.set('n', 'gd', vim.lsp.buf.definition)
```

Turn on inlay hints and semantic tokens per buffer:

```lua
vim.api.nvim_create_autocmd('LspAttach', {
  callback = function(args)
    vim.lsp.inlay_hint.enable(true, { bufnr = args.buf })
  end,
})
```

Semantic tokens are on by default. The server uses the token types `keyword`,
`function`, `type`, `enumMember`, `variable`, `number` and `macro`, and the
modifiers `declaration`, `readonly` and `priority` (rule priorities), so a
colorscheme can style them through the `@lsp.type.*` and `@lsp.mod.*` groups.
