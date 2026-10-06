# Changelog

## Unreleased

- `lint` gains 47 pycodestyle- and Pyflakes-compatible codes (E401-E743,
  F404-F901 without scope analysis), computed by RefacTrail's own engine.
  Ruff 0.16.9 used as an external oracle reports the same 6,009 findings
  on a 6,988-file corpus. Select them with `--select E4,E7,F`.
- R1b: the scope-based Pyflakes codes (F401, F402, F403, F405, F406,
  F811, F821, F822, F823, F841, F842) with a flow-ordered binding model.
  With them, Ruff's whole default rule set agrees on 19,827 of 19,830
  corpus findings and 2,050 of 2,054 standard-library findings.
- R1c: the Rust engine implements all of these codes with findings
  identical to the Python engine (2,066 standard-library and 21,552
  corpus findings); `lint --engine rust` accepts any RC, E or F
  selection.
- R1d: the native linter is about a third faster with identical findings
  (corpus 0.71 s to 0.48 s on 24 threads; Ruff takes 0.37 s): tokens are
  shared between parser and checks, E721 name resolution and bound-name
  sets are computed only when needed, and the scope checker borrows names
  and shares branch paths.
- Compiler scope analysis understands PEP 695 type parameter scopes.
- `# noqa` on the first line of a multi-line import or `__all__`
  statement applies to the names inside it.
- `# ruff: noqa` and `# flake8: noqa` exempt a whole file (optionally for
  listed codes) in both engines.
- Compiler-rejected statements such as `break` outside a loop are reported
  as F701-F707 when selected; RT001 remains whenever that finding is
  suppressed or not selected.
- The lint cache file is now `correctness-v4.json`.

## 0.4.0a0 - Python 3.13 and 3.14 grammar, 2026-10-06

- The Rust engine parses Python 3.13 and 3.14 natively: type parameter
  defaults, t-strings, `except A, B:`, 3.14's string prefix checks and
  lazy annotations, and each version's Unicode data. Tokens, syntax trees,
  `compile()` outcomes and symbol tables match CPython 3.12, 3.13 and 3.14
  on their standard libraries and a 6,988-file corpus; engine findings
  match the Python engine with no fallbacks.
- refactrail-core uses the grammar of the running Python
  (`PYTHON_GRAMMAR`); the native CLI takes `--python-version` (default
  3.14) or `REFACTRAIL_PYTHON_VERSION`.
- Formatter: t-strings are kept exactly as written, like f-strings.
- Parser fixes found by the new fuzzing (all versions): `U"..."` no longer
  gets `kind='u'`; folded text keeps the `u` kind of its first literal;
  `except A, B:` reports CPython's "must be parenthesized" message; a
  failing optional type parameter list backtracks like CPython's.

## 0.3.2a0 - Python 3.13/3.14 syntax fix, 2026-10-05

- Fix: with the Rust engine (the default once installed), `refactrail
  check` reported a false RT001 syntax error for valid code that uses
  syntax newer than the engine's Python 3.12 grammar (Python 3.13 type
  parameter defaults, 3.14 template strings and `except A, B:`). When the
  engine's parser rejects a file that the running Python accepts, the file
  is now checked by the Python engine, so results are correct on every
  supported Python. Engine contract 4.
- Deselecting RT001 no longer hides findings for such files.
- Regression tests for 3.13/3.14 syntax run on every interpreter.

## 0.3.1a0 - first PyPI release, 2026-10-05

- Easy installation: `pip install refactrail` now also installs the Rust
  engine (refactrail-core). Native wheels cover Windows x86_64, macOS arm64
  and x86_64, and Linux x86_64 and ARM64 (glibc and musl); every other
  system gets a pure-Python fallback wheel, so installation never needs
  Rust. refactrail-core no longer depends back on refactrail.
- `refactrail` without arguments shows a short quick start.
- VS Code: the extension uses the interpreter selected in the Python
  extension when `refactrail.pythonPath` is empty (the new default), and
  offers one-click Install/Update when RefacTrail is missing or outdated.
- pre-commit hooks: refactrail-lint, refactrail-format,
  refactrail-format-check and refactrail-check.
- Identifier parity with CPython in the Rust engine: NFKC normalization
  of non-ASCII names (`ﬁle` is `file`), and CPython's rejection of
  characters that cannot appear in names (`a = €`, `x² = 1`,
  U+00A0 inside a name) with its exact messages and columns. Previously
  such files were accepted or reported differently.
