<p align="center">
  <img src="ramos-logo.png" alt="Ramos" width="160">
</p>

<h1 align="center">Ramos</h1>

Ramos (plant branches) is a small, **indentation-driven**, **immutable**, and **strict** 
programming language. It borrows its feel from Elixir and Ruby: everything is an expression,
values are persistent and immutable.

## Philosophy

Ramos is intentionally **limited** and **opinionated**. Its design philosophy is
to offer fewer ways to do things and to enforce a consistent code style strictly.
Rather than providing multiple syntaxes or constructs for the same operation,
Ramos gives you one clear, idiomatic way. This reduces decision fatigue, makes code
more predictable, and helps teams maintain a uniform style across codebases.

You won't find ternary operators or symbolic logical operators (`&&`, `||`,
`!`), and `if` stops at two branches — `else if` is a `cond`. Ramos provides
`if`, `cond`, and `case` for branching, and `and`, `or`, and `not` for logical
operations. There are exactly two spaces for indentation — no tabs, no
configurable spacing. This strictness is by design.

## Strict rules (fails interpreter)

These are not style suggestions: violating any of them is a hard error that
stops the interpreter before your program runs. They exist so that all Ramos code
looks the same, everywhere.

**Layout**

- **2 space indentation** — exactly 2 spaces per indentation level, never tabs
- **Whitespace around operators** — `x = 1` is correct, `x=1` is an error
- **Whitespace after commas** — `fn test(n, x)` is correct, `fn test(n,x)` is
  an error (applies to arguments, lists, tuples and maps)
- **Parentheses on calls** — `foo()`, never bare `foo`; a bare name is always a
  variable or a field access
- **Assigned blocks start on their own line** — when the value being assigned
  is an `if`, `case`, `cond` or `run` block, a `"""` multiline string, or a
  `do` lambda with an indented body, it begins on the line *after* the `=`,
  indented one level:

  ```
  # error: the block hangs off the end of the assignment
  result = case value
    true -> :ok
    _ -> :error

  # correct
  result =
    case value
      true -> :ok
      _ -> :error

  # the same rule for a multiline string
  banner =
    """
      Ramos
    """

  # and for a `do` whose body is a block
  double =
    do x
      y = x * 2
      y
  ```

  Otherwise the value starts on the `=` line but finishes several lines below
  it, and the deeper the surrounding code, the harder it is to see where the
  assignment ends. A value that *does* end on the `=` line is unaffected, so
  the single-expression lambda `f = do x -> x + 1` is correct — the `->` is
  what makes it one line rather than a block.

- **One `:` on a map key** — a symbol key is `{name: 1}`; the leading colon of
  `{:name: 1}` writes the symbol twice over and is an error

**Naming**

- **Modules are CamelCase**, `[a-zA-Z]` only — `SystemUser`, optionally
  namespaced as `MyApp.Business.SystemUser`
- **Variables and functions are snake_case**, `[a-z_]` only — `main_user_name`,
  `read_safely`. No trailing `?` or `!` as in Ruby or Elixir

**Files**

- **`.ramos` extension** — no other extension is loaded
- **One definition per file** — a file holds exactly one module, trait or struct
- **File name matches the definition** — `SystemUser` lives in `system_user.ramos`
- **Path follows the namespace** — `MyApp.Business.SystemUser` lives at
  `src/my_app/business/system_user.ramos`

This README is a tour of the language. For the full standard library, see the
module sources under [`stdlib/`](stdlib/).

## Hello, world

```
# comments start with # and run to the end of the line
print("Ola #{name}")

greet = do name -> "Ola #{name}"
print(greet("Andrew"))
```

