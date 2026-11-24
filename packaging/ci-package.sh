#!/usr/bin/env bash
# Packs the release binaries of one CI build into dist/: a .tar.gz on Linux and macOS (macOS also
# gets an unsigned AIS.ChatCraft.app inside), a .zip on Windows. Called by .github/workflows/ci.yml
# with NAME (archive name), TARGET (Rust target triple) and OS (Linux, macOS, Windows).
set -euo pipefail

: "${NAME:?}" "${TARGET:?}" "${OS:?}"
BIN="target/$TARGET/release"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
STAGE="dist/stage/$NAME"
rm -rf dist && mkdir -p "$STAGE"


cp LICENSE* "$STAGE/" 2>/dev/null || true

case "$OS" in
  Windows)
    for b in aiscraft aiscraft-cli aiscraft-collab-server aiscraft-preview; do cp "$BIN/$b.exe" "$STAGE/"; done
    (cd dist/stage && 7z a -tzip -bso0 "../$NAME.zip" "$NAME")
    ;;
  macOS)
    for b in aiscraft aiscraft-cli aiscraft-collab-server aiscraft-preview; do cp "$BIN/$b" "$STAGE/"; done
    APP="$STAGE/AIS.ChatCraft.app/Contents"
    mkdir -p "$APP/MacOS" "$APP/Resources"
    cp "$BIN/aiscraft" "$APP/MacOS/aiscraft"
    cp "$BIN/aiscraft-preview" "$APP/MacOS/aiscraft-preview"   # found next to the app binary
    if [ -f packaging/icons/aischatcraft-512.png ]; then
      cp packaging/icons/aischatcraft-512.png "$APP/Resources/icon.png"
    fi
    cat > "$APP/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>AIS.ChatCraft</string>
  <key>CFBundleDisplayName</key><string>AIS.ChatCraft</string>
  <key>CFBundleIdentifier</key><string>ai.uds.aischatcraft</string>
  <key>CFBundleExecutable</key><string>aiscraft</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Für die Spracheingabe im Chat.</string>
</dict>
</plist>
PLIST
    tar -C dist/stage -czf "dist/$NAME.tar.gz" "$NAME"
    ;;
  *)
    for b in aiscraft aiscraft-cli aiscraft-collab-server aiscraft-preview; do cp "$BIN/$b" "$STAGE/"; done
    cp packaging/ai.uds.aischatcraft.desktop packaging/install.sh "$STAGE/"
    cp -r packaging/icons "$STAGE/"
    tar -C dist/stage -czf "dist/$NAME.tar.gz" "$NAME"
    ;;
esac

rm -rf dist/stage
ls -l dist
