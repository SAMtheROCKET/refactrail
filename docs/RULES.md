# RefacTrail rule specification

## pycodestyle- and Pyflakes-compatible codes (R1a and R1b, unreleased)

`lint --select E4,E7,F` (or any prefix) adds codes with the meaning and
numbering of pycodestyle and Pyflakes, so existing `# noqa: E701` comments
and configurations carry over. They are computed by RefacTrail itself from
Python's tokenizer, syntax tree and compiler symbol tables; no other
linter's code is used. Ruff 0.16.9 serves only as an external test oracle
(`scripts/ruff_oracle.py`). For Ruff's whole default set (E4, E7, E9
and F; 58 codes, E902 aside) on the 6,988-file corpus, RefacTrail reports
19,827 of Ruff's 19,830 findings at the same code, line and column (17
more of its own); on the Python 3.12 standard library 2,050 of 2,054 (1
more). The remaining differences are listed below.

| Codes | Meaning |
| --- | --- |
| E401, E402 | Several imports on one line; a module import after other code. Docstrings, dunder assignments, `if`/`try`/`with` blocks and `sys.path`, `os.environ`, `matplotlib.use` or `pytest.importorskip` setup may come first. |
| E701, E702, E703 | Several statements on one line (colon or semicolon); a trailing semicolon. `def f(): ...` lines and `class C: ...` stubs are allowed. |
| E711, E712 | `==`/`!=` with None, True or False (not between two constants). |
| E713, E714 | `not x in y`, `not x is y`. |
| E721 | `==`/`!=` where a side is `type(...)` or a builtin class (also exceptions); `.dtype` comparisons are exempt. |
| E722 | Bare `except:` that does not re-raise. |
| E731 | A lambda assigned to a name. |
| E741, E742, E743 | The names `l`, `O`, `I` for variables, classes or functions. |
| F404, F407 | Late or unknown `from __future__` import. |
| F501-F509 | `%`-format problems: invalid string, mapping/sequence mismatch, unused or missing named arguments, mixed placeholders, count mismatch, `*` with a mapping, unsupported character. |
| F521-F525 | `str.format()` problems: invalid string, unused named or positional arguments, missing arguments, mixed automatic and manual numbering. |
| F541 | An f-string without placeholders. |
| F601, F602 | A dict key (literal or variable) repeated. |
| F621, F622 | Too many or two starred targets in an assignment. |
| F631, F632, F633, F634 | Assert or if on a non-empty tuple; `is` with a literal; `print >>`. |
| F701, F702, F704, F706, F707 | `break`/`continue` outside a loop, `yield`/`await`/`return` outside a function, bare `except:` before other handlers. |
| F722 | A string annotation that is not a valid expression. |
| F901 | `raise NotImplemented`. |
| F401 | An import never read. Explicit re-exports (`import x as x`), names in `__all__` (top-level `=`, `+=`, `.append()`, `.extend()`), `__future__` and class-body imports are exempt; a read of a package name uses every `import package.sub` before it. |
| F402 | A `for` loop variable shadowing an import. |
| F403, F405, F406 | `from m import *` used; a name that may come from it; a star import outside module level. |
| F811 | A definition, class or import replaced while unused in the same branch, or shadowed by a function's parameter or first local binding; `@overload`, property setters, `_` names and `TYPE_CHECKING` imports are exempt. |
| F821 | An undefined name, following execution order: module code sees only earlier bindings, function bodies see the finished module; `except NameError` guards are honoured. |
| F822 | A name in `__all__` that the module never binds (package submodule files are fine). |
| F823 | A local read before its assignment while an outer binding exists. |
| F841, F842 | A local (or `except ... as` name) assigned or annotated but never read; tuple unpacking, `_` names and `locals()` users are exempt. |

The compiler rejects the F404/F407/F62x/F70x cases outright. When the
matching F code is selected, linting continues with the syntax tree and
reports them as findings; if that finding is suppressed or not selected,
RT001 is still reported, so a file that does not compile is never clean.
`# ruff: noqa` and `# flake8: noqa` (optionally with codes) exempt the
whole file in both engines.

The scope codes use a flow-ordered binding model of their own
(`compat_bindings.py`, `compat_scope_checker.py`); RefacTrail's compiler
scope analysis now also understands PEP 695 type parameter scopes.
`# noqa` on the first line of a multi-line import or `__all__` statement
applies to the names inside it.

