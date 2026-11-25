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
pub(crate) const READ_SCRIPT: &str = r#"[ -f "$1" ] || { echo "Datei fehlt: $1" >&2; exit 2; }; s=$(stat -c %s -- "$1"); [ "$s" -le "$2" ] || { echo "Die Datei ist größer als $2 Bytes ($s)." >&2; exit 3; }; base64 -w0 -- "$1""#;
/// `$1` file, `$2` target format: LibreOffice writes the result next to the file.
pub(crate) const CONVERT_SCRIPT: &str =
    r#"cd -- "$(dirname -- "$1")" && exec soffice --headless --norestore --nolockcheck --convert-to "$2" --outdir . "$(basename -- "$1")""#;
/// `$1` folder: type, size and path of up to 500 entries.
pub(crate) const LIST_SCRIPT: &str = r#"cd -- "$1" && find . -mindepth 1 -maxdepth 4 -printf '%y %s %P\n' | head -n 500"#;

/// In the private computer as root, before a container: Docker running, the workspace image
/// loaded (only after its tar matched the pinned checksum, as Nod does), every other image (an
/// older workspace) and Nod's database image we never use removed, the network for the switch
/// there, the chat's folder there and owned by the container's user. Nod's browser image and its
/// tar stay: the browser loads it when the model first needs it (#107).
/// `$1` image id (`Workspace::image`), `$2` chat (may be empty), `$3` the image tar, `$4` its
/// SHA-256, `$5` `1` when the network switch is on, `$6` the pinned browser image id (kept).
pub(crate) const PREPARE_SCRIPT: &str = r#"set -e
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
rm -f /opt/nod-assets/images/postgres.tar.zst 2>/dev/null || true
docker image ls --no-trunc --format '{{.ID}}' | grep -vx -e "$1" -e "${6:-$1}" | xargs -r docker image rm -f >/dev/null 2>&1 || true
if [ "$5" = 1 ]; then docker network inspect aiscraft-net >/dev/null 2>&1 || docker network create aiscraft-net >/dev/null; fi
if [ -n "$2" ]; then install -d -o 1000 -g 1000 -m 0700 "/work/$2"; fi"#;
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

// ---- the browser (#107) ---------------------------------------------------------------------
//
// A second container from Nod's browser image, started only when the model first needs the
// browser and ended after a while without use (`browser.rs`). As in Nod it shares nothing with
// the workspace: no folder, no network. Chromium runs headed on a private virtual display (Xvfb)
// inside it, driven by agent-browser through Nod's worker on `/data/control.sock`.

/// The browser container's name (one per private computer; chats are sessions in it).
pub(crate) const BROWSER_NAME: &str = "aiscraft-browser";
/// Its own network (bridge `aiscraft-br0`, 172.31.92.0/24, made by `BROWSER_PREPARE_SCRIPT`): no
/// other container on it, containers on it cannot reach each other, and the rules there keep it
/// to public addresses.
pub(crate) const BROWSER_NETWORK: &str = "aiscraft-browser";
/// Nod's local desktop limits for the browser.
pub(crate) const BROWSER_PIDS: &str = "768";

/// As root in the private computer, before the browser starts: the pinned browser image loaded
/// (only after its tar matched the pinned checksum), the browser's network there and kept to
/// public addresses, as Nod's `protectNetworks` does: every private, loopback, link-local,
/// carrier-grade and multicast range is rejected, nothing reaches the computer itself. If the
/// rules cannot be set, the browser does not start.
///
/// Seam for the allowlist (#106): the chain `AISCRAFT-BROWSER` is where every new connection of
/// the browser passes; an allowlist proxy replaces its last rule (`RETURN`, „every public
/// address“) with its own, and nothing else here changes.
/// `$1` image id, `$2` the image tar, `$3` its SHA-256.
pub(crate) const BROWSER_PREPARE_SCRIPT: &str = r#"set -e
if [ "$(docker image inspect --format '{{.Id}}' "$1" 2>/dev/null)" != "$1" ]; then
  [ -f "$2" ] || { echo "Das Browser-Abbild fehlt im eigenen Rechner. Bitte unter Einstellungen › Fähigkeiten den eigenen Rechner entfernen und neu einrichten." >&2; exit 4; }
  echo "$3  $2" | sha256sum -c --quiet - >/dev/null 2>&1 || { echo "Das Browser-Abbild ist beschädigt. Bitte den eigenen Rechner entfernen und neu einrichten." >&2; exit 3; }
  docker load --input "$2" >/dev/null
  [ "$(docker image inspect --format '{{.Id}}' "$1" 2>/dev/null)" = "$1" ] || { echo "Das geladene Browser-Abbild ist nicht das erwartete." >&2; exit 3; }
