"""Resolve a recorded admin/tools inventory into verified benchmark executables."""
import hashlib
import json
from pathlib import Path


def installed_tools(inputs):
    root = Path(inputs.get("state", "/var/lib/thelxinoe")) / "tools/packages"
    tools = {}
    for item in inputs["tools"]["items"]:
        directory = root / item["installed"]["id"]
        manifest = json.loads((directory / "manifest.json").read_text())
        for name, relative in manifest["executables"].items():
            executable = directory / relative
            with executable.open("rb") as source:
                if hashlib.file_digest(source, "sha256").hexdigest() != manifest["files"][relative]:
                    raise RuntimeError("Benchmark tool integrity failed")
            tools[name] = executable
    return tools
