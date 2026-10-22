#!/usr/bin/env bash
set -euo pipefail

# The FFmpeg build the server image ships, from its pinned release: Ubuntu's
# package mirrors stall too often to install it from apt. CI caches the
# extracted copy by the pin.
read -r url sha256 < <(node -e '
  const lock = require("./scripts/server-tools.lock.json");
  const { artifacts } = lock.find(
    (tool) => tool.tool === "ffmpeg" && tool.platform === "linux-x86_64",
  );
  console.log(artifacts[0].url, artifacts[0].sha256);
')
directory="$PWD/.local/ci-tools/ffmpeg-${sha256:0:12}"

if [[ ! -x "$directory/ffmpeg" || ! -x "$directory/ffprobe" ]]; then
  archive="$(mktemp)"
  trap 'rm -f "$archive"' EXIT
  curl --fail --location --silent --show-error --retry 5 --retry-all-errors \
    --connect-timeout 20 --max-time 300 --output "$archive" "$url"
  if ! echo "$sha256  $archive" | sha256sum --check --quiet; then
    echo "$url no longer matches its pinned digest" >&2
    exit 1
  fi
  rm -rf "$directory"
  mkdir -p "$directory"
  tar -xJf "$archive" -C "$directory" --strip-components=2 \
    --wildcards '*/bin/ffmpeg' '*/bin/ffprobe'
fi

export PATH="$directory:$PATH"
if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$directory" >>"$GITHUB_PATH"
fi
ffmpeg -version
ffprobe -version
