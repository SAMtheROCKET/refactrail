# refactrail-core

The Rust engine for [RefacTrail](https://github.com/SAMtheROCKET/refactrail),
the Python refactorizer. This optional distribution makes `refactrail check`,
`lint` and `format` faster with output identical to RefacTrail's Python
engine. It requires `refactrail==0.3.1a0`.

The engine is RefacTrail's own: a Rust lexer, parser, compile checks and
symbol table, written from the language reference with CPython 3.12 as the
test oracle. It contains no Ruff code. Neither engine imports or executes
the checked source. CPython is asked only for the exact message of a file
that fails to compile, so both engines report it identically.

Install RefacTrail and FuncLoom first, then this wheel. `--engine auto`
uses it when installed; `--engine rust` selects it explicitly. Wheels
(CPython 3.12+ ABI3) are built and tested in CI for Linux x86_64
(manylinux2014), Windows x86_64 and macOS arm64; other platforms use the
Python engine. Parity tests cover the recorded corpora and fuzzed inputs,
not every possible Python program.

Build a wheel with `maturin build --release --locked`. Building requires
Rust and the locked Cargo dependencies. Release builds must rerun the
parent package's tests and the parity scripts.

The core's own code is MIT licensed. Dependency notices and their upstream
license declarations are in `THIRD_PARTY_LICENSES`, with an index;
regenerate them with `scripts/gen_third_party_licenses.py`. Local build
commands upload nothing.
