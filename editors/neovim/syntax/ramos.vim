" syntax/ramos.vim — syntax highlighting for Ramos
"
" Language spec: README.md (indentation driven, 2 spaces, no tabs).
"   keywords:  module struct trait implements attributes function helper do end
"              alias as self case cond if else run when
"              and or not   true false nil   _
"   literals:  int float string "..." with #{...} interpolation
"              symbol :foo / :"..."   list [..] tuple (..) map {..}
"   operators: + - * / % **  == != < > <= >=  <>  ->  |
if exists('b:current_syntax')
  finish
endif

" Ramos is case-sensitive: keywords are lowercase, types capitalized.
syntax case match

" Sync on block-opening keywords.
syntax sync fromstart

" ── Keywords: definitions ───────────────────────────────────────────────────
syntax keyword ramosModule     module
syntax keyword ramosStruct     struct
syntax keyword ramosTrait      trait
syntax keyword ramosImplements implements
syntax keyword ramosDefine     function helper
syntax keyword ramosAttr       attributes
syntax keyword ramosLambda     do
syntax keyword ramosEnd        end
syntax keyword ramosAlias      alias as

" ── Keywords: control flow (no `if` in Ramos) ─────────────────────────────────
syntax keyword ramosConditional case cond if else run when

" ── Constants ───────────────────────────────────────────────────────────────
syntax keyword ramosBoolean true false
syntax keyword ramosConstant nil
syntax keyword ramosSelf     self

" ── Logical word-operators ──────────────────────────────────────────────────
syntax keyword ramosOperator and or not

" ── Wildcard pattern ────────────────────────────────────────────────────────
syntax match ramosWildcard "\<_\>"

" ── Numbers ─────────────────────────────────────────────────────────────────
syntax match   ramosFloat  "\C\<\d\+\.\d\+\([eE][-+]\?\d\+\)\?\>"
syntax match   ramosFloat  "\C\<\d\+[eE][-+]\?\d\+\>"
syntax match   ramosInt    "\C\<\d\+\>"

" ── Operators & punctuation ─────────────────────────────────────────────────
syntax match   ramosArrow    "->"
syntax match   ramosOperator "\*\*\||\|<>\|==\|!=\|<=\|>=\|[-+*/%<>]"
syntax match   ramosAssign   "="
syntax match   ramosPunct    "[,;:.(){}\[\]]"

" ── Symbols ─────────────────────────────────────────────────────────────────
" Defined after ramosPunct so the leading `:` is claimed by the symbol rather
" than the punctuation match (in Vim the later item wins at a shared start col).
"   :foo   :ok?   :"with spaces"
syntax match   ramosSymbol "\C:[A-Za-z_][A-Za-z0-9_?!]*"
syntax region  ramosSymbol matchgroup=ramosSymbol start=+:"+ skip=+\\\\\|\\"+ end=+"+ oneline

" ── Comments ────────────────────────────────────────────────────────────────
syntax keyword ramosTodo FIXME TODO NOTE XXX HACK contained
syntax match   ramosComment "#.*$" contains=ramosTodo,@Spell

" ── Identifiers ─────────────────────────────────────────────────────────────
syntax match   ramosType       "\C\<[A-Z][A-Za-z0-9_]*\>"

" A lowercase identifier immediately followed by `(` is a function call.
" (Ramos allows parens-less calls, so we only highlight the unambiguous case.)
syntax match   ramosFuncCall   "\C\<[a-z_][A-Za-z0-9_?!]*\ze("

" An identifier right after a `.` is a member / method access.
syntax match   ramosMethodCall "\C\.\zs[A-Za-z_][A-Za-z0-9_?!]*"

" Highlight the name being defined after a definition keyword. Module/struct/
" trait names may be namespaced, e.g. `MyApp.Business.SystemUser`.
syntax match   ramosDefName   "\C\<\(module\|struct\|trait\)\s\+\zs[A-Z][A-Za-z0-9_.]*"
syntax match   ramosFnName    "\C\<\(helper\|function\)\>\s\+\zs[A-Za-z_][A-Za-z0-9_?!]*"
" The short name introduced by `alias Foo.Bar as Bar`.
syntax match   ramosAliasName "\C\<as\s\+\zs[A-Z][A-Za-z0-9_.]*"

" ── Strings with #{...} interpolation ───────────────────────────────────────
"   "Ola #{self.name}"  -> the #{...} is highlighted as embedded Ramos code.
syntax region  ramosString matchgroup=ramosStringDelim
      \ start=+"+ skip=+\\\\\|\\"+ end=+"+
      \ contains=ramosInterp,ramosEscape,@Spell

" Escape sequences inside strings.
syntax match   ramosEscape contained +\\[\\"nrt#0]+
syntax match   ramosEscape contained +\\x\x\{2}+
syntax match   ramosEscape contained +\\u\x\{4}+

" The interpolation region: `#{ ... }` runs nested Ramos, so it contains TOP.
syntax region  ramosInterp matchgroup=ramosInterpDelim
      \ start=+#{+ end=+}+
      \ contained contains=TOP

" ── Built-in modules (from types/*.md) ──────────────────────────────────────
syntax keyword ramosBuiltin
      \ Integer Float Bool String Symbol Nil
      \ List Tuple Map Struct Module

" ── Highlighting links ──────────────────────────────────────────────────────
highlight def link ramosModule       Keyword
highlight def link ramosStruct       Keyword
highlight def link ramosTrait        Keyword
highlight def link ramosImplements   Keyword
highlight def link ramosDefine       Keyword
highlight def link ramosAttr         Keyword
highlight def link ramosLambda       Keyword
highlight def link ramosEnd          Keyword
highlight def link ramosAlias        Keyword

highlight def link ramosConditional  Conditional

highlight def link ramosBoolean      Boolean
highlight def link ramosConstant     Constant
highlight def link ramosSelf         Keyword
highlight def link ramosWildcard     Special

highlight def link ramosOperator     Operator
highlight def link ramosArrow        Operator
highlight def link ramosAssign       Operator
highlight def link ramosPunct        Delimiter

highlight def link ramosComment      Comment
highlight def link ramosTodo         Todo

highlight def link ramosInt          Number
highlight def link ramosFloat        Float

highlight def link ramosSymbol       Special

highlight def link ramosString       String
highlight def link ramosStringDelim  Delimiter
highlight def link ramosEscape       SpecialChar
highlight def link ramosInterpDelim  Delimiter

highlight def link ramosType         Type
highlight def link ramosBuiltin      Type
highlight def link ramosFuncCall     Function
highlight def link ramosMethodCall   Function
highlight def link ramosDefName      Identifier
highlight def link ramosFnName       Function
highlight def link ramosAliasName    Identifier

let b:current_syntax = 'ramos'
