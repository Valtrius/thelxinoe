"""Bounded package acquisition/validation. Selection and policy belong to Rust storage."""
import hashlib
import errno
from concurrent.futures import ThreadPoolExecutor
import http.client
import base64
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request
import urllib.error
import uuid
import zipfile

MAX_ASSET = 512 * 1024 * 1024
MAX_EXPANDED = 2 * 1024 * 1024 * 1024
HOSTS = {"api.github.com", "github.com", "release-assets.githubusercontent.com",
         "objects.githubusercontent.com", "dl.deno.land", "pypi.org", "files.pythonhosted.org"}
PLATFORM = "linux-x86_64"
FFMPEG_BRANCH = "8.1"
CACHE_ROOT = None


def emit(**value):
    print(json.dumps(value, separators=(",", ":")), flush=True)


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    with open(path, "rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def identity(value):
    return hashlib.sha256(canonical({k: v for k, v in value.items() if k != "id"})).hexdigest()


def safe_url(url):
    parsed = urllib.parse.urlsplit(url)
    require(parsed.scheme == "https" and parsed.hostname in HOSTS and
            parsed.port in (None, 443) and not parsed.username and not parsed.password,
            "Package address is outside the supported release services")
    return url


class Redirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        safe_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def response(url, headers=None):
    request = urllib.request.Request(safe_url(url), headers={"User-Agent": "Thelxinoe-tools/1", **(headers or {})})
    return urllib.request.build_opener(Redirects()).open(request, timeout=30)


def metadata(url, limit=8 * 1024 * 1024):
    cached = None
    path = CACHE_ROOT / (hashlib.sha256(url.encode()).hexdigest() + ".json") if CACHE_ROOT else None
    if path and path.is_file():
        try:
            cached = json.loads(path.read_bytes())
        except (ValueError, OSError):
            pass
    headers = {"If-None-Match": cached["etag"]} if cached and cached.get("etag") else {}
    try:
        with response(url, headers) as source:
            body = source.read(limit + 1)
            etag = source.headers.get("ETag")
    except urllib.error.HTTPError as error:
        if error.code != 304 or not cached:
            raise
        body, etag = base64.b64decode(cached["body"], validate=True), cached.get("etag")
    require(len(body) <= limit, "Release metadata exceeds its size limit")
    if path and etag:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(".pending")
        temporary.write_bytes(canonical({"etag": etag, "body": base64.b64encode(body).decode()}))
        os.replace(temporary, path)
    return body


def asset(url, digest, size, name):
    safe_url(url)
    require(re.fullmatch(r"[a-f0-9]{64}", digest) and 0 < size <= MAX_ASSET,
            "Release has no valid digest or bounded artifact size")
    require(PurePosixPath(name).name == name and name not in (".", ".."), "Unsafe asset name")
    return {"url": url, "sha256": digest, "size": size, "name": name}


def github(repo, endpoint="latest"):
    value = json.loads(metadata(f"https://api.github.com/repos/{repo}/releases/{endpoint}"))
    require(not value["draft"], "Unpublished release")
    return value


def github_asset(release, name):
    selected = next(a for a in release["assets"] if a["name"] == name)
    return asset(selected["browser_download_url"], selected.get("digest", "").removeprefix("sha256:"),
                 selected["size"], name)


def run(args, timeout=60, cwd=None, request=None):
    env = {"PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8", "PYTHONDONTWRITEBYTECODE": "1",
           "PIP_CONFIG_FILE": os.devnull, "PIP_DISABLE_PIP_VERSION_CHECK": "1"}
    # The standalone extractor unpacks native libraries; /tmp may be small and noexec.
    with tempfile.TemporaryDirectory(prefix=".run-", dir=CACHE_ROOT.parent if CACHE_ROOT else Path.cwd()) as home:
        env.update({"HOME": home, "DENO_DIR": home, "XDG_CACHE_HOME": home, "TMPDIR": home})
        with tempfile.TemporaryFile() as output:
            try:
                result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, input=request,
                                        stdin=subprocess.DEVNULL if request is None else None,
                                        stdout=output, stderr=output, timeout=timeout, check=False)
            except subprocess.TimeoutExpired as error:
                raise RuntimeError("Package validation timed out") from error
            output.seek(0)
            text = output.read(8 * 1024 * 1024 + 1)
    require(len(text) <= 8 * 1024 * 1024, "Package subprocess output exceeds its limit")
    require(result.returncode == 0, "Package subprocess failed: " + text.decode(errors="replace")[-1500:])
    return text.decode(errors="replace")


