# RefacTrail

[![CI](https://github.com/SAMtheROCKET/refactrail/actions/workflows/ci.yml/badge.svg)](https://github.com/SAMtheROCKET/refactrail/actions/workflows/ci.yml)
[![PyPI](https://img.shields.io/pypi/v/refactrail?include_prereleases)](https://pypi.org/project/refactrail/)
[![Python](https://img.shields.io/pypi/pyversions/refactrail)](https://pypi.org/project/refactrail/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Website](https://img.shields.io/badge/website-refactrail-informational)](https://samtherocket.github.io/refactrail/)
[![VS Code](https://img.shields.io/visual-studio-marketplace/v/samtherocket.refactrail?label=VS%20Code)](https://marketplace.visualstudio.com/items?itemName=samtherocket.refactrail)

**The Python refactorizer.** A linter, formatter and verified refactoring
tool with two independent engines, pure Python and Rust, built on its own
parser. Run correctness checks and bounded formatting, or check structure,
naming, types and documentation against a chosen profile.

Website: [samtherocket.github.io/refactrail](https://samtherocket.github.io/refactrail/)

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

**Status:** alpha, version 0.5.1a0 on PyPI. Free and open source (MIT).
Works offline: no AI model, account or internet connection needed.

## Start in one minute

1. **Install** (needs Python 3.12 or newer;
   [older Python? see below](#your-project-uses-an-older-python)):

   ```bash
   pip install refactrail
   ```

   This also installs FuncLoom and the fast Rust engine. Nothing needs
   compiling: every system gets a ready-made package.

2. **Run it in your project folder:**

   ```bash
   refactrail lint                # find likely bugs
   refactrail lint --fix          # fix the safe ones (--diff to preview)
   refactrail format --diff       # preview tidy formatting (--write applies)
   refactrail check               # structure, naming, type hints, docstrings
   ```

3. **Read the result.** Each finding shows the file, line and a short
   reason. Exit code 0 means clean, 1 means findings, 2 means an error,
   so it drops straight into CI.

![RefacTrail finds seven issues with lint --select E4,E7,F, previews the safe fixes with --diff, applies them with --fix and the re-run is clean](https://raw.githubusercontent.com/SAMtheROCKET/refactrail/main/docs/media/refactrail-lint-fix.gif)

Already use Ruff? `refactrail lint --select E4,E7,F` checks the same
default rules with the same codes, so your `# noqa` comments keep
working. Typing just `refactrail` shows the most useful commands. If your
computer blocks the `refactrail` command, type `python -m refactrail`.

## In VS Code

1. Install **[RefacTrail](https://marketplace.visualstudio.com/items?itemName=samtherocket.refactrail)** from the
   Extensions view (search *RefacTrail*).
2. Open a Python file, press **Ctrl+Shift+P** (**Cmd+Shift+P** on a Mac)
   and type **RefacTrail**.
3. The first time, click **Install** when asked; the extension sets
   RefacTrail up in your Python for you.

Commands: **Check active file** and **Lint general correctness**
(findings in the Problems panel), **Preview fixes** / **Apply fixes**, and
**Preview** / **Apply independent formatting** (notebooks too).

## Check on every commit (optional)

Add this to `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/SAMtheROCKET/refactrail
    rev: v0.5.1a0
    hooks:
      - id: refactrail-lint
      - id: refactrail-format
```

`refactrail-format-check` (fails instead of rewriting) and
`refactrail-check` are also available.

## What's new in 0.5.1a0

- **VS Code extension on the Marketplace,** now with an icon: search
  *RefacTrail* in the Extensions view.
- **Older-Python projects:** a step-by-step
  [guide](https://github.com/SAMtheROCKET/refactrail/blob/main/docs/OLDER_PYTHON.md), tested with code for Python 3.8, 3.9 and
  3.10.
- **A simpler README.** Rules, fixes and engines are unchanged
  (refactrail-core 0.5.1a0 is the 0.5.0a0 engine, versioned in step);
  0.5.0a0's Ruff-compatible rules and safe fixes are in the
  [changelog](https://github.com/SAMtheROCKET/refactrail/blob/main/CHANGELOG.md).

## Your project uses an older Python?

No problem. RefacTrail only **reads** your code, so it runs on its own
Python 3.12 or newer while your project stays on Python 3.8, 3.9, 3.10 or
3.11. Do this once:

1. **Make a separate Python for the tools** (no admin rights needed):

   ```bash
   # Windows (Command Prompt)
   py -3.12 -m venv %USERPROFILE%\py-tools
   %USERPROFILE%\py-tools\Scripts\python -m pip install refactrail

   # macOS / Linux
   python3.12 -m venv ~/py-tools
   ~/py-tools/bin/python -m pip install refactrail
   ```

   No Python 3.12 on the machine? Run `pip install uv`, then
   `uvx --python 3.12 refactrail lint my_project`: uv downloads Python 3.12 for you.

2. **Run RefacTrail with that Python:**

   ```bash
   %USERPROFILE%\py-tools\Scripts\python -m refactrail lint my_project     # Windows
   ~/py-tools/bin/python -m refactrail lint my_project                       # macOS / Linux
   ```

3. **In VS Code**, open Settings, search for `refactrail.pythonPath` and paste
   the path of that Python (for example
   `C:\Users\YOU\py-tools\Scripts\python.exe`).

Your project keeps using its own Python to run. More detail, limits and
fixes for common errors: [the older-Python guide](https://github.com/SAMtheROCKET/refactrail/blob/main/docs/OLDER_PYTHON.md).

If `pip install refactrail` says `from versions: none`, your Python is older
than 3.12 (use the steps above) or your company's package mirror does not
carry it yet (ask IT to allow it).

## From source (for contributors)

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
library the fixed files are byte-identical to Ruff's `--fix`. The commands,
options, codes, exit codes and output formats that stay stable through
the beta are listed in [stable interfaces](docs/INTERFACES.md). `format` uses an independent CPython AST/tokenize implementation; its
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

<!-- GIF placeholder: docs/media/refactrail-vscode.gif
     VS Code, about 15 s: run "RefacTrail: Check active file", hover a
     squiggle, then "RefacTrail: Preview fixes". See docs/MEDIA.md. -->

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
the standalone `refactrail-native` binary runs `check`, `lint`, `rename`, `scope`,
`format`, `index` and `rules` without Python. Outputs are byte-identical to the
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
diff-preview and fix commands. Install it from the Marketplace, select the Python
environment containing these packages, and use a trusted workspace.

```powershell
python scripts/verify.py
python scripts/release_check.py --output dist/0.5.1a0 --dependency-wheel path/to/funcloom-0.10.5a0-py3-none-any.whl
```

The release script builds and checks a wheel and source archive, installs
both in fresh environments, tests installed code and checks uninstallation.
CI runs the tests on Windows, Linux and macOS with Python 3.12-3.14, and
the Release check workflow verifies the identical built files on all three
systems. [Release preparation](docs/RELEASE_PREPARATION.md) lists the
remaining launch work. No public release has been made.

[MIT](LICENSE), copyright 2026 Sambit Supriya Dash.
