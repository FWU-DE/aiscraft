#!/usr/bin/env bash
# Builds the optional H5P player for AIS.ChatCraft (#62): one zip with
#
#   player/      h5p-standalone (npm, MIT) with the H5P core inside (GPL-3.0)
#   libraries/   the H5P content-type libraries the app creates and plays (QuestionSet,
#                MultiChoice, TrueFalse, Blanks, DragText) with their dependencies (JoubelUI,
#                Question, FontAwesome, jQuery.ui, …), from the official H5P Hub packages
#   LICENSES/    licence texts; NOTICE.txt says what is in it and where the source is
#   manifest.json  name, bundle version, versions of everything inside
#
# and SHA256SUMS.txt next to it. Because of the GPL the player is never part of the app package:
# the workflow .github/workflows/h5p-player.yml runs this script and publishes both files on the
# release tag h5p-player-v<BUNDLE_VERSION>, where the app downloads them after the owner's yes.
#
# Every download is checked against a pin in this file: the npm tarball against its
# `dist.integrity` (sha512), the GPL text and the Hub packages against their SHA-256. The Hub
# always serves the newest version of a content type, so a pin can go stale: the build then
# stops and says so. Check what changed, then update the pins (`--print-pins` builds without
# checking the Hub pins and prints the current hashes).
#
# Run locally:   packaging/h5p-player/build.sh [--out DIR] [--print-pins]
# Needs: bash, curl, tar, unzip, node (18 or newer), and zip or bsdtar.
# Output: DIR (default dist/h5p-player)/aiscraft-h5p-player-<BUNDLE_VERSION>.zip and SHA256SUMS.txt.
#
# A new bundle (other libraries, a new h5p-standalone) gets a new BUNDLE_VERSION, and the app's
# `crates/engine/src/learn/h5p_player.rs` (BUNDLE_VERSION, TAG, ASSET, PINNED_SHA256) with it;
# a published tag is never overwritten.
set -euo pipefail

BUNDLE_VERSION=1
ASSET="aiscraft-h5p-player-${BUNDLE_VERSION}.zip"

H5PS_VERSION=3.8.2
H5PS_INTEGRITY="sha512-r7eQtTzBk/dbaQZ+tA9XD5FeSmyMxlypRZXudiheCnC8zAT0/zNgUvO4MZtZaznwfYVo98o7do5uxLTUQrxxwg=="

# The GPL-3.0 text of the H5P core, from h5p-php-library at a fixed commit (tag wp-1.16.0).
GPL_URL="https://raw.githubusercontent.com/h5p/h5p-php-library/10e979adcc71bfa1f882e45423ddb9c4ce1e23a6/LICENSE.txt"
GPL_SHA256=c53a65c2fd561c87eaabf1072ef5dcab8653042bc15308465f52413585eb6271

# Content types from the H5P Hub, as an LMS installs them (built code included), with the
# SHA-256 of the package the Hub served when the pin was set (2026-11-25).
HUB="https://api.h5p.org/v1/content-types"
HUB_TYPES=(
  "H5P.QuestionSet 8b066b98e88a238fae31638ee2c5d12f26707bfb2555a7dbff08609038e68c07"
  "H5P.MultiChoice 1c60785f1e9b6630b30d4e9bcc4b7ec1c8976f05bf50c41ddbb25e59fd2c8fdb"
  "H5P.TrueFalse 088b932daf65e80fbd4bf79332634272ae81760463bf7191ad4aac1921239273"
  "H5P.Blanks 0eaf030d4ca0a8a799d960fc17d6fcf58632bb074162b38ccdfe224c985a49de"
  "H5P.DragText 67042de60a5b18e3142c375c59a8d1fe74965d039330e31fd849827f7c43f208"
)

out="dist/h5p-player"
print_pins=0
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="${2:?--out braucht einen Ordner}"; shift 2 ;;
    --print-pins) print_pins=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "build.sh: unbekannte Option $1" >&2; exit 2 ;;
  esac
done

die() { echo "build.sh: $*" >&2; exit 1; }
for tool in curl tar unzip node; do command -v "$tool" >/dev/null || die "$tool fehlt"; done

