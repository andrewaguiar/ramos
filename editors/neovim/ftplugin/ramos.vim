" ftplugin/ramos.vim — buffer-local options for Ramos
" Ramos is indentation driven: 2 spaces, never tabs (see README.md).
if exists('b:did_ftplugin')
  finish
endif
let b:did_ftplugin = 1

" The two hard rules from the language spec.
setlocal expandtab        " spaces, never tabs
setlocal shiftwidth=2     " exactly 2 spaces per level
setlocal softtabstop=2
setlocal tabstop=2

" Indentation is driven by indent/ramos.vim; `autoindent` is the fallback.
" (No `smartindent`: it zeroes the indent of `#`-comment lines.)
setlocal autoindent

" Comments use `#`.
setlocal comments=:#
setlocal commentstring=#\ %s

" Movement & folding niceties.
let b:match_words = '\<case\>:\<cond\>'
let b:undo_ftplugin = 'setlocal expandtab< shiftwidth< softtabstop< tabstop<'
      \ . ' autoindent< comments< commentstring<'
