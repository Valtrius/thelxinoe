"""Build and qualify the offline seed using the same package path as the server."""
import errno
import http.client
import importlib.util
import json
import random
import shutil
import socket
import ssl
from pathlib import Path
import sys
import time
import urllib.error

sys.dont_write_bytecode = True
source = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("package", source / "apps/server/src/tools/package.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)
download = package.acquire
ATTEMPTS = 5
# Pinned artifacts never change, so any other HTTP answer (404, 403, 410) is final.
RETRY_STATUS = {408, 425, 429, 500, 502, 503, 504}
NETWORK_ERRNO = {errno.ENETUNREACH, errno.EHOSTUNREACH, errno.ENETDOWN}


def transient(error):
    """Network trouble another attempt can ride out. Digest and size failures
    (RuntimeError), certificate failures and local I/O errors are final."""
    if isinstance(error, urllib.error.HTTPError):
        return error.code in RETRY_STATUS
    if isinstance(error, urllib.error.URLError):
        error = error.reason
    if isinstance(error, ssl.SSLCertVerificationError):
        return False
    # TimeoutError includes the TLS handshake timeout; RemoteDisconnected is a ConnectionError.
    return (isinstance(error, (TimeoutError, ConnectionError, socket.gaierror, EOFError,
                               ssl.SSLEOFError, ssl.SSLZeroReturnError, ssl.SSLSyscallError,
                               http.client.IncompleteRead, http.client.BadStatusLine))
            or isinstance(error, OSError) and error.errno in NETWORK_ERRNO)


def backoff(error, attempt):
    """Exponential with jitter; a numeric Retry-After wins, up to two minutes."""
    retry_after = getattr(error, "headers", None) and error.headers.get("Retry-After", "")
    if retry_after and retry_after.strip().isdigit():
        return min(120, int(retry_after))
    return min(60, 5 * 2 ** (attempt - 1)) * random.uniform(0.5, 1)


def acquire(root, value):
    """Every image build fetches all pinned artifacts; ride out brief outages.
    Anything else, a 404 or a digest mismatch included, fails at once."""
    for attempt in range(1, ATTEMPTS + 1):
        try:
            return download(root, value)
        except Exception as error:
            if attempt == ATTEMPTS or not transient(error):
                print(f"Download failed after {attempt} attempt(s): {value['url']}: {error!r}",
                      file=sys.stderr, flush=True)
                raise
            delay = backoff(error, attempt)
            print(f"Retrying {value['url']} in {delay:.0f} s after {error!r}", file=sys.stderr, flush=True)
            time.sleep(delay)


package.acquire = acquire
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
