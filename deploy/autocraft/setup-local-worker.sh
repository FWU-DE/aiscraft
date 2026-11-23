#!/usr/bin/env bash
# Sets up a local AutoCraft build worker on this Arch machine:
#   1. rootless Podman (+ passt for the fetch network)
#   2. a separate OS user "craftsandbox" that runs the worker, without access to your home,
#      GitHub token or Claude login
# and connects it to the local AutoCraft runner (worker API on 127.0.0.1:8981).
#
# Run it from your own account (not as root):   sudo bash deploy/autocraft/setup-local-worker.sh
# Safe to run again; finished steps are skipped.
set -euo pipefail

SANDBOX=craftsandbox
ENV_ID=laptop
OPERATOR=${SUDO_USER:-}
AUTOCRAFT_SRC=/home/${OPERATOR}/code/autocraft
AUTOCRAFT_ENV=/home/${OPERATOR}/.local/share/autocraft-aiscraft/env.sh

say() { printf '\n== %s\n' "$*"; }
die() { printf 'Fehler: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "mit sudo starten: sudo bash $0"
[ -n "$OPERATOR" ] && [ "$OPERATOR" != root ] || die "über sudo aus deinem eigenen Konto starten, nicht als root"
[ -f "$AUTOCRAFT_ENV" ] || die "$AUTOCRAFT_ENV fehlt (lokales AutoCraft nicht eingerichtet)"
[ -d "$AUTOCRAFT_SRC" ] || die "$AUTOCRAFT_SRC fehlt"

say "1/6 Podman installieren"
pacman -S --needed --noconfirm podman passt crun fuse-overlayfs git nodejs pnpm >/dev/null
podman --version

say "2/6 Benutzer $SANDBOX anlegen"
if ! id "$SANDBOX" >/dev/null 2>&1; then
  useradd --create-home --shell /usr/bin/nologin --user-group "$SANDBOX"
fi
HOME_S=$(getent passwd "$SANDBOX" | cut -d: -f6)
chmod 700 "$HOME_S"
# Rootless Podman needs a subordinate uid/gid range.
grep -q "^$SANDBOX:" /etc/subuid || usermod --add-subuids 231072-296607 "$SANDBOX"
grep -q "^$SANDBOX:" /etc/subgid || usermod --add-subgids 231072-296607 "$SANDBOX"
# Keep its user services running without a login.
loginctl enable-linger "$SANDBOX"
UID_S=$(id -u "$SANDBOX")
for _ in $(seq 1 20); do [ -d "/run/user/$UID_S" ] && break; sleep 0.5; done
[ -d "/run/user/$UID_S" ] || die "/run/user/$UID_S fehlt (systemd --user für $SANDBOX startet nicht)"
as_sandbox() { runuser -u "$SANDBOX" -- env HOME="$HOME_S" XDG_RUNTIME_DIR="/run/user/$UID_S" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$UID_S/bus" "$@"; }

say "3/6 AutoCraft-Code für den Worker (eigene Kopie, nur lesend aus GitHub)"
if [ ! -d "$HOME_S/autocraft/.git" ]; then
  REV=$(runuser -u "$OPERATOR" -- git -C "$AUTOCRAFT_SRC" rev-parse HEAD)
  as_sandbox git clone --quiet https://github.com/storytold/autocraft.git "$HOME_S/autocraft"
  as_sandbox git -C "$HOME_S/autocraft" checkout --quiet "$REV"
fi
as_sandbox bash -c "cd ~/autocraft && pnpm install --prod --frozen-lockfile --silent"

say "4/6 Toolchain-Image holen und festnageln"
as_sandbox podman pull --quiet docker.io/library/rust:1-bookworm >/dev/null
RUST_IMG=$(as_sandbox podman image inspect --format '{{index .RepoDigests 0}}' docker.io/library/rust:1-bookworm)
as_sandbox podman pull --quiet docker.io/library/node:24-bookworm >/dev/null
NODE_IMG=$(as_sandbox podman image inspect --format '{{index .RepoDigests 0}}' docker.io/library/node:24-bookworm)
echo "rust-stable = $RUST_IMG"
echo "node-24     = $NODE_IMG"

say "5/6 Build-Umgebung bei AutoCraft anmelden"
CFG_DIR="$HOME_S/.config/craft-worker"
install -d -m 700 -o "$SANDBOX" -g "$SANDBOX" "$CFG_DIR" "$HOME_S/work"
if [ ! -s "$CFG_DIR/key" ]; then
  TMPD=$(mktemp -d)
  chown "$OPERATOR" "$TMPD"
  TMP="$TMPD/key"
  # Runs as you: only your account can write AutoCraft's database. The key is shown once.
  runuser -u "$OPERATOR" -- bash -c ". '$AUTOCRAFT_ENV' && cd '$AUTOCRAFT_SRC' && node --no-warnings src/cli.ts envs add $ENV_ID --label 'Laptop (craftsandbox)' --kind local-sandbox --slots 2 --out '$TMP'"
  install -m 600 -o "$SANDBOX" -g "$SANDBOX" "$TMP" "$CFG_DIR/key"
  rm -rf "$TMPD"
fi
CPUS=$(( $(nproc) > 2 ? $(nproc) - 2 : 1 ))
MEM=$(( $(awk '/MemTotal/{print int($2/1024/1024)}' /proc/meminfo) / 2 ))
[ "$MEM" -ge 2 ] || MEM=2
cat > "$CFG_DIR/config.json" <<JSON
{
  "coordinatorUrl": "http://127.0.0.1:8981",
  "keyFile": "$CFG_DIR/key",
  "workDir": "$HOME_S/work",
  "slots": 2,
  "images": { "rust-stable": "$RUST_IMG", "node-24": "$NODE_IMG" },
  "runtime": "podman",
  "limits": { "cpus": $CPUS, "memoryGb": $MEM, "pids": 1024, "minFreeDiskGb": 20 },
  "fetchNetwork": "pasta",
  "display": false,
  "gpu": false
}
JSON
chown "$SANDBOX:$SANDBOX" "$CFG_DIR/config.json"
chmod 600 "$CFG_DIR/config.json"

say "6/6 Worker als Dienst starten"
install -d -o "$SANDBOX" -g "$SANDBOX" "$HOME_S/.config/systemd/user"
install -m 644 -o "$SANDBOX" -g "$SANDBOX" "$HOME_S/autocraft/deploy/craft-worker.service" "$HOME_S/.config/systemd/user/craft-worker.service"
as_sandbox systemctl --user daemon-reload
as_sandbox systemctl --user enable --now craft-worker
sleep 3
as_sandbox systemctl --user --no-pager --lines=8 status craft-worker || true

say "Fertig"
echo "Logs:   sudo -u $SANDBOX XDG_RUNTIME_DIR=/run/user/$UID_S journalctl --user -u craft-worker -f"
echo "Stopp:  sudo -u $SANDBOX XDG_RUNTIME_DIR=/run/user/$UID_S systemctl --user stop craft-worker"
echo "Im Dashboard unter Build environments sollte \"$ENV_ID\" online sein."
