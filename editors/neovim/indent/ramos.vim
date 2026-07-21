" indent/ramos.vim — indentation for Ramos
"
" Ramos is indentation driven (like Python). A line whose tail opens a block
" increases the indent of the following line by `shiftwidth` (2). Blank lines
" and lines that start a new block carry the indent forward via autoindent.
"
" Block-opening keywords: module / struct / trait / attributes / fn / fnp /
" case / cond / if / else / run / do / begin. A `rescue` clause dedents to its
" `begin`. A trailing binary operator, comma, or `=` also continues onto an
" indented line.
if exists('b:did_indent')
  finish
endif
let b:did_indent = 1

setlocal autoindent
setlocal expandtab
setlocal shiftwidth=2
setlocal softtabstop=2

let b:undo_indent = 'setlocal autoindent< expandtab< shiftwidth< softtabstop< indentexpr<'

function! RamosIndent(lnum) abort
  " Previous non-blank line.
  let plnum = prevnonblank(a:lnum - 1)
  if plnum == 0
    return 0
  endif

  let pind = indent(plnum)
  let pline = getline(plnum)
  let cur   = getline(a:lnum)

  " A `rescue` clause closes the current handler body and aligns with its
  " matching `begin`.
  if cur =~# '^\s*rescue\>'
    let blnum = a:lnum - 1
    while blnum > 0
      if getline(blnum) =~# '^\s*begin\>'
        return indent(blnum)
      endif
      let blnum -= 1
    endwhile
  endif

  " Ignore a trailing comment when detecting openers/continuations.
  let bare = substitute(pline, '\s*#.*$', '', '')

  " A line that opens an indented block: a definition head (`module Name`,
  " `fn name(args)`, …), a control-flow head (`case x`, `cond`, `begin`), a
  " `rescue` clause, a multi-line lambda (has `do` but no `->`), or an
  " arm/clause whose body is on the next line (a trailing `->`).
  let opens_block =
        \    bare =~# '\C^\s*\(module\|struct\|trait\)\>'
        \ || bare =~# '\C^\s*attributes\s*$'
        \ || bare =~# '\C^\s*\(fn\|fnp\)\>'
        \ || bare =~# '\C^\s*\(case\|cond\|if\|else\|run\|begin\)\>'
        \ || bare =~# '\C^\s*rescue\>'
        \ || bare =~# '\C\<\(case\|cond\|run\|begin\)\s*$'
        \ || bare =~# '=\s*$'
        \ || bare =~# '->\s*$'
        \ || (bare =~# '\C\<do\>' && bare !~# '->')

  " Continuation: the line ends with a binary operator, pipe, comma, or an
  " open bracket, so the next line continues it and indents once.
  let continues = bare =~# '[-+*/%<>|,([{]\s*$'

  " An arm or lambda body (`pattern -> ...`) and a fresh top-level definition
  " should not pick up a continuation indent.
  let is_arm = cur =~# '->'
  let starts_toplevel = cur =~# '^\s*\(module\|struct\|trait\)\>'

  if opens_block && !starts_toplevel
    return pind + shiftwidth()
  endif
  if continues && !is_arm
    return pind + shiftwidth()
  endif
  return pind
endfunction

setlocal indentexpr=RamosIndent(v:lnum)
