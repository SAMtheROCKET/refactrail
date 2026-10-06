"""Build a local VSIX archive without contacting the Marketplace.

The layout follows Microsoft's vsce: the licence is stored as LICENSE.txt,
content types are keyed by dotted extensions, preview extensions carry the
"Public Preview" gallery flag and the icon is a declared asset.
"""

import argparse
import json
from pathlib import Path
import xml.etree.ElementTree as xml
import zipfile

# Source file -> path inside the archive's extension/ folder.
FILES = {"package.json": "package.json", "extension.js": "extension.js",
         "runner.js": "runner.js", "setup.js": "setup.js",
         "README.md": "readme.md", "LICENSE": "LICENSE.txt",
         "icon.png": "icon.png"}
NAMESPACE = "http://schemas.microsoft.com/developer/vsx-schema/2011"
CONTENT_TYPES = {".js": "application/javascript", ".json": "application/json",
                 ".md": "text/markdown", ".png": "image/png",
                 ".txt": "text/plain", ".vsixmanifest": "text/xml"}


def link_properties(metadata):
    """Return the Marketplace links derived from repository and bugs."""
    repository = metadata["repository"]["url"]
    return {
        "Microsoft.VisualStudio.Services.Links.Source": repository,
        "Microsoft.VisualStudio.Services.Links.Getstarted": repository,
        "Microsoft.VisualStudio.Services.Links.GitHub": repository,
        "Microsoft.VisualStudio.Services.Links.Support": metadata["bugs"]["url"],
        "Microsoft.VisualStudio.Services.Links.Learn": metadata["homepage"],
    }


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
    xml.SubElement(details, "Tags").text = ",".join(metadata.get("keywords", []))
    xml.SubElement(details, "Categories").text = ",".join(metadata["categories"])
    flags = ["Public"] + (["Preview"] if metadata.get("preview") else [])
    xml.SubElement(details, "GalleryFlags").text = " ".join(flags)
    properties = xml.SubElement(details, "Properties")
    for name, value in {
        "Microsoft.VisualStudio.Code.Engine": metadata["engines"]["vscode"],
        "Microsoft.VisualStudio.Code.ExtensionDependencies": "",
        "Microsoft.VisualStudio.Code.ExtensionPack": "",
        "Microsoft.VisualStudio.Code.ExtensionKind": ",".join(metadata["extensionKind"]),
        "Microsoft.VisualStudio.Code.LocalizedLanguages": "",
        "Microsoft.VisualStudio.Code.EnabledApiProposals": "",
        "Microsoft.VisualStudio.Code.ExecutesCode": "true",
        **link_properties(metadata),
        "Microsoft.VisualStudio.Services.Branding.Color":
            metadata["galleryBanner"]["color"],
        "Microsoft.VisualStudio.Services.Branding.Theme":
            metadata["galleryBanner"]["theme"],
        "Microsoft.VisualStudio.Services.GitHubFlavoredMarkdown": "true",
        "Microsoft.VisualStudio.Services.Content.Pricing": "Free",
    }.items():
        xml.SubElement(properties, "Property", {"Id": name, "Value": value})
    xml.SubElement(details, "License").text = "extension/LICENSE.txt"
    xml.SubElement(details, "Icon").text = "extension/icon.png"
    installation = xml.SubElement(root, "Installation")
    xml.SubElement(installation, "InstallationTarget", {"Id": "Microsoft.VisualStudio.Code"})
    xml.SubElement(root, "Dependencies")
    assets = xml.SubElement(root, "Assets")
    for kind, filename in (
        ("Microsoft.VisualStudio.Code.Manifest", "package.json"),
        ("Microsoft.VisualStudio.Services.Content.Details", "readme.md"),
        ("Microsoft.VisualStudio.Services.Content.License", "LICENSE.txt"),
        ("Microsoft.VisualStudio.Services.Icons.Default", "icon.png"),
    ):
        xml.SubElement(assets, "Asset", {
            "Type": kind, "Path": f"extension/{filename}", "Addressable": "true",
        })
    return xml.tostring(root, encoding="utf-8", xml_declaration=True)


def content_types_str():
    """Return [Content_Types].xml covering every packaged file type."""
    defaults = "".join(f'<Default Extension="{suffix}" ContentType="{kind}"/>'
                       for suffix, kind in CONTENT_TYPES.items())
    return ('<?xml version="1.0" encoding="utf-8"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            f'{defaults}</Types>')


def main():
    """Package the explicit extension allowlist to a new local archive."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--extension", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    metadata = json.loads((arguments.extension / "package.json").read_text("utf-8"))
    if metadata.get("icon") != "icon.png":
        raise ValueError('package.json must declare "icon": "icon.png"')
    for name in FILES:
        if not (arguments.extension / name).is_file():
            raise ValueError(f"Missing extension file: {name}")
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(arguments.output, "x", zipfile.ZIP_DEFLATED) as archive:
        archive.writestr("extension.vsixmanifest", manifest_bytes(metadata))
        archive.writestr("[Content_Types].xml", content_types_str())
        for name, packaged in FILES.items():
            archive.write(arguments.extension / name, f"extension/{packaged}")
    print(arguments.output)


if __name__ == "__main__":
    main()