here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/stage"
mkdir -p "$stage/player" "$stage/libraries" "$stage/LICENSES" "$work/hub" "$out"

fetch() { curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$2" "$1"; }
digest() { node -e 'const c=require("crypto"),f=require("fs");process.stdout.write(c.createHash(process.argv[1]).update(f.readFileSync(process.argv[2])).digest(process.argv[3]))' "$@"; }

echo "h5p-standalone $H5PS_VERSION (npm)"
fetch "https://registry.npmjs.org/h5p-standalone/-/h5p-standalone-${H5PS_VERSION}.tgz" "$work/h5ps.tgz"
got="sha512-$(digest sha512 "$work/h5ps.tgz" base64)"
[ "$got" = "$H5PS_INTEGRITY" ] || die "h5p-standalone: Prüfsumme stimmt nicht ($got)"
mkdir "$work/h5ps"
tar -xzf "$work/h5ps.tgz" -C "$work/h5ps"
cp -R "$work/h5ps/package/dist/." "$stage/player/"
rm -f "$stage/player/"*.d.ts
cp "$work/h5ps/package/LICENSE" "$stage/LICENSES/h5p-standalone-MIT.txt"

echo "GPL-3.0 (H5P-Kern)"
fetch "$GPL_URL" "$stage/LICENSES/GPL-3.0.txt"
[ "$(digest sha256 "$stage/LICENSES/GPL-3.0.txt" hex)" = "$GPL_SHA256" ] || die "GPL-Text: Prüfsumme stimmt nicht"

sources="{}"
for entry in "${HUB_TYPES[@]}"; do
  name="${entry%% *}"
  pin="${entry##* }"
  echo "$name (H5P Hub)"
  fetch "$HUB/$name" "$work/hub/$name.h5p"
  got="$(digest sha256 "$work/hub/$name.h5p" hex)"
  if [ "$print_pins" = 1 ]; then
    echo "  \"$name $got\""
  elif [ "$got" != "$pin" ]; then
    die "$name: der Hub liefert eine andere Fassung als die festgelegte ($got). Änderungen prüfen, dann den Pin in build.sh erneuern (--print-pins)."
  fi
  mkdir "$work/hub/$name"
  unzip -q "$work/hub/$name.h5p" -d "$work/hub/$name"
  sources="$(node -e 'const s=JSON.parse(process.argv[1]);s[process.argv[2]]=process.argv[3];process.stdout.write(JSON.stringify(s))' "$sources" "$name" "$got")"
done

HUB_SOURCES="$sources" node "$here/assemble.js" "$work/hub" "$stage" "$H5PS_VERSION" "$BUNDLE_VERSION" "$H5PS_INTEGRITY"

# Same input, same zip: fixed times, sorted names, no extra attributes.
find "$stage" -exec touch -t 198001010000 {} +
rm -f "$out/$ASSET" "$out/SHA256SUMS.txt"
zipfile="$(cd "$out" && pwd)/$ASSET"
if command -v zip >/dev/null; then
  (cd "$stage" && find . -type f | LC_ALL=C sort | sed 's|^\./||' | zip -X -q -D -9 "$zipfile" -@)
else
  bsdtar=""
  for t in bsdtar tar /c/Windows/System32/tar.exe; do
    if command -v "$t" >/dev/null && "$t" --version 2>/dev/null | grep -q bsdtar; then bsdtar="$t"; break; fi
  done
  [ -n "$bsdtar" ] || die "zip oder bsdtar fehlt"
  (cd "$stage" && find . -type f | LC_ALL=C sort | sed 's|^\./||' > "$work/files.txt" && "$bsdtar" --format zip -cf "$zipfile" -T "$work/files.txt")
fi

(cd "$out" && echo "$(digest sha256 "$ASSET" hex)  $ASSET" > SHA256SUMS.txt)
size=$(wc -c < "$out/$ASSET" | tr -d ' ')
echo "Fertig: $out/$ASSET ($((size / 1024)) KB)"
cat "$out/SHA256SUMS.txt"
