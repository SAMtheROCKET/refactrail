# Two independent engines, no Ruff code

Owner decision, 2 October 2026:

1. RefacTrail contains **no part of Ruff**. The Rust core's current
   dependency on Ruff's parser crates (`ruff_python_parser`,
   `ruff_python_ast`, `ruff_text_size`) is removed and replaced by our own
   lexer, parser and syntax tree. Ruff's published *ideas* (hand-written
   lexer, recursive-descent parser, single parse per file, parallel files,
   content-hash caching) may inform the design; its code may not be used.
2. There are **two complete engines** with the same operations and
   byte-identical results:
   - **Python engine**: standard library only, no Rust needed. Easiest to
     embed and extend.
   - **Rust engine**: no Python at runtime. A native `refactrail` binary and
     a Rust library; the PyO3 module is only an optional way for Python
     callers to reach it. The Rust engine never calls back into CPython
     (today it does, for `compile()` validation; that goes away).
3. Callers choose: Rust for speed, Python for easy integration. Either
   engine may be the only one installed.

Third-party crates that are not Ruff (rayon, regex, pyo3) remain allowed.
Speed is claimed only from recorded benchmarks; "faster than Ruff" is a
goal to measure, not a promise.

## Operation contract

Every operation exists in both engines, takes the same inputs and returns
the same bytes (JSON, text, SARIF or source). The contract lives in
`docs/RULES.md` and the JSON schemas; parity tests enforce it.

| Operation | Input | Output |
|---|---|---|
| `tokens` (debug) | source bytes | token stream in CPython `tokenize` terms |
| `parse` (debug) | source bytes, Python version | syntax tree dump in CPython `ast.dump` form, or a syntax error (line, column, message class) |
| `check` | files, settings | RT findings |
| `lint` | files, selection | RC findings |
| `scope` | file | `refactrail-scope-1` JSON |
| `format` | file, width, bracket style | formatted source / diff |
| `index` | import root | `refactrail-index-1` JSON |

## The oracle: CPython itself

The Python engine uses CPython's own `tokenize`, `ast`, `symtable` and
`compile()`. Each Rust component is proven against them before it
replaces anything:

| Rust component | Differential test against CPython | Corpus |
|---|---|---|
| Lexer | identical token types, strings and positions | stdlib + ~13k installed files + projects |
| Parser | identical `ast.dump` for every file | same |
| Syntax errors | same accept/reject decision; same line | corpus + crafted invalid sources |
| Compile-time checks (`return` outside function, scope errors, ...) | same accept/reject as `compile()` | crafted cases + corpus |
| Symbol table | same scopes and bindings as `symtable` | corpus |
| Rules, lint, scope, format | identical outputs to the Python engine | corpus + fixtures |

The grammar target is a Python version (3.12, 3.13 or 3.14). The Python
engine reports what its interpreter accepts; the Rust engine is told the
same version, so parity is per version.

## Rust layout (planned)

```text
rust/
  crates/lexer      tokens, f-string/t-string modes, positions
  crates/parser     recursive descent -> own syntax tree (index arena)
  crates/semantic   scopes, symbol table, compile-time errors
  crates/rules      RT rules and RC lint on the own tree
  crates/format     spacing and statement layout
  crates/cli        native binary, same arguments and outputs
  crates/python     optional PyO3 bindings
```

## Increments and acceptance evidence

| # | Deliverable | Done when |
|---|---|---|
| 1 | Rust lexer | token parity with `tokenize` on the full corpus (all 3 versions) |
| 2 | Rust parser and tree | `ast.dump` parity on the full corpus; error parity on invalid sources |
| 3 | Compile-time and symbol checks | `compile()` and `symtable` parity; core no longer calls CPython |
| 4 | RT rules on the own tree | existing RT parity (13k+ files) with Ruff crates removed from Cargo |
| 5 | Native CLI | same outputs as `python -m refactrail` for check; no Python installed |
| 6 | RC lint and scope in Rust | output parity |
| 7 | Formatter in Rust | output parity, idempotence, AST validation |
| 8 | Benchmarks | recorded comparison with Ruff and the Python engine |

FuncLoom (functions, modularize, refine) stays Python-first; a Rust port
is a later, separate decision.

## Status, 2 October 2026 (Python 3.12 grammar)

