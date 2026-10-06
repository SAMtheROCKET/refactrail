# RefacTrail for VS Code

RefacTrail, the Python refactorizer: linting, formatting and verified
refactoring, without leaving the editor.

## Getting started

1. Install this extension, open a project and trust the workspace.
2. Run any **RefacTrail:** command from the Command Palette (Ctrl+Shift+P).
3. The first time, if RefacTrail is not in your Python environment yet,
   click **Install**. The extension runs `pip install refactrail==0.5.1a0`
   in that interpreter (FuncLoom and the fast Rust engine come with it),
   and only after your click.

The interpreter is the one selected in VS Code's Python extension (or
`python` on your PATH). To use another, click **Choose interpreter** or set
`refactrail.pythonPath`.

## Commands

- **Check active file** and **Lint general correctness**: findings appear in
  the Problems panel.
- **Preview fixes** / **Apply fixes**: bounded fixes; the preview opens a
  diff and changes nothing.
- **Preview independent formatting** / **Apply independent formatting**:
  also for notebooks, keeping cell outputs and metadata.
- **Inspect lexical scopes**, **Inspect project import impact** and
  **Preview local variable rename**: read-only reports.

Settings: `refactrail.profile` (standard or strict), `refactrail.lineLength`
(40-200, default 79), `refactrail.bracketStyle` (own-line or hug) and
`refactrail.importRoot` (for example `src`).

Writes happen only through the explicit **Apply** commands, after a check
that the file has not changed and that the result still compiles to the
same program. Save files before running a command. Nothing is sent to any
service, the checked code is never run, and each command is limited to 60
seconds and 8 MB of output.
