# Stable interfaces (beta contract)

This page lists what RefacTrail keeps stable from the first beta onward,
so that scripts, CI jobs, pre-commit hooks and editor extensions can rely
on it. Anything not listed here may still change between releases; such
changes are named in the [changelog](../CHANGELOG.md).

During the beta, a stable interface changes only when a release note
says so and only after a deprecation period of at least one minor
release (for example, an option keeps working with a warning before it
is removed). Bug fixes that make output more correct (for example a
finding that should not have been reported) are not interface changes.

## Commands

| Command | Stable | Purpose |
| --- | --- | --- |
| `refactrail check PATHS` | yes | RT style and RC correctness findings for the configured profile |
| `refactrail fix PATHS` | yes | Apply bounded RT refactoring edits (`--diff` to preview) |
| `refactrail lint PATHS` | yes | Correctness checks without style rules: RC codes, and the pycodestyle-/Pyflakes-compatible E and F codes with `--select` |
| `refactrail lint --fix` / `--diff` | yes | Safe fixes for F401, F541, F632, E703, E713 and E714 |
| `refactrail format PATHS` | yes | The independent formatter (`--diff`, `--check`, `--write`) |
| `refactrail rules` | yes | List the RT and RC codes |
| `refactrail scope`, `index`, `rename` | experimental | Evidence and analysis previews; output may change |

Stable options keep their names and meaning: `--select`, `--ignore`,
`--profile`, `--line-length`, `--output-format`, `--exit-zero`,
`--statistics`, `--no-cache`, `--jobs`, `--engine`, `--diff`, `--fix`,
`--check`, `--write`, `--notebooks` and `--bracket-style`. Option names
follow Ruff where the meaning is the same.

## Rule codes

- A code keeps its meaning once released. Codes are not reused.
- E and F codes keep pycodestyle's and Pyflakes' numbers, so `# noqa`
  comments written for Ruff, Flake8 or pycodestyle carry over.
- `# noqa`, `# noqa: CODE`, and file-level `# ruff: noqa` and
  `# flake8: noqa` keep working.
- RT001 (syntax error) and RT002 (unreadable or non-UTF-8 file) cannot
  be suppressed.
- New codes may be added in minor releases; they are only reported when
  their family is selected (RT style codes follow the profile).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | No findings (or `--exit-zero`), nothing to change, or changes written |
| 1 | Findings reported, or `--diff` / `--check` found pending changes |
| 2 | Invalid options or configuration, or a file that could not be read |

## Output formats

- `text`: one `path:line:column: CODE message` line per finding and a
  summary line. Meant for people; the summary wording may change.
- `json`: an array of objects with exactly the fields `path`, `line`,
  `column`, `code`, `severity` and `message`, sorted by path, line,
  column and code. `line` and `column` are 1-based; columns count
  Unicode characters. New fields may be added; existing ones keep their
  names and types.
- `sarif`: SARIF 2.1.0 for code-scanning services.
- `github` (`check`): GitHub Actions workflow annotations.

## Configuration

`[tool.refactrail]` in the nearest `pyproject.toml` accepts `profile`
(`standard` or `strict`), `line_length`, `function_preferred_lines`,
`function_max_lines`, `main_max_lines`, `select`, `ignore` and
`exclude`. Unknown keys are an error (exit code 2), so a misspelt key is
never silently ignored. Command-line options override the file.

## Engines

`--engine python` and `--engine rust` give identical findings; `auto`
(the default) uses the Rust engine when refactrail-core is installed
with its native extension, and the Python engine otherwise. Files the
Rust compiler checks reject are handled by the Python engine. Engine
choice never changes results, only speed.

## Safety guarantees

- Analysis never imports or runs the code it checks.
- `fix`, `lint --fix` and `format --write` change a file only after the
  new text compiles and passes its structural check, keep its line
  endings and BOM, write atomically, and refuse to write when the file
  changed after it was read or is a link.
- `--diff` and `--check` never write.

## Not yet stable

- The Python API (`refactrail.*` modules) other than running
  `python -m refactrail`. Import paths may move before 1.0.
- The `refactrail_core` extension module's functions, which serve the
  CLI.
- The native `refactrail-native` binary's command set, which does not
  yet include `fix`, `lint --fix` or `rename`.
- The text of messages (codes and positions are stable).
