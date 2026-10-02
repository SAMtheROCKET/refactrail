"""Build and test local distributions without any upload or publication.

Install the project's [release] extra first. --artifacts reuses an existing
wheel and sdist, allowing the identical files to be checked on another OS.
Only trusted RefacTrail distributions should be passed to this script: their
build backend and the project's authored regression fixtures are executed.
"""

import argparse
from email.parser import BytesParser
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

PROJECT_PATH = Path(__file__).resolve().parent.parent
PRIVATE_NAMES = {"AGENTS.md", "PROJECT_CONTEXT.md", "BUILD_VERIFICATION.md",
                 "PRODUCT_PLAN.md", "ROADMAP.md"}
PRIVATE_PARTS = {".git", ".venv", "reports", "__pycache__"}


def run_command(arguments, folder, log_path):
    """Run a local verification command, saving all output and failing early."""
    environment = dict(os.environ)
    environment.pop("PYTHONPATH", None)
    environment.pop("PYTHONHOME", None)
    environment["PYTHONNOUSERSITE"] = "1"
    with log_path.open("w", encoding="utf-8") as log:
        result = subprocess.run(
            list(map(str, arguments)), cwd=folder, env=environment,
            stdout=log, stderr=subprocess.STDOUT, check=False,
        )
    if result.returncode:
        tail = log_path.read_text(encoding="utf-8").splitlines()[-25:]
        raise RuntimeError(f"Command failed ({result.returncode}): "
                           f"{arguments}\n" + "\n".join(tail))
    return log_path.read_text(encoding="utf-8")


def distribution_paths(folder):
    """Require exactly one wheel and one source distribution."""
    wheels, archives = list(folder.glob("*.whl")), list(folder.glob("*.tar.gz"))
    if len(wheels) != 1 or len(archives) != 1:
        raise ValueError("Expected exactly one wheel and one .tar.gz sdist")
    return wheels[0], archives[0]


def inspect_distributions(wheel, archive):
    """Check license metadata, source contents and absence of local history."""
    with zipfile.ZipFile(wheel) as package:
        wheel_names = package.namelist()
        metadata_name = next(name for name in wheel_names
                             if name.endswith(".dist-info/METADATA"))
        metadata = BytesParser().parsebytes(package.read(metadata_name))
        if metadata.get("License-Expression") != "MIT":
            raise ValueError("Wheel must declare the selected MIT license")
        if not any(name.endswith("/licenses/LICENSE") for name in wheel_names):
            raise ValueError("Wheel is missing its license file")
        if "refactrail/py.typed" not in wheel_names:
            raise ValueError("Wheel is missing the typing marker")
    with tarfile.open(archive) as package:
        archive_names = package.getnames()
    for name in wheel_names + archive_names:
        path = Path(name)
        if set(path.parts) & PRIVATE_PARTS or path.name in PRIVATE_NAMES:
            raise ValueError(f"Local-only content in distribution: {name}")
    for suffix in ("/tests/test_release_safety.py", "/scripts/verify.py",
                   "/scripts/release_check.py", "/LICENSE",
                   "/docs/RELEASE_PREPARATION.md"):
        if not any(name.endswith(suffix) for name in archive_names):
            raise ValueError(f"Required sdist content missing: {suffix}")
    return {"name": metadata["Name"], "version": metadata["Version"],
            "requires_python": metadata["Requires-Python"],
            "license": metadata["License-Expression"],
            "wheel_members": len(wheel_names),
            "sdist_members": len(archive_names)}


