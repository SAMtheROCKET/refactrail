# Resumed independent-engine workflow

RefacTrail 0.3.0a0 is a local experimental alpha. The owner chose independent
linting and formatting implementations. No public upload, tag or release is
performed by these workflows. The existing native RT parser dependency is
unchanged; new RC, formatting, lexical and project tools use Python's standard
library. FuncLoom remains the separate structural-rewrite dependency.

## Run locally

From the RefacTrail directory in PowerShell:

```powershell
.\.venv\Scripts\python.exe -m refactrail lint src --jobs 4
.\.venv\Scripts\python.exe -m refactrail format examples\analysis.ipynb --diff
.\.venv\Scripts\python.exe -m refactrail format examples --notebooks --line-length 79 --check
.\.venv\Scripts\python.exe -m refactrail scope examples\project_demo\invoice.py
.\.venv\Scripts\python.exe -m refactrail index examples\project_demo --changed invoice.py
.\.venv\Scripts\python.exe -m refactrail rename examples\project_demo\invoice.py --function calculate_total --old amount --new base_amount_int
.\.venv\Scripts\python.exe scripts\verify.py
```

`format` defaults to a diff; add `--write` only to apply reviewed formatting.
The sample notebook's `base_amount=120` becomes `base_amount = 120`, while
markdown, metadata, cell order, outputs and execution counts retain their
original JSON bytes. Source arrays remain arrays and source strings remain
strings. Magics, shell escapes and invalid code refuse the complete notebook
with the cell index and parser location. Kernel code is never executed.

`--line-length 79` splits existing bracketed comma groups. It can turn a long
call or signature into continuation-indented lines without changing tokens.
It leaves indivisible expressions and protected lines long. It does not sort
imports: importing modules may have observable effects. Zero formatting findings
means this supported subset is satisfied, not complete conformance to every style.

## Scope and correctness

`lint` runs nine RC review checks, independent of strict suffix/documentation
rules. RC201 finds loads without compiler lexical bindings; RC202 finds imported
bindings without lexical use, with explicit re-export/type-comment exemptions.
Use `check --profile strict` for the owner's naming, annotations, docstrings,
constants, 79-column and function-size requirements.

`scope` returns `refactrail-scope-1` JSON, the original SHA-256, source-located
reads, resolution categories, lexical scope summaries and unresolved limitations.
Compiler bindings can be conditional, deleted or not yet initialized. The report
therefore never claims initialization or exception-path correctness. Wildcard
imports, dynamic namespace operations and type-parameter scopes suppress the
uncertain RC2 pass and record why. Runtime types and domain meanings are not guessed.

## Project impact and rename proposals

`index IMPORT_ROOT --changed RELATIVE_PATH` returns `refactrail-index-1` JSON.
For a `src` layout, pass `src` as the import root. It includes current file hashes,
class/function definitions, static import candidates and transitively possible
importers. Package prefixes, relative imports, re-export chains, cycles, stubs and
explicitly named deleted modules are considered. Dynamic imports, sys.path changes,
plugins and external modules remain unresolved. Candidate edges are not runtime
import resolution, a call graph, or complete compatibility analysis. The graph is
rebuilt from current source each invocation; no stale graph cache is reused.

`rename` returns `refactrail-rename-1` JSON and a unified diff. In the example,
`amount` becomes `base_amount_int` within `calculate_total`; callers do not change.
The input file is untouched. Only a unique undecorated top-level function with an
ordinary local non-parameter binding is supported. Nested scopes, parameters,
imports, exception/pattern bindings, reflection, replacement collisions and
unsupported type scopes are refused with reasons and source locations. Names come
from the user's explicit request, not inferred domain context.

The proposal has `can_apply=false` and `behavior_verified=false`. There is no rename
apply command. Comments, string references, tracing and frame inspection can observe
names; a structural check is not a general behavioral proof. Review the complete
diff, recheck the source hash, and run project-specific regressions before manual use.
Cross-module rename, extraction/move transactions and rollback remain future work.

## Caching, parallelism and API

`lint` caches advisory diagnostics in `.refactrail_cache/correctness-v4.json` under
the current directory. Keys include original bytes, configuration, tool/interpreter
version, and stub/initializer policy. Malformed cache data is discarded. Use
`--no-cache` for an uncached run. All pending checks use immutable byte snapshots;
`--jobs N` uses worker processes for batches of at least 64 pending files. Small
batches remain serial. Cache data never authorizes a write or verifies behavior.

Public APIs include `check_correctness_list`, `check_correctness_paths_list`,
`format_source_str`, `plan_format`, `write_format_none`, `analyze_lexical_dict`,
`analyze_project_dict` and `plan_rename_dict`. Reports are partial evidence with
versioned schemas. Checked projects are never imported or executed for analysis.

The VS Code extension exposes lint, formatting, scope, project index and local
rename proposals. Notebook formatting requires a saved local Python notebook;
`refactrail.lineLength` and `refactrail.importRoot` control the new options.
Runner tests, local installation and real editor-host acceptance are distinct.
See `reports/RESUMED_ENGINES.md` for actual release evidence and unresolved blockers.

## Readiness

This release completes the resumed implementation batch described here. It does
not complete every expansion-roadmap item, establish production readiness, or
outperform every feature of Ruff/Black and other tools. Broader semantics, formatter
coverage, real-world labeled accuracy, native optimization and platform/editor
acceptance remain open. The owner's publication hold remains in force.