Known differences from Ruff on the corpora (about 25 findings in
26,000): a function that rebinds itself through `global` (pydoc's
`pager`), loop variables in some functions shadowing a module import
(F402), a few F811 cases with names re-imported in `try` bodies or
redefined after `if`/`else` definitions, one `except ... as` name in a
handler that only re-raises, and three F401 cases involving submodule
imports. E902 (unreadable file) is reported as RT002 instead.

Both engines implement every code above (R1c): the Rust engine's
findings are identical to the Python engine's on the Python 3.12
standard library (2,066 findings) and the 6,988-file corpus (21,552
findings), as well as on edge-case smoke files. Files the compiler
rejects are linted by the Python engine. Timing on a 24-thread machine,
the standalone `refactrail-native lint --jobs 0 --select E4,E7,F` takes
0.11 s on the standard library and 0.48 s on the corpus (0.71 s before
the R1d work below); Ruff 0.16.9 takes 0.04 s and 0.37 s. On one thread
the corpus takes 3.2 s against Ruff's 1.8 s. The Python engine takes
2.5 s and 22.5 s. Ruff remains faster; RefacTrail is not advertised as
faster than Ruff.

R1d speed work, with findings unchanged on every corpus: the parser hands
its token stream to the lint checks instead of tokenizing twice; the
name resolution E721 needs runs only when a comparison could be a type
comparison; the bound-name set is computed on first use; the scope
checker shares branch paths, borrows names from the syntax tree and
walks expressions without building child lists. Profiling shows the
remaining time in tokenizing and parsing (about 27%), the scope checker
and other tree walks (about 34%) and the compiler checks (about 12%).

### Safe fixes (`lint --fix`, `lint --diff`)

`lint --fix` rewrites selected, unsuppressed findings of six codes;
`--diff` prints the same changes without writing. Other findings are
reported as usual afterwards.

