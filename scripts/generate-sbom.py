#!/usr/bin/env python3
import json
import subprocess
import sys
import os

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--format-version", "1",
], text=True))

packages = []
relationships = []
for package in sorted(metadata["packages"], key=lambda item: item["id"]):
    spdx_id = "SPDXRef-" + "".join(char if char.isalnum() else "-" for char in package["id"])
    packages.append({
        "SPDXID": spdx_id,
        "name": package["name"],
        "versionInfo": package["version"],
        "downloadLocation": package.get("source") or "NOASSERTION",
        "licenseConcluded": package.get("license") or "NOASSERTION",
        "licenseDeclared": package.get("license") or "NOASSERTION",
        "filesAnalyzed": False,
    })
    relationships.append({
        "spdxElementId": "SPDXRef-DOCUMENT",
        "relationshipType": "DESCRIBES",
        "relatedSpdxElement": spdx_id,
    })

document = {
    "spdxVersion": "SPDX-2.3",
    "dataLicense": "CC0-1.0",
    "SPDXID": "SPDXRef-DOCUMENT",
    "name": "worko-ai-usage",
    "documentNamespace": "https://github.com/revolution-uz/worko-ai-usage/sbom/" + os.environ.get("GITHUB_SHA", "local"),
    "creationInfo": {
        "creators": ["Tool: worko-ai-usage-sbom-generator"],
        "created": subprocess.check_output(["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"], text=True).strip(),
    },
    "packages": packages,
    "relationships": relationships,
}
json.dump(document, sys.stdout, indent=2, sort_keys=True)
print()
