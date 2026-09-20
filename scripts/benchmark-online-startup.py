"""Run inside the server image. Input JSON contains public Twitch logins only.

Prints timings, never signed stream URLs. Uses temporary isolated tool homes.
Usage: /opt/streamlink/bin/python benchmark-online-startup.py inputs.json
"""

import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time


def isolated(home):
    return {
        **{key: home for key in ["HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME", "TMPDIR"]},
        "LANG": "C.UTF-8",
        "PATH": "/usr/bin:/bin",
    }


def pipeline(url, seconds, probe, directory):
    path = Path(directory)
    command = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-nostdin", "-y",
               "-protocol_whitelist", "https,tls,tcp,crypto", "-rw_timeout", "15000000"]
    if probe:
        command += ["-probesize", "524288", "-analyzeduration", "1000000"]
    command += ["-i", url, "-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn", "-map_metadata", "-1",
                "-c:v", "libx264", "-preset", "veryfast", "-threads", "2", "-pix_fmt", "yuv420p",
                "-vf", "scale=w='min(1920,iw)':h=-2", "-b:v", "8000000", "-maxrate", "8000000",
                "-bufsize", "16000000", "-c:a", "aac", "-b:a", "192k", "-ac", "2", "-ar", "48000",
                "-force_key_frames", f"expr:gte(t,n_forced*{seconds})", "-avoid_negative_ts", "make_zero",
                "-f", "hls", "-hls_time", str(seconds), "-hls_list_size", "20", "-hls_delete_threshold", "2",
                "-hls_flags", "delete_segments+independent_segments+temp_file",
                "-hls_segment_filename", str(path / "segment-%06d.ts"), str(path / "index.m3u8")]
    start = time.monotonic()
    process = subprocess.Popen(command, env=isolated(directory), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        while time.monotonic() - start < 25:
            if (path / "index.m3u8").exists():
                return round(time.monotonic() - start, 3)
            if process.poll() is not None:
                return None
            time.sleep(0.02)
        return None
    finally:
        process.kill()
        process.wait()


def main():
    inputs = json.loads(Path(sys.argv[1]).read_text())
    with tempfile.TemporaryDirectory(prefix="thelxinoe-benchmark-") as home:
        os.environ.clear()
        os.environ.update(isolated(home))
        import streamlink
        session = streamlink.Streamlink()
        session.set_option("http-timeout", 15)
        results = []
        for repeat in range(3):
            for index, channel in enumerate(inputs["twitch"]):
                address = "https://www.twitch.tv/" + channel["login"]
                order = ["cli", "worker"] if repeat % 2 == 0 else ["worker", "cli"]
                stream_url = None
                for mode in order:
                    start = time.monotonic()
                    try:
                        if mode == "cli":
                            with tempfile.TemporaryDirectory(prefix="cli-", dir=home) as cli_home:
                                output = subprocess.run(["/opt/streamlink/bin/streamlink", "--no-config", "--loglevel", "error",
                                    "--stream-url", "--http-timeout", "15", address,
                                    "1080p,1080p60,720p,720p60,480p,best"], env=isolated(cli_home), capture_output=True, timeout=45)
                            if output.returncode:
                                raise RuntimeError("extraction")
                            stream_url = output.stdout.decode().strip()
                        else:
                            streams = session.streams(address)
                            selected = next(streams[k] for k in ["1080p", "1080p60", "720p", "720p60", "480p", "best"] if k in streams)
                            stream_url = selected.to_url()
                        result = {"channel": index, "repeat": repeat, "stage": mode, "seconds": round(time.monotonic() - start, 3)}
                    except Exception:
                        result = {"channel": index, "repeat": repeat, "stage": mode, "seconds": None}
                    results.append(result)
                    print(json.dumps(result), flush=True)
                if stream_url and repeat < 2:
                    variants = [(6, False), (2, False), (2, True)]
                    if repeat % 2:
                        variants.reverse()
                    for seconds, probe in variants:
                        with tempfile.TemporaryDirectory(prefix="hls-", dir=home) as directory:
                            elapsed = pipeline(stream_url, seconds, probe, directory)
                        result = {"channel": index, "repeat": repeat, "stage": f"hls-{seconds}s-{'short-probe' if probe else 'default'}", "seconds": elapsed}
                        results.append(result)
                        print(json.dumps(result), flush=True)
        summary = {}
        for stage in dict.fromkeys(r["stage"] for r in results):
            runs = [r["seconds"] for r in results if r["stage"] == stage]
            valid = [r for r in runs if r is not None]
            summary[stage] = {"succeeded": len(valid), "failed": len(runs) - len(valid), "median": statistics.median(valid) if valid else None}
        print(json.dumps({"summary": summary}), flush=True)


if __name__ == "__main__":
    main()
