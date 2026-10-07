# RefacTrail design

RefacTrail is a separately versioned checker and bounded refactoring tool.
Its Python package depends on FuncLoom's public rewrite API. FuncLoom has no
RefacTrail runtime dependency. A future architecture product can consume
both APIs; it is not implemented in this delivery.

## Independent engines in 0.3.0a0

The owner selected independent linting and formatting implementations.
The new `lint` command runs the RC correctness family using CPython AST;
`format` uses original token boundaries and validated whitespace edits.
Neither calls a formatter/linter subprocess. RT profiles remain available
through `check`, with the unchanged Python/native rule contract. RC
rules, scope/index/rename analysis and formatting are Python-only. See
[contracts](RULES.md) and [the larger roadmap](EXPANSION_ROADMAP.md).

## Components and contract

- The Python reference engine and optional Rust core implement
  [the same rule contract](RULES.md). Parity tests compare findings.
- Compilation rejects contextually invalid Python before either rule engine
  runs. Target source is never imported or executed for analysis.
- Rust uses its own lexer, parser, compile checks and symbol table, with
  PyO3 and Rayon; it contains no Ruff code. The PyO3 module asks CPython
  only for the exact message of a file that fails to compile; the
  standalone `refactrail-native` executable needs no Python at all.
- The cache includes source content, interpreter version, rule version and
  settings. Malformed cache entries are discarded and checked again.
- CLI and editor wrappers invoke the same public engines. The editor uses
  an isolated Python process, explicit arguments and no shell.

## Refactoring scope

Current fix groups are RT201 renames of single-character loop targets and
open() handles, RT102 UPPER_CASE renames of module constants outside
packages, RT402 return annotations (None, or one literal type), RT301
docstrings built from the code, RT101 wrapping and RT501/RT502 splitting
via FuncLoom. Other findings are review guidance. The `rename` command prepares bounded local-name review proposals; it has
no apply mode. There is no `--unsafe-fixes` rename engine. Parameter, return and variable suffix rules do not rename
public interfaces. User-provided names and domain meanings remain explicit.

Rewrites compile and pass the applicable structural checks. This does not
prove behavior preservation: annotations, documentation, scope changes and
introspection remain observable. Decorated functions are excluded from
None-return insertion. Review the diff and run behavioral regressions.

Each write checks the original SHA-256, compiles the result, flushes a
same-directory temporary file, preserves permission bits and replaces the
original atomically after another hash check. This is protection against
ordinary stale edits and interrupted writes, not a lock against a hostile
concurrent filesystem writer. Multi-file operations are not transactions;
Windows ACLs, ownership and extended filesystem metadata are not promised
to be preserved. Direct links are refused and discovered links skipped.

## Owner's style profile

Strict checking covers docstring sections, declared annotations, names and
narrowly evidenced dtype suffixes, constant placement, 79-column lines,
40 preferred / 50 maximum function lines and 100-line main guards. Protocol
names are exempt where required. Variadic argument annotations describe
elements rather than the stored tuple/dict and are excluded from RT207.
Natural-language meaning and full object-oriented design cannot be proven
by these rules. See the rule contract for exact coverage and exemptions.

## Delivery boundaries

The base package, optional native core and VSIX are independent artifacts.
Native notices accompany the core distribution. Local release validation
is recorded separately from prepared CI workflows. Platform support and
benchmarks describe measured configurations only. Publication remains an
owner action, after FuncLoom's required version is available.
