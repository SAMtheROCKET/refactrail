"""Regenerate rust/THIRD_PARTY_LICENSES from the locked dependency graph.

Usage: python3 scripts/gen_third_party_licenses.py RUST_FOLDER

Collects every registry crate that the workspace's normal and build
dependencies reach (dev-only dependencies are left out), copies its
license, copying and notice files, and writes index.json. The folder is
replaced only after every crate's files are found.
"""

from pathlib import Path
import json
import re
import shutil
import subprocess
import sys
import tempfile

LICENSE_PATTERN = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)",
                             re.IGNORECASE)


def collect_packages_list(rust_path: Path) -> list[dict]:
    """Return the registry packages reachable without dev dependencies.

    Args:
        rust_path (Path): Workspace folder with Cargo.lock.
    Returns:
        list[dict]: cargo metadata package records, sorted by name.
    """
    metadata_dict = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=rust_path, capture_output=True, text=True, check=True).stdout)
    packages_dict = {package["id"]: package
                     for package in metadata_dict["packages"]}
    nodes_dict = {node["id"]: node
                  for node in metadata_dict["resolve"]["nodes"]}
    pending_list = list(metadata_dict["workspace_members"])
    reached_set = set(pending_list)
    while pending_list:
        for dependency in nodes_dict[pending_list.pop()]["deps"]:
            kinds_set = {kind["kind"] for kind in dependency["dep_kinds"]}
            if kinds_set <= {"dev"} or dependency["pkg"] in reached_set:
                continue
            reached_set.add(dependency["pkg"])
            pending_list.append(dependency["pkg"])
    return sorted((packages_dict[package_id] for package_id in reached_set
                   if packages_dict[package_id]["source"]),
                  key=lambda package: (package["name"], package["version"]))


def copy_licenses_list(packages_list: list[dict], output_path: Path) -> list:
    """Copy each package's license files and return the index rows.

    Args:
        packages_list (list[dict]): Registry packages.
        output_path (Path): Empty folder to fill.
    Returns:
        list: Index rows (name, version, license, files).
    """
    index_list = []
    for package in packages_list:
        folder_str = f"{package['name']}-{package['version']}"
        source_path = Path(package["manifest_path"]).parent
        files_list = sorted(path for path in source_path.iterdir()
                            if path.is_file()
                            and LICENSE_PATTERN.match(path.name))
        if not files_list:
            raise SystemExit(f"No license file in {source_path}")
        (output_path / folder_str).mkdir()
        for path in files_list:
            shutil.copyfile(path, output_path / folder_str / path.name)
        index_list.append({
            "name": package["name"], "version": package["version"],
            "license": package["license"],
            "files": [f"THIRD_PARTY_LICENSES/{folder_str}/{path.name}"
                      for path in files_list]})
    return index_list


def main() -> None:
    """Rebuild the license folder next to the workspace's Cargo.toml."""
    rust_path = Path(sys.argv[1]).resolve()
    target_path = rust_path / "THIRD_PARTY_LICENSES"
    with tempfile.TemporaryDirectory(dir=rust_path) as folder_str:
        output_path = Path(folder_str) / "THIRD_PARTY_LICENSES"
        output_path.mkdir()
        index_list = copy_licenses_list(collect_packages_list(rust_path),
                                        output_path)
        (output_path / "index.json").write_text(
            json.dumps(index_list, indent=2) + "\n", encoding="utf-8",
            newline="\n")
        if target_path.exists():
            shutil.rmtree(target_path)
        shutil.move(output_path, target_path)
    print(len(index_list), "crates:",
          ", ".join(row["name"] for row in index_list))


if __name__ == "__main__":
    main()