| Code | Fix |
| --- | --- |
| F401 | Remove the unused name, or the whole import statement. A block left empty gets `pass` (keeping the last statement's trailing comment). Parentheses, line breaks and trailing commas of the remaining names stay. |
| F541 | Drop the `f` prefix; `{{` and `}}` become `{` and `}`. |
| F632 | `is` / `is not` with a literal become `==` / `!=` (one-operator comparisons). |
| E703 | Delete the semicolon; spaces before it stay, as in Ruff. |
| E713, E714 | `not x in y` becomes `x not in y`, `not x is y` becomes `x is not y`; brackets that only wrapped the comparison are removed. |

Safety rules: F401 is not fixed in `__init__.py` files and `.pyi` stubs
(imports may be re-exports), in a `try` body that handles `ImportError`
(the import is an availability check), on a line shared with another
statement, or when a cut would delete a comment; each such finding is
reported as a note. Removing an import also removes its import-time side
effects, as Ruff's fix does. Every pass is verified: the fixed text must
compile to exactly the original syntax tree with the intended changes
(f-strings of constants compared as the strings they equal); fixes that
fail are left out with a note, and overlapping fixes wait for the next
pass (for example `not x is ""` becomes `x != ""` in two passes). Files
are written atomically, keeping line endings and a BOM, and only if they
are unchanged since they were read.

Oracle comparison with Ruff 0.16.9 (`--select F401,F541,F632,E703,E713,E714
--fix` on copies): on the Python 3.12 standard library both change the
same 56 files and every fixed file is byte-identical. On the 6,988-file
corpus RefacTrail changes 443 files and Ruff 471; apart from stubs and
notebooks (not fixed by `lint`), 8 files differ: guarded imports that
RefacTrail keeps (5), two F401 findings where the linters disagree, and
one multi-line condition whose line break RefacTrail keeps. No pass was
discarded.

## Scope and formatting expansion (0.3.0a0)

RC201 reports a loaded name for which compiler lexical scope information
finds no binding and which is neither a builtin nor an implicit module name.
RC202 reports an imported binding with no lexical read. Explicit `as name`
re-exports, `__init__.py`, stubs, future imports and names mentioned in string
literals or real type comments are exempt from RC202. Assignment alongside an import is ambiguous
and exempt. These are review warnings, never permission to remove imports.
Wildcard imports, dynamic namespace access and PEP 695 type scopes make the
scope pass incomplete; it records limitations and suppresses RC2 diagnostics.
Annotations are tracked as uses but excluded from RC201 (forward references).
Bindings are lexical, not proof of initialization, reachability or runtime
types. Branches, deletion, exception cleanup and call timing remain unknown.
CLI `scope` exposes these facts and limitations in JSON; no target is run.

Formatting additionally supports opt-in `--line-length 40..200` layout of
over-long statements. A statement is joined (same-line spacing kept) and
split again only inside existing brackets: the outermost bracket pair is
chosen (right-most first; left-most first for `def`/`class` headers, so
parameters split before a return annotation), preferring the fitting
layout with the fewest lines; a bracket holding a single bracketed
element (such as `f(*(...))`) is split inside, in the hug style. Inside a
bracket, contents stay on one line when they fit and are otherwise split
at the strongest delimiter present: comprehension `for`/`if` clauses,
commas (not lambda parameters), conditional `if`/`else`, `or`, `and`,
adjacent string literals, comparisons, then binary arithmetic. Long
`from module import a, b` lines may gain parentheses (the syntax tree is
unchanged); continuation lines inside brackets are re-packed at commas.
`--bracket-style own-line` (default) puts a closing bracket on its own
line and one element per line when a list is split; `--bracket-style hug`
keeps closing brackets on the last content line, packs elements while
they fit and indents compound-statement headers by eight spaces. Only
over-long statements are re-laid out. Long indivisible atoms (a single
long string or f-string) remain long. Statements containing comments,
multiline strings, explicit continuations or disabled regions are
protected; files with type-ignore directives do not change line
structure. Spacing is normalized again after layout, so one run reaches
a fixed point. Line endings and literal/comment spellings remain preserved.
`.pyi` files are discovered automatically. `--notebooks` includes notebooks
when walking folders; explicit notebooks are supported by `format`.
Only Python code cells are formatted; markdown, outputs, metadata and cell
order are preserved. Unsupported notebook syntax (including magics) refuses
the complete notebook with cell and line context, without partial writes.
Notebook edits preserve original JSON bytes outside changed source values.

## Initial correctness family (0.2.0a0; retained in 0.3.0a0)

`refactrail lint` checks general Python correctness patterns independently
of the owner's style profile. The RC family is implemented in the Python
engine using CPython AST/tokenize; it does not invoke Ruff or Black. The
existing `check` command and optional native core retain the RT contract
below. RC native acceleration is not implemented. Findings are warnings,
not proofs of bugs, and have no automatic semantic fixes in this increment.
`--select`/`--ignore` and real comment-token `noqa` suppression apply.
Compilation/encoding failures remain unsuppressible RT001/RT002 errors.

| Rule | Exact first-increment contract | Location |
|---|---|---|
| RC101 | Function/async-function/lambda default is a list, dict, set or comprehension display, or a tuple recursively containing one. Calls and annotations are not inferred. | Default expression |
| RC102 | A dictionary display repeats a hashable scalar/tuple literal key under Python builtin equality, including `1`, `True` and `1.0`. Only constants, signed numeric constants and literal tuples are evaluated; dynamic keys and unpacking reset the tracking window. | Later key |
| RC103 | `is`/`is not` compares either operand with a numeric, string, bytes or tuple literal. None, True, False and Ellipsis are excluded. | Whole comparison |
| RC104 | An except handler has no exception type. | Handler keyword |
| RC105 | A statement directly follows an unconditional return, raise, break or continue in the same statement list. Only the first unreachable statement of each list is reported. No reachability inference across branches. | First unreachable statement |
| RC106 | An assert test is a nonempty tuple display without starred entries. | Tuple test |
| RC107 | A return/break/continue occurs in a finally block. Nested function/class scopes are excluded. Break/continue inside loops wholly nested in that finally are excluded. | Control-flow statement |

All RC positions are 1-based Unicode character columns, with sorted
findings and source paths. Suspicious constructs may be intentional;
messages describe the syntax evidence without guessing domain intent.

## Base formatting contract (0.2.0a0; wrapping extended above)

`format` proposes bounded whitespace edits using CPython's AST and tokenizer,
without a formatter subprocess or formatter dependency. It normalizes
supported assignment/binary/comparison operator spacing, comma spacing and
trailing code whitespace, and adds a missing final newline. It preserves
indentation, existing line breaks, literal spelling and comments. Lines
containing comments, multiline literals, f-strings or explicit backslash
continuations are conservatively retained. `fmt: off/on/skip` comment
directives are honored. Without --line-length it retains existing line breaks. It does not sort
imports, rewrite quotes, change names, infer types or claim Black-compatible
formatting.

Both input and output must compile without execution and retain their
ASTs including type comments and type-ignore locations. Literal/comment
token spelling must also match. These are structural checks, not behavioral
equivalence proof. UTF-8/BOM and each original line ending are preserved;
other declared encodings are refused. Default and `--diff` are read-only;
`--check` returns 1 for pending changes. Only `--write` writes, with stale
SHA-256 checks and atomic replacement per file. Unsupported/read/validation
failures return nonzero and are never presented as formatted success.

## Contract update for 0.1.1a0

Read, encoding, internal and compilation errors cannot be suppressed by
rule selection/ignore lists. Unknown rich comparisons do not establish
a bool result: only identity/membership comparisons and comparisons of
scalar literals qualify for RT203 bool proposals.

RT001 includes contextual CPython compilation failures such as a module
level return, not merely a parsable syntax tree. Neither engine executes
the checked source. A supported interpreter must compile the source before
findings from either engine are accepted.

Suppression markers count only inside Python comment tokens; text within
string literals never suppresses findings. 

Strict profile adds RT206 for function return suffixes and RT207 for
parameter suffixes based on explicit, unshadowed builtin annotations:
`int`, `float`, `str`, `bool`, `list`, `dict`, `set`, `tuple`, `bytes`,
`complex`, and `None` (return suffix `none`). Subscripted list/dict/set/
tuple annotations use their outer type. Other annotations, unions, quoted
annotations and aliases remain unknown. These are declared types, not
runtime guarantees. Dunders, main, framework hooks, accessors and overloads
are exempt using RT202's exemptions; implicit receivers and variadic
parameters are exempt.
Messages: `Function 'name' declares return type dtype; use suffix '_dtype'.`
and `Parameter 'name' declares type dtype; use suffix '_dtype'.`
Neither rule renames public APIs or keyword parameters automatically.

Configuration lists must be lists/tuples of nonempty strings; widths are
40-200 and preferred function size cannot exceed its maximum. Direct
symlink/junction targets are rejected; traversal skips linked files and
directories. In-place fixes require unchanged original bytes and use an
atomic replacement; `--diff` writes nothing and fails on skipped inputs.

This file is the contract both engines implement. A finding is
`(path, line, column, code, severity, message)`. `line` is 1-based;
`column` is the 1-based position in Unicode characters (not bytes).
Findings are sorted by path, line, column, code, severity, then message. A behavior change in
either engine must first change this file and the parity tests.

## Settings and profiles

| Setting | standard | strict |
| --- | --- | --- |
| `line-length` | 79 | 79 |
| `function-preferred-lines` | 40 | 40 |
| `function-max-lines` | 50 | 50 |
| `main-max-lines` | 100 | 100 |
| RT203 (`<name>_<dtype>` suffix) | off | on |
| RT205 (generic names such as `data`, `temp`) | off | on |
| RT206 / RT207 (declared return / parameter suffixes) | off | on |
| Warnings section required (RT302) | no | yes |
| `__init__` return annotation required (RT402) | no | yes |

`select` / `ignore` take codes or prefixes (`RT2`). A comment token
containing `# noqa` (case-insensitive) suppresses ordinary rule findings
on that line; `# noqa: RT101, RT201` suppresses only the listed codes
(comma or space separated, prefixes allowed).

Files are read as UTF-8 (a leading BOM is ignored). A file that is not
valid UTF-8 or declares another encoding in a coding comment on its first
two lines gives one `RT002` error at 1:1 and no other findings. Files
that do not parse give one `RT001` error and no other findings; the
engines may differ in the RT001 position and wording, because parsers
word syntax errors differently, so parity covers only the code.

Physical lines are split at `\r\n`, `\r` or `\n` only.

**Positions.** Findings about a function or class are reported at its
name (the identifier after `def`/`class`). Findings about a module are at
1:1. Parameter findings are at the parameter name; variable findings at
the name; import findings at the start of the imported alias
(`numpy` in `import numpy as np`); `except ... as` findings at the
`except` keyword.

## Shared definitions

- **Function**: `def` or `async def` at any depth. Its **span** runs from
  its first decorator (or the `def` line) to its last line.
- **Method**: a function directly inside a class body. Its first
  parameter is **implicit** (`self`/`cls`) unless decorated with
  `staticmethod`.
- **Main block**: a top-level `if __name__ == "__main__":` (either operand
  order, `==` only).
- **Dunder**: a name that starts and ends with two underscores.
- **Constant candidate**: a top-level `NAME = value` or `NAME: T = value`
  with a single plain-name target, where `value` is an immutable literal
  (number, string, bytes, bool, `None`, a unary minus/plus on a number, or
  a tuple of these), the name is not a dunder and not `_`, and the name is
  bound exactly once at module level with no `global NAME` anywhere.
- **Type alias** (0.3.0a0): a top-level `type X = ...` statement, a
  top-level `X: TypeAlias = value` / `X: typing.TypeAlias = value`, or a
  top-level `X = value` with one plain-name target matching
  `^_?[A-Z][A-Za-z0-9]*$` (and not UPPER_CASE) whose value is a type
  expression: a dotted name, a subscript of a dotted name such as
  `tuple[ast.AST, int]`, or a `|` union of type expressions and `None`.
  This is a shape rule; the alias is not evaluated.
- **Bindings** of a scope (module, class, function, lambda, comprehension)
  are, in source order: function/class names, parameters, assignment,
  annotated-assignment and augmented-assignment targets (names, including
  inside tuples/lists/starred), `for` targets, `with ... as` targets,
  `except ... as` names, walrus targets (bound in the enclosing function
  scope), comprehension targets, `import ... as` / `from ... import`
  names, and `global`/`nonlocal` declarations (which make the name
  non-local to that scope). A name is reported once per scope, at its
  first binding.

## RT000 - internal error (error)

A rule raised an exception on this file. Reported at 1:1 with the rule
and exception; the remaining rules and files are still checked. Parity
tests treat any RT000 as a failure.

## RT001 - syntax error (error)

Reported at the parser's error position; message `Syntax error: <msg>`.

## RT101 - line too long (error)

Every physical line whose length in characters (without the line ending)
exceeds `line-length`. Column = `line-length + 1`.
Message: `Line has N characters; limit is L.`

## RT102 - constant not UPPER_CASE (warning)

A constant candidate whose name does not match `^_?[A-Z][A-Z0-9_]*$`.
Reported at the target. Message: `Constant 'x' should be UPPER_CASE
('X').`

## RT103 - constant after other code (warning)

A top-level assignment whose target name matches `^_?[A-Z][A-Z0-9_]*$`
(or a constant candidate) that appears after the first top-level statement
that is not a docstring, `__future__`/other import, constant candidate,
UPPER_CASE assignment, `__all__`/dunder assignment, type alias, or
`if TYPE_CHECKING:` block. Message: `Constant 'X' should be defined after the imports, before
other code.`

## RT201 - single-character name (warning)

A binding whose name is one character long, other than `_`. Import
bindings are exempt only when the imported name itself is one character
(`from x import y`). Message: `Name 'x' is a single character; use a
meaningful name.`

## RT202 - function name is not a verb (warning)

A function whose name, after stripping leading underscores and taking the
text before the first `_`, is not in the verb list (`VERBS` in the
reference engine; lowercase words such as `get`, `compute`, `load`,
`build`, `is`, `has`, `test`). Exempt: dunders, `main`, functions
decorated with `property`, `cached_property`, `functools.cached_property`,
`*.setter`, `*.getter`, `*.deleter`, `overload`, `typing.overload`, and methods named like known protocol hooks (`setUp`,
`tearDown`, `setUpClass`, `tearDownClass`, `run`, `visit_*` for
`ast.NodeVisitor`). Message: `Function 'x' should start with a verb
describing its operation.`

## RT203 - missing dtype suffix (warning, strict)

A function-local or module-level variable (not a dunder, parameter,
constant candidate, import, function or class) whose **first** binding is an
assignment whose value has narrow static type evidence, when the name does not end with
the matching suffix: int literal / `int()` / `len()` ->
`_int`; float literal / `float()` -> `_float`; str literal, f-string /
`str()` -> `_str`; `True`/`False` / `bool()` / identity, membership or scalar-literal
comparison / `not` ->
`_bool`; list display or comprehension / `list()` / `sorted()` -> `_list`;
dict display or comprehension / `dict()` -> `_dict`; set display or
comprehension / `set()` -> `_set`; tuple display / `tuple()` -> `_tuple`;
bytes literal / `bytes()` -> `_bytes`. Only plain single-name targets of
`=`, annotated assignment with a value, and `:=` count. Builtin calls
count only when that builtin name is bound nowhere in the file.
Message: `Variable 'x' holds a list; name it 'x_list'.` (the suggestion
is the name with trailing underscores removed, plus the suffix).

## RT204 - naming convention (warning)

Functions, parameters and variables (not constants, not imports) must
match `^_{0,2}[a-z][a-z0-9_]*_{0,2}$`; classes must match
`^_?[A-Z][A-Za-z0-9]*$`. Dunders, names made only of underscores (`_`),
framework hook methods (the protocol hook list) and constant candidates are
exempt, and
so are UPPER_CASE names bound in module or class scope (constants,
enum members) and module-level type aliases. Message: `Name 'x' should be snake_case.` /
`Class 'x' should be PascalCase.`

## RT205 - generic name (warning, strict)

A binding named one of `data`, `temp`, `tmp`, `val`, `value`, `var`,
`obj`, `res`, `result`, `ret`, `foo`, `bar`, `baz`, `thing`, `stuff`,
`info`, `item` (exactly). Message: `Name 'x' is generic; describe what it
holds.`

## RT301 - missing docstring (warning)

A module with at least one statement, and every class and function,
without a docstring. Exempt: functions decorated with `overload`, and
functions whose body is only `...` or `pass`. Reported at the name
(module: line 1, column 1).
Message: `Missing docstring in function 'x'.`

## RT302 - incomplete docstring (warning)

For a function with a docstring: the first line (after stripping) must be
non-empty (`summary`); an `Args:` section is required when the function
has parameters other than the implicit one; a `Returns:` (or `Yields:`
for generators) section is required unless the function is `__init__`;
in strict profile a `Warnings:` section is required too. A section is a
line whose stripped text is exactly `Args:`, `Arguments:`, `Parameters:`,
`Returns:`, `Yields:`, `Warnings:`, `Raises:`, or a NumPy-style heading
(`Parameters`, `Returns`, `Yields`, `Warnings` followed by a dashes line).
One finding per missing part. Message: `Docstring of 'x' has no Args
section.`

## RT303 - docstring arguments mismatch (warning)

When an Args section exists, entries are its lines indented deeper than
the heading (NumPy style: at the heading's own indentation, after the
dashes line) whose text starts with `name` followed by optional spaces and
`(`, `:` or end of line (`*args`/`**kwargs` stripped of stars). The entry
indentation is that of the first non-blank body line (NumPy: the
heading's). The section ends at another heading, or at a line indented
no deeper than the heading (NumPy: less than it). An entry named `None`
(the "no arguments" convention) is ignored. Each parameter (except the
implicit one) missing from the entries, and each entry that is not a
parameter, gives one finding. Message: `Docstring of 'x' does not describe
parameter 'y'.` / `... describes unknown parameter 'y'.`

## RT401 - parameter annotation missing (warning)

Every parameter without an annotation, except the implicit one of a
method. Message: `Parameter 'x' of 'f' has no type annotation.`

## RT402 - return annotation missing (warning)

Every function without a return annotation, except `__init__` in the
standard profile. Message: `Function 'f' has no return annotation.`

## RT501 / RT502 - long function

Span over `function-preferred-lines` gives RT501 (warning); over
`function-max-lines` gives RT502 (error) instead. Message: `Function 'f'
spans N lines; preferred L.` / `...; limit L.`

## RT503 - long main block (error)

A main block whose span exceeds `main-max-lines`. Message: `The main
block spans N lines; limit L.`

## RT504 - code outside functions (warning)

The first top-level statement that is not: a string or other constant
expression statement, import, function, class, main block,
`if TYPE_CHECKING:` / `if typing.TYPE_CHECKING:` block, `type X = ...`
statement, `=` or annotated
assignment whose targets are all plain UPPER_CASE or dunder names (computed
constants such as `PATTERN = re.compile(...)`), other `=` or annotated
assignment whose value contains no call anywhere, `pass`, or a `try`
statement all of whose inner statements (at any depth) are among these
kinds. One finding per file, at that statement's first line and column. Message: `Module runs code at
import time; move it into functions called from the main block.`
