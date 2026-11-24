#!/usr/bin/env bash
# Install AIS.ChatCraft for the current user (no sudo): binaries in ~/.local/bin, the launcher
# entry and icons under ~/.local/share. Run from the repository root.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p aiscraft -p aiscraft-cli -p aiscraft-preview
bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
share="${XDG_DATA_HOME:-$HOME/.local/share}"
install -Dm755 target/release/aiscraft "$bin/aiscraft"
install -Dm755 target/release/aiscraft-cli "$bin/aiscraft-cli"
install -Dm755 target/release/aiscraft-preview "$bin/aiscraft-preview"
install -Dm644 packaging/ai.uds.aischatcraft.desktop "$share/applications/ai.uds.aischatcraft.desktop"
install -Dm644 packaging/icons/aischatcraft.svg "$share/icons/hicolor/scalable/apps/aischatcraft.svg"
for s in 32 48 64 128 256 512; do
  install -Dm644 "packaging/icons/aischatcraft-$s.png" "$share/icons/hicolor/${s}x${s}/apps/aischatcraft.png"
done
command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" || true
echo "AIS.ChatCraft installiert: $bin/aiscraft (Starter: AIS.ChatCraft)"