- `refactrail lint` and `refactrail format` take `--engine
  auto|python|rust`; the Rust engine lints and formats through
  refactrail-core with byte-identical output (31/31 recorded runs).
- Native `refactrail-native lint` refuses notebooks like the Python CLI.
- Rust third-party license bundle regenerated from the locked dependency
  graph (`scripts/gen_third_party_licenses.py`); it no longer lists the
  removed Ruff crates.
- Package metadata: RefacTrail is described as the Python refactorizer,
  with keywords and family links to FuncLoom.

## 0.3.1a0 - statement layout and rule fixes, local candidate, 2026-10-02

- Formatter: new whole-statement layout engine. Over-long statements are
  joined and split again inside existing brackets (right-hand split,
  left-hand for definition headers, fewest lines), with delimiter
  priorities for comprehensions, commas, conditionals, boolean,
  string-concatenation, comparison and arithmetic splits. Long
  `from ... import` lines gain parentheses; continuation lines are
  re-packed. `--bracket-style own-line|hug` (editor: `refactrail.bracketStyle`).
  Spacing re-runs after layout so one run is idempotent (found on
  `http/server.py`). AST and literal-token validation still guard writes.
- Rules (contract 3, both engines): module-level type aliases (`type X =`,
  `X: TypeAlias =`, PascalCase `X = tuple[...]`) are no longer reported as
  misnamed variables, as code before constants, or as import-time code.
  14 verbs added to the shared verb list.
- RC2 scope evidence: a name such as `locals` that is only assigned (for
  example a dataclass field) no longer disables RC201/RC202 for the file.
- Caches key on a fingerprint of RefacTrail's own code and word lists, not
  only its version, so engine changes never reuse stale results.
- refactrail-core is versioned with RefacTrail (0.3.1a0, contract 3); the
  previous pins required refactrail 0.1.1a0 and could not be installed.
- Editor: automated extension-host workflow test covering every command.

## 0.3.0a0 - resumed independent engines, 2026-10-01

- Add compiler-backed partial scope evidence, RC201 unresolved-name checks and
  RC202 imports without lexical use; retain explicit unknowns and type comments.
- Add independent bracket-group wrapping and byte-local Python notebook source
  edits; preserve outputs, metadata, directives, literals and original endings.
- Discover Python stubs and optionally notebook containers.
- Add project symbol/import inventories and possible transitive impact queries,
  including cycles, stubs, re-exports and explicitly changed deleted modules.
- Add bounded local-variable rename review proposals with original source hashes,
  collision/reflection/scope refusals and structural validation; no rename apply.
- Add source/configuration/version-aware RC caching and parallel snapshot checks.
- Add editor scope, project index and rename commands and notebook formatting.
- Preserve the native RT contract and one-way FuncLoom dependency. Publication
  remains held; this does not establish universal feature or speed superiority.

## 0.2.0a0 - independent engine increment, 2026-10-01

- Add `lint`: seven independent correctness checks, separate from the RT
  style profiles. Diagnose mutable defaults, duplicate literal keys,
  identity/literal comparisons, bare except, directly unreachable code,
  tuple assertions and control-flow exits from finally.
- Add `format`: independent bounded whitespace proposals, check/diff modes
  and explicit hash-checked atomic writes. Preserve literal/comment tokens,
  directives, BOM and original line endings; validate compilation and ASTs.
- Add SARIF 2.1.0 reporting for `lint` and `check`; keep statistics on stderr
  so JSON/SARIF stdout remains parseable.
- Add public correctness/formatting APIs, examples and editor commands.
- Keep the existing RT native contract unchanged. New engines use CPython
  AST/tokenize and do not invoke Ruff, Black or an LLM. Native RT parsing
  still uses the previously documented Ruff parser crates.
- Document the larger independent-engine roadmap and current limitations.
  This is an experimental increment, not complete formatter parity or a
  demonstrated replacement for every existing refactoring tool.

## 0.1.1a0 - local candidate, 2026-10-01

- Validate compilation in both engines without executing source.
- Honor suppression comments only outside strings; retain parse errors.
- Add strict return and parameter suffix checks (RT206, RT207).
- Avoid assuming rich comparisons return bool.
- Validate configurations and ignore damaged caches.
- Reject linked sources; check source hashes before atomic fix writes.
- Skip return-annotation edits on decorated functions.
- Add a native rule-contract compatibility check and interpreter cache key.
- Add a local VS Code extension, CI configuration and release checks.
- Keep FuncLoom as the one-way rewrite dependency; publish nothing.
