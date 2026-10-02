# General linting and independent formatting

RefacTrail 0.3.0a0 includes two independent engines using CPython AST/tokenize.
Neither invokes Ruff, Black, another formatter, an LLM or the checked code.
The existing `check` and `fix` operations remain available for the RT style
profiles and FuncLoom-backed structural refactoring.

## Commands

```powershell
.\.venv\Scripts\python.exe -m refactrail lint src
.\.venv\Scripts\python.exe -m refactrail lint src --select RC101,RC102
.\.venv\Scripts\python.exe -m refactrail lint src --output-format sarif
.\.venv\Scripts\python.exe -m refactrail format examples\general_demo.py
.\.venv\Scripts\python.exe -m refactrail format examples\general_demo.py --check
.\.venv\Scripts\python.exe -m refactrail format examples\general_demo.py --write
```

`lint` does not impose suffixes, docstrings, verb lists or other personal
style choices. Its nine RC rules report syntax evidence for review, not
proven program defects; intentional constructs can be suppressed with
ordinary real-comment `noqa` markers. No semantic fixes are automatically
applied. Syntax and encoding failures cannot be suppressed.

`format` defaults to a read-only diff. `--check` is useful in local checks
or CI. Both return 1 when edits are pending. `--write` is the explicit
source-changing operation and returns 0 on success. Invalid or unsupported
inputs produce errors. All proposals are validated before the first write,
and original hashes are rechecked; writes remain atomic per file, not a
transaction across the entire input set.

Both commands discover `.py` and `.pyi` files with the configured directory
exclusions. Formatting accepts explicit Python notebooks; `--notebooks` also
includes them in folder discovery. Notebook linting refuses because cell
execution order and kernel bindings are not inferred. Empty
input sets and direct linked files are refused. A nearby RefacTrail config
supplies discovery exclusions; lint rule selection is explicit on its CLI.

## Formatting boundary

Base changes are operator/comma spacing, trailing code whitespace and
a missing final newline; explicit width requests also enable bounded wrapping. Unary operators and argument-default spacing are
retained. Existing line breaks, indentation, quote choices, literal text
and comment text are preserved. Comment-bearing lines, f-string lines,
multiline literal spans and explicit continuations are kept conservatively.
`fmt: off/on/skip` comments disable edits in the corresponding region.

`--line-length 40..200` enables wrapping at existing bracketed comma boundaries
in calls, containers, signatures and parenthesized imports. It reaches a fixed
point before returning a proposal. Protected lines, indivisible expressions,
and files with type-ignore directives retain their line structure. The formatter
does not sort imports, reflow comments, normalize indentation/quotes or implement
Black's complete style. A zero exit means
the supported formatting subset is satisfied, not full 79-column or strict
style compliance. Use `check` for that profile's separate findings.

Both versions compile without execution, keep their ASTs including type
comments/type-ignore locations, and preserve literal/comment token spelling.
These checks do not prove arbitrary runtime equivalence: reflection and
source-column observations remain outside that claim.

## Python API

```python
from refactrail import (
    check_correctness_list, format_source_str, plan_format, write_format_none,
)

findings = check_correctness_list("example.py", b"def compute(values=[]): pass\n")
formatted = format_source_str("total=120+12\n")
proposal = plan_format("example.py")  # Reads; does not write.
# After reviewing the proposal:
# write_format_none(proposal)
```

SARIF output identifies RefacTrail's version, uses Unicode code-point
columns and escaped file URIs, and marks parse/encoding/internal failures
as unsuccessful analysis. It follows the
[OASIS SARIF 2.1.0 schema](https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json).
JSON output is separate from statistics on stderr.

The new commands are exposed by the local editor extension. Command-runner
tests and actual editor-host acceptance are distinct checks; do not assume
host validation from packaging alone.

## Independence and future work

RC linting and formatting run in both engines: Python, and the Rust
engine built on RefacTrail's own parser (no Ruff code; see
[DUAL_ENGINES.md](DUAL_ENGINES.md)). No Ruff linting or formatting engine
has been integrated. FuncLoom remains a
package dependency for the existing structural rewrite APIs. See
[the exact rules](RULES.md) and [the expansion roadmap](EXPANSION_ROADMAP.md).
No speed superiority, broad accuracy superiority or public release is claimed.


See [the expansion workflow](EXPANSION_USAGE.md) for lexical reports, project
impact, read-only local rename proposals, notebook preservation, caching and
current acceptance evidence. Those features use the standard-library parser
and compiler metadata without running customer source.
