"""Private stdin/stdout protocol. No user configuration, credentials or URL logs."""
import json
import logging
import re
import sys

import streamlink

logging.disable(logging.CRITICAL)
session = streamlink.Streamlink()
session.set_option("http-timeout", 15)
print('{"ready":true}', flush=True)
for line in sys.stdin:
    try:
        request = json.loads(line)
        provider, channel = request["provider"], request["channel"]
        if provider not in ("twitch", "kick") or not re.fullmatch(r"[A-Za-z0-9_-]{1,100}", channel):
            raise ValueError("Invalid channel")
        domain = "www.twitch.tv" if provider == "twitch" else "kick.com"
        streams = session.streams(f"https://{domain}/{channel}")
        formats = []
        for name, stream in streams.items():
            match = re.fullmatch(r"(\d+)p(\d+)?", name)
            if match:
                formats.append({"height": int(match[1]), "fps": int(match[2] or 30), "url": stream.to_url(), "protocol": "m3u8_native", "vcodec": "h264", "acodec": "aac", "ext": "mp4"})
        if not formats and "best" in streams:
            formats.append({"height": 1080, "fps": 30, "url": streams["best"].to_url(), "protocol": "m3u8_native", "vcodec": "h264", "acodec": "aac", "ext": "mp4"})
        if not formats:
            raise ValueError("No public formats")
        result = {"is_live": True, "formats": formats}
    except Exception:
        result = {"error": "Public stream extraction failed"}
    print(json.dumps(result, separators=(",", ":")), flush=True)
