#!/usr/bin/env bash
# Builds the image of „Eigener Rechner (Sandbox)“ (#94):
#   1. the workspace image (Dockerfile.workspace), saved as workspace-image.tar,
#   2. the private computer (Dockerfile.machine) with Docker and that tar inside,
#   3. its root file system, exported as aiscraft-sandbox-<LAYOUT>-x86_64.tar.gz for `wsl --import`,
# plus SHA256SUMS.txt. Needs Docker. Usage: build.sh [--out DIR]
set -euo pipefail

LAYOUT=1   # keep in step with ASSET_X86_64 in crates/engine/src/sandbox/image.rs
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/dist"
if [ "${1:-}" = "--out" ]; then out="$2"; fi
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"; docker rm -f aiscraft-sandbox-export >/dev/null 2>&1 || true' EXIT

cp "$here/wsl.conf" "$here/NOTICE.txt" "$here/Dockerfile.machine" "$work/"
docker build --pull -f "$here/Dockerfile.workspace" -t aiscraft-workspace:1 "$here"
docker save aiscraft-workspace:1 -o "$work/workspace-image.tar"
docker build --pull -f "$work/Dockerfile.machine" -t aiscraft-sandbox-machine:$LAYOUT "$work"
docker create --name aiscraft-sandbox-export aiscraft-sandbox-machine:$LAYOUT /bin/true >/dev/null
asset="aiscraft-sandbox-$LAYOUT-x86_64.tar.gz"
docker export aiscraft-sandbox-export | gzip -n -9 > "$out/$asset"
(cd "$out" && sha256sum "$asset" > SHA256SUMS.txt)
ls -l "$out"
cat "$out/SHA256SUMS.txt"
