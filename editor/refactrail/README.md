# RefacTrail for VS Code

Experimental local extension 0.1.1; requires Python package
`refactrail==0.3.0a0` in your selected interpreter. It installs no
Python package automatically and sends no source to a service.

Install the local VSIX with VS Code's **Extensions: Install from VSIX**.
Set `refactrail.pythonPath` to your environment's Python executable, for
example `.venv/Scripts/python.exe`. Open a trusted local workspace and
use the `RefacTrail:` commands in the Command Palette. Checks appear in
Problems; drafts and diffs open for review. Save files before checking or
writing transformations. RefacTrail commands operate on saved local Python files.

Generated output still requires project-specific tests. RefacTrail's
**Apply fixes** command changes the active file; **Preview fixes** does
not. FuncLoom's refine/modularize commands require a new destination.
No automatic save-time edits, arbitrary shell commands or target imports
are used. Command output is limited to 8 MB and execution to 60 seconds.

Publisher ID `samtherocket` is the intended Marketplace identity and is
not claimed or registered by this build. Publication is the owner's step.

The 0.2.0 preview adds general correctness linting and independent formatting
preview/apply commands for saved Python files. Formatting is bounded to the
contract in docs/INDEPENDENT_ENGINES.md; notebooks are not supported by these
new commands. A formatting preview does not write. The explicitly named
apply command writes using the engine source-hash and atomic-write checks.


Version 0.3.0 adds lexical scope and project import-impact JSON reports and a
local-variable rename proposal command. Rename prompts for a top-level function,
old name and new name, then displays the read-only plan. Set `refactrail.importRoot`
to `src` for a source-layout package. `refactrail.lineLength` (40-200, default 79)
controls supported comma-group wrapping. Independent format commands accept saved
Python notebooks and preserve cell outputs/metadata. Other commands require Python
or stub files. Notebook linting does not infer kernel execution state.
