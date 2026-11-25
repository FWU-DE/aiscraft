//! The workspace container inside the private computer (#94): how it is started, the same for
//! every runtime. Each call is one short-lived container:
//!
//! - an unprivileged user (1000:1000, no sudo in the image), every capability dropped,
//!   `no-new-privileges`, a read-only root file system with a small `/tmp`;
//! - 2 GB RAM (no extra swap), 2 CPUs, 1024 processes;
//! - `--network none` unless the person switched the network on;
//! - nothing of the host: the only mount is the chat's working folder `/work/<chat>` of the
//!   private computer, at `/work`. The private computer itself mounts no Windows drive (`wsl.rs`).
//!
//! [`docker_run`] is the only place that builds a start command; the quality gate checks every
//! start the fake runtime saw.

use aiscraft_core::{Error, Result};

/// The network containers use while the network switch is on (Nod's runtime turns Docker's
/// default bridge off).
pub(crate) const NETWORK: &str = "aiscraft-net";
pub(crate) const MEMORY: &str = "2g";
pub(crate) const CPUS: &str = "2";
pub(crate) const PIDS: &str = "1024";
pub(crate) const USER: &str = "1000:1000";

/// `$1` folder (relative to /work), `$2` the command, `$3` the time limit in seconds (exit code
/// 124 when it ends the command).
pub(crate) const RUN_SCRIPT: &str = r#"mkdir -p -- "$1" && cd -- "$1" && exec timeout -k 5 "$3" /bin/sh -c "$2""#;
/// `$1` file; the bytes come on stdin.
pub(crate) const WRITE_SCRIPT: &str = r#"mkdir -p -- "$(dirname -- "$1")" && cat > "$1""#;
/// `$1` file, `$2` most bytes: the file as base64 (binary-safe through every pipe).
pub(crate) const READ_SCRIPT: &str = r#"[ -f "$1" ] || { echo "Datei fehlt: $1" >&2; exit 2; }; s=$(stat -L -c %s -- "$1"); [ "$s" -le "$2" ] || { echo "Die Datei ist größer als $2 Bytes ($s)." >&2; exit 3; }; base64 -w0 -- "$1""#;
/// `$1` file, `$2` target format, `$3` the time limit in seconds: LibreOffice writes the result
/// next to the file and is ended by its own `timeout` (exit code 124).
pub(crate) const CONVERT_SCRIPT: &str =
    r#"cd -- "$(dirname -- "$1")" && exec timeout -k 5 "$3" soffice --headless --norestore --nolockcheck --convert-to "$2" --outdir . "$(basename -- "$1")""#;
/// How long one conversion may take.
pub(crate) const CONVERT_SECS: u64 = 240;
/// Most a chat's working folder, and all of them together, may hold (MB); one file at most (bytes).
pub(crate) const CHAT_FOLDER_MB: u64 = 2048;
pub(crate) const ALL_FOLDERS_MB: u64 = 10240;
pub(crate) const FSIZE: &str = "fsize=2147483648";
/// `$1` folder: type, size and path of up to 500 entries.
pub(crate) const LIST_SCRIPT: &str = r#"cd -- "$1" && find . -mindepth 1 -maxdepth 4 -printf '%y %s %P\n' | head -n 500"#;

/// In the private computer as root, before a container: Docker running, the workspace image
/// loaded (only after its tar matched the pinned checksum, as Nod does), every other image (an
/// older workspace) and Nod's images we never
/// use removed, the network for the switch there, the chat's folder there and owned by the
/// container's user. `$1` image id (`Workspace::image`), `$2` chat (may be empty), `$3` the image tar, `$4` its
/// SHA-256, `$5` `1` when the network switch is on, `$6` and `$7` the most a chat's folder and all
/// folders may hold (MB): beyond that nothing runs until the person makes room.
pub(crate) const PREPARE_SCRIPT: &str = r#"set -e
# Docker maps every container user to an unprivileged range (userns-remap): even root that broke
# out of a container is nobody in this computer and cannot mount a Windows drive. Switched on
# once, with a restart of Docker, for computers set up before.
if ! grep -q '"userns-remap"' /etc/docker/daemon.json 2>/dev/null; then
  mkdir -p /etc/docker
  [ -s /etc/docker/daemon.json ] || echo '{}' > /etc/docker/daemon.json
  sed -i 's/^{/{"userns-remap":"default",/; s/,}$/}/' /etc/docker/daemon.json
  if docker info >/dev/null 2>&1; then
    pkill -x dockerd || true
    i=0
    while docker info >/dev/null 2>&1; do i=$((i+1)); [ $i -gt 40 ] && break; sleep 0.5; done
  fi
