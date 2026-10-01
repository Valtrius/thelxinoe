#!/usr/bin/env bash
set -euo pipefail

if command -v ffmpeg >/dev/null && command -v ffprobe >/dev/null; then
  ffmpeg -version
  ffprobe -version
  exit 0
fi

# Hosted Ubuntu runners can use an Azure mirror that stalls while fetching
# package indexes. Use the public Ubuntu mirror when installation is needed.
if [[ -f /etc/apt/apt-mirrors.txt ]]; then
  printf '%s\n' 'https://archive.ubuntu.com/ubuntu/' | sudo tee /etc/apt/apt-mirrors.txt >/dev/null
fi

apt_options=(
  -o Acquire::Retries=2
  -o Acquire::http::Timeout=20
  -o Acquire::https::Timeout=20
  -o DPkg::Lock::Timeout=30
)
sudo timeout --kill-after=10s 180s apt-get "${apt_options[@]}" update
sudo timeout --kill-after=10s 180s apt-get "${apt_options[@]}" install -y --no-install-recommends ffmpeg
ffmpeg -version
ffprobe -version