fi
docker network inspect aiscraft-browser >/dev/null 2>&1 || docker network create --driver bridge --subnet 172.31.92.0/24 --opt com.docker.network.bridge.name=aiscraft-br0 --opt com.docker.network.bridge.enable_icc=false aiscraft-browser >/dev/null
fail() { echo "Die Sperre privater Adressen für den Browser ließ sich nicht setzen; der Browser startet nicht." >&2; exit 5; }
{
  printf '%s\n' '*filter' ':AISCRAFT-BROWSER - [0:0]' '-F AISCRAFT-BROWSER'
  for range in 0.0.0.0/8 10.0.0.0/8 100.64.0.0/10 127.0.0.0/8 169.254.0.0/16 172.16.0.0/12 192.0.0.0/24 192.168.0.0/16 198.18.0.0/15 224.0.0.0/3; do
    printf '%s\n' "-A AISCRAFT-BROWSER -d $range -j REJECT"
  done
  printf '%s\n' '-A AISCRAFT-BROWSER -j RETURN' 'COMMIT'
} | iptables-restore --wait 10 --noflush || fail
iptables -w 10 -C DOCKER-USER -i aiscraft-br0 -j AISCRAFT-BROWSER 2>/dev/null || iptables -w 10 -I DOCKER-USER 1 -i aiscraft-br0 -j AISCRAFT-BROWSER || fail
iptables -w 10 -C INPUT -i aiscraft-br0 -j DROP 2>/dev/null || iptables -w 10 -I INPUT 1 -i aiscraft-br0 -j DROP || fail"#;

/// Wait until Nod's worker in the browser answers on its socket. `$1` the check (`READY_JS`).
pub(crate) const BROWSER_READY_SCRIPT: &str = r#"i=0
until docker exec aiscraft-browser node -e "$1" >/dev/null 2>&1; do
  [ "$(docker inspect --format '{{.State.Running}}' aiscraft-browser 2>/dev/null)" = true ] || { echo "Der Browser hat sich beim Start beendet." >&2; exit 1; }
  i=$((i+1))
  if [ $i -gt 120 ]; then echo "Der Browser antwortet nicht." >&2; exit 1; fi
  sleep 0.5
done"#;

/// Nod's own readiness check: the worker's socket accepts a connection.
pub(crate) const READY_JS: &str =
    r#"const s=require("node:net").connect("/data/control.sock");s.on("connect",()=>{s.end();process.exit(0)});s.on("error",()=>process.exit(1))"#;

/// One request to Nod's worker, as Nod's stdin bridge does it but with the request as an
/// argument: `$1` the request frame, `$2` an optional second frame (input during a take-over),
/// `$3` an optional marker: the first line holding it ends the call (a view frame, an input's
/// answer). Every line the worker sends is printed; the worker ends the connection after its
/// answer.
pub(crate) const BRIDGE_JS: &str = r#"const [req,extra,stop]=process.argv.slice(1);const s=require("node:net").connect("/data/control.sock");let b="";s.setEncoding("utf8");s.on("connect",()=>{s.write(req+"\n");if(extra)s.write(extra+"\n")});s.on("data",d=>{b+=d;for(let i=b.indexOf("\n");i>=0;i=b.indexOf("\n")){const l=b.slice(0,i);b=b.slice(i+1);process.stdout.write(l+"\n");if(stop&&l.includes(stop)){s.destroy();return}}});s.on("end",()=>{if(b)process.stdout.write(b+"\n")});s.on("error",e=>{process.stderr.write("Der Browser antwortet nicht: "+e.message+"\n");process.exitCode=2})"#;

/// The one way the browser container is started: the same hardening as the workspace (no
/// capabilities, no new privileges, an unprivileged user, a read-only root, limits), Nod's
/// browser limits, its own network, and no mount at all: `/tmp`, the home folder and `/data`
/// (the worker's socket, Chromium's profile) live in memory and are gone when it ends.
pub(crate) fn browser_run(image: &str) -> Vec<String> {
    let mut v: Vec<String> = [
        "docker",
        "run",
        "-d",
        "--rm",
        "--name",
        BROWSER_NAME,
        "--init",
        "--network",
        BROWSER_NETWORK,
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--user",
        USER,
        "--read-only",
        "--memory",
        MEMORY,
        "--memory-swap",
        MEMORY,
        "--cpus",
        CPUS,
        "--pids-limit",
        BROWSER_PIDS,
        "--shm-size",
        "256m",
        "--tmpfs",
        "/tmp:rw,nosuid,nodev,size=1g",
        "--tmpfs",
        "/home/node:rw,nosuid,nodev,uid=1000,gid=1000,mode=700,size=256m",
        "--tmpfs",
        "/data:rw,nosuid,nodev,uid=1000,gid=1000,mode=700,size=512m",
        "--env",
        "TZ=Europe/Berlin",
        "--log-driver",
        "none",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push(image.into());
    v
}

/// A call into the running browser: `docker exec` as its unprivileged user, never as root.
pub(crate) fn browser_exec(request: &str, extra: &str, stop: &str) -> Vec<String> {
    ["docker", "exec", "-i", "--user", "node", BROWSER_NAME, "node", "-e", BRIDGE_JS, request, extra, stop].iter().map(|s| s.to_string()).collect()
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