| # | Status | Evidence |
|---|---|---|
| 1 Lexer (`crates/lexer`) | done | 19,403/19,403 corpus files and 170,000/170,000 fuzzed broken files give tokens and errors identical to `tokenize` (`scripts/lexer_parity.py`, `scripts/lexer_fuzz.py`) |
| 2 Parser (`crates/parser`) | done | `ast.dump(..., include_attributes=True)` identical on 19,403/19,403 files; 60,000 fuzzed broken files: accept/reject identical, error line identical on 99.96% (`scripts/parser_parity.py`, `scripts/parser_fuzz.py`); 34 recorded CPython cases in `tests/recorded.rs` |
| 3 Compile checks (`crates/parser/src/compile.rs`) | done | `compile()` outcome identical on 19,403/19,403 files; on 40,000 fuzzed broken files accept/reject identical and (line, offset, message) identical on ~99.1% (`scripts/compile_parity.py`) |
| 4 RT rules on the own tree (`crates/engine`) | done | Ruff crates removed from `Cargo.toml`; RT parity 0 differences on 13,545 files x 2 profiles |
| 5 Native CLI (`crates/cli`, `refactrail-native check`) | done | 34/34 outputs (text, json, github, sarif; options; errors) byte-identical to `python -m refactrail check` |
| 6 RC lint and scope in Rust (`crates/parser/src/symtable.rs`, `crates/engine/src/{lexical,correctness,lint}.rs`, `refactrail-native lint`, `refactrail-native scope`) | done | Symbol tables identical to CPython's `symtable` (raw flags and scopes, every table) on 19,406/19,406 files (`scripts/symtable_parity.py`); the same implementation now raises `compile()`'s symbol-table errors (corpus and fuzz parity unchanged). Lint findings identical to `refactrail lint` on the standard library (423), the Big-Project tree (36) and other projects (`scripts/compare_lint.py`); scope documents byte-identical on samples (`scripts/scope_parity.py`) |
| 7 Formatter in Rust (`crates/engine/src/{format,format_wrap,notebook}.rs`, `refactrail-native format`) | done | Proposals (unified diffs, `difflib`-identical) identical to `refactrail format` on 6,490/6,490 corpus files in each of spacing, 79-column own-line and 100-column hug modes, and on 22/22 notebooks in every mode (`scripts/format_parity.py`, `--suffix=.ipynb`) |
| 8 Benchmarks | done for `check`, `lint`, `format` | see below |
| 9 Native index (`crates/engine/src/index.rs`, `refactrail-native index`) | done | refactrail-index-1 documents byte-identical to `refactrail index` on 9/9 roots, with and without `--changed` |
| 10 Engine choice in the Python CLI | done | `refactrail lint` and `refactrail format` take `--engine auto\|python\|rust` (as `check` does); 31/31 lint/format runs (text, json, sarif; spacing, wrap, hug; notebooks; errors) byte-identical between `--engine rust` and `--engine python`; regression tests in `tests/test_engine_parity.py` |

The native CLI's `lint`, `scope`, `format` (including `--write`) and
`index` outputs are byte-identical to the Python CLI in 37/37 + 9/9
recorded comparisons, including refusals and exit codes.

With `--engine rust` the PyO3 core lints byte snapshots on threads and
formats documents; for a file that does not compile, or a document the
formatter refuses, the Python engine runs instead so the refusal keeps
CPython's exact message. The Python CLI's wall time on the standard
library (Linux, 4 cores, including interpreter start-up) fell from 7.0 s
to 1.3 s for `lint` and from 25.6 s to 3.6 s for `format --line-length
79`.

The current crates are `lexer`, `parser` (tree, compile checks), `engine`
(rules, pure Rust), `cli` (native binary) and the root `refactrail-core`
(thin PyO3 wrapper). The syntax tree is typed: `crates/parser/src/ast.rs`
is generated by `scripts/gen_ast.py` from CPython 3.12's ASDL, together
with its exact `ast.dump` writer and a visitor that walks children in
CPython's field order. Do not edit `ast.rs` by hand; change the generator.

Performance design, each step verified against every parity suite:

- The lexer writes the parser's token arrays directly through a
  `TokenSink` (no intermediate token vector) and reports byte columns in
  parser mode; character columns are computed only for error offsets.
