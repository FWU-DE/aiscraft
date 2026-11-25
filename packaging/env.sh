# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
# craft-common-actions reads it too: `resolve-version` takes VERSION from here, and
# `export-app-build-env` finds the AISCHATCRAFT_ prefix in the VERSION line below.
#
# Exports:
#   ROOT                       workspace root
#   VERSION                    [workspace.package] version from Cargo.toml (override: AISCHATCRAFT_VERSION)
#   DIST                       output directory for release artifacts (default: $ROOT/dist/release)
#   AISCHATCRAFT_BUILD_SHA     git commit (macOS Info.plist)
#   AISCHATCRAFT_BUILD_DATE    UTC build date, YYYY-MM-DD (AppStream release date)
#   CARGO_TARGET_DIR           cargo's target dir (default: $ROOT/target)
#   BINS                       the programs every package ships

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# The version lives in one place: `[workspace.package] version` in the root Cargo.toml.
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]") ; next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
  return
}

VERSION="${AISCHATCRAFT_VERSION:-$(workspace_version)}"
if [[ -z "$VERSION" ]]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST

if [[ -z "${AISCHATCRAFT_BUILD_SHA:-}" ]]; then
  AISCHATCRAFT_BUILD_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
fi
export AISCHATCRAFT_BUILD_SHA
export AISCHATCRAFT_BUILD_DATE="${AISCHATCRAFT_BUILD_DATE:-$(date -u +%Y-%m-%d)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# The desktop app, the CLI, the learning-app preview (found next to `aiscraft`) and the relay
# for live collaboration.
# shellcheck disable=SC2034 # used by the scripts that source this file
BINS=(aiscraft aiscraft-cli aiscraft-preview aiscraft-collab-server)
# shellcheck disable=SC2034
CARGO_PACKAGES=(-p aiscraft -p aiscraft-cli -p aiscraft-preview -p aiscraft-collab-server)

# Emit a GitHub Actions warning (plain stderr outside Actions).
warn() {
  if [[ -n "${GITHUB_ACTIONS:-}" ]]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
  return
}

# Copy licence and readme files that exist into a package directory.
copy_docs() {
  local dest="$1" f
  for f in README.md LICENSE LICENSE-MIT LICENSE-APACHE NOTICE ATTRIBUTION.md; do
    if [[ -f "$ROOT/$f" ]]; then cp "$ROOT/$f" "$dest/"; fi
  done
  return
}

# The hicolor icon tree (PNG sizes and the SVG), named after the bundle id.
install_icons() {
  local dest="$1" size
  for size in 32 48 64 128 256 512; do
    install -Dm644 "$ROOT/packaging/icons/aischatcraft-$size.png" "$dest/hicolor/${size}x${size}/apps/ai.uds.aischatcraft.png"
  done
  install -Dm644 "$ROOT/packaging/icons/aischatcraft.svg" "$dest/hicolor/scalable/apps/ai.uds.aischatcraft.svg"
  return
}