def discover(tool, channel):
    require(platform.system() == "Linux" and platform.machine() == "x86_64", "Managed server tools require Linux x86-64")
    candidate = {"tool": tool, "channel": channel, "platform": PLATFORM, "artifacts": [], "python_abi": None}
    if tool == "yt-dlp":
        require(channel in ("nightly", "stable"), "Unsupported yt-dlp channel")
        repo = "yt-dlp/yt-dlp-nightly-builds" if channel == "nightly" else "yt-dlp/yt-dlp"
        release = github(repo)
        candidate.update(version=release["tag_name"], notes_url=release["html_url"], license="Unlicense",
                         source=f"https://github.com/{repo}",
                         artifacts=[github_asset(release, name) for name in ("yt-dlp_linux", "yt-dlp")])
    elif tool == "deno":
        require(channel == "lts", "Only Deno LTS is qualified")
        version = metadata("https://dl.deno.land/release-lts-latest.txt", 100).decode().strip()
        require(re.fullmatch(r"v\d+\.\d+\.\d+", version), "Invalid LTS version")
        name = "deno-x86_64-unknown-linux-gnu.zip"
        url = f"https://dl.deno.land/release/{version}/{name}"
        digest = metadata(url + ".sha256sum", 1024).decode().split()[0]
        with response(url) as source:
            size = int(source.headers["Content-Length"])
        candidate.update(version=version.removeprefix("v"), notes_url=f"https://github.com/denoland/deno/releases/tag/{version}",
                         source="https://dl.deno.land/release", license="MIT", artifacts=[asset(url, digest, size, name)])
    elif tool == "ffmpeg":
        require(channel == "stable", "Only the qualified FFmpeg branch is supported")
        releases = json.loads(metadata("https://api.github.com/repos/BtbN/FFmpeg-Builds/releases?per_page=10"))
        release = next(r for r in releases if r["tag_name"].startswith("autobuild-") and not r["draft"])
        suffix = f"-linux64-gpl-{FFMPEG_BRANCH}.tar.xz"
        name = next(a["name"] for a in release["assets"] if a["name"].endswith(suffix))
        candidate.update(version=FFMPEG_BRANCH, notes_url=release["html_url"],
                         source="https://github.com/BtbN/FFmpeg-Builds", license="GPL-3.0-or-later",
                         artifacts=[github_asset(release, name)])
    elif tool == "streamlink":
        require(channel == "stable", "Only stable Streamlink is qualified")
        version = json.loads(metadata("https://pypi.org/pypi/streamlink/json"))["info"]["version"]
        require(re.fullmatch(r"\d+(\.\d+)+", version), "Not a stable Streamlink release")
        with tempfile.TemporaryDirectory() as directory:
            report = Path(directory) / "report.json"
            run([sys.executable, "-I", "-m", "pip", "--isolated", "install", "--dry-run", "--ignore-installed",
                 "--only-binary=:all:", "--index-url", "https://pypi.org/simple", "--report", report,
                 f"streamlink=={version}"], timeout=240)
            graph = json.loads(report.read_text())["install"]
        wheels = []
        for item in graph:
            download = item["download_info"]
            url = download["url"]
            name = urllib.parse.unquote(urllib.parse.urlsplit(url).path.rsplit("/", 1)[-1])
            require(name.endswith(".whl"), "Source distributions are not permitted")
            with response(url) as source:
                size = int(source.headers["Content-Length"])
            wheel = asset(url, download["archive_info"]["hashes"]["sha256"], size, name)
            wheel.update(package=item["metadata"]["name"], version=item["metadata"]["version"])
            wheels.append(wheel)
        candidate.update(version=version, notes_url=f"https://github.com/streamlink/streamlink/releases/tag/{version}",
                         source="https://pypi.org/project/streamlink/", license="BSD-2-Clause",
                         python_abi=sys.implementation.cache_tag, artifacts=sorted(wheels, key=lambda a: a["name"]))
    else:
        raise RuntimeError("Unknown managed tool")
    candidate["id"] = identity(candidate)
    return candidate


