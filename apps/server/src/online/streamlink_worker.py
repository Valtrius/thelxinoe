"""Private stdin/stdout protocol. No user configuration, credentials or URL logs."""
import json
import logging
import re
import sys

import streamlink

logging.disable(logging.CRITICAL)
session = streamlink.Streamlink()
session.set_option("http-timeout", 15)


def formats(streams):
    result = []
    seen = set()
    for name, stream in streams.items():
        if name in ("best", "worst", "audio_only"):
            continue
        address = stream.to_url()
        if address in seen:
            continue
        playlist = next((p for p in getattr(getattr(stream, "multivariant", None), "playlists", [])
                         if p.uri == address and not p.is_iframe), None)
        info = playlist.stream_info if playlist else None
        resolution = getattr(info, "resolution", None)
        match = re.fullmatch(r"(\d+)p(\d+)?(?:_alt\d*)?", name)
        height = resolution.height if resolution else int(match[1]) if match else 0
        if not height:
            continue
        width = resolution.width if resolution else None
        fps = getattr(info, "framerate", None) or (int(match[2]) if match and match[2] else None)
        codecs = getattr(info, "codecs", None) or []
        video = next((c for c in codecs if c.startswith(("avc1", "hvc1", "hev1", "vp09", "av01"))), "h264")
        audio = next((c for c in codecs if c.startswith(("mp4a", "opus"))), "aac")
        result.append({"format_id": name, "url": address, "width": width, "height": height,
                       "fps": fps, "vcodec": video, "acodec": audio, "protocol": "m3u8_native",
                       "ext": "mp4", "tbr": (getattr(info, "bandwidth", 0) or 0) / 1000})
        seen.add(address)
    if not result or len(result) > 128:
        raise ValueError("No supported variants")
    return {"is_live": True, "duration": 0, "formats": result}


print('{"ready":true}', flush=True)
for line in sys.stdin:
    try:
        request = json.loads(line)
        provider, channel = request["provider"], request["channel"]
        if provider not in ("twitch", "kick") or not re.fullmatch(r"[A-Za-z0-9_-]{1,100}", channel):
            raise ValueError("Invalid channel")
        domain = "www.twitch.tv" if provider == "twitch" else "kick.com"
        streams = session.streams(f"https://{domain}/{channel}")
        result = formats(streams)
    except Exception:
        result = {"error": "Public stream extraction failed"}
    print(json.dumps(result, separators=(",", ":")), flush=True)
