"""Build and qualify the offline seed using the same package path as the server."""
import importlib.util
import json
import shutil
from pathlib import Path
import sys

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("package", source / "apps/server/src/tools/package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
destination = Path(sys.argv[1])
destination.mkdir(parents=True, exist_ok=True)
package.CACHE_ROOT = destination / "catalog"
lock = source / "scripts/server-tools.lock.json"
if "--refresh" in sys.argv:
    candidates = [package.discover(tool, channel) for tool, channel in
                  [("yt-dlp", "nightly"), ("deno", "lts"), ("streamlink", "stable"), ("ffmpeg", "stable")]]
    lock.write_text(json.dumps(candidates, indent=2) + "\n")
else:
    candidates = json.loads(lock.read_text())
(destination / "catalog.json").write_text(json.dumps(candidates, indent=2) + "\n")
generations = []
for candidate in candidates:
    generation = package.prepare(destination, candidate)
    generations.append(generation)
    package.emit(qualified=candidate["tool"], version=candidate["version"], candidate=candidate["id"])
selected = {generation["candidate"]["tool"]: generation for generation in generations}
package.protocol(destination, {"kind": "youtube", "yt_dlp": selected["yt-dlp"], "deno": selected["deno"],
                               "script": (source / "apps/server/src/online/youtube_worker.py").read_text()})
package.protocol(destination, {"kind": "streamlink", "streamlink": selected["streamlink"],
                               "script": (source / "apps/server/src/online/streamlink_worker.py").read_text()})
(destination / "seed.json").write_text(json.dumps(generations, indent=2) + "\n")
if "--compact" in sys.argv:
    # Keep the exact, qualified input artifacts. Initial startup reconstructs the
    # writable packages offline, without storing a second expanded copy in the image.
    shutil.rmtree(destination / "packages")
