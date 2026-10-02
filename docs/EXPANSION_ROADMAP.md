# Independent RefacTrail expansion

Owner decision, 1 October 2026: build independent linting and formatting
engines, accepting a larger development scope. Do not introduce a Ruff
runtime adapter. No publication is authorized.

The objective is a broadly useful, independently implemented refactoring
tool. Superiority is evaluated for declared tasks, correctness, latency,
memory and usability; it is not assumed from feature count.

| Increment | Deliverable | Acceptance evidence |
|---|---|---|
| 1 | General RC correctness rules, independent bounded formatter, text/JSON/SARIF output and CLI | Positive/negative rule cases; Unicode positions; directive/literal preservation; compilation, idempotence, stale-write and no-execution tests; existing RT parity |
| 2 | Formatter coverage for comments, imports, signatures, expression wrapping, notebooks and stubs | Published style specification; mixed syntax corpus; byte-preservation exclusions; idempotence and differential evidence against established formatters |
| 3 | Lexical name resolution and control-flow/data-flow facts | Scope, closures, walrus/comprehension bindings, imports and exception counterexamples; explicit unknowns |
| 4 | Undefined/unused bindings, import and exception analysis, targeted semantic fixes | Labeled expected diagnostics; false-positive/negative accounting; reviewed patches and behavioral regressions |
| 5 | Project symbol index, rename/extract/move plans, dependency and impact analysis | Cross-module callers, re-exports, public APIs, dynamic imports and rollback/stale-source tests |
| 6 | Performance and editor incrementality | Shared facts, content/config/version-aware caching, changed-file dependency invalidation, measured native ports, cancellation and large repositories |
| 7 | Extension/notebook integration, plugin contracts and release hardening | Real editor host tests, platform matrix, reproducible artifacts and examples; owner-controlled publication |

Each increment may need multiple releases. AI-assisted proposals are a
later optional layer; they must retain source evidence and pass independent
validation. Never execute user projects just to infer behavior or types.

The native engine now uses RefacTrail's own Rust parser (the Ruff parser
crates were removed in October 2026), and implements the RT, RC, scope,
format and index operations with output identical to the Python engine;
see [DUAL_ENGINES.md](DUAL_ENGINES.md).

Initial RC rules deliberately diagnose without guessing repairs. Broad
public-API renaming and arbitrary semantic optimization are not enabled.
Keep the original RT standard/strict profiles available for the owner's
requirements while allowing users to run general linting without them.


## Resumed implementation status: 0.3.0a0

Increment 1 remains complete for its stated contract. This delivery implements
substantial parts of increments 2 through 7, with the following exact boundaries:

| Area | Implemented | Still open |
|---|---|---|
| Formatting | Token-preserving spacing, opt-in comma-group wrapping, stubs, Python notebooks | General expression layout, comment reflow, import organization, full style specification coverage |
| Scope/data flow | Compiler lexical owners, closure/global/local evidence, comprehension handling and explicit limitations | Reaching definitions, exception-sensitive initialization, alias/effect proof, type-parameter scopes |
| Correctness | Nine RC rules, suppression, unused-import review, undefined-name evidence | Broader rule families, labeled real-world accuracy evaluation, semantic fixes |
| Project refactoring | Symbols, static import candidates, transitive possible impact, bounded local rename proposals | Cross-module rename/move/apply, public-API compatibility, transactional rollback |
| Performance | Immutable snapshot workers and source/config/version-aware RC cache | Incremental project graph cache, native RC/formatter, cancellation, large-scale memory profiling |
| Editor/release | Commands for the new APIs, notebook format interface, runner/host fixtures, local release checks | Full editor-host acceptance and wider Python/macOS matrix |

The project's broad competitive objective is not complete. Completing a bounded
release does not mean every feature of every competing tool exists. No benchmark
or syntax check establishes universal performance, correctness or readiness.
