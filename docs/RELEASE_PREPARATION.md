# Local release preparation

Version **0.2.0a0**, prepared 1 October 2026. Publication is on hold at
the owner's request, including TestPyPI, Marketplace and Product Hunt.
MIT was selected by the owner. The intended GitHub repository is
`SAMtheROCKET/refactrail`; prepared links do not establish that it is public.

## Candidate scope

This is an experimental release of deterministic source-analysis and bounded
rewrite tools. Unknown domain meanings and runtime types remain explicit.
Review generated edits and run the target project's tests. Passing the
included suite does not prove equivalence for arbitrary repositories.
FuncLoom and RefacTrail have separate packages and extensions. RefacTrail
requires FuncLoom's public rewrite API; the dependency is one-way.
Architecture generation and optional AI integrations are future work.

## Local checks

```powershell
python -m pip install -e ".[release]"
python scripts/verify.py
python scripts/release_check.py --output dist/0.2.0a0 --dependency-wheel path/to/funcloom-0.10.2a0-py3-none-any.whl
```

The release script requires a new output directory. It builds a wheel and
source archive, checks metadata with strict Twine validation, checks MIT and
typing files, and rejects private history in archives. It installs the wheel
offline into a new environment, verifies its import location and CLI, runs
full verification and checks uninstall. It repeats those checks on a wheel
rebuilt solely from the source archive, then records SHA-256 hashes and
`verification.json`. Developer build dependencies must already be installed.

Use `--artifacts path/to/artifacts` with a new `--output` to test the exact
same wheel and source archive on another OS. RefacTrail additionally needs
its local FuncLoom dependency wheel. Logs and checksums, rather than version
numbers or prepared CI files, establish which checks actually passed.

## Editor and native core

The local VS Code extension lives in `editor/refactrail`. Build a new VSIX with:

```powershell
python scripts/package_editor.py --extension editor/refactrail --output path/to/refactrail.vsix
```

Install through **Extensions: Install from VSIX**, select the interpreter
containing the matching package version and open a trusted workspace.
The extension never downloads Python packages. Its `samtherocket` publisher
identity is intended, not registered by these scripts.

RefacTrail's optional native checker is separately packaged under `rust/`.
Its build includes third-party license notices, regenerated from the
locked dependency graph by `scripts/gen_third_party_licenses.py`. CI builds
and tests its wheels for Linux x86_64, Windows x86_64 and macOS arm64; other
platforms use the Python engine. FuncLoom itself has no native or third-party runtime dependency.

## Evidence boundaries

CI runs the test suite on Windows, Linux and macOS with Python 3.12-3.14.
The Release check workflow (run by hand or on a `v*` tag) builds the wheel
and sdist once and verifies the identical files on all three systems with
Python 3.12 and 3.14; it uploads nothing. First green runs: 3 October 2026.
Editor installation and automated command checks do not establish a complete
interactive notebook/user-interface acceptance test. Benchmarks are local
measurements and do not establish superiority over Ruff or Black.

For a stable release, execute the advertised platform/interpreter matrix,
exercise the full editor/notebook workflows, and collect representative
user-project regressions and compatibility feedback. This candidate should
be described as an alpha, not as a universal production refactoring engine.

## Owner's publishing sequence

1. Recheck package names, repository paths and the Marketplace publisher.
2. Create reviewed public repositories; exclude reports, environments,
   private handoffs and historical chat material from Git as well as sdists.
3. Configure trusted publishing or credentials on the owner's accounts.
4. Publish the verified FuncLoom dependency before RefacTrail; verify index
   installation in a fresh environment, then publish the optional core.
5. Publish each verified VSIX and matching GitHub release separately.
6. Launch with the demonstrated examples and limitations; do not claim
   universal safety, proven speed leadership or complete rule enforcement.

No script here creates a public repository or uploads an artifact.
