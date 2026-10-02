# Changelog

## Unreleased

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