def check_candidate(candidate):
    require(candidate["id"] == identity(candidate), "Candidate identity changed")
    require(candidate["platform"] == PLATFORM and platform.system() == "Linux" and platform.machine() == "x86_64",
            "Package platform is unsupported")
    require(candidate["tool"] in ("yt-dlp", "deno", "ffmpeg", "streamlink"), "Unknown package")
    require(0 < len(candidate["artifacts"]) <= 100, "Invalid dependency count")
    require(sum(value["size"] for value in candidate["artifacts"]) <= MAX_EXPANDED, "Package exceeds its total download limit")
    for value in candidate["artifacts"]:
        asset(value["url"], value["sha256"], value["size"], value["name"])
        prefixes = {
            "yt-dlp": ("https://github.com/yt-dlp/yt-dlp/releases/download/", "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/download/"),
            "deno": ("https://dl.deno.land/release/",),
            "ffmpeg": ("https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-",),
            "streamlink": ("https://files.pythonhosted.org/packages/",),
        }
        require(value["url"].startswith(prefixes[candidate["tool"]]), "Artifact is outside its tool's release source")
        if candidate["tool"] == "streamlink":
            require(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", value["package"]) and
                    re.fullmatch(r"[A-Za-z0-9_.+!-]+", value["version"]) and value["name"].endswith(".whl"), "Invalid wheel requirement")


def acquire(root, value):
    cache = root / "artifacts"
    cache.mkdir(parents=True, exist_ok=True)
    destination = cache / value["sha256"]
    if destination.is_file() and destination.stat().st_size == value["size"] and sha(destination) == value["sha256"]:
        return destination
    require(shutil.disk_usage(root).free > value["size"] + MAX_EXPANDED, "Insufficient space for tool staging")
    temporary = cache / (value["sha256"] + "." + uuid.uuid4().hex + ".part")
    started = time.monotonic()
    received = 0
    reported = 0
    try:
        with response(value["url"]) as source, temporary.open("xb") as output:
            while data := source.read(256 * 1024):
                received += len(data)
                require(received <= value["size"] and time.monotonic() - started < 600, "Download exceeded its size or time limit")
                output.write(data)
                if time.monotonic() - reported >= 0.5:
                    emit(stage="downloading", received=received, total=value["size"])
                    reported = time.monotonic()
            output.flush()
            os.fsync(output.fileno())
        emit(stage="verifying", received=received, total=value["size"])
        if received != value["size"]:
            raise EOFError("Artifact transfer ended before its expected size")
        require(sha(temporary) == value["sha256"], "Artifact digest mismatch")
        os.replace(temporary, destination)
        return destination
    finally:
        temporary.unlink(missing_ok=True)


def safe_member(name):
    path = PurePosixPath(name)
    require(not path.is_absolute() and all(p not in ("..", ".") for p in path.parts)
            and "\\" not in name and ":" not in name, "Unsafe archive member")
    return path


