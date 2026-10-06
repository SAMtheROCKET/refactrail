# RefacTrail

**The Python refactorizer.** A linter, formatter and verified refactoring
tool with two independent engines, pure Python and Rust, built on its own
parser. Run correctness checks and bounded formatting, or check structure,
naming, types and documentation against a chosen profile.

Part of a family of standalone Python tools: [FuncLoom](https://github.com/SAMtheROCKET/funcloom) (functionizer), [RefacTrail](https://github.com/SAMtheROCKET/refactrail) (refactorizer) and [FlowBlueprint](https://github.com/SAMtheROCKET/flowblueprint) (architect: script or notebook to architecture diagram). Each installs and works on its own.

**Use RefacTrail to:**

- auto-refactor Python safely: bounded fixes and rename previews that are
  verified, never guessed;
- lint for likely bugs and format code with a fast Rust engine, or the
  identical pure-Python engine, in CI (text, JSON or SARIF output);
- enforce naming, structure, type-hint and docstring conventions across a
  team with a chosen rule profile;
- check and format Jupyter notebooks as well as scripts.

Built for developers, data scientists and teams that want clean,
consistent code; pair it with FuncLoom to turn scripts and notebooks into
functions and FlowBlueprint to draw the architecture.

**0.4.0a0 is an experimental alpha, published on PyPI.** Python 3.12 or newer is
required. RefacTrail has its own package, CLI and VS Code extension. It
installs FuncLoom automatically for its rewrite API; FuncLoom has no
dependency on RefacTrail. No LLM, account or network is needed at runtime.

## Quick start

```bash
pip install refactrail
```

That one command also installs FuncLoom and the fast Rust engine (a native
wheel for Windows, macOS and Linux; on other systems a small fallback is
installed and the Python engine gives the same results, so nothing ever
needs compiling). Python 3.12 or newer is required.

Then, in any project folder:

```bash
refactrail                     # the most useful commands
refactrail lint                # find likely bugs
refactrail format --diff       # preview formatting; --write applies it
refactrail check               # structure, naming, type hints, docstrings
refactrail fix --diff          # preview safe fixes; drop --diff to apply
```

Paths default to the current folder; pass files or folders to narrow it.
Exit codes are 0 (clean), 1 (findings) and 2 (error), ready for CI. If your
system blocks the `refactrail` command, use `python -m refactrail`.

**In VS Code**, install the RefacTrail extension and run *RefacTrail: ...*
from the Command Palette. It uses the Python interpreter selected in VS Code
and offers to install RefacTrail there with one click.

**On every commit**, add the hooks to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/SAMtheROCKET/refactrail
    rev: v0.4.0a0
    hooks:
      - id: refactrail-lint
      - id: refactrail-format
```

`refactrail-format-check` (fails instead of rewriting) and
`refactrail-check` are also available.

### From source (development)

Install FuncLoom from its source folder and the local engine fallback
first, then RefacTrail:

```bash
python -m pip install -e ../funcloom ./rust/fallback
python -m pip install -e .
```

Nothing is uploaded by these local tools.

## General correctness and independent formatting

```powershell
python -m refactrail lint src
python -m refactrail lint src --output-format sarif
python -m refactrail lint src --select E4,E7,F --diff   # preview safe fixes
python -m refactrail lint src --select E4,E7,F --fix    # apply them
python -m refactrail format examples/general_demo.py --diff
python -m refactrail format examples/general_demo.py --check
python -m refactrail format examples/general_demo.py --write
```

`lint` provides nine RC correctness checks without imposing personal naming
rules. `lint --select E4,E7,F` adds pycodestyle- and Pyflakes-compatible
codes (same numbers, so `# noqa: E701` comments carry over), including
unused imports (F401), undefined names (F821) and unused variables
(F841). Across Ruff's whole default rule set they agree with Ruff on
19,827 of 19,830 findings in a 6,988-file corpus; see
[the rules](docs/RULES.md). `lint --fix` (or `--diff` to preview) applies
safe fixes for unused imports (F401), f-strings without placeholders
(F541), `is` with literals (F632), trailing semicolons (E703) and
negated membership and identity tests (E713, E714); on the standard
library the fixed files are byte-identical to Ruff's `--fix`. `format` uses an independent CPython AST/tokenize implementation; its
default is a read-only preview. It makes bounded whitespace edits and retains
literal/comment spelling, directives, BOM and original line endings.
Use `--line-length 79` for bracketed comma-group wrapping. Explicit Python
notebooks and discovered stubs are supported; `--notebooks` includes notebooks
in folder formatting. This is a documented style subset, not full Black parity.

```powershell
python -m refactrail format examples/analysis.ipynb --line-length 79 --diff
python -m refactrail scope examples/project_demo/invoice.py
python -m refactrail index examples/project_demo --changed invoice.py
python -m refactrail rename examples/project_demo/invoice.py --function calculate_total --old amount --new base_amount_int
```

`scope` exposes partial compiler binding evidence. `index` inventories symbols
and possible import impact under an explicit import root. `rename` produces a
read-only, hash-linked proposal for a bounded local-variable subset; parameters,
public APIs, nested scopes and dynamic namespaces are refused. See
[the workflow and examples](docs/EXPANSION_USAGE.md).

`lint --jobs 4` enables worker processes on larger batches. Lint caching hashes
source bytes, rule selection, tool/interpreter version and path-sensitive policy;
use `--no-cache` to disable it. No cache is used to authorize source changes.

See [the independent engines](docs/INDEPENDENT_ENGINES.md) for contracts,
APIs and limitations, and [the expansion roadmap](docs/EXPANSION_ROADMAP.md)
for broader formatter, data-flow and repository-refactoring work. These
engines do not invoke or contain Ruff or Black. The FuncLoom
structural-rewrite dependency remains as documented below.

## Checking and fixing

`check` reports line, function and main-block sizes; docstrings and their
sections; missing annotations; naming conventions and verb prefixes;
constants and their location; and top-level script execution. The strict
profile adds generic-name and dtype-suffix checks. Defaults are 79 columns,
40 preferred / 50 maximum function lines, and 100 main-block lines.

```powershell
python -m refactrail check src --profile strict --statistics
python -m refactrail check src --output-format json
python -m refactrail rules
python -m refactrail fix src --diff
python -m refactrail fix src
```

`fix --diff` previews edits without writing. `fix` edits files in place:
selected fixes add eligible `-> None` annotations, create function docstring
skeletons, wrap supported lines and split supported long functions. It
checks compilation, rejects linked files and changed source, and replaces
each file atomically. Review diffs and run your project's tests. A failed
file does not roll back other files in a multi-file run.

These are structural checks, not proof of unchanged behavior. Annotations
and docstrings can affect reflection. There is no automatic domain naming,
public-API renaming, inferred annotation insertion beyond the bounded None
case, or arbitrary repository restructuring. Missing meanings need user
context. Unsupported functions can remain long. Use FuncLoom's explicit
`modularize` command to create a separate package draft.

Exit codes: check returns 0 with no findings, 1 for findings, 2 for usage
errors. `--exit-zero` explicitly overrides finding failures. Diff returns 1
when edits are proposed or files are skipped; fix returns 1 for skipped
files. Syntax, encoding and internal errors cannot be hidden by selection
or ignore lists. Suppression markers apply only inside real comment tokens.

## Configuration

```toml
[tool.refactrail]
profile = "strict"
line-length = 79
function-preferred-lines = 40
function-max-lines = 50
main-max-lines = 100
select = ["RT"]
ignore = []
```

Widths must be 40-200; the preferred function size cannot exceed the maximum.
See [the rule contract](docs/RULES.md) and [design](docs/DESIGN.md).

## The Rust engine

`--engine auto` selects a compatible `refactrail-core` when installed and
otherwise uses Python. `--engine python` selects the reference implementation.
The native core is a separate PyO3/Rayon distribution built on
RefacTrail's own Rust lexer, parser, compile checks and symbol table; it
contains no Ruff code. It parses Python 3.12, 3.13 and 3.14 code with
each version's own grammar (the running Python's in the package;
`--python-version` in `refactrail-native`, default 3.14), verified
against each CPython version. `check`, `lint` and `format` accept `--engine`, and
the standalone `refactrail-native` binary runs `check`, `lint`, `scope`,
`format` and `index` without Python. Outputs are byte-identical to the
Python engine on the recorded corpora and fuzzed inputs, with CPython as the
oracle; see [the dual-engine record](docs/DUAL_ENGINES.md). Parity evidence
is not a guarantee for every Python program.

Native wheels (CPython 3.12+ ABI3) are built and tested in CI for Linux
x86_64 (manylinux2014), Windows x86_64 and macOS arm64; other platforms use
the Python engine. Install a local wheel with
`pip install --no-index --no-deps WHEEL`. See
[benchmark evidence](docs/BENCHMARKS.md) for measured timings and their
limits.

## Editor and release preparation

[The VS Code extension](editor/refactrail/README.md) provides explicit check,
diff-preview and fix commands. Install its local VSIX, select the Python
environment containing these packages, and use a trusted workspace.

```powershell
python scripts/verify.py
python scripts/release_check.py --output dist/0.4.0a0 --dependency-wheel path/to/funcloom-0.10.3a0-py3-none-any.whl
```

The release script builds and checks a wheel and source archive, installs
both in fresh environments, tests installed code and checks uninstallation.
CI runs the tests on Windows, Linux and macOS with Python 3.12-3.14, and
the Release check workflow verifies the identical built files on all three
systems. [Release preparation](docs/RELEASE_PREPARATION.md) lists the
remaining launch work. No public release has been made.

[MIT](LICENSE), copyright 2026 Sambit Supriya Dash.
