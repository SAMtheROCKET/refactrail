# Current measurement status

## 0.3.1a0, own Rust engine, 3 October 2026

200 standard-library files, strict profile, native disk in WSL2 (Ubuntu
24.04, 4 CPUs used), median of repeated runs, including process start-up
(`scripts/` harnesses; method in [DUAL_ENGINES.md](DUAL_ENGINES.md)):

| Tool | Threads | Median |
| --- | ---: | ---: |
| RefacTrail native, strict profile | 4 | 0.035 s |
| RefacTrail native, standard profile | 4 | 0.033 s |
| Ruff check, E and F rules | default | 0.040 s |
| Ruff check, broad rule set | default | 0.222 s |
| RefacTrail native, strict profile | 1 | 0.091 s |
| Ruff check, E and F rules | 1 | 0.079 s |
| Ruff check, broad rule set | 1 | 0.293 s |

On four threads RefacTrail was faster than Ruff's E,F rule set on this
corpus; on one thread it was slower. The rule sets differ, so this is not a
like-for-like comparison and no general speed claim is made. Through the
Python CLI on the standard library (Linux, 4 cores, including interpreter
start-up), `--engine rust` took 1.3 s for `lint` (Python engine 7.0 s) and
3.6 s for `format --line-length 79` (25.6 s), with byte-identical output.

The sections below are historical: they were measured before RefacTrail had
its own parser, when the native core still used Ruff parser crates.

## 0.1.1a0 local check, 1 October 2026

113 authored source/test/example files; strict profile; Ubuntu WSL,
CPython 3.12.3, 24 logical CPUs. Three runs per configuration, no RefacTrail
cache. Includes file reads, compilation and rules; excludes CLI startup and
output rendering. This small corpus was on the Windows-mounted filesystem;
other local build activity and OS caching were not controlled.

| Engine | Workers | Median seconds |
| --- | --- | --- |
| python | 1 | 1.502 |
| python | 4 | 0.426 |
| rust | 1 | 0.731 |
| rust | 4 | 0.686 |

All findings matched in every run. The native engine was faster than serial
Python here, but slower than Python with four workers. These exploratory
measurements do not support a speed-leadership claim. Compilation preflight
and filesystem costs matter. Broader dedicated benchmarks remain necessary.
The same corpus also passed both-profile differential comparison with zero
differences; contextual invalid-code and suppression fixtures pass separately.

---

# Engine parity and benchmarks

Recorded on 1 October 2026 in WSL2 (Ubuntu 24.04, Python 3.12.3, 24
logical cores), refactrail 0.1.0a0 and refactrail-core 0.1.0a0 built with
`maturin develop --release` (Rust 1.98.1, Ruff parser crates tag 0.16.9).

## Parity

`scripts/compare_engines.py` checks every file with both engines in the
standard and strict profiles and compares the complete findings (path,
line, column, code, severity, message). RT001 is compared by code only,
because CPython and Ruff place and word syntax errors differently (for an
unclosed bracket CPython points at the bracket, Ruff at the end of file).

| Corpus | Files | Differing file checks |
| --- | ---: | ---: |
| RefacTrail and FuncLoom sources, tests and examples | 107 | 0 |
| `/usr/lib/python3.12` | 584 | 0 |
| `/usr/lib/python3/dist-packages` plus three private repositories | 15,565 | 0 |

On dist-packages alone (6,988 files) both engines report 742,344 findings
in the standard profile and 852,859 in the strict profile, with equal
counts for every code. A negative control (different settings) is detected
as a difference. `tests/test_engine_parity.py` adds 36 crafted edge cases
(encodings, BOM, CR and CRLF line endings, `# noqa` forms, syntax errors,
null bytes, continued `def` lines, `except*`, `match`, type parameters)
under three settings.

## Timing (dist-packages, 6,988 files)

| Measurement | Standard | Strict |
| --- | ---: | ---: |
| Python engine, serial, in-process | 47.71 s | 56.32 s |
| Rust engine, serial (`check_source` per file) | 2.06 s | 2.36 s |
| Rust engine, parallel (`check_files`) | 0.64 s | 0.72 s |

Whole CLI (`refactrail check ... --no-cache --output-format json`,
standard profile, including discovery, 742,344 findings and the JSON
report): Python engine with its process pool 7.58 s, Rust engine 2.7 s.
Both write byte-identical JSON. The remaining CLI time is mostly creating
the Python `Finding` objects and the report, which scale with the number
of findings, not files.

Measure again with `scripts/compare_engines.py` and the CLI; do not reuse
these numbers for other machines or corpora.
