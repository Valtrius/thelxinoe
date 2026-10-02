"""Live provider qualification, separate from deterministic CI.

Run with image Python: qualify-server-tools.py <tools root> <admin/tools JSON>.
Output contains public provider names, outcomes and exact package identities only.
An inconclusive probe is not evidence of a package regression.
"""
import importlib.util
import json
from pathlib import Path
import sys
from datetime import datetime, timezone

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("package", source / "apps/server/src/tools/package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
root = Path(sys.argv[1])
package.CACHE_ROOT = root / "catalog"
inventory = json.loads(Path(sys.argv[2]).read_text())["items"]
selected = {item["id"]: json.loads((root / "packages" / item["installed"]["id"] / "manifest.json").read_text()) for item in inventory}
report = {"checked_at": datetime.now(timezone.utc).isoformat(), "command": "python3 -B scripts/qualify-server-tools.py <tools root> <admin-tools.json>",
          "packages": {key: value["candidate"] for key, value in selected.items()}, "results": []}
for kind in ("youtube", "streamlink"):
    result = package.probe(root, {"kind": kind, "candidate": selected, "current": selected,
        "youtube_script": (source / "apps/server/src/online/youtube_worker.py").read_text(),
        "streamlink_script": (source / "apps/server/src/online/streamlink_worker.py").read_text()})
    report["results"].append(result)
print(json.dumps(report, indent=2))