fi
if ! docker info >/dev/null 2>&1; then
  (setsid dockerd </dev/null >/var/log/aiscraft-dockerd.log 2>&1 &)
  i=0
  while ! docker info >/dev/null 2>&1; do
    i=$((i+1))
    if [ $i -gt 60 ]; then echo "Docker startet nicht: $(tail -n 3 /var/log/aiscraft-dockerd.log)" >&2; exit 1; fi
    sleep 0.5
  done
fi
if [ "$(docker image inspect --format '{{.Id}}' "$1" 2>/dev/null)" != "$1" ]; then
  echo "$4  $3" | sha256sum -c --quiet - >/dev/null 2>&1 || { echo "Das Abbild des Arbeitsbereichs ist beschädigt. Bitte den eigenen Rechner entfernen und neu einrichten." >&2; exit 3; }
  docker load --input "$3" >/dev/null
  [ "$(docker image inspect --format '{{.Id}}' "$1" 2>/dev/null)" = "$1" ] || { echo "Das geladene Abbild ist nicht das erwartete." >&2; exit 3; }
fi
rm -f /opt/nod-assets/images/browser.tar.zst /opt/nod-assets/images/postgres.tar.zst 2>/dev/null || true
docker image ls --no-trunc --format '{{.ID}}' | grep -vx "$1" | xargs -r docker image rm -f >/dev/null 2>&1 || true
if [ "$5" = 1 ]; then docker network inspect aiscraft-net >/dev/null 2>&1 || docker network create aiscraft-net >/dev/null; fi
if [ -n "$2" ]; then
  # With userns-remap the container's user 1000 is dockremap's first id + 1000 here.
  base=$(sed -n 's/^dockremap:\([0-9]*\):.*/\1/p' /etc/subuid 2>/dev/null | head -n 1)
  owner=$(( ${base:-0} + 1000 ))
  install -d -m 0700 "/work/$2"
  [ "$(stat -c %u "/work/$2")" = "$owner" ] || chown -R "$owner:$owner" "/work/$2"
  used=$(du -sm "/work/$2" | cut -f1)
  [ "$used" -le "$6" ] || { echo "Der Arbeitsordner dieses Chats ist voll (über $6 MB). Bitte Dateien darin löschen oder in den Einstellungen den Arbeitsordner leeren." >&2; exit 4; }
  all=$(du -sm /work | cut -f1)
  [ "$all" -le "$7" ] || { echo "Die Arbeitsordner aller Chats sind zusammen über $7 MB groß. Bitte alte Arbeitsordner leeren." >&2; exit 4; }
fi"#;
/// Remove a chat's working folder. `$1` chat.
pub(crate) const CLEAR_SCRIPT: &str = r#"[ -n "$1" ] && rm -rf -- "/work/$1""#;

/// A machine script: `/bin/sh -c SCRIPT aiscraft ARGS…` (`$0` is `aiscraft`).
pub(crate) fn machine_script(script: &str, args: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = vec!["/bin/sh".into(), "-c".into(), script.into(), "aiscraft".into()];
    v.extend(args.iter().map(|a| a.to_string()));
    v
}

/// The folder name of a chat: its id, letters, digits and dashes only.
pub(crate) fn chat_folder(conv: &str) -> Result<String> {
    let s: String = conv.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
    if s.is_empty() || s.starts_with('-') {
        return Err(Error::invalid("Kein Chat für den Arbeitsordner"));
    }
    Ok(s)
}

