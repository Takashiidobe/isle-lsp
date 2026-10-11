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

## Custom requests and HTML views

The [custom requests](features.md#custom-requests) and
[HTML views](features.md#html-views) are not bound to any key. This defines one
command for each. Text results open in a scratch buffer and HTML pages open in
your browser through `vim.ui.open`.

```lua
local function show(text)
  vim.cmd('new')
  vim.bo.buftype, vim.bo.bufhidden = 'nofile', 'wipe'
  vim.api.nvim_buf_set_lines(0, 0, -1, false, vim.split(text, '\n'))
end

local function request(method, params, render)
  local client = vim.lsp.get_clients({ bufnr = 0, name = 'isle_lsp' })[1]
  params.textDocument = vim.lsp.util.make_text_document_params()
  client:request(method, params, function(err, result)
    if err then
      vim.notify(err.message, vim.log.levels.ERROR)
    else
      local text = render(result)
      if text then show(text) end
    end
  end)
end

local function tree_lines(nodes, depth, out)
  for _, node in ipairs(nodes) do
    table.insert(out, string.rep('  ', depth) .. node.label)
    tree_lines(node.children, depth + 1, out)
  end
end

local function render_view(v)
  local out = { v.title, '' }
  for _, s in ipairs(v.sections) do
    if s.kind == 'html' then
      local page = vim.fn.tempname() .. '.html'
      vim.fn.writefile(vim.split(s.html, '\n'), page)
      vim.ui.open(page)
    else
      table.insert(out, '== ' .. s.title .. ' ==')
      if s.kind == 'text' then
        table.insert(out, s.text)
      elseif s.kind == 'table' then
        table.insert(out, table.concat(s.columns, '\t'))
        for _, row in ipairs(s.rows) do
          table.insert(out, table.concat(row, '\t'))
        end
      elseif s.kind == 'tree' then
        tree_lines(s.nodes, 0, out)
      end
      table.insert(out, '')
    end
  end
  return table.concat(out, '\n')
end

local function view(command, name)
  vim.api.nvim_create_user_command(command, function(o)
    local params = { view = name, term = o.fargs[1] }
    if not params.term then
      params.position = vim.lsp.util.make_position_params(0, 'utf-16').position
    end
    request('isle/view', params, render_view)
  end, { nargs = '?' })
end

view('IsleTrie', 'trie')
view('IsleRust', 'rust')
view('IsleGraph', 'graph')

vim.api.nvim_create_user_command('IsleRules', function()
  request('isle/rulesForTerm',
    { position = vim.lsp.util.make_position_params(0, 'utf-16').position },
    function(rules)
      local items = {}
      for _, rule in ipairs(rules) do
        table.insert(items, {
          filename = vim.uri_to_fname(rule.location.uri),
          lnum = rule.location.range.start.line + 1,
          col = rule.location.range.start.character + 1,
          text = 'priority ' .. rule.priority .. ', wins over ' .. #rule.winsOver,
        })
      end
      vim.fn.setqflist(items, 'r')
      vim.cmd('copen')
    end)
end, {})
```

- `:IsleTrie [term]` opens the decision trie of a term in the browser and
  prints the rules in evaluation order.
- `:IsleRust [term]` opens the Rust that `cranelift-isle` generates for a term.
- `:IsleGraph [term]` opens the term call graph, with the overlapping rules of
  `term` printed in the buffer when you pass one.
- `:IsleRules` lists the rules of the term under the cursor, highest priority
  first, in the quickfix list.

Without an argument the commands use the term under the cursor, so put the
cursor on a `decl`, a rule head or a call. `:IsleGraph` with the cursor on
anything else shows the whole project.
