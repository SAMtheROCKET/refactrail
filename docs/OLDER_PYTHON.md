# Using the tools on projects that use an older Python

FuncLoom, RefacTrail and FlowBlueprint need **Python 3.12 or newer** to
run. **Your project does not.** Code written for Python 3.8, 3.9, 3.10 or
3.11 can be checked, refactored and drawn by tools running on Python 3.12,
3.13 or 3.14.

## Why this works

The tools **read** your code as text and analyse it. They never import
it, run it or install its dependencies. So two Pythons can sit side by
side on one machine:

| | Tools Python | Project Python |
| --- | --- | --- |
| Version | 3.12, 3.13 or 3.14 | whatever your project uses, e.g. 3.8 |
| Used for | `funcloom`, `refactrail`, `flowblueprint` | running your program and its tests |
| Installed packages | the three tools only | your project's requirements |

Neither Python needs to know about the other, and the tools never change
which Python your project uses.

**Tested on 7 October 2026:** the PyPI releases (FuncLoom 0.10.4a0,
RefacTrail 0.5.0a0, FlowBlueprint 0.2.0a0) ran on Python 3.14 over a
script written for Python 3.8. The files they wrote (a refined script, a
long function split into parts, and a modular package with `main.py`)
compiled and gave identical results on Python 3.8, 3.9 and 3.10.

## Step 1: get a Python 3.12 or newer for the tools

Pick **one** of these.

### A. You already have Python 3.12+ installed (for example 3.14)

Create a separate environment just for the tools. It lives in your user
folder and needs no administrator rights.

Windows (Command Prompt; in PowerShell write `$env:USERPROFILE` instead of
`%USERPROFILE%`):

```bat
py -3.14 -m venv %USERPROFILE%\py-tools
%USERPROFILE%\py-tools\Scripts\python -m pip install funcloom refactrail flowblueprint
```

macOS and Linux:

```bash
python3.14 -m venv ~/py-tools
~/py-tools/bin/python -m pip install funcloom refactrail flowblueprint
```

Use `3.12` or `3.13` in place of `3.14` if that is what you have.

### B. You only have an old Python, or cannot install software

[uv](https://docs.astral.sh/uv/) downloads its own Python into your user
folder, so no administrator rights are needed. Install uv once:

```bash
# Windows (PowerShell)
powershell -ExecutionPolicy ByPass -c "irm https://astral.sh/uv/install.ps1 | iex"
# macOS and Linux
curl -LsSf https://astral.sh/uv/install.sh | sh
# or, with the Python you already have (then type `python -m uv` for `uv`)
python -m pip install --user uv
```

Then create the tools environment with a Python that uv fetches for you
(Windows lines are for Command Prompt, as above):

```bash
# Windows
uv venv --python 3.12 %USERPROFILE%\py-tools
uv pip install --python %USERPROFILE%\py-tools\Scripts\python.exe funcloom refactrail flowblueprint
# macOS and Linux
uv venv --python 3.12 ~/py-tools
uv pip install --python ~/py-tools/bin/python funcloom refactrail flowblueprint
```

For a quick one-off run without creating anything permanent:

```bash
uvx --python 3.12 funcloom check my_script.py
uvx --python 3.12 refactrail check my_script.py
uvx --python 3.12 flowblueprint my_script.py
```

## Step 2: run the tools

Call them through the tools Python. In these examples `TOOLS` stands for
`%USERPROFILE%\py-tools\Scripts\python` on Windows or `~/py-tools/bin/python`
on macOS and Linux.

```bash
TOOLS -m funcloom check my_script.py
TOOLS -m funcloom refine my_script.py --output my_script_refined.py
TOOLS -m funcloom modularize my_script.py --output my_package
TOOLS -m refactrail check my_script.py
TOOLS -m refactrail lint my_script.py --diff
TOOLS -m flowblueprint my_script.py -o my_script.svg
```

Using `python -m <tool>` also works where company security software blocks
the `funcloom.exe`-style launchers.

## Step 3: check the output with your project's Python

The tools reuse your own code and syntax, and in testing their output ran
unchanged on Python 3.8 to 3.10. There is no "target Python version"
setting yet, so confirm once with **your project's** Python:

```bash
# compile everything that was written (no code is run)
python3.8 -m compileall -q my_script_refined.py my_package
# then run your program or tests as usual
python3.8 my_script_refined.py
```

On Windows, use your project's interpreter, for example
`C:\my_project\.venv\Scripts\python -m compileall -q my_package`.

RefacTrail's `lint --fix` and `format --write` change files in place, so
keep them under version control (or work on a copy) and run your tests
afterwards with the project's Python.

## In VS Code

The extensions use the interpreter selected for your project. If that is
an old Python, they cannot install the tools there. Point them at the tools
Python instead, in **Settings** (search for *pythonPath*) or in
`.vscode/settings.json`:

```json
{
  "funcloom.pythonPath": "C:\\Users\\YOU\\py-tools\\Scripts\\python.exe",
  "refactrail.pythonPath": "C:\\Users\\YOU\\py-tools\\Scripts\\python.exe",
  "flowblueprint.pythonPath": "C:\\Users\\YOU\\py-tools\\Scripts\\python.exe"
}
```

On macOS and Linux use `/home/YOU/py-tools/bin/python` (or
`/Users/YOU/...`). Your project keeps its own interpreter for running and
debugging.

## Limits

- **Python 2 code is not supported.** A file such as `print "hello"` is
  refused with a parse error (`PARSE001` in FuncLoom, "cannot read" in
  FlowBlueprint) instead of being misread. Convert it to Python 3 first.
- **Python 3.6 or older code that uses `async` or `await` as ordinary
  names** cannot be read by Python 3.12, for the same reason.
- Other code that Python 3.7 to 3.11 accepts is read normally.

## Troubleshooting

| Message | Meaning and fix |
| --- | --- |
| `Could not find a version that satisfies the requirement funcloom (from versions: none)` | pip is running on Python 3.11 or older. Use the tools Python from Step 1. If it still appears, your company's package mirror does not carry the tools: check `python -m pip config list` and ask IT to allow them. |
| `'funcloom' is not recognized` / `command not found` | Call it as `TOOLS -m funcloom`, or after `uv tool install` run `uv tool update-shell` and open a new terminal. |
| The VS Code extension offers to install into the wrong Python | Set `funcloom.pythonPath`, `refactrail.pythonPath` and `flowblueprint.pythonPath` as shown above. |