/// A path inside the chat's working folder, relative to `/work`: no `..`, nothing absolute
/// outside `/work`, no control characters, no part starting with `-` (it would read as an
/// option). `""` and `.` are the folder itself.
pub(crate) fn rel_path(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.chars().count() > 1024 || raw.chars().any(char::is_control) || raw.contains('\\') {
        return Err(Error::invalid("Kein gültiger Pfad im Arbeitsordner (ohne \\ und Steuerzeichen, höchstens 1024 Zeichen)"));
    }
    let rest = if raw == "/work" {
        ""
    } else if let Some(r) = raw.strip_prefix("/work/") {
        r
    } else if raw.starts_with('/') || raw.starts_with('~') {
        return Err(Error::invalid(format!("{raw}: Pfade liegen im Arbeitsordner /work (z. B. daten/tabelle.csv)")));
    } else {
        raw
    };
    let mut parts = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(Error::invalid(format!("{raw}: „..“ führt aus dem Arbeitsordner"))),
            p if p.starts_with('-') => return Err(Error::invalid(format!("{raw}: Namen dürfen nicht mit „-“ beginnen"))),
            p if p.len() > 255 => return Err(Error::invalid(format!("{raw}: ein Name ist zu lang"))),
            p => parts.push(p),
        }
    }
    Ok(if parts.is_empty() { ".".into() } else { parts.join("/") })
}

/// The one way a container is started. `name` lets a time limit stop it (`docker kill`).
pub(crate) fn docker_run(chat: &str, name: &str, network: bool, image: &str, script: &str, args: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = [
        "docker",
        "run",
        "--rm",
        "-i",
        "--name",
        name,
        "--network",
        if network { NETWORK } else { "none" },
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--user",
        USER,
        "--memory",
        MEMORY,
        "--memory-swap",
        MEMORY,
        "--cpus",
        CPUS,
        "--pids-limit",
        PIDS,
        "--ulimit",
        FSIZE,
        "--read-only",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,size=512m",
        "--env",
        "HOME=/tmp",
        "--log-driver",
        "none",
        "--workdir",
        "/work",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push("--mount".into());
    v.push(format!("type=bind,src=/work/{chat},dst=/work"));
    v.push(image.into());
    v.extend(machine_script(script, args));
    v
}

/// Formats `workspace_convert` makes, as LibreOffice names them.
pub(crate) const FORMATS: &[&str] = &["pdf", "docx", "xlsx", "pptx", "odt", "ods", "odp", "csv", "html", "txt"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_in_the_working_folder() {
        assert_eq!(rel_path("").unwrap(), ".");
        assert_eq!(rel_path("/work").unwrap(), ".");
        assert_eq!(rel_path("/work/a/b.txt").unwrap(), "a/b.txt");
        assert_eq!(rel_path("./a//b.txt").unwrap(), "a/b.txt");
        for bad in ["../x", "a/../../x", "/etc/passwd", "~/x", "-rf", "a/-x", "a\\b", "a\u{0}b", "/mnt/c/Users"] {
            assert!(rel_path(bad).is_err(), "{bad}");
        }
        assert_eq!(chat_folder("3f2a-xx/../..").unwrap(), "3f2a-xx");
        assert!(chat_folder("../").is_err());
    }

    #[test]
    fn a_start_is_locked_down() {
        assert!(
            PREPARE_SCRIPT.contains("\"userns-remap\":\"default\"") && PREPARE_SCRIPT.contains("pkill -x dockerd"),
            "every computer maps container users to an unprivileged range"
        );
        let argv = docker_run("c1", "aiscraft-x", false, "sha256:abc", RUN_SCRIPT, &[".", "ls"]);
        let joined = argv.join(" ");
        for need in [
            "--network none",
            "--cap-drop ALL",
            "--security-opt no-new-privileges",
            "--user 1000:1000",
            "--memory 2g",
            "--cpus 2",
            "--pids-limit 1024",
            "--read-only",
        ] {
            assert!(joined.contains(need), "{need}: {joined}");
        }
        assert_eq!(argv.iter().filter(|a| a.as_str() == "--mount").count(), 1, "one mount only");
        assert!(!joined.contains("--privileged") && !joined.contains("docker.sock"));
        assert!(docker_run("c1", "n", true, "sha256:abc", RUN_SCRIPT, &[]).join(" ").contains("--network aiscraft-net"));
    }
}
