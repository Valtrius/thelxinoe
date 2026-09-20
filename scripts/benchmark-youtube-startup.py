"""Compare isolated yt-dlp CLI and resident API extraction in the server image.

Usage: /opt/streamlink/bin/python benchmark-youtube-startup.py inputs.json
Input: {"youtube": [{"id": "public-video-id"}, ...]}. Output contains timings only.
Install the same yt-dlp version with its default extras into
/tmp/thelxinoe-benchmark-packages first. This leaves installed server tools intact.
"""
import hashlib
import json
import logging
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


class QuietLogger:
    def debug(self, *_args):
        pass

    warning = error = debug


def main():
    inputs = json.loads(Path(sys.argv[1]).read_text())
    root = Path("/var/lib/thelxinoe/tools")
    # Select the single installed test bundle; refuse an ambiguous installation.
    tools = {}
    for name in ["yt-dlp", "deno"]:
        paths = list((root / name).glob(f"*/*/{name}"))
        if len(paths) != 1:
            raise RuntimeError("Expected one installed development tool bundle")
        tools[name] = paths[0]
    with tempfile.TemporaryDirectory(prefix="thelxinoe-youtube-benchmark-") as home:
        os.environ.clear()
        os.environ.update({key: home for key in ["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "TMPDIR"]})
        os.environ.update({"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8"})
        os.chdir(home)
        logging.disable(logging.CRITICAL)
        sys.path.insert(0, "/tmp/thelxinoe-benchmark-packages")
        import yt_dlp
        installed = subprocess.check_output([str(tools["yt-dlp"]), "--version"], text=True).strip()
        if yt_dlp.version.__version__ != installed:
            raise RuntimeError("Benchmark package and managed executable versions differ")
        options = {
            "quiet": True, "no_warnings": True, "logger": QuietLogger(),
            "cachedir": False, "noplaylist": True, "skip_download": True,
            "socket_timeout": 15, "retries": 2, "extractor_retries": 2,
            "js_runtimes": {"deno": {"path": str(tools["deno"])}},
            "remote_components": set(), "plugin_dirs": [],
        }
        results = []
        with yt_dlp.YoutubeDL(options) as extractor:
            for repeat in range(3):
                for index, video in enumerate(inputs["youtube"]):
                    address = "https://www.youtube.com/watch?v=" + video["id"]
                    for mode in (["cli", "worker"] if repeat % 2 == 0 else ["worker", "cli"]):
                        start = time.monotonic()
                        succeeded = False
                        try:
                            if mode == "cli":
                                command = [str(tools["yt-dlp"]), "--ignore-config", "--no-plugin-dirs",
                                    "--no-cache-dir", "--no-cookies", "--no-cookies-from-browser",
                                    "--no-playlist", "--no-remote-components", "--no-js-runtimes",
                                    "--js-runtimes", "deno:" + str(tools["deno"]), "--socket-timeout", "15",
                                    "--retries", "2", "--extractor-retries", "2", "--dump-single-json",
                                    "--skip-download", "--", address]
                                output = subprocess.run(command, capture_output=True, timeout=120)
                                metadata = json.loads(output.stdout) if output.returncode == 0 else None
                            else:
                                metadata = extractor.extract_info(address, download=False)
                            formats = metadata.get("formats", []) if metadata else []
                            supported = [f for f in formats if
                                str(f.get("vcodec", "")).startswith(("avc1", "h264"))
                                and 0 < (f.get("height") or 0) <= 1080
                                and f.get("protocol") in ["https", "m3u8_native", "m3u8"]]
                            audio = any(f.get("acodec") not in [None, "none"] for f in formats)
                            succeeded = bool(metadata and metadata.get("id") == video["id"] and supported and audio)
                        except Exception:
                            pass
                        row = {"video": index, "repeat": repeat, "stage": mode,
                               "seconds": round(time.monotonic() - start, 3), "succeeded": succeeded}
                        results.append(row)
                        print(json.dumps(row), flush=True)
        start = time.monotonic()
        for tool in tools.values():
            with tool.open("rb") as source:
                hashlib.file_digest(source, "sha256")
        summary = {"tool_hash_seconds": round(time.monotonic() - start, 3)}
        for mode in ["cli", "worker"]:
            runs = [row["seconds"] for row in results if row["stage"] == mode and row["succeeded"]]
            summary[mode] = {"succeeded": len(runs), "failed": len(inputs["youtube"]) * 3 - len(runs),
                             "median": statistics.median(runs) if runs else None}
        print(json.dumps({"summary": summary}), flush=True)


if __name__ == "__main__":
    main()
