"""Build a local VSIX archive without contacting the Marketplace."""

import argparse
import json
from pathlib import Path
import xml.etree.ElementTree as xml
import zipfile

FILES = ("package.json", "extension.js", "runner.js", "README.md", "LICENSE")
NAMESPACE = "http://schemas.microsoft.com/developer/vsx-schema/2011"


def manifest_bytes(metadata):
    """Create the standard VSIX manifest for a workspace extension."""
    xml.register_namespace("", NAMESPACE)
    root = xml.Element("PackageManifest", {"Version": "2.0.0", "xmlns": NAMESPACE})
    details = xml.SubElement(root, "Metadata")
    xml.SubElement(details, "Identity", {
        "Language": "en-US", "Id": metadata["name"],
        "Version": metadata["version"], "Publisher": metadata["publisher"],
    })
    xml.SubElement(details, "DisplayName").text = metadata["displayName"]
    xml.SubElement(details, "Description").text = metadata["description"]
    xml.SubElement(details, "Tags").text = "python,refactoring,preview"
    xml.SubElement(details, "Categories").text = "Programming Languages"
    properties = xml.SubElement(details, "Properties")
    for name, value in {
        "Microsoft.VisualStudio.Code.Engine": metadata["engines"]["vscode"],
        "Microsoft.VisualStudio.Code.ExtensionKind": "workspace",
        "Microsoft.VisualStudio.Code.ExecutesCode": "true",
        "Microsoft.VisualStudio.Services.Preview": "true",
    }.items():
        xml.SubElement(properties, "Property", {"Id": name, "Value": value})
    installation = xml.SubElement(root, "Installation")
    xml.SubElement(installation, "InstallationTarget", {"Id": "Microsoft.VisualStudio.Code"})
    xml.SubElement(root, "Dependencies")
    assets = xml.SubElement(root, "Assets")
    for kind, filename in (
        ("Microsoft.VisualStudio.Code.Manifest", "package.json"),
        ("Microsoft.VisualStudio.Services.Content.Details", "README.md"),
        ("Microsoft.VisualStudio.Services.Content.License", "LICENSE"),
    ):
        xml.SubElement(assets, "Asset", {
            "Type": kind, "Path": f"extension/{filename}", "Addressable": "true",
        })
    return xml.tostring(root, encoding="utf-8", xml_declaration=True)


def main():
    """Package the explicit extension allowlist to a new local archive."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extension", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    metadata = json.loads((arguments.extension / "package.json").read_text("utf-8"))
    for name in FILES:
        if not (arguments.extension / name).is_file():
            raise ValueError(f"Missing extension file: {name}")
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    types = ('<?xml version="1.0" encoding="utf-8"?>'
             '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
             '<Default Extension="json" ContentType="application/json"/>'
             '<Default Extension="js" ContentType="application/javascript"/>'
             '<Default Extension="md" ContentType="text/markdown"/>'
             '<Default Extension="vsixmanifest" ContentType="text/xml"/>'
             '<Default Extension="" ContentType="text/plain"/>'
             '</Types>')
    with zipfile.ZipFile(arguments.output, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("[Content_Types].xml", types)
        archive.writestr("extension.vsixmanifest", manifest_bytes(metadata))
        for name in FILES:
            archive.write(arguments.extension / name, f"extension/{name}")
    print(arguments.output)


if __name__ == "__main__":
    main()