def unpack(archive, output, tool):
    total = 0
    seen = set()
    if tool == "deno":
        with zipfile.ZipFile(archive) as source:
            require(source.namelist() == ["deno"], "Unexpected Deno archive members")
            entry = source.infolist()[0]
            require(entry.file_size <= MAX_EXPANDED and (entry.external_attr >> 16) & 0o170000 != 0o120000, "Unsafe Deno entry")
            with source.open(entry) as data, (output / "deno").open("xb") as target:
                shutil.copyfileobj(data, target)
        return
    with tarfile.open(archive, "r:xz") as source:
        for entry in source:
            path = safe_member(entry.name)
            require(entry.name not in seen and len(seen) < 10000, "Duplicate or excessive archive members")
            seen.add(entry.name)
            require(entry.isfile() or entry.isdir(), "Archive links and special files are not permitted")
            total += entry.size
            require(total <= MAX_EXPANDED, "Expanded archive exceeds its limit")
            if entry.isfile() and (path.parts[-2:] in (("bin", "ffmpeg"), ("bin", "ffprobe"))
                                   or path.name.lower().startswith(("license", "copying"))):
                destination = output / path.name
                with source.extractfile(entry) as data, destination.open("xb") as target:
                    shutil.copyfileobj(data, target)


def validate(directory, candidate):
    tool = candidate["tool"]
    if tool == "yt-dlp":
        version = run([directory / "yt-dlp", "--ignore-config", "--version"]).strip()
        require(version == candidate["version"], "yt-dlp CLI version mismatch")
        module = directory / "yt-dlp-module"
        require(run([sys.executable, "-I", "-B", "-c",
                     "import sys;sys.path.insert(0,sys.argv[1]);from yt_dlp.version import __version__;print(__version__)", module]).strip() == version,
                "Resident extractor differs from its CLI")
        with zipfile.ZipFile(module) as source:
            require(any("yt_dlp_ejs" in name or name.endswith("yt.solver.lib.js") for name in source.namelist()), "Extractor is missing bundled EJS")
        return {"yt_dlp": "yt-dlp", "module": "yt-dlp-module"}
    if tool == "deno":
        version = run([directory / "deno", "--version"])
        # --version labels LTS as "stable" too. Channel identity is the frozen
        # dl.deno.land/release artifact and its checksum, never the version label.
        require(version.startswith("deno " + candidate["version"] + " ") and candidate["channel"] == "lts"
                and candidate["artifacts"][0]["url"].startswith("https://dl.deno.land/release/"), "Deno version or LTS channel mismatch")
        require(run([directory / "deno", "eval", "console.log(6*7)"]).strip() == "42", "Deno execution failed")
        return {"deno": "deno"}
    if tool == "streamlink":
        python = directory / "env/bin/python"
        version = run([python, "-I", "-B", "-c", "import streamlink, lxml.etree, Crypto; print(streamlink.__version__)"]).strip()
        require(version == candidate["version"], "Streamlink version mismatch")
        run([python, "-I", "-B", "-c", "import streamlink; s=streamlink.Streamlink(); s.resolve_url('https://www.twitch.tv/thelxinoe'); s.resolve_url('https://kick.com/thelxinoe')"])
        return {"python": "env/bin/python"}
    ffmpeg, ffprobe = directory / "ffmpeg", directory / "ffprobe"
    first = run([ffmpeg, "-version"]).splitlines()[0]
    second = run([ffprobe, "-version"]).splitlines()[0]
    require(first.split()[2] == second.split()[2] and first.split()[2].startswith("n" + candidate["version"]), "FFmpeg/FFprobe build mismatch")
    encoders = run([ffmpeg, "-hide_banner", "-encoders"])
    filters = run([ffmpeg, "-hide_banner", "-filters"])
    protocols = run([ffmpeg, "-hide_banner", "-protocols"])
    formats = run([ffmpeg, "-hide_banner", "-formats"])
    require(all(name in encoders for name in ("libx264", "aac")) and
            all(name in filters for name in ("zscale", "tonemap", "yadif")) and
            all(name in protocols for name in ("https", "tls", "crypto")) and "chromaprint" in formats,
            "FFmpeg is missing required encoders, filters, protocols or fingerprint support")
    with tempfile.TemporaryDirectory() as temporary:
        media, remux = Path(temporary) / "sample.mp4", Path(temporary) / "seek.mkv"
        run([ffmpeg, "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=10", "-f", "lavfi", "-i", "sine=frequency=440",
             "-t", "1", "-c:v", "libx264", "-threads", "1", "-c:a", "aac", media])
        probe = json.loads(run([ffprobe, "-v", "error", "-show_streams", "-of", "json", media]))
        require(len(probe["streams"]) == 2, "Local media probe failed")
        run([ffmpeg, "-v", "error", "-ss", "0.2", "-i", media, "-c", "copy", remux])
        run([ffprobe, "-v", "error", "-show_format", remux])
    return {"ffmpeg": "ffmpeg", "ffprobe": "ffprobe"}


