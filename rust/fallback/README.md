# refactrail-core

The Rust engine for [RefacTrail](https://github.com/SAMtheROCKET/refactrail),
the Python refactorizer. It makes `refactrail check`, `lint` and `format`
faster with output identical to RefacTrail's Python engine, and is
installed automatically with RefacTrail.

The engine is RefacTrail's own: a Rust lexer, parser, compile checks and
symbol table, written from the language reference with CPython 3.12 as the
test oracle. It contains no Ruff code. Neither engine imports or executes
the checked source. CPython is asked only for the exact message of a file
that fails to compile, so both engines report it identically.

Native wheels (CPython 3.12+ ABI3) cover Windows x86_64, macOS (Apple
silicon and Intel) and Linux x86_64 and ARM64 (glibc and musl). On any
other system pip installs this project's small pure-Python fallback wheel
instead, and RefacTrail uses its Python engine, so installation never
needs Rust. `--engine auto` (the default) uses the native engine when
present; `--engine rust` requires it. Parity tests cover the recorded corpora and fuzzed inputs,
not every possible Python program.

Build a wheel with `maturin build --release --locked`. Building requires
Rust and the locked Cargo dependencies. Release builds must rerun the
parent package's tests and the parity scripts.

The core's own code is MIT licensed. Dependency notices and their upstream
license declarations are in `THIRD_PARTY_LICENSES`, with an index;
regenerate them with `scripts/gen_third_party_licenses.py`. Local build
commands upload nothing.