def check_installed(wheel, source, temporary, logs, label, version, dependencies):
    """Install a wheel offline in a fresh venv and run the full verification."""
    environment = temporary / label
    run_command([sys.executable, "-m", "venv", "--without-pip", environment],
                temporary, logs / f"{label}-venv.txt")
    executable = environment / ("Scripts/python.exe" if os.name == "nt"
                                else "bin/python")
    pip = [sys.executable, "-m", "pip", "--python", executable]
    run_command([*pip, "install", "--no-index", "--no-deps", *dependencies, wheel],
                temporary, logs / f"{label}-install.txt")
    run_command([*pip, "check"], temporary, logs / f"{label}-dependencies.txt")
    probe = (
        "import refactrail,importlib.metadata,json; "
        "print(json.dumps({'version':importlib.metadata.version('refactrail'),"
        "'file':refactrail.__file__}))"
    )
    runtime = json.loads(run_command([executable, "-I", "-c", probe],
                         temporary, logs / f"{label}-runtime.json"))
    if runtime["version"] != version or not Path(runtime["file"]).resolve(
    ).is_relative_to(environment.resolve()):
        raise ValueError(f"Verification did not use the installed wheel: {runtime}")
    entry = environment / ("Scripts/refactrail.exe" if os.name == "nt"
                            else "bin/refactrail")
    run_command([entry, "--version"], temporary, logs / f"{label}-entry.txt")
    run_command([executable, source / "scripts/verify.py"], source,
                logs / f"{label}-verify.txt")
    run_command([*pip, "uninstall", "-y", "refactrail"], temporary,
                logs / f"{label}-uninstall.txt")
    run_command([executable, "-I", "-c", "import importlib.util; "
                 "assert importlib.util.find_spec('refactrail') is None"],
                temporary, logs / f"{label}-removed.txt")
    return {**runtime, "verification": "passed", "uninstall": "passed"}


def check_artifacts(output, dependencies, skip_twine=False):
    """Test the wheel and a wheel rebuilt solely from the source archive."""
    wheel, archive = distribution_paths(output)
    metadata = inspect_distributions(wheel, archive)
    if skip_twine:
        (output / "twine-check.txt").write_text(
            "skipped: --skip-twine (run twine check --strict on these exact "
            "files elsewhere; see SHA256SUMS)\n", encoding="utf-8")
    else:
        run_command([sys.executable, "-m", "twine", "check", "--strict",
                     wheel, archive], PROJECT_PATH, output / "twine-check.txt")
    with tempfile.TemporaryDirectory(prefix="refactrail_release_") as raw:
        temporary = Path(raw).resolve()
        extracted = temporary / "source"
        extracted.mkdir()
        with tarfile.open(archive) as package:
            package.extractall(extracted, filter="data")
        source = next(path for path in extracted.iterdir() if path.is_dir())
        wheel_result = check_installed(wheel, source, temporary, output,
                                       "wheel", metadata["version"], dependencies)
        rebuilt = temporary / "rebuilt"
        run_command([sys.executable, "-m", "build", "--wheel",
                     "--no-isolation", "--outdir", rebuilt, source],
                    temporary, output / "sdist-rebuild.txt")
        source_wheel = next(rebuilt.glob("*.whl"))
        sdist_result = check_installed(source_wheel, source, temporary, output,
                                       "sdist", metadata["version"], dependencies)
    hashes = {path.name: sha256(path.read_bytes()).hexdigest()
              for path in (wheel, archive)}
    (output / "SHA256SUMS").write_text(
        "".join(f"{value}  {name}\n" for name, value in hashes.items()),
        encoding="utf-8")
    return {"metadata": metadata, "sha256": hashes,
            "wheel": wheel_result, "sdist_rebuild": sdist_result,
            "python": sys.version, "platform": sys.platform,
            "twine_check": "skipped" if skip_twine else "passed",
            "public_upload": False}


def main():
    """Create a new local release-check directory; never upload artifacts."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dependency-wheel", required=True, type=Path,
                        action="append",
                        help="Trusted local FuncLoom and refactrail-core "
                        "wheels (repeat the option for each)")
    parser.add_argument("--output", required=True, type=Path,
                        help="New directory for artifacts and verification logs")
    parser.add_argument("--artifacts", type=Path,
                        help="Existing trusted wheel/sdist to test instead of building")
    parser.add_argument("--skip-twine", action="store_true",
                        help="Record twine check as skipped (for hosts whose "
                        "policy blocks its native modules)")
    arguments = parser.parse_args()
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    try:
        if arguments.artifacts:
            for path in distribution_paths(arguments.artifacts.resolve()):
                shutil.copyfile(path, output / path.name)
        else:
            run_command([sys.executable, "-m", "build", "--no-isolation",
                         "--outdir", output, PROJECT_PATH], PROJECT_PATH,
                        output / "build.txt")
        dependencies = [path.resolve() for path in arguments.dependency_wheel]
        result = check_artifacts(output, dependencies, arguments.skip_twine)
        result["dependency_sha256"] = {
            path.name: sha256(path.read_bytes()).hexdigest()
            for path in dependencies}
        result["status"] = "passed"
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        result = {"status": "failed", "error": str(error),
                  "public_upload": False}
    (output / "verification.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