- Identifiers and integer literals use `SmallStr` (up to 22 bytes inline,
  no allocation).
- Keywords and operators are compared as one-byte symbol codes; binary
  operators use precedence climbing; the expression levels are inlined.
- Symbol tables keep names and flags in small vectors per scope (a hash
  index only past eight names), resolve scopes by reference instead of
  copying name sets, and run the analysis pass only when a `global` or
  `nonlocal` directive exists (the only way it can fail) for `check`.
- Deep nesting runs on 64 MiB analysis-stack threads in both the CLI pool
  and the PyO3 module; the Python engine raises its recursion limit for
  the same inputs.
- Line splitting uses `memchr`; the encoding check reads two lines only.
- The CLI reads and checks each file in the same parallel task.

A possible further speed-up, not done: allocating the tree in an arena
would remove most of the remaining allocation and drop cost (about 5% of
a check).

Identifiers follow CPython's own rules (3 October 2026). Non-ASCII
names are NFKC-normalized like CPython's parser (`ﬁle` is the name
`file`; `crates/parser/src/nfkc.rs`, tables from
`scripts/gen_nfkc_tables.py`): 1,412,063/1,412,063 strings identical to
`unicodedata.normalize` in each of two seeds (every code point plus mixed
combining, Hangul and compatibility sequences; `scripts/nfkc_parity.py`).
The lexer takes every non-ASCII character into a name as the C tokenizer
does, and in parser mode rejects non-identifiers with CPython's exact
message and column (`invalid character '€' (U+20AC)`, `invalid
non-printable character U+00A0`; tables from
`scripts/gen_identifier_tables.py`). Before this fix both cases were
accepted or worded differently. On 20,000 generated files with random
non-ASCII names, tokens, `ast.dump` and `compile()` outcomes are identical.

Known differences, documented rather than hidden:

- Parity is proven for the Python 3.12 grammar only; 3.13/3.14 need
  their own oracle runs.
- On ~0.9% of syntactically broken files the native engine's syntax
  error message or offset differs from CPython's (both reject the
  file). The PyO3 module therefore still asks CPython for the exact
  message of a rejected file, so the two engines' findings stay
  identical; it never calls CPython for files that compile.
- `\N{...}` aliases come from the Unicode Consortium's NameAliases.txt,
  validated against `unicodedata.lookup`.
- The same ~0.9% applies to `lint`, `scope` and `format` refusals of
  broken files: both engines refuse; the native message can differ.
- Builtin names (RC201) and Python's regular-expression `\w` (import
  mentions, `fmt:` directives) come from tables generated from CPython
  3.12 by `scripts/gen_lexical_tables.py`; other versions need their own
  tables.
- `refactrail-native format --output-format json` reports
  `"engine": "refactrail-rust"`; every other field matches.
- Error texts for operating-system failures (unreadable files) are worded
  by each engine's platform library.
- A notebook string holding a lone UTF-16 surrogate escape is read as
  U+FFFD by the native engine (Python keeps the surrogate).

Benchmark, 3 October 2026 (200 stdlib files, 3.6 MB, 4 CPUs, WSL2,
median of 7 runs, process start and file reading included; the
`stdlib200` corpus of funcloom-m2a/reports/tool-comparison-20261001,
Ruff with `--no-cache`, `RAYON_NUM_THREADS` for its thread count):

| Command | 4 threads | 1 thread |
|---|---:|---:|
| `refactrail-native check --profile strict` (20 RT rules + full compile checks) | 34 ms | 81 ms |
| `refactrail-native check` (standard profile) | 32 ms | — |
| Ruff, broad selection (E,F,D,ANN,N,PL) | 232 ms | 298 ms |
| Ruff, E,F only | 40 ms | 82 ms |

Earlier record (2 October, generic tree): strict 54 ms / 141 ms.

So on this corpus the native engine is about 7x faster than Ruff for a
comparable broad rule set (3.7x single-threaded), and at least as fast
as Ruff's minimal E,F set (about 15% faster with 4 threads, equal within
noise on one thread). In-process phases for the 200 files on one thread:
lexing 9.5 ms (382 MB/s), lexing and parsing 37.6 ms, plus compile checks
44.5 ms, full check 64.6 ms. These are measurements on one machine and
corpus, not guarantees.
