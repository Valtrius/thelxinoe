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
        selected = next(streams[k] for k in ["1080p", "1080p60", "720p", "720p60", "480p", "best"] if k in streams)
        result = {"url": selected.to_url()}
    except Exception:
        result = {"error": "Public stream extraction failed"}
    print(json.dumps(result, separators=(",", ":")), flush=True)