`print` lives in the `Kernel` module — and `Kernel` is the one module that is
implicitly in scope everywhere, so `print(x)` is sugar for `Kernel.print(x)`
and you never write the `Kernel.` prefix (or `alias` it). See the
[Standard library](#standard-library) section for the rest of what `Kernel`
provides.

## Entrypoints

A runnable Ramos program starts from an **entrypoint**: a `.ramos` file whose
module exposes a public `fn main()`. `ramos run` loads the file and calls that
module's `main()` with no arguments:

```
# app.ramos
module App
  fn main()
    print("Ola world")
```

```
ramos run app.ramos        # calls App.main()
```

`main()` is only the way in, not the whole module: an entrypoint may define
whatever other functions it needs, public or private. Keeping it thin is still
the habit worth having — `fnp` helpers break `main()` up without widening what
the module exposes:

```
# app.ramos
module App
  fn main()
    ["Andrew", "Ana"]
    | List.map(do name -> greeting(name))
    | List.each(do line -> print(line))

  fnp greeting(name)          # private helper, fine
    "Ola #{name}"
```

Most real work still belongs in other modules, and an entrypoint reaches those
through `alias`, which drops a namespaced module's prefix so you can call it by
its short name:

```
# cli.ramos
module Cli
  alias MyApp.Business.Greeter

  fn main()
    Greeter.greet_all(["Andrew", "Ana"])
```

Alias several modules the same way; each `alias` line sits at the top of the
entrypoint body:

```
module Cli
  alias MyApp.Business.Greeter
  alias MyApp.Reporting.Summary

  fn main()
    ["Andrew", "Ana"]
    | Greeter.greet_all()
    | Summary.print_report()
```

A codebase can have **as many entrypoints as it needs** — one per runnable
program — each in its own file, following the same naming rules as any other
module (`App` → `app.ramos`, `MyApp.Cli` → `src/my_app/cli.ramos`).

Bare top-level code, like the [Hello, world](#hello-world) snippet above, still
runs for quick experiments — `ramos run` executes a file's top-level statements
when it has no entrypoint module. But a real program you ship is structured as
an entrypoint.

## Syntax

### Comments

```
# single line comment, everything after # is ignored
```

### Documentation comments

Modules and functions carry their documentation *inline*, as a comment block
placed **immediately below** the definition it describes. A module doc starts
with `@module_doc`; a function doc starts with `@doc`. In both cases the marker
is followed by an empty `#` line, then the prose:

```
module List
  # @module_doc
  #
  # List — operations on persistent, immutable lists.
  #
  #   [1, 2, 3] | List.map(do x -> x * 2)   # == [2, 4, 6]

  fn map(list, f)
    # @doc
    #
    # Applies `f` to every element of `list`, returning a new list of the
    # results. Preserves order and length.
    #
    #   List.map([1, 2, 3], do x -> x * 2)   # == [2, 4, 6]
    case list
      [] -> []
      [head | tail] -> [f(head)] ++ map(tail, f)
```

The module doc is indented to the module body; a function doc is indented to
the function body (so it lives inside the function it documents). Doc blocks
are just comments, so every line begins with `#`, and blank `#` lines separate
the summary from examples. See [`stdlib/`](stdlib/) for the convention applied across
the standard library.

**Examples in a doc block are tests.** Any line carrying `# ==` is a claim, and
`ramos doctest` runs it:

```
ramos doctest --stdlib stdlib     # the standard library documents itself
ramos doctest ./mylib             # a project, against the embedded stdlib
```

Each example runs as its own program, in its own empty directory, with the
project's modules beside it — so an example must be self-contained. Write the
file before reading it back, and start an actor before calling it:

```
#   File.write("greeting.txt", "hello\n")
#   File.read("greeting.txt")   # == (:ok, "hello\n")
```

Lines above an assertion in the same snippet are its setup, and a blank line of
prose ends the snippet (and the scope). When several examples in a file share a
fixture, declare it once in the `@module_doc` under `# ramos doctest setup`: that
snippet is never asserted itself and runs before every example in the file.

```
#   # ramos doctest setup
#   struct Person
#     attributes
#       name: nil
```

Everything after `# ==` is parsed as Ramos, so a note like `(a directory)` after
the expected value is a syntax error, not a comment — put it in the prose.

### Variables

Assignment binds a name to a value. Bindings are immutable: a name can be
**rebound** to a new value, but the underlying values themselves never mutate.

```
x = 100
name = "andrew"
pi = 3.14

x = x + 1          # rebinding, not mutation
```

### Literals

```
42            # int
3.14          # float
"andrew"      # string
:symbol       # symbol
true          # bool
false         # bool
nil           # nil
[1, 2, 3]     # list
(1, 2, 3)     # tuple
{name: "andrew"}  # map
```

Only `false` and `nil` are falsy; everything else (including `0`, `""` and `[]`)
is truthy.

#### Map keys

A map key is a **symbol**, a **string** or an **integer** — nothing else, at any
point, so every map can be written as a literal. A symbol key is a bare name
followed by the `:` that separates it from its value — that is the only `:`,
and `{:name: 1}` is an error:

```
{name: "andrew"}      # symbol key  :name
{"host": "local"}     # string key  "host"
{8080: :http}         # integer key 8080

{name: 1} == {"a": 1}   # == false — different key types never collide
```

Keys are literals: they are looked up by value, so a key cannot interpolate
(`{"#{host}": 1}` is an error). The same three forms work in patterns:

```
case config
  {"host": h, 8080: p} -> "#{h}:#{p}"
  _ -> "no match"
```

The rule holds at runtime too: `Map.put` rejects a key of any other type. So a
grouping function must yield one of the three —

```
List.group_by([1, 2, 3, 4], do x -> Integer.is_even(x))
# runtime error: a Map key must be an Integer, String or Symbol, got Bool

parity = do x -> cond
  Integer.is_even(x) -> :even
  true -> :odd

List.group_by([1, 2, 3, 4], parity)   # == {even: [2, 4], odd: [1, 3]}
```

### Indentation

Ramos is **indentation driven**, like Python. There is no `do`/`end` and no
`begin`/`end`. A keyword or clause followed by an indented line opens a block;
the block ends as soon as indentation returns to a lower level. Two rules:

- use **exactly 2 spaces** per indentation level
- **never tabs**

```
module Geometry
  fn area(r)
    r * r * 3.14
```

Blocks are introduced by `module`, `struct`, `trait`, `fn`, `if`, `case`,
`cond` and `do`.

### Operators

Arithmetic:

```
1 + 2       # sum
10 - 4      # minus
3 * 4       # multiply
10 / 3      # divide (int / int -> int, truncates toward zero)
-7 % 3      # modulo (non-negative, follows sign of divisor)
2 ** 10     # pow
-5          # neg (unary)
```

Mixing an `Integer` and a `Float` widens the integer to a float:

```
1 + 1.5     # == 2.5
10 / 4.0    # == 2.5 (float division; int / int truncates)
```

Comparison (return a `bool`):

```
1 == 1      # equals?
1 != 2      # not equals?
1 < 2       # less?
1 > 2       # greater?
1 <= 2      # less or equal?
1 >= 2      # greater or equal?
```

Logical. There are **no symbolic** logical operators (`&&`, `||`, `!`); use the
word forms. `and` / `or` short-circuit.

```
true and false   # and
true or false    # or
not true         # not
```

String:

```
"a" <> "b"   # concatenate
```

List:

```
[1, 2] ++ [3, 4]   # concatenate  # == [1, 2, 3, 4]
```

Map:

```
{a: 1} ++ {b: 2}   # merge  # == {a: 1, b: 2}
{a: 1} ++ {a: 2}   # right side wins on duplicate keys  # == {a: 2}
```

Range. `a..b` is an **inclusive** range of integers, and evaluates to an ordinary
`List` — so every `List` function works on it. Like `.` field access, `..` needs
no surrounding whitespace. It binds looser than arithmetic but tighter than
comparison, so `1..n + 1` is `1..(n + 1)`. A range whose start is past its end is
empty (`5..1 == []`); counting down is `Integer.downto`'s job.

```
1..5                          # == [1, 2, 3, 4, 5]
1..1                          # == [1]
5..1                          # == []
List.sum(1..100)              # == 5050
List.map(1..3, do n -> n * n) # == [1, 4, 9]
```

See also the **Pipes** section below for `|`.

### String interpolation

Use `#{...}` inside double-quoted strings

```
name = "andrew"
print("Ola #{name}, you have #{1 + 1} new messages")
```

Escape sequences such as `\n`, `\t`, `\"`, `\\` and `\#{` are supported.

### Multiline strings

Triple quotes open a **multiline string**. Like everything in Ramos, their shape
is strict:

- the opening `"""` is immediately followed by a newline
- content lines are indented **one level (2 spaces) past the line that opened
  the string**; that prefix is stripped, and any deeper spaces are content
- blank lines need no indentation and contribute a bare newline
- the closing `"""` stands alone on its line, at the same indentation as the
  opening line
- when it is being assigned, it opens on the line *after* the `=` — the same
  [strict rule](#strict-rules-fails-interpreter) `case` and `run` follow

Every content line keeps its trailing newline, so the value always ends with
`\n`. Interpolation and escapes work exactly as in single-line strings.

```
str =
  """
    Hi there

    This is a multiline string
  """

print(str)

fn usage(name)
  """
    Usage: #{name} <file>
      -h  show this help
  """
```

In `usage`, the string opens at the function-body level, so its content is
indented one level deeper and `"Usage: ..."` is flush left in the value, while
`  -h ...` keeps its two extra spaces.

### Control flow

Three constructs, each for a different shape of decision: `if` for two
branches, `cond` for a chain of conditions, `case` for matching a value.

```
if x > 0
  :positive
else
  :zero_or_less

cond
  x > 0 -> :positive
  x < 0 -> :negative
  true  -> :zero

case x
  1 -> :one
  2 -> :two
  _ -> :other
```

`if` takes a condition, an indented body, and an optional `else`. There is
**no `else if`** — the moment you want a third branch, that is `cond`, and the
interpreter says so:

```
if x > 8
  :high
else if x > 3     # error: `else if` is not valid: `if` has exactly two
  :mid            # branches — use `cond` for a chain of conditions
```

A single guarded statement can be written on one line, with the condition
trailing it after `when`:

```
print("big") when x > 3
```

`when`, not `if`, so that `if` means exactly one thing — the two-branch block —
and the trailing guard reads as the guard it already is in a `case` arm. The
parser builds the same conditional either way, so a trailing `when` takes no
`else` and guards exactly one statement.

It cannot guard an assignment: the guarded statement has its own scope, so the
binding in `x = 1 when ready` could never be read on the next line, and the
interpreter rejects it rather than binding into nowhere.

**Any value works as a condition**, not just a boolean: only `false` and `nil`
are falsy, so `0`, `""` and `[]` all take the `if` branch. `Kernel.is_truthy`
is that rule written down (`not is_truthy(x)` is the inverse):

```
is_truthy(0)       # == true
is_truthy("")      # == true
is_truthy(nil)     # == false
is_truthy(false)   # == false
```

All three are expressions, so they can be assigned. Like any block on the right
of `=`, the construct starts on the **next** line, indented:

```
grade =
  if score > 8
    :high
  else
    :low
```

Without an `else`, an `if` whose condition is falsy is `nil`.

An arm body can also be a **block**: keep the `->` at the end of the arm and
indent the body on the following lines; the last expression is the arm's
value. A `when` guard sits between the pattern and the `->`. Both are used
throughout the standard library:

```
case list
  [] -> []
  [head | tail] when head > 0 ->
    doubled = head * 2
    [doubled] ++ keep_positive_doubled(tail)
  [_ | tail] -> keep_positive_doubled(tail)
```

### Sequential matching with `run`

`run` executes a block sequentially, like any other block — but every
`pattern = value` in it is also a checkpoint. The first match that **fails**
ends the block right there, and the value that failed to match becomes the
block's result. If every match succeeds, the block's result is its last
statement's value, exactly as usual.

```
result =
  run
    :ok = validate_number(1)
    :ok = validate_string(1)
    :ok = validate_symbol(1)

print(result) # (:error, (:invalid_string, "1 is not a valid string"))
```

Here `validate_string(1)` returns an error tuple, which does not match `:ok`.
The block stops — `validate_symbol` is never called — and that error tuple is
`result`. Had all three returned `:ok`, `result` would be `:ok`, the last
statement's value.

Without `run`, the same logic needs a `case` nested inside a `case` inside a
`case`, one level deeper per validation, with the error path repeated in every
one of them. `run` is the flat version of that.

A `run` may close with a `case` written **without a subject** — the block's
result *is* the subject — so the happy path and the failure path sit side by
side at the same indentation:

```
run
  :ok = validate_id(data)
  :ok = validate_name(data)
  :ok = validate_age(data)
  :ok = validate_birthday(data)
case
  :ok ->
    print("valid")

  err ->
    print(err)
```

Bindings made inside a `run` are visible to the statements after them in the
same block, which is what lets each step build on the last:

```
total =
  run
    (:ok, price) = fetch_price(item)
    (:ok, quantity) = fetch_quantity(item)
    price * quantity
```

Those bindings do **not** escape the block. A `run` that halts part-way would
otherwise leave some names bound and others not, depending on how far it got;
keeping them inside means the code after a `run` never has to wonder.

`run` is how a chain of fallible steps stays flat. A failed match is an
ordinary value, not an interruption — which is the whole model, described
next.

### Error handling

Ramos has no exceptions. Nothing interrupts execution, unwinds the stack, or
jumps to a handler somewhere up the call chain. A function that can fail says so
in what it **returns**: a tagged tuple, `(:ok, value)` on success and
`(:error, (:type, message))` on failure.

```
fn withdraw(balance, amount)
  cond
    amount <= 0 -> (:error, (:invalid_amount, "amount must be positive"))
    amount > balance -> (:error, (:insufficient_funds, "balance is too low"))
    true -> (:ok, balance - amount)
```

The error is an ordinary value, so it is read with the same `case` as anything
else — and because the error carries its own tag, each failure is handled where
it is named:

```
case withdraw(100, 250)
  (:ok, remaining) -> print("left: #{remaining}")
  (:error, (:invalid_amount, message)) -> print("bad request: #{message}")
  (:error, (:insufficient_funds, message)) -> print("declined: #{message}")
  (:error, (_, message)) -> print("failed: #{message}")
```

The `(:type, message)` pair is the convention the standard library follows: a
lowercase symbol naming *what* went wrong, and a string describing it. The tag
is what you match on; the message is for the human reading the output. Where
there is nothing useful to say, `(:error, reason)` with a bare symbol reason is
enough — that is what `File` and `Dir` return (`:enoent`, `:eacces`, …).

Because a failure is a value, a function that cannot handle one passes it along
by returning it, and the choice is visible at every step rather than hidden in
whether some caller installed a handler:

```
module FileHandler
  fn default_content()
    "Default content"

  fn read_safely(path)
    case File.read(path)
      (:ok, text) -> text
      (:error, reason) ->
        print("Could not read #{path}: #{reason}")
        default_content()
```

Chains of fallible steps do not nest, because [`run`](#run) flattens them: each
line must match, and the first that does not becomes the block's value.

```
total =
  run
    (:ok, after_rent) = withdraw(1000, 400)
    (:ok, after_food) = withdraw(after_rent, 200)
    after_food
```

If `withdraw(1000, 400)` returns an `(:error, _)` tuple, it does not match
`(:ok, after_rent)`; the block stops there and `total` is that error tuple. The
happy path reads top to bottom, and the failure path needs no branch of its own.

## Keywords

Ramos has a small set of reserved keywords. Each serves a specific purpose; there
are no redundant constructs.

| Keyword      | Purpose |
| ------------ | ------- |
| `module`     | Define a namespace of functions |
| `struct`     | Define a typed record with fields |
| `trait`      | Define a contract of functions for structs to implement |
| `implements` | Declare that a struct implements a trait |
| `attributes` | Declare struct fields with default values |
| `fn`         | Define a public function |
| `fnp`        | Define a private function (module-scoped) |
| `case`       | Pattern match a value against patterns |
| `if`         | Branch two ways on a condition (no `else if` — use `cond`) |
| `else`       | The other branch of an `if` |
| `cond`       | Branch on a sequence of boolean conditions |
| `run`        | Run a block of matches, halting on the first that fails |
| `do`         | Create an anonymous function (a lambda) |
| `alias`      | Create a shorter name for a module |
| `as`         | Rename a module in an `alias` statement (defaults to the last segment) |
| `self`       | Reference the current struct instance or module |
| `true`       | Boolean true value |
| `false`      | Boolean false value |
| `nil`        | Represents absence of value |
| `when`       | Guard clause in a `case` pattern, or on a single trailing statement |
| `and`        | Logical AND (short-circuits) |
| `or`         | Logical OR (short-circuits) |
| `not`        | Logical NOT |
| `_`          | Wildcard pattern (matches anything) |

## Pattern matching

`case` matches a value against patterns left-to-right and runs the body of the
first match. `_` is the wildcard. Patterns can bind variables and match tuples,
structs, and lists:

```
case result
  (:ok, value)    -> value
  (:error, reason) -> print("failed: #{reason}")
  _                -> :unknown
```

Lists can be pattern matched with the cons operator `|`:

```
case numbers
  []          -> "empty"
  [x]         -> "single: #{x}"
  [head | _]  -> "head is #{head}"

# Direct destructuring assignment
[head | tail] = [1, 2, 3]           # head == 1, tail == [2, 3]
[first, second | rest] = [1, 2, 3, 4]  # first == 1, second == 2, rest == [3, 4]
```

Tuples destructure in plain assignments too, which is the idiomatic way to
unpack small fixed records:

```
person = ("Andrew", 40)
(name, age) = person          # name == "Andrew", age == 40
```

Nested pattern matching works for any combination:

```
case user
  ({name: n, age: a}, :admin) when a > 18 -> "Admin #{n}"
  ({name: n}, :guest) -> "Guest #{n}"
  _ -> "Unknown"

# Destructuring nested structures
((first, _), [head | _]) = ((1, 2), [3, 4, 5])
# first == 1, head == 3
```

Tagged tuples like `(:ok, value)` / `(:error, reason)` are the standard result
type for fallible operations.

Each name in a pattern binds **once**. `(p, p)` looks like it asks for the two
positions to be equal, but there is no pin operator to ask that with, so the
second `p` would just rebind and the pattern could never fail — it is rejected
instead:

```
(p, p) = (1, 1)     # error: a pattern cannot bind `p` twice
(a, b) = (1, 1)     # correct — compare them afterwards if that is the intent
(_, _) = (1, 2)     # fine: `_` binds nothing, so it may repeat
```

A struct pattern is held to the struct's declared attributes, exactly as
construction is. Naming one that does not exist can never match, so it is an
error rather than a silently skipped branch:

```
case andrew
  Person{nickname: n} -> n     # error: `Person` has no attribute `nickname`
  _ -> :none
```

A **map** pattern is different: a map has no fixed shape, so a key that is not
there is an ordinary non-match, not an error.

`cond` is just `case` over a sequence of guard expressions, useful when no
single value is being matched.

## Types

Practically every value is tied to a module — `42` to `Integer`, `"hi"` to
`String`, `[1,2,3]` to `List`, and so on. See the
[Standard library](#standard-library) section for what each module provides.

| Value                | Module    |
| -------------------- | --------- |
| `42`                 | `Integer` |
| `3.14`               | `Float`   |
| `"andrew"`           | `String`  |
| `:symbol`            | `Symbol`  |
| `true` / `false`     | `Bool`    |
| `nil`                | `Nil`     |
| `[1, 2, 3]`          | `List`    |
| `(1, 2, 3)`          | `Tuple`   |
| `{name: "andrew"}`   | `Map`     |
| module               | `Module`  |
| struct instance      | `Struct`  |

```
x = 100
y = 100.5
s = "andrew"
sym = :symbol
b = true or false
n = nil
list = [1, 2, 3]
tuple = (1, 2, 3)
map = {name: "andrew"}
```

### Pure modules

A **pure module** is a namespace of functions with no per-instance state. It is
itself a value, so it can be passed around and inspected.

```
module PersonUtils
  fn hello(person)
    print("Ola #{person.name}, eu tenho #{person.age}")

  fnp secret()          # fnp = private, only callable from inside the module
    42

PersonUtils.hello(andrew)
andrew | PersonUtils.hello()
```

A name resolves to exactly one function: Ramos does not overload on arity, and
`fn` and `fnp` share the one namespace. Defining a name twice in the same body
is an error rather than a second definition that could never be reached:

```
module Dup
  fn twice(x)
    x * 2

  fn twice(x, y)     # error: `Dup` defines `twice` more than once
    x + y
```

There is no `const`: a constant is a function that takes no arguments, so it is
defined and called like everything else. One construct covers both, and the
call parentheses say plainly that a name is being resolved in a module.

```
module SystemUser
  fn default_email()
    "system@ramos.com.br"

  fn max_retries()
    3

print(SystemUser.default_email())
print(SystemUser.max_retries())
```

```
module Greet
  fn hi_all(people)
    people
    | List.map(do p -> "hi #{p.name}")
    | List.join(", ")
```

### Alias

`alias` drops a module's namespace prefix so you can call it by its **last
segment** alone, cutting the verbosity of long names:

```
alias Geometry.Shapes.Circle       # now referred to as `Circle`

Circle.area(5)  # == 78.5
```

The local name defaults to that last segment, so most aliases need no extra
syntax. `as` **renames** the module — pick a different short name, chiefly to
avoid a collision when two aliased modules would otherwise claim the same one:

```
alias Geometry as Geo              # rename to a shorter name

Geo.circle_area(5)  # == 78.5
```

```
alias Geometry.Circle              # -> Circle
alias Drawing.Circle as Ellipse    # would clash with Circle, so rename it

Circle.area(5)
Ellipse.area(10, 20)
```

Aliases work the same for pure modules and struct modules.

### Struct modules

A **struct module** is a module that also produces typed record instances.
`attributes` declares a field with its default. Instances are created with the
map-literal syntax prefixed by the struct name — any field you don't supply
takes its declared default. Instances are immutable maps under the hood,
tagged with the struct name.

```
struct Person
  attributes
    name: nil
    age: 0

  fn hello(self)
    print("Ola #{self.name}, eu tenho #{self.age}")

andrew = Person{name: "Andrew", age: 40}
andrew.hello()

with_defaults = Person{}      # == Person{name: nil, age: 0}
```

Construction mirrors pattern matching: the same `Person{name: n}` shape that
builds an instance also destructures one in `case` patterns.

Field access is dot syntax, and updates return a new instance:

```
andrew.name                       # == "Andrew"
older = andrew | Struct.put(:age, 41)
```

Setting a field has its own form, which is sugar for exactly that call:

```
andrew.age = 41                   # andrew = Struct.put(andrew, :age, 41)
```

It **rebinds** rather than mutates: the instance `andrew` pointed at is
untouched, and the name is bound to a new one. That is why the left side has to
be a plain variable — there has to be a name to rebind, so `f().age = 41` is an
error rather than a value quietly thrown away.

### Traits

Traits declare a contract of functions. A function **with a body** is a default
implementation; a function **without a body** is required and every implementer
must define it. A struct declares its traits with the `implements` keyword.

```
trait Helloable
  fn hello(self)
    print("Ola #{self.name}, eu tenho #{self.age}")

  fn is_over_eighteen(self)         # required, no body

struct Person
  implements Helloable

  attributes
    name: nil
    age: 0

  fn is_over_eighteen(self)
    self.age > 18

andrew = Person{name: "Andrew", age: 40}

case andrew.is_over_eighteen()
  true  -> andrew.hello()         # uses the trait's default hello
  _     -> :none
```

There is no built-in `Error` trait, because an error is not a special kind of
value. When a symbol and a message are not enough detail, return a struct in the
error slot — it destructures in `case` like any other pattern:

```
struct DeclineReason
  attributes
    code: 0
    message: nil

fn charge(amount)
  cond
    amount <= 0 -> (:error, DeclineReason{code: 555, message: "must be positive"})
    amount > 10000 -> (:error, DeclineReason{code: 556, message: "too large"})
    true -> (:ok, amount)

case charge(50000)
  (:ok, charged) -> print("charged #{charged}")
  (:error, DeclineReason{code: 556, message: message}) -> print("too large: #{message}")
  (:error, DeclineReason{message: message}) -> print("declined: #{message}")
```

### Actors

An **actor** is a named holder of state that answers messages. A module becomes
one by implementing the `Actor` trait, which requires a single function:
`call(f, args, state, config)`, returning a `(reply, new_state)` tuple. The
reply goes back to the caller; `new_state` is what the actor holds from then on.
Nothing mutates — the actor simply keeps the newest value.

```
module Cache
  implements Actor

  # server
  fn call(f, args, state, config)
    case f
      :get ->
        [key] = args
        (Map.get(state, key, nil), state)
      :set ->
        [key, value] = args
        (:ok, Map.put(state, key, value))

  # client
  fn start()
    start_actor(:cache, Cache, {}, {})

  fn get(key)
    call_actor(:cache, Cache, :get, [key])

  fn set(key, value)
    call_actor(:cache, Cache, :set, [key, value])
```

```
Cache.start()
Cache.set("key", "v2")
value = Cache.get("key")     # == "v2"
```

Two `Kernel` functions drive them, so neither needs a prefix:

| Call | Purpose |
| ---- | ------- |
| `start_actor(id, module, initial_state, config)` | Register an actor under `id`. Returns `:ok`. |
| `call_actor(id, module, op, args)` | Send one message and wait; returns the handler's reply. |
| `cast_actor(id, module, op, args)` | Send one message without waiting; returns `:ok` at once. |
| `list_actors()` | Every running actor as an `(id, module)` pair, sorted by id. |
| `actor_stats(id)` | A `Map` describing one actor. |
| `is_actor_started(id)` | Whether `id` names a running actor. |
| `stop_actor(id)` | Finish what was sent, then end the actor. Returns `:ok`. |

The `id` is what identifies a running actor, so one module can back many of
them, each with its own state:

```
Counter.start(:a)
Counter.start(:b)      # independent state from :a
```

#### Inspecting and stopping

Running actors can be listed and asked about themselves:

```
list_actors()             # == [(:alpha, Counter), (:beta, Gauge)]
is_actor_started(:alpha)  # == true
actor_stats(:alpha)
# == {id: :alpha, module: "Counter", calls: 2, casts: 1, pending: 1, alive: true}
```

Each row carries the module itself, not just its name, so a pair goes straight
back into a call:

```
[(id, mod) | _] = list_actors()
call_actor(id, mod, :ping, [])
```

`calls` and `casts` count messages **sent**. Because a cast is asynchronous it
may still be running, so `pending` is the part not accounted for yet — it drops
to zero once the handler has finished and its result been collected.

`stop_actor(id)` lets the actor finish everything already sent to it, then ends
its thread and frees the id for reuse. Nothing in flight is discarded: the stop
is queued behind the messages already in the mailbox. Sending to a stopped
actor is an error, the same as sending to one that was never started.

```
stop_actor(:beta)         # == :ok
is_actor_started(:beta)   # == false
```

#### call and cast

`call` waits: the message goes to the actor's thread, the handler runs there,
and its reply comes back before the next statement. `cast` does not — `:ok`
comes back as soon as the message is sent, and the handler runs on the actor's
thread while this one carries on. One actor handles its own mailbox in order,
so a `call` always sees every message sent to that actor before it.

The second callback returns only the new state, because a cast has no reply to
send anywhere:

```
fn cast(f, args, state, config)
  [key] = args
  Map.delete(state, key)
```

`cast` is a trait *default*, not a requirement: unless a module defines its own,
a cast runs that module's `call` and discards the reply. So an actor written
only for `call` can be cast to without writing anything extra, and you override
`cast` for work that only makes sense fire-and-forget.

The concurrency is real, not just deferral. With a handler that sleeps 400ms:

```
cast_actor(:s, Sleeper, :work, [])
print("main continued immediately")
sleep(50)
```

the whole program takes ~405ms, not 450ms — the two sleeps overlap. Two actors
each sleeping 300ms finish in ~307ms, not 600ms.

A handler's output is buffered on its thread and written by whichever thread
receives it, so a line of actor output never lands inside a line of yours. One
consequence: because a cast runs on its own thread, its output and any failure
inside it surface when the reply is collected — at the next call to that actor,
or at the end-of-program drain.

`config` is fixed for the actor's lifetime and handed to every `call` and
`cast`; `state` is what advances. The convention is to keep `call` as the server half and wrap
each operation in a plain function — the client half — so callers never write
`call_actor` themselves.

Mistakes are named rather than left to surface later: starting a module that
does not implement `Actor`, starting the same id twice, calling an id that was
never started, calling one id through another module's handler, and a `call`
that returns something other than a `(reply, new_state)` pair are all errors.
So is an actor calling itself — the re-entrant call would read the state from
before the outer call and have its write overwritten.

> **Each actor owns its memory.** An actor's thread gets a fresh root scope, so
> a handler cannot see the bindings around the call that started it, and its
> state is reachable only by sending a message. That memory does not include the
> actor registry either, so **a handler cannot message another actor or itself**
> — a self-call would block the actor waiting on a reply only it could produce.
> Actors are not yet a unit of failure isolation: an error in a handler
> propagates to whoever collects the reply rather than restarting the actor.

#### Global

The stdlib ships one actor already written: `Global`, a single process-wide
`Map` behind the id `:global`.

```
Global.start()
Global.put(:user, "andrew")
Global.get(:user)             # == "andrew"
Global.clear(:user)           # a cast: returns :ok at once
```

`start` takes no arguments — there is exactly one of it — and `get` returns
`nil` for a key that was never set. Only `get` waits: `put` and `clear` are
casts that return `:ok` as soon as the message is sent, and a `get` behind them
still sees the write, because the actor handles its mailbox in order.

**Use it with caution.** It is shared mutable state wearing an actor's clothes:
a function that reads it declares nothing about what it needs, every reader and
writer in the program queues behind one thread, the key space is flat and
unscoped, and between a `get` and the `put` that acts on it another caller may
have written the key. An actor handler cannot reach it at all, because an
actor's thread cannot see the registry. Use it for what is genuinely
process-wide and written once — config read at startup, a feature flag, a run
id — and pass the value everywhere else.

#### Config

The other actor the stdlib ships is `Config`: the settings for this run, read
once and answered from memory. It is **read-only** — there is no `put`.

`APP_ENV` names the environment, and the file is `.env` followed by its
downcased name: `APP_ENV=PROD` reads `.env.prod`, `APP_ENV=staging` reads
`.env.staging`, and an unset or empty `APP_ENV` reads the plain `.env`. The path
is relative, so it resolves against the directory the program was run from, and
`Config.path()` reports the choice without reading anything.

```
# .env.prod
[database]
host = "db.internal"
port = 5432
password = 'p#ss'   # a comment after an unquoted value is trimmed off
```

```
Config.start()                     # == :ok, or (:error, :enoent)
Config.get("database", "host")     # == "db.internal"
Config.get("database", "port")     # == "5432"
Config.get("database", "nope")     # == nil
```

The file is TOML, of the subset a settings file uses: `[section]` headers,
`key = value` pairs, `#` comments, blank lines. Every value comes back as a
`String` — `port = 5432` reads as `"5432"` — because the file cannot say what a
value was meant to be, and guessing would let the same key return a different
type on a different deploy. Reach for `to_integer` on the way out. Quotes are
stripped, single or double, and a `#` inside them is part of the value; a key
written before the first `[section]` has no section to be asked for, so it is
skipped.

Reading happens in `start`, not in the actor, so an unreadable file surfaces at
the call that started it and the caller decides whether that is fatal:

```
case Config.start()
  :ok -> run()
  (:error, :enoent) -> run_with_defaults()
```

### Threads

Where an actor holds **state** and answers messages over its lifetime, a
**thread** is a one-shot piece of parallel work that produces a **value**.
`Thread.spawn` starts a zero-argument lambda on its own thread and hands back a
handle at once; `Thread.await` waits on the handle and returns what the lambda
produced.

```
t = Thread.spawn(do -> slow_sum())
other_work()                  # runs while the thread does
total = Thread.await(t)       # now wait for it
```

The parallelism is real: two threads each sleeping 300ms finish in ~300ms, not
600. To fan a batch out and gather it, spawn them all and `Thread.await_all` the
handles — every thread is already running, so the wait is for the slowest, not
the sum:

```
[do -> a(), do -> b(), do -> c()]
| List.map(do f -> Thread.spawn(f))
| Thread.await_all()
```

`Thread.map` wraps that pattern: it fans a list out one thread per element and
gathers the results in order.

```
Thread.map([1, 2, 3], do n -> n * n)   # == [1, 4, 9]
```

A spawned lambda carries its own captured scope, but — exactly like an actor's
handler — it gets a fresh root and **cannot see the caller's running actors**. A
failure inside the lambda is not lost; it surfaces as an error at the `await`
that collects it. Its output is **live** — it prints to the same place as you,
as it happens, so a spawned `println` may land in either order relative to
yours, though never split mid-line. (An actor is the opposite: it buffers and
delivers its output at the reply, so actor lines never interleave.)

> **Await what you spawn.** A handle is a value you can hold, pass, and await
> from anywhere, but a thread nobody awaits is detached: its result is never
> collected, and it may be cut off when the program ends. There is no registry of
> threads and no automatic wait at the end — the handle is the only way back to
> the work.

Two `Kernel` functions drive them, and `Thread.spawn` / `Thread.await` (and
`Thread.await_all` for a list of handles) are the client-side names to prefer:

| Call | Purpose |
| ---- | ------- |
| `spawn_thread(f)` | Run the no-argument lambda `f` on its own thread; return a handle at once. |
| `await_thread(t)` | Wait for thread `t` and return what its lambda produced. |

### Tests

A test is a function. There is no test DSL and nothing registers itself: a
module implements `Test`, and every public function whose name begins with
`test_` is run by `ramos test`.

```
# src/test/my_app/user_test.ramos
module MyApp.UserTest
  # @module_doc
  # The behaviour a User promises its callers.
  implements Test

  alias MyApp.User

  fn test_user_hello()
    # @doc
    # A user greets by name.
    user = User{name: "Andrew"}
    assert(user.hello() == "Ola Andrew")

  fn test_default_age()
    # @doc
    # A user built with no age is 18, not 0.
    assert(User{}.age == 0)
```

```
ramos test                              # everything under src/test/
ramos test src/test/my_app/user_test.ramos   # one file
ramos test --quietly                    # results only, no doc lines
```

The report carries the [documentation comments](#documentation-comments) along
with the results, so it says what the suite covers and not only that it ran:

```
MyApp.UserTest
  The behaviour a User promises its callers.
  ok test_user_hello
     A user greets by name.
  FAIL test_default_age
      assertion failed: expected 0 to equal 18
     A user built with no age is 18, not 0.

2 test(s): 1 passed, 1 failed
```

A test with no `@doc` simply prints its name, and `--quietly` drops every doc
line — for a CI log, or a run whose tests you already know by heart.

`ramos test` exits non-zero when anything fails, so it drops straight into a
build script.

Tests live under **`src/test/`** and follow the same file rules as everything
else — the namespace is the path, so `MyApp.UserTest` is
`src/test/my_app/user_test.ramos`. A test module's name must **end in `Test`**,
and it must `implements Test`: the name makes it read as a test in a failure
report, and the trait makes being run something a module opts into rather than
something its name causes. A test reaches the code it exercises through the
ordinary `alias`, resolved against `src/`.

Only public `test_` functions run, each with no arguments, so a module can keep
`fnp` helpers beside them. A test stops at its first failed `assert` and the run
continues with the next test.

#### assert

`assert(condition)` is a `Kernel` function. It returns `:ok` when the condition
holds — any truthy value, since only `false` and `nil` are falsy — and fails the
test otherwise. A failure travels the ordinary error path; nothing is thrown,
because Ramos has nothing to throw.

When the condition is a comparison, the failure reports both sides, which is
usually the whole story:

```
assert(name == "andrew")
# assertion failed: expected "Andrew" to equal "andrew"

assert(age < 18)
# assertion failed: expected 21 to be less than 18
```

A second argument replaces the generated message when the comparison is not the
point:

```
assert(is_valid(user), "a user with no email should be invalid")
```

## Pipes

`|` is the pipe, and the only one: it passes the value on its left as the
**first argument** to the module function on its right, so data flows
top-to-bottom.

```
{}
| Map.put(:new_value_a, 100)
| Map.put(:new_value_b, 50)
| Map.put(:new_value_c, 1)
# == {new_value_a: 100, new_value_b: 50, new_value_c: 1}
```

`a | M.f(b)` means exactly `M.f(a, b)`, so a pipeline is a chain of ordinary
calls read in the order they happen. Naming the module at each step is the
point: the value's type is on the page rather than inferred from the call.

```
["ana", "bob"]
| List.map(do n -> String.upcase(n))
| List.join(", ")
# == "ANA, BOB"
```

## Lambdas

`do` opens an anonymous function — a **lambda**. Single-expression lambdas
use `->` and fit on one line:

```
add = do x, y -> x + y
add(1, 2)          # == 3
```

Multi-statement lambdas drop the `->` and indent the body; the last expression
is the return value:

```
double_then_add =
  do x, y
    z = x + y
    z * 2

double_then_add(2, 3)   # == 10
```

Lambdas are values, so they pipe naturally:

```
[1, 2, 3, 4]
| List.filter(do x -> Integer.is_even(x))    # == [2, 4]
| List.map(do x -> x * 10)                 # == [20, 40]
```

### Closures

A lambda closes over the scope it was written in, so its body can read the
names that surrounded it:

```
v = 1
lb = do x -> x + v

lb(2)   # == 3
```

A lambda may not refer to the name it is being bound to
(`f = do x -> f(x)`): lambdas are anonymous and non-recursive, and a named
`fn` is how you recurse.

## Standard library

Each type ships with a rich, pipe-friendly module. A few highlights (`Kernel`,
`String`, `List`, and `Tuple` have inline-documented sources under
[`stdlib/`](stdlib/); the remaining modules are not in this repo yet):

- **`Kernel`** — the only module whose functions you call *bare*: `Kernel` is
  implicitly in scope everywhere, so `print(x)` is sugar for `Kernel.print(x)`
  and no `alias` is ever needed for it. It hosts:
  - **console I/O** — `print`, `println`, `new_line`, `read`, `read_all`,
    `read_password`, and `eprint`/`eprintln` for diagnostics on standard error
  - **process / CLI** — `get_args`, `get_arg`, `get_env`, `sleep`, `exit`
  - **time** — `now` (wall clock) and `monotonic` (only moves forward, so it is
    the one to measure elapsed time with)
  - **randomness** — `random`, `random_int` (not cryptographic)
  - **collections** — `size`, `at`, `to_list`, each working on a list, tuple or
    map, which is why the type modules do not repeat them; plus `is_empty`,
    which also spans strings
  - **conversions** — `to_string` (display form) and `inspect` (debug form: the
    rendering the REPL prints), `to_integer`, `to_float`, and `type_of`
  - **predicates** — the `is_*` family (`is_integer`, `is_string`, …) and
    `is_truthy`, the falsy rule written down
  - **calling** — `apply(f, args)`, to call a lambda whose arity the call site
    does not know
  - **actors** — `start_actor` / `call_actor` (see [Actors](#actors))
  - **threads** — `spawn_thread` / `await_thread` (see [Threads](#threads))
  - **seams** — `native(str, args)`, plus `current_stacktrace`

  Every other module must be referenced by name (or `alias`ed).
- **`Integer`** — `compare`/`clamp`/`min`/`max`, `times`/`upto`/`downto`,
  `gcd`/`lcm`, `abs`, `digits`, predicates (`is_even`, `is_odd`, `is_zero`,
  `is_positive`, `is_negative`).
- **`Float`** — `round`/`floor`/`ceil` (all returning an `Integer`),
  `abs`/`sqrt`/`min`/`max`/`clamp`/`compare`, constants (`pi()`, `e()`,
  `infinity`, `nan`), predicates (`is_nan`, `is_infinite`, `is_finite`).
- **`String`** — `<>` / `at` / `repeat`, casing, trimming, `split`/`replace`
  (joining a list of strings is `List.join`), `slice`, `find`/`contains`,
  conversions.
- **`List`** — `map`/`filter`/`reject`/`reduce`/`flat_map`, `sort`/`uniq`,
  `chunk_every`/`group_by`/`partition`, `append`/`insert`, slicing, search,
  aggregates.
- **`Map`** — `get`/`put`/`delete`/`update`/`merge` (`++` is sugar for `merge`),
  `filter`/`reject` to screen pairs with a `(key, value)` predicate,
  `keys`/`entries`/`from_list`, `map_keys`/`map_values`/`map_entries`,
  `has_key`. Keys are integers, strings and symbols only.
- **`Tuple`** — `set`, `last`, `from_list`/`to_map`. Read by position with
  `at(t, 0)`, or destructure: `(a, b) = pair`.
- **`Struct`** — `get`/`put`/`update`, `to_map`, `is_a`, `keys`/`values`.
  Instances are built with the literal `Name{...}` syntax.
- **`Date`** — a calendar date (`year`/`month`/`day`, no time-of-day or time
  zone): `new`/`today`/`from_epoch_day`/`parse`, `to_epoch_day`/`to_iso`,
  `add_days`/`add_years`, `compare`/`diff_days`/`day_of_week`,
  `is_leap_year`/`days_in_month`. An out-of-range `day` normalizes rather than
  raising.
- **`NaiveDateTime`** — a `Date` plus `hour`/`minute`/`second`/`millisecond`,
  with no time zone: `new`/`now`/`from_epoch_millis`/`parse`,
  `to_epoch_millis`/`to_iso`, `add_days`/`add_hours`/`add_minutes`/
  `add_seconds`, `compare`/`diff_millis`/`day_of_week`, `date` (drops the time
  of day).
- **`TimeZone`** — a name paired with a fixed offset from UTC (no IANA
  database — no daylight saving, no historical rules, just `-720`..`840`
  minutes checked against the real-world range): `utc`/`fixed`/
  `from_offset_minutes`/`parse`, `offset_text`/`parse_offset_text` (the
  shared `"Z"`/`±HH:MM` text form `DateTime.to_iso`/`parse` also use),
  `compare`.
- **`DateTime`** — a `NaiveDateTime` plus a fixed `offset_minutes` from UTC
  (no time zone database — a plain numeric offset, not a zone name):
  `new`/`now`/`now_at`/`now_in`/`from_epoch_millis`/`from_naive`/`parse`,
  `to_utc_epoch_millis`/`to_iso`/`to_naive`/`to_utc`/`with_offset`/
  `in_time_zone`/`time_zone`, `add_days`/`add_hours`/`add_minutes`/
  `add_seconds`, `compare`/`diff_millis`/`day_of_week`. `compare`/
  `diff_millis` order by the instant, not the printed local fields; `now_in`/
  `in_time_zone` take a `TimeZone`, the rest a bare `offset_minutes`.
- **`Actor`** — the trait a module implements to hold state and answer
  messages; driven by `Kernel`'s `start_actor` / `call_actor`.
- **`Global`** — one process-wide map held by an actor: `start`, `get`, `put`,
  `clear`.
- **`Config`** — the environment's `.env` file, read once at `start` and
  answered from memory: `get(section, key)`, `path`. Read-only. Shared mutable state; see [Global](#global) before reaching for it.
- **`Thread`** — one-shot parallel work: `spawn`, `await`, `await_all`, and a
  parallel `map`/`each`; see [Threads](#threads).
- **`Test`** — the marker trait a test module implements; see [Tests](#tests).
- **`Module`** — `functions`, `apply`, `is_open`/`define`.
- **`Nil`** — `or`/`or_else` nil-coalescing, `unwrap`/`expect`, predicates.

## Command line

This repository *is* the interpreter — a Rust crate whose `src/` holds the
implementation and whose [`stdlib/`](stdlib/) holds the Ramos standard library.
Build it once and the `ramos` binary drives everything:

```
cargo build --release        # binary at target/release/ramos
```

The binary takes a subcommand and (except for the REPL) a single `.ramos` file:

| Command                | What it does |
| ---------------------- | ------------ |
| `ramos run <file.ramos>`   | Execute a program — calls the file's [entrypoint](#entrypoints) `main()` |
| `ramos repl`             | Start an interactive read-eval-print loop |
| `ramos check <file.ramos>` | Enforce the strict rules and parse the file **without running it** |
| `ramos ast <file.ramos>`   | Print the parsed abstract syntax tree |
| `ramos lexer <file.ramos>` | Print the raw token stream |
| `ramos test [--quietly] [file]` | Run the tests under `src/test/`, or one file's — `--quietly` leaves the `@doc` lines out |
| `ramos doctest [--stdlib DIR] [DIR]` | Run the `# ==` examples in `DIR/src/*.ramos`'s doc comments |
| `ramos doc`              | Generate Hexdocs-style HTML for [`stdlib/`](stdlib/) into `docs/` |

> `run` loads the standard library (embedded in the binary), then the entry
> file and every module it reaches, then calls the entrypoint's `main()` — or
> runs top-level statements when the file has none. `--stdlib DIR` reads the
> stdlib from disk instead, which is how the stdlib itself is developed.
>
> `ramos doc` reads the inline [`@module_doc`](#documentation-comments) / `@doc`
> comments out of every `.ramos` file under [`stdlib/`](stdlib/) and writes a
> static site — one HTML page per module plus an `index.html` overview — into
> `docs/`, alongside an `examples.html` guide built from the feature fixtures in
> [`tests/fixtures/features/`](tests/fixtures/features/) — each fixture's header
> comment becomes the prose, its code the snippets, so the guide can't drift
> from code that lexes and parses. Options: `--stdlib DIR` (source, default
> `./stdlib`), `--out DIR` (output, default `./docs`) and `--examples DIR`
> (fixtures, default `./tests/fixtures/features`; a missing dir just drops the
> page). Open `docs/index.html` to browse it.

### Running a program

```
ramos run app.ramos        # loads the stdlib, then app.ramos, then calls App.main()
```

`run` calls the entrypoint's `main()` (see [Entrypoints](#entrypoints)); a file
of bare top-level statements runs top-to-bottom instead.

### Checking without running

`check` is `run` up to but not including execution — it loads the entry file and
everything it reaches, enforcing every [strict rule](#strict-rules-fails-interpreter)
along the way and reporting the first violation with file, line, column and a
source excerpt. Because it loads the whole program, it also catches a module
that no file defines. Ideal for editors and CI:

```
ramos check src/my_app/business/system_user.ramos
```

### The REPL

```
ramos repl
```

An interactive session: type an expression, see its value. The prompt is
indentation-aware, so multi-line `case`, `cond`, `fn` and `do` blocks carry
across lines.

### Inspecting the lexer and AST

Two debug commands expose the interpreter's internals — useful when working on
the language itself rather than a program:

```
ramos lexer app.ramos      # the INDENT / DEDENT / NEWLINE token stream
ramos ast   app.ramos      # the parsed tree
```

Add `--dump` to either to print the source alongside the output. Both views are
**syntax coloured** from the lexer's own token spans when stdout is a terminal;
colour honours `NO_COLOR` and can be forced with `--color` / `--no-color`:

```
ramos ast --dump app.ramos --color | less -R   # keep colour through a pager
ramos lexer app.ramos --no-color               # force plain output
```

The interpreter runs on a large stack (256 MB, a virtual reservation that costs
only the pages it touches), because the standard library recurses once per list
element. `RAMA_STACK_SIZE` resizes it when a program recurses deeper still — or
to prove the point with less:

```
RAMA_STACK_SIZE=1G ramos run deep.ramos    # 1 gibibyte; also accepts 512M, 64K, a byte count
```

## Project layout

```
.
├── README.md                # this file — language tour
├── Cargo.toml               # the `ramos` Rust crate (lib + binary)
├── src/                     # the interpreter (Rust): lexer, parser, interp/…
├── stdlib/                  # standard-library modules in Ramos (documented inline)
│   ├── kernel.ramos  integer.ramos  float.ramos  list.ramos
│   ├── map.ramos     string.ramos   tuple.ramos
│   └── file.ramos    dir.ramos
├── tests/                   # Rust integration tests + .ramos fixtures
└── editors/
    └── neovim/              # syntax / indent / filetype support
```

## Editor support

Syntax highlighting, indentation and filetype detection for `*.ramos` are
available for **Neovim**. See [`editors/neovim/README.md`](editors/neovim/README.md)
for install options (symlink, `:packadd`, or `runtimepath`).
# ramos
