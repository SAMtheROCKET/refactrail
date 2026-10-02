# RefacTrail native checker

Experimental optional checker for RefacTrail 0.1.1a0. This separate Python
distribution uses PyO3, Rayon and Ruff parser crates. It requires
`refactrail==0.1.1a0`, which supplies settings and CPython compilation
validation. Neither engine executes the target source.

Install the base RefacTrail and FuncLoom wheels first, then this local wheel.
`refactrail check src --engine rust` selects it explicitly. Only Linux
x86_64 has been built and tested locally; other platforms use Python until
native wheels are validated. No performance superiority over other tools
is claimed. Parity tests cover the supported contract and observed corpus,
not every possible Python program.

Build a wheel with `maturin build --release` or a source archive with
`maturin sdist`. Building requires Rust, the Python build toolchain and the
locked Cargo dependencies, including Ruff source crates. Release builds
must rerun the parent package's tests and `scripts/compare_engines.py`.

The core's own code is MIT licensed. Dependency notices and their upstream
license declarations are included in `THIRD_PARTY_LICENSES`, with an index.
Publication remains the owner's decision; local build commands upload nothing.