def verify(directory, generation):
    require(re.fullmatch(r"[a-f0-9]{32}", generation["id"]), "Invalid generation identity")
    def check(entry):
        name, digest = entry
        safe_member(name)
        path = directory / name
        require(path.is_file() and not path.is_symlink() and sha(path) == digest, "Installed package integrity failed")
    with ThreadPoolExecutor(max_workers=4) as workers:
        for _ in workers.map(check, generation["files"].items()):
            pass
    for name in generation["executables"].values():
        require(name in generation["files"], "Unverified executable")


def protocol(root, value):
    if value["kind"] == "youtube":
        yt, deno = value["yt_dlp"], value["deno"]
        yt_root, deno_root = root / "packages" / yt["id"], root / "packages" / deno["id"]
        verify(yt_root, yt)
        verify(deno_root, deno)
        module, runtime = yt_root / yt["executables"]["module"], deno_root / deno["executables"]["deno"]
        run([sys.executable, "-I", "-B", "-c", "import sys;sys.path.insert(0,sys.argv[1]);from yt_dlp.utils._jsruntime import DenoJsRuntime; r=DenoJsRuntime(sys.argv[2]).info; assert r and r.supported, 'The selected Deno cannot run this extractor'", module, runtime])
        output = run([sys.executable, "-I", "-B", "-u", "-c", value["script"],
                      yt_root / yt["executables"]["module"], deno_root / deno["executables"]["deno"], yt["candidate"]["version"]])
        require(json.loads(output)["ready"] is True, "Resident YouTube protocol failed")
    else:
        generation = value["streamlink"]
        directory = root / "packages" / generation["id"]
        verify(directory, generation)
        output = run([directory / generation["executables"]["python"], "-I", "-B", "-u", "-c", value["script"]])
        require(json.loads(output)["ready"] is True, "Resident Streamlink protocol failed")


