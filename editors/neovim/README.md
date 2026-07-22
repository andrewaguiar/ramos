# Ramos — Neovim support

Syntax highlighting, indentation and filetype detection for [Ramos](../../README.md)
(`*.rmo`), matching the current language spec:

- indentation driven, **2 spaces, never tabs**
- keywords: `module` `struct` `trait` `implements` `attributes` `function` `helper`
  `do` `end` `alias` `as` `self` `case` `cond`
  `run` `when` `and` `or` `not`
- literals: int, float, string with `#{...}` interpolation, symbol (`:foo`,
  `:"..."`), `true`/`false`/`nil`, list, tuple, map
- operators: `+ - * / % **` `== != < > <= >=` `<>` `->` `|`

## Layout

```
editors/neovim/
├── ftdetect/ramos.vim   # registers *.rmo → filetype ramos
├── ftplugin/ramos.vim   # 2-space / no-tabs options, comment string
├── indent/ramos.vim     # Python-style indentation after block keywords
└── syntax/ramos.vim     # highlighting (incl. nested #{...} interpolation)
```

## Install

These are standard Neovim runtime files. Any of the methods below works.

### Option A — symlink into your config

```sh
ln -s "$PWD/editors/neovim/ftdetect" ~/.config/nvim/ftdetect
ln -s "$PWD/editors/neovim/ftplugin" ~/.config/nvim/ftplugin
ln -s "$PWD/editors/neovim/indent"   ~/.config/nvim/indent
ln -s "$PWD/editors/neovim/syntax"   ~/.config/nvim/syntax
```

### Option B — packpath / `:packadd`

Drop the directory into a pack, e.g.:

```
~/.config/nvim/pack/ramos/start/ramos/
├── ftdetect/ramos.vim
├── ftplugin/ramos.vim
├── indent/ramos.vim
└── syntax/ramos.vim
```

Then Neovim loads it automatically.

### Option C — manual `runtimepath`

In `init.lua`:

```lua
vim.opt.runtimepath:append("/path/to/ramos/editors/neovim")
```

## Verify

```vim
:e examples/library.rmo
:echo &filetype        " -> ramos
:setlocal shiftwidth?  " -> shiftwidth=2
:setlocal expandtab?   " -> expandtab
:syntax list           " -> ramos* groups
```

String interpolation (`"Ola #{self.name}"`) highlights the embedded expression;
symbols (`:ok`, `:"not found"`), the arrow (`->`), the pipe (`|`) and
`**` all get their own faces.
