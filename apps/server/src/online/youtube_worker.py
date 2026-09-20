"""Resident public extraction. Only framed JSON reaches stdout; errors are codes."""
import json
import logging
import re
import sys

sys.path.insert(0, sys.argv[1])
import yt_dlp
from yt_dlp.version import __version__
from yt_dlp.globals import plugin_dirs

logging.disable(logging.CRITICAL)
plugin_dirs.value = []
if __version__ != sys.argv[3]:
    raise RuntimeError("Extractor module version mismatch")


class QuietLogger:
    def debug(self, *_args):
        pass

    warning = error = debug


def failure(error):
    text = str(error).lower()
    if any(word in text for word in ["sign in", "login required", "log in", "use --cookies",
                                    "members-only", "private video", "confirm your age"]):
        return "extractor_authentication_required"
    if any(word in text for word in ["video unavailable", "not available", "has been removed", "copyright"]):
        return "unavailable"
    return "extraction_failed"


options = {
    "quiet": True, "no_warnings": True, "logger": QuietLogger(),
    "cachedir": False, "noplaylist": True, "skip_download": True,
    "socket_timeout": 15, "retries": 2, "extractor_retries": 2,
    "js_runtimes": {"deno": {"path": sys.argv[2]}},
    "remote_components": set(), "plugin_dirs": [],
}
with yt_dlp.YoutubeDL(options) as extractor:
    print(json.dumps({"ready": True, "version": __version__}), flush=True)
    for line in sys.stdin:
        try:
            request = json.loads(line)
            video = request["id"]
            if not isinstance(video, str) or not re.fullmatch(r"[A-Za-z0-9_-]{11}", video):
                raise ValueError("Invalid video")
            info = extractor.extract_info("https://www.youtube.com/watch?v=" + video, download=False)
            result = {"metadata": extractor.sanitize_info(info)}
        except Exception as error:
            result = {"error": failure(error)}
        encoded = json.dumps(result, separators=(",", ":"))
        if len(encoded.encode("utf-8")) >= 8 * 1024 * 1024:
            encoded = '{"error":"extraction_failed"}'
        print(encoded, flush=True)