def prepare(root, candidate):
    check_candidate(candidate)
    root.mkdir(parents=True, exist_ok=True)
    require(shutil.disk_usage(root).free > MAX_EXPANDED, "Insufficient space for tool staging")
    archives = [acquire(root, value) for value in candidate["artifacts"]]
    packages = root / "packages"
    packages.mkdir(exist_ok=True)
    generation_id = uuid.uuid4().hex
    stage = packages / (".stage-" + generation_id)
    stage.mkdir()
    tool = candidate["tool"]
    try:
        if tool == "yt-dlp":
            require(len(archives) == 2, "Incomplete extractor package")
            for source, name in zip(archives, ("yt-dlp", "yt-dlp-module")):
                shutil.copyfile(source, stage / name)
        elif tool in ("deno", "ffmpeg"):
            require(len(archives) == 1, "Invalid archive count")
            unpack(archives[0], stage, tool)
        else:
            wheels = stage / "wheels"
            wheels.mkdir()
            lines = []
            for value, source in zip(candidate["artifacts"], archives):
                shutil.copyfile(source, wheels / value["name"])
                lines.append(f"{value['package']}=={value['version']} --hash=sha256:{value['sha256']}")
            (stage / "requirements.txt").write_text("\n".join(lines) + "\n")
            run([sys.executable, "-I", "-m", "venv", "--copies", "--without-pip", stage / "env"], timeout=120)
            run([sys.executable, "-I", "-m", "pip", "--isolated", "--python", stage / "env/bin/python", "install", "--no-index", "--no-compile",
                 "--only-binary=:all:", "--find-links", wheels, "--require-hashes", "-r", stage / "requirements.txt"], timeout=240)
            # venv's lib64 alias is optional; all imports use lib. Backups forbid links.
            alias = stage / "env/lib64"
            if alias.is_symlink():
                alias.unlink()
        for name in ("yt-dlp", "deno", "ffmpeg", "ffprobe"):
            path = stage / name
            if path.exists():
                path.chmod(0o755)
        emit(stage="validating")
        executables = validate(stage, candidate)
        # -I ignores PYTHONDONTWRITEBYTECODE, so discard validation caches before hashing.
        for path in stage.rglob("__pycache__"):
            shutil.rmtree(path)
        paths = []
        for path in stage.rglob("*"):
            require(not path.is_symlink(), "Managed packages must not contain symbolic links")
            if path.is_file():
                paths.append(path)
        def record(path):
            with path.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
                os.fsync(source.fileno())
            return path.relative_to(stage).as_posix(), digest
        with ThreadPoolExecutor(max_workers=4) as workers:
            files = dict(workers.map(record, sorted(paths)))
        generation = {"id": generation_id, "candidate": candidate, "files": files,
                      "executables": executables, "python_abi": sys.implementation.cache_tag}
        manifest = stage / "manifest.json"
        manifest.write_bytes(canonical(generation))
        with manifest.open("rb") as source:
            os.fsync(source.fileno())
        os.rename(stage, packages / generation_id)
        descriptor = os.open(packages, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        return generation
    finally:
        if stage.exists():
            shutil.rmtree(stage)


def probe(root, value):
    # Only public identifiers enter the probes. Signed URLs, stderr and account
    # data never enter the persisted qualification report.
    def attempt(packages, provider, identifier):
        started = time.monotonic()
        try:
            def executable(tool, name):
                generation = packages[tool]
                return root / "packages" / generation["id"] / generation["executables"][name]
            if provider == "youtube":
                video = identifier
                cli = json.loads(run([executable("yt-dlp", "yt_dlp"), "--ignore-config", "--no-plugin-dirs",
                    "--no-cache-dir", "--no-cookies", "--no-cookies-from-browser", "--no-playlist", "--no-remote-components",
                    "--no-js-runtimes", "--js-runtimes", "deno:" + str(executable("deno", "deno")),
                    "--socket-timeout", "15", "--retries", "0", "--extractor-retries", "0", "--no-warnings", "--quiet",
                    "--dump-single-json", "--skip-download", "--", "https://www.youtube.com/watch?v=" + video], timeout=90))
                output = run([sys.executable, "-I", "-B", "-u", "-c", value["youtube_script"],
                    executable("yt-dlp", "module"), executable("deno", "deno"), packages["yt-dlp"]["candidate"]["version"]],
                    timeout=120, request=canonical({"id": video}) + b"\n")
                worker = json.loads(output.splitlines()[-1]).get("metadata", {})
                require(bool(cli.get("formats")) and bool(worker.get("formats")) and cli.get("id") == worker.get("id") == video,
                        "Public extraction did not return matching formats")
            else:
                channel = identifier
                domain = "www.twitch.tv" if provider == "twitch" else "kick.com"
                cli = run([executable("streamlink", "python"), "-I", "-B", "-m", "streamlink", "--no-config", "--loglevel", "error",
                    "--stream-url", "--http-timeout", "15", f"https://{domain}/{channel}", "best"], timeout=60).strip()
                output = run([executable("streamlink", "python"), "-I", "-B", "-u", "-c", value["streamlink_script"]],
                    timeout=60, request=canonical({"provider": provider, "channel": channel}) + b"\n")
                worker = json.loads(output.splitlines()[-1])
                require(cli.startswith("https://") and worker.get("url", "").startswith("https://"), "Public stream is unavailable")
            return {"status": "passed", "seconds": round(time.monotonic() - started, 2)}
        except Exception as error:
            message = str(error).lower()
            reason = "provider_unavailable"
            for code, markers in (("authentication_required", ("sign in", "login required", "cookies", "403")),
                                  ("offline_or_removed", ("unavailable", "no playable streams", "offline", "404")),
                                  ("network", ("timed out", "network", "resolve", "connection", "429")),
                                  ("tool_invocation_failed", ("no such option", "no module named"))):
                if any(marker in message for marker in markers):
                    reason = code
                    break
            return {"status": "inconclusive", "reason": reason, "seconds": round(time.monotonic() - started, 2)}
    results = []
    identifiers = {"youtube": ["jNQXAC9IVRw", "aqz-KE-bpKQ"], "twitch": ["monstercat", "lck", "riotgames"], "kick": ["lofi", "xqc", "adinross"]}
    for provider in (["youtube"] if value["kind"] == "youtube" else ["twitch", "kick"]):
        attempts = []
        for identifier in identifiers[provider]:
            candidate = attempt(value["candidate"], provider, identifier)
            current = attempt(value["current"], provider, identifier) if candidate["status"] != "passed" else None
            attempts.append({"identifier": identifier, "candidate": candidate, "current": current})
            if candidate["status"] == "passed":
                break
        results.append({"provider": provider, "candidate": candidate, "attempts": attempts})
    return {"ready": all(result["candidate"]["status"] == "passed" for result in results), "probes": results}


def main():
    global CACHE_ROOT
    CACHE_ROOT = Path.cwd() / "catalog"
    action = sys.argv[1]
    if action == "discover":
        emit(result=discover(sys.argv[2], sys.argv[3]))
    elif action == "prepare":
        emit(result=prepare(Path(sys.argv[2]), json.load(sys.stdin)))
    elif action == "verify":
        generation = json.load(sys.stdin)
        directory = Path(sys.argv[2]) / "packages" / generation["id"]
        verify(directory, generation)
        validate(directory, generation["candidate"])
        emit(result=True)
    elif action == "protocol":
        protocol(Path(sys.argv[2]), json.load(sys.stdin))
        emit(result=True)
    elif action == "probe":
        emit(result=probe(Path(sys.argv[2]), json.load(sys.stdin)))
    elif action == "runtime":
        emit(result=sys.implementation.cache_tag)
    elif action == "rebuild":
        root = Path(sys.argv[2])
        candidate = json.load(sys.stdin)
        for value in candidate["artifacts"]:
            cached = root / "artifacts" / value["sha256"]
            require(cached.is_file() and sha(cached) == value["sha256"], "Offline interpreter repair is missing a verified wheel")
        emit(result=prepare(root, candidate))
    elif action == "bootstrap":
        root, seed = Path(sys.argv[2]), Path(sys.argv[3])
        generation = json.load(sys.stdin)
        check_candidate(generation["candidate"])
        (root / "artifacts").mkdir(exist_ok=True)
        for value in generation["candidate"]["artifacts"]:
            cached = seed / "artifacts" / value["sha256"]
            require(sha(cached) == value["sha256"], "Seed artifact integrity failed")
            target = root / "artifacts" / value["sha256"]
            if not target.is_file() or sha(target) != value["sha256"]:
                temporary = target.with_suffix(".part")
                shutil.copyfile(cached, temporary)
                with temporary.open("rb") as source:
                    os.fsync(source.fileno())
                os.replace(temporary, target)
        emit(result=prepare(root, generation["candidate"]))
    else:
        raise RuntimeError("Unknown package command")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        retry_at = None
        if isinstance(error, MemoryError) or isinstance(error, OSError) and error.errno in (errno.EAGAIN, errno.ENOMEM) or str(error) == "can't start new thread":
            retry_at = int(time.time()) + 30
        elif isinstance(error, urllib.error.HTTPError) and error.code in (403, 429, 503):
            try:
                retry_at = max(int(time.time()) + 60, min(int(time.time()) + 86400,
                               int(error.headers.get("X-RateLimit-Reset", int(time.time()) + int(error.headers.get("Retry-After", 3600))))))
            except ValueError:
                retry_at = int(time.time()) + 3600
        elif not isinstance(error, urllib.error.HTTPError) and isinstance(error, (urllib.error.URLError, TimeoutError, ConnectionError, EOFError, http.client.IncompleteRead)):
            retry_at = int(time.time()) + 3600
        emit(error=str(error)[:1800], retry_at=retry_at)
        sys.exit(1)
