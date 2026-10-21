# Run FuncLoom, RefacTrail and FlowBlueprint together

Use the same Python interpreter for all three installed packages. This
unpublished increment requires FuncLoom 0.10.8a0, RefacTrail 0.5.3a0 and
FlowBlueprint 0.3.1a0. A different computer running older packages or older
VSIX files does not have these changes.

```powershell
python -m funcloom --version
python -m refactrail --version
python -m flowblueprint --version
python -m flowblueprint . --entry src/analysis.py --pipeline-to ../analysis-review
```

The destination must be new and outside the input repository. Use `--entry`
again for another entry script. Passing the repository keeps its supporting
SQL files, data and imported modules in the copy. A single-file input is
also supported, but cannot copy dependencies outside that file automatically.
Notebooks are currently supported individually. No target code is executed.

Optional flags: `--externalize-sql` extracts supported literal SQL queries;
`--png` adds an editable implementation PNG (requires the PNG extra).
The default line limit for this workflow is 79.

The stages run in this order:

1. FuncLoom creates `project/`: bounded module splitting, orchestration,
   eligible literal constants, docstrings and long-function splitting.
2. RefacTrail fixes and formats the new copy using its strict profile, then
   checks structural rules and E4/E7/E9/F correctness rules. It records each
   diff, source hash, declined edit and remaining finding.
3. FlowBlueprint reads that final copy and draws its architecture. Final
   source hashes in `workflow.json` tie diagrams to those exact files.

Open these artifacts:

| File | What it contains |
| --- | --- |
| `project/` | Reviewable generated code; original files stay unchanged |
| `review.md` | Completed stages and remaining work |
| `refactrail.json` | Applied changes, diffs, hashes, notes and findings |
| `context-questions.json` | Located questions about unresolved names/types |
| `architecture.drawio` | High-level and implementation canvases |
| `high_level.drawio`, `detailed.drawio` | Separate copies of the two levels |
| `function_details.drawio` | Declared function bodies, when available |
| `workflow.json` | Conversion coverage, final hashes and stage outcomes |

Function details include nested definitions with lexical names. These pages
show declarations, not proof of runtime calls. At most 100 function pages are
included; the record identifies incomplete detail coverage. Dynamic dispatch,
closures, decorators and third-party behavior remain unresolved.

Exit code 0 means selected conversion and final configured checks completed
without findings. It does not prove application behavior. Exit code 1 means
there is a reviewable bundle with findings or partial conversion (or a
located modularization refusal without a bundle). Exit code 2 means an input,
configuration or rendering error. Errors during staging do not leave a
completed bundle. Read the printed explanation and `review.md` when present.

## What still requires context or more engine work

- General domain renaming of `a`, `df`, `m`, `rev`, etc. is not implemented.
  Existing bounded loop/handle renames may apply; arbitrary names stay visible
  in the strict findings. A single word is not inherently a bad identifier.
- Missing argument types and arbitrary library-return types are not invented.
  Only the existing narrow return-annotation rules add annotations.
- Parameter/public-function renaming, general dtype suffix insertion,
  cross-file keyword callers and reflection need broader reference analysis.
- Constants are relocated only by the bounded FuncLoom transformation. A
  call such as `matplotlib.use()` may have to precede a later import; moving
  setup or dependent expressions just to satisfy RT103 can break the program.
- Existing docstring text is retained and missing sections completed. New
  descriptions use source evidence and explicitly unknown types. Tool
  directives and inline bodies can prevent completion; inspect findings.
- Context questions are a human review checklist, not an executable rename
  specification. Filling them does not silently apply code changes.
- Generated helpers may remain above preferred limits where splitting is
  unsupported. RefacTrail records the actual final findings.

## Choosing an individual command

`funcloom plan` requires an explicit region and function name. `snippet`
handles the supported snippet subset; an entire analysis module with imports
and definitions belongs in `modularize` or the combined workflow.
`refactrail check`, `scope` and `index` analyze; they do not rewrite code.
`refactrail fix --diff` previews bounded edits; `fix` writes those edits.
The combined workflow runs fixes only on its newly generated copy.
`flowblueprint --view reference` draws operational canvases without rewriting.
The older `--modularize-to` mode remains the two-stage FuncLoom/drawing mode;
use `--pipeline-to` to include RefacTrail and the final review reports.

## Windows and VS Code

The editor wrappers now pass `-I -X utf8`, because isolated Python ignores
PYTHONUTF8 environment variables. All three VSIX files need updating on the
other computer. FlowBlueprint's two drafting commands now run the complete
pipeline and open the architecture while pointing to remaining review work.

Console RefacTrail output escapes unsupported characters on legacy encodings
instead of crashing. For an exact Unicode diff use:

```powershell
python -X utf8 -m refactrail fix src/analysis.py --diff
```

Nothing in this workflow publishes packages, uploads source, executes project
code, or establishes production readiness. Run application regressions on
representative data before replacing the original project with a draft.
