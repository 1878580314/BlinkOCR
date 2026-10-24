#!/usr/bin/env bash
set -euo pipefail
if [[ $# -lt 1 ]]; then
  echo 'Usage: scripts/build_windows_blinkocr.sh /mnt/<drive>/<build-directory> [Auto|CUDA|CPU]' >&2
  exit 2
fi
project_root="$(cd "$(dirname "$0")/.." && pwd)"
stage_root="$1"
backend="${2:-Auto}"
mkdir -p "$stage_root"
rsync -a --exclude .git --exclude .cache --exclude node_modules --exclude target \
  --exclude gen --exclude dist --exclude web-dist "$project_root/" "$stage_root/"
win_stage="$(wslpath -w "$stage_root")"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_stage\scripts\package_windows.ps1" -Backend "$backend" -InstallDependencies
