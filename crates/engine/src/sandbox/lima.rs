//! The private computer on Linux (#102), as n8n Nod runs it there: a Lima VM with QEMU and KVM.
//!
//! - **KVM is required** (`/dev/kvm`, readable and writable). Without hardware virtualization the
//!   computer is not started instead of emulating it slowly; the message names the next step
//!   (BIOS/UEFI, the `kvm` group, the Flatpak permission).
//! - **Lima** comes with the download after the yes (`image.rs`, pinned); its own folder
//!   (`LIMA_HOME`, `~/.aiscraft-lima/<kürzel>`, short because Lima keeps sockets there) is never
//!   the person's `~/.lima`. **QEMU and the OpenSSH client** come from the system, as Nod uses
//!   nothing else of it but KVM only because its Flatpak builds QEMU in; without them the message
//!   names the package. In the Flatpak (no host programs) the computer is not offered yet.
//! - **The guest** is the Ubuntu disk of Nod's runtime with Nod's Docker for the guest and its
//!   iptables packages, configured like Nod's `guest-config.mjs`: Docker only in the VM, the
//!   checked assets mounted read-only at `/opt/nod-assets` (the same paths as on Windows), no other
//!   host folder, every port forward off, no ssh agent or X11 forwarding.
//! - Commands run as root in the VM through `limactl shell`; arguments travel base64-encoded, as
//!   for WSL. The containers follow `container.rs` exactly as on Windows.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::future::BoxFuture;

use super::image::{LINUX_GUEST_FILES, LINUX_WORKSPACE_TAR_SHA256, WORKSPACE_TAR};
use super::process::{DECODER, b64, run};
use super::runtime::{Output, SandboxRuntime, Workspace};

pub(crate) const NO_KVM: &str = "Dieser Rechner bietet keine Hardware-Virtualisierung an (/dev/kvm fehlt). Im BIOS/UEFI „Intel VT-x“ bzw. „AMD-V/SVM“ einschalten, in einer virtuellen Maschine die verschachtelte Virtualisierung erlauben. Ohne sie startet AIS.ChatCraft den eigenen Rechner nicht, weil Emulieren viel zu langsam wäre.";
pub(crate) const NO_KVM_FLATPAK: &str = "Das Flatpak darf /dev/kvm nicht benutzen. Einmal im Terminal „flatpak override --user --device=kvm ai.uds.aischatcraft“ ausführen und AIS.ChatCraft neu starten.";
pub(crate) const KVM_GROUP: &str =
    "/dev/kvm ist da, aber dein Benutzer darf es nicht benutzen. Einmal im Terminal „sudo usermod -aG kvm $USER“ ausführen, danach ab- und wieder anmelden.";
pub(crate) const FLATPAK_QEMU: &str =
    "Im Flatpak bringt AIS.ChatCraft QEMU noch nicht mit. Für den eigenen Rechner bitte das AppImage oder das deb- bzw. rpm-Paket verwenden.";
pub(crate) const NO_QEMU: &str = "QEMU fehlt. Einmal im Terminal installieren: Ubuntu/Debian „sudo apt install qemu-system-x86 qemu-utils“, Fedora „sudo dnf install qemu-kvm qemu-img“, Arch „sudo pacman -S qemu-base“. Danach hier noch einmal „Einrichten“.";
pub(crate) const NO_SSH: &str = "Der OpenSSH-Client fehlt (Lima steuert den eigenen Rechner darüber). Einmal installieren: Ubuntu/Debian „sudo apt install openssh-client“, Fedora „sudo dnf install openssh-clients“.";
pub(crate) const NOT_X86: &str = "Unter Linux gibt es den eigenen Rechner bisher nur für x86-64-Prozessoren.";
const NO_TAR: &str = "Das Programm „tar“ fehlt auf diesem Rechner.";
const NO_LIMACTL: &str = "Lima fehlt im Datenordner. Bitte den eigenen Rechner entfernen und neu einrichten.";

/// The VM's settings: what Nod's `guestConfiguration` sets for Linux, with AIS.ChatCraft's
/// Docker unit and no port at all forwarded. `{assets}` and `{digest}` are filled in.
const LIMA_YAML: &str = r#"# AIS.ChatCraft – eigener Rechner (#102). Von der App geschrieben; nicht von Hand ändern.
minimumLimaVersion: "2.2.0"
vmType: qemu
arch: x86_64
images:
  - location: {assets_guest}
    arch: x86_64
    digest: "sha256:{digest}"
cpus: 2
memory: 4GiB
disk: 24GiB
mountType: 9p
mounts:
  - location: {assets}
    mountPoint: /opt/nod-assets
    writable: false
ssh:
  loadDotSSHPubKeys: false
  forwardAgent: false
  forwardX11: false
containerd:
  system: false
  user: false
hostResolver:
  enabled: true
  ipv6: false
portForwards:
  - guestPortRange: [1, 65535]
    ignore: true
provision:
  - mode: dependency
    skipDefaultDependencyResolution: true
    script: |
      #!/bin/bash
      set -eu
      export DEBIAN_FRONTEND=noninteractive
      if ! command -v iptables >/dev/null; then
        apt-get install -y --no-install-recommends /opt/nod-assets/guest-packages/*.deb
      fi
  - mode: system
    script: |
      #!/bin/bash
      set -euo pipefail
      tar -xzf /opt/nod-assets/docker-guest.tar.gz --strip-components=1 -C /usr/local/bin
      mkdir -p /etc/docker /work
      chmod 0711 /work
      cat > /etc/docker/daemon.json <<'JSON'
      {"bridge":"none","storage-driver":"overlay2","userland-proxy":false,"iptables":true,"ip6tables":false,"ipv6":false,"log-driver":"local","log-opts":{"max-size":"10m","max-file":"2"}}
      JSON
      cat > /etc/systemd/system/aiscraft-docker.service <<'UNIT'
      [Unit]
      Description=AIS.ChatCraft Docker im eigenen Rechner
      After=network-online.target
      Wants=network-online.target
      [Service]
      Type=notify
      ExecStart=/usr/local/bin/dockerd --host=unix:///var/run/docker.sock
      Restart=on-failure
      TimeoutStartSec=120
      LimitNOFILE=1048576
      Delegate=yes
      KillMode=process
      [Install]
      WantedBy=multi-user.target
      UNIT
      systemctl daemon-reload
      systemctl enable --now aiscraft-docker.service
probes:
  - mode: readiness
    description: Docker im eigenen Rechner
    script: |
      #!/bin/bash
      set -e
      sudo /usr/local/bin/docker info >/dev/null
"#;

/// The Lima settings for the checked assets in `assets`. Paths are written as JSON strings, which
/// YAML reads as quoted scalars, so no folder name can change the file's meaning.
pub(crate) fn lima_yaml(assets: &Path) -> String {
    let q = |p: &Path| serde_json::to_string(&p.display().to_string()).unwrap_or_else(|_| "\"\"".into());
    let guest = LINUX_GUEST_FILES.iter().find(|(p, ..)| *p == "guest.img").map(|(_, s, _)| *s).unwrap_or("");
    LIMA_YAML.replace("{assets_guest}", &q(&assets.join("guest.img"))).replace("{assets}", &q(assets)).replace("{digest}", guest)
}

pub(crate) struct Lima {
    machine: String,
    /// `<data>/sandbox`.
    base: PathBuf,
}

impl Lima {
    pub(crate) fn new(machine: String, base: PathBuf) -> Lima {
        Lima { machine, base }
    }

    fn limactl(&self) -> PathBuf {
        self.base.join("lima").join("bin").join("limactl")
    }

    fn assets(&self) -> PathBuf {
        self.base.join("nod")
    }

    /// Lima's own folder: short (Lima keeps unix sockets there), one per data folder.
    fn home(&self) -> PathBuf {
        use sha2::Digest as _;
        let h = sha2::Sha256::digest(self.base.display().to_string().as_bytes());
        let short: String = h.iter().take(4).map(|b| format!("{b:02x}")).collect();
        directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_else(std::env::temp_dir).join(".aiscraft-lima").join(short)
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![("LIMA_HOME", self.home().display().to_string())]
    }

    fn guard(&self) -> Result<()> {
        if self.machine.starts_with("aiscraft-") && self.machine.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            Ok(())
        } else {
            Err(Error::refused("Nur der eigene Rechner aiscraft-… wird verändert."))
        }
    }

    /// `limactl shell` for `argv` as root, arguments base64-encoded.
    pub(crate) fn shell_args(&self, argv: &[String]) -> Vec<String> {
        let mut a: Vec<String> =
            ["--tty=false", "shell", "--workdir", "/", &self.machine, "sudo", "/bin/sh", "-c", DECODER, "aiscraft"].iter().map(|s| s.to_string()).collect();
        a.extend(argv.iter().map(|x| b64(x)));
        a
    }
}

/// A program in `PATH`.
fn in_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file()))
}

/// What is missing on this computer, checked in the order a person fixes it.
pub(crate) async fn missing(kvm: &Path, flatpak: bool, has: impl Fn(&str) -> bool) -> std::result::Result<(), String> {
    if !cfg!(target_arch = "x86_64") {
        return Err(NOT_X86.into());
    }
    if !kvm.exists() {
        return Err(if flatpak { NO_KVM_FLATPAK.into() } else { NO_KVM.into() });
    }
    let sh = Path::new("/bin/sh");
    let path = kvm.display().to_string();
    let usable =
        run(sh, &["-c".into(), "test -r \"$1\" && test -w \"$1\"".into(), "aiscraft".into(), path], &[], Vec::new(), Duration::from_secs(10), 4096, "sh fehlt")
            .await
            .is_ok_and(|o| o.ok());
    if !usable {
        return Err(KVM_GROUP.into());
    }
    if flatpak {
        return Err(FLATPAK_QEMU.into());
    }
    if !(has("qemu-system-x86_64") && has("qemu-img")) {
        return Err(NO_QEMU.into());
    }
    if !has("ssh") {
        return Err(NO_SSH.into());
    }
    Ok(())
}

/// What a failed `limactl create/start` means for the person.
fn start_error(out: &Output) -> String {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let t = text.to_lowercase();
    if t.contains("kvm") && (t.contains("permission denied") || t.contains("could not access")) {
        KVM_GROUP.into()
    } else if t.contains("qemu") && (t.contains("not found") || t.contains("executable file")) {
        NO_QEMU.into()
    } else {
        let tail: String = text.trim().lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
        format!("Der eigene Rechner ließ sich nicht starten: {}", tail.chars().take(400).collect::<String>())
    }
}

/// Take `members` out of `archive` into `dir` with the system's `tar`.
async fn untar(archive: &Path, dir: &Path, members: &[&str]) -> Result<()> {
    crate::disk::create_dir(dir)?;
    let mut args = vec!["-xzf".to_string(), archive.display().to_string(), "-C".into(), dir.display().to_string()];
    args.extend(members.iter().map(|m| m.to_string()));
    let out = run(Path::new("tar"), &args, &[], Vec::new(), Duration::from_secs(30 * 60), 64 * 1024, NO_TAR).await?;
    if out.ok() { Ok(()) } else { Err(Error::storage(format!("{} ließ sich nicht entpacken: {}", archive.display(), out.why()))) }
}

/// Every guest file against its pin (they are about to run in the VM).
async fn check_guest_files(assets: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        for (path, sha, bytes) in LINUX_GUEST_FILES {
            let file = assets.join(path);
            let size = std::fs::metadata(&file).map(|m| m.len()).map_err(|e| Error::io(file.display(), &e))?;
            let got = super::image::sha256_file(&file).map_err(|e| Error::io(file.display(), &e))?;
            if size != *bytes || got != *sha {
                return Err(Error::protocol(format!("Die Prüfsumme von „{path}“ stimmt nicht; nichts eingerichtet.")));
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| Error::storage(e.to_string()))?
}

/// The workspace image's id from Nod's `manifest.json` (checked against its pin first).
pub(crate) fn workspace_id(manifest: &Path) -> Result<String> {
    let pin = LINUX_GUEST_FILES.iter().find(|(p, ..)| *p == "manifest.json").map(|(_, s, _)| *s).unwrap_or("");
    let got = super::image::sha256_file(manifest).map_err(|e| Error::io(manifest.display(), &e))?;
    if got != pin {
        return Err(Error::protocol(
            "Nods manifest.json stimmt nicht mit der eingetragenen Prüfsumme überein. Bitte den eigenen Rechner entfernen und neu einrichten.",
        ));
    }
    let bytes = std::fs::read(manifest).map_err(|e| Error::io(manifest.display(), &e))?;
    let v: serde_json::Value = serde_json::from_slice(&bytes)?;
    let id = v
        .get("images")
        .and_then(serde_json::Value::as_array)
        .and_then(|a| a.iter().find(|i| i.get("name").and_then(serde_json::Value::as_str) == Some("workspace")))
        .and_then(|i| i.get("id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let ok = id.strip_prefix("sha256:").is_some_and(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()));
    if ok { Ok(id.to_string()) } else { Err(Error::protocol("In Nods manifest.json fehlt die Abbild-ID des Arbeitsbereichs.")) }
}

impl SandboxRuntime for Lima {
    fn kind(&self) -> &'static str {
        "Lima"
    }

    fn machine(&self) -> String {
        self.machine.clone()
    }

    fn probe(&self) -> BoxFuture<'static, std::result::Result<(), String>> {
        Box::pin(async { missing(Path::new("/dev/kvm"), std::env::var_os("FLATPAK_ID").is_some(), in_path).await })
    }

    fn exists(&self) -> BoxFuture<'static, bool> {
        let file = self.home().join(&self.machine).join("lima.yaml");
        Box::pin(async move { file.is_file() })
    }

    fn create(&self, files: Vec<PathBuf>, _dir: PathBuf) -> BoxFuture<'static, Result<()>> {
        let (guard, base, assets, limactl, env, machine) = (self.guard(), self.base.clone(), self.assets(), self.limactl(), self.env(), self.machine.clone());
        Box::pin(async move {
            guard?;
            let (Some(lima_tar), Some(nod_tar)) = (files.first(), files.get(1)) else {
                return Err(Error::invalid("Lima und Nods Laufzeit fehlen im Download."));
            };
            untar(lima_tar, &base.join("lima"), &[]).await?;
            if !limactl.is_file() {
                return Err(Error::protocol(NO_LIMACTL));
            }
            let members: Vec<&str> = LINUX_GUEST_FILES.iter().map(|(p, ..)| *p).collect();
            untar(nod_tar, &assets, &members).await?;
            check_guest_files(assets.clone()).await?;
            let yaml = base.join("aiscraft-lima.yaml");
            crate::disk::write(&yaml, lima_yaml(&assets).as_bytes())?;
            let lima = |args: Vec<String>, secs: u64| {
                let (limactl, env) = (limactl.clone(), env.clone());
                async move { run(&limactl, &args, &env, Vec::new(), Duration::from_secs(secs), 256 * 1024, NO_LIMACTL).await }
            };
            let created = lima(vec!["--tty=false".into(), "create".into(), "--name".into(), machine.clone(), yaml.display().to_string()], 600).await?;
            if !created.ok() {
                return Err(Error::unsupported(start_error(&created)));
            }
            let started = lima(vec!["--tty=false".into(), "start".into(), machine.clone()], 1200).await?;
            if !started.ok() {
                let _ = lima(vec!["--tty=false".into(), "delete".into(), "--force".into(), machine], 300).await;
                return Err(Error::unsupported(start_error(&started)));
            }
            Ok(())
        })
    }

    fn workspace(&self) -> Result<Workspace> {
        Ok(Workspace { image: workspace_id(&self.assets().join("manifest.json"))?, tar: WORKSPACE_TAR, tar_sha256: LINUX_WORKSPACE_TAR_SHA256 })
    }

    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, timeout: Duration, max_output: usize) -> BoxFuture<'static, Result<Output>> {
        let (limactl, env, args) = (self.limactl(), self.env(), self.shell_args(&argv));
        Box::pin(async move { run(&limactl, &args, &env, stdin, timeout, max_output, NO_LIMACTL).await })
    }

    fn stream(&self, argv: Vec<String>) -> BoxFuture<'static, Result<super::runtime::Pipe>> {
        let r = super::process::spawn(&self.limactl(), &self.shell_args(&argv), &self.env(), NO_LIMACTL);
        Box::pin(async move { r })
    }

    fn stop(&self) -> BoxFuture<'static, Result<()>> {
        let (guard, limactl, env, machine) = (self.guard(), self.limactl(), self.env(), self.machine.clone());
        Box::pin(async move {
            guard?;
            run(&limactl, &["--tty=false".into(), "stop".into(), machine], &env, Vec::new(), Duration::from_secs(120), 64 * 1024, NO_LIMACTL).await.map(|_| ())
        })
    }

    fn destroy(&self) -> BoxFuture<'static, Result<()>> {
        let (guard, limactl, env, machine, home) = (self.guard(), self.limactl(), self.env(), self.machine.clone(), self.home());
        Box::pin(async move {
            guard?;
            if home.join(&machine).join("lima.yaml").is_file() && limactl.is_file() {
                let out = run(
                    &limactl,
                    &["--tty=false".into(), "delete".into(), "--force".into(), machine],
                    &env,
                    Vec::new(),
                    Duration::from_secs(300),
                    64 * 1024,
                    NO_LIMACTL,
                )
                .await?;
                if !out.ok() {
                    return Err(Error::storage(format!("Der eigene Rechner ließ sich nicht entfernen: {}", out.why())));
                }
            }
            // Lima's own folder holds only this computer and Lima's keys for it.
            if home.exists() {
                std::fs::remove_dir_all(&home).map_err(|e| Error::io(home.display(), &e))?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vm_mounts_only_the_checked_assets_read_only_and_forwards_nothing() {
        let y = lima_yaml(Path::new("/home/a/.local/share/aiscraft/sandbox/nod \"x\""));
        assert!(y.contains("vmType: qemu") && y.contains("digest: \"sha256:7bdaa53c"));
        assert!(y.contains("location: \"/home/a/.local/share/aiscraft/sandbox/nod \\\"x\\\"\""), "{y}");
        assert!(y.contains("mountPoint: /opt/nod-assets\n    writable: false"));
        assert_eq!(y.matches("mountPoint:").count(), 1, "one mount only");
        assert!(y.contains("guestPortRange: [1, 65535]\n    ignore: true"));
        assert!(y.contains("forwardAgent: false") && y.contains("loadDotSSHPubKeys: false"));
        assert!(!y.contains("{assets") && !y.contains("{digest}"));
    }

    #[test]
    fn exec_arguments_cannot_be_reinterpreted() {
        let l = Lima::new("aiscraft-default".into(), PathBuf::from("/tmp/x"));
        let a = l.shell_args(&["echo".into(), "a \"b\" $c; rm -rf /".into()]);
        assert_eq!(a.get(4).map(String::as_str), Some("aiscraft-default"));
        assert!(a.iter().skip(10).all(|x| x.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))), "{a:?}");
        assert!(Lima::new("default".into(), PathBuf::new()).guard().is_err());
        assert!(l.home().to_string_lossy().contains(".aiscraft-lima"));
    }

    #[test]
    fn missing_pieces_are_named_in_order() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap_or_else(|_| std::process::abort());
        let none = Path::new("/gibt/es/nicht/kvm");
        let r = rt.block_on(missing(none, false, |_| true));
        if cfg!(target_arch = "x86_64") {
            assert_eq!(r, Err(NO_KVM.to_string()));
            assert_eq!(rt.block_on(missing(none, true, |_| true)), Err(NO_KVM_FLATPAK.to_string()));
        }
        // A readable and writable file stands in for /dev/kvm (Unix only: `sh` and `test`).
        if cfg!(unix) && cfg!(target_arch = "x86_64") {
            let kvm = std::env::temp_dir().join(format!("aiscraft-kvm-{}", crate::new_id()));
            let _ = crate::disk::write(&kvm, b"");
            assert_eq!(rt.block_on(missing(&kvm, true, |_| true)), Err(FLATPAK_QEMU.to_string()));
            assert_eq!(rt.block_on(missing(&kvm, false, |n| n == "ssh")), Err(NO_QEMU.to_string()));
            assert_eq!(rt.block_on(missing(&kvm, false, |n| n != "ssh")), Err(NO_SSH.to_string()));
            assert_eq!(rt.block_on(missing(&kvm, false, |_| true)), Ok(()));
            let _ = std::fs::remove_file(kvm);
        }
    }

    #[test]
    fn the_workspace_id_comes_only_from_the_pinned_manifest() {
        let dir = std::env::temp_dir().join(format!("aiscraft-manifest-{}", crate::new_id()));
        let _ = crate::disk::create_dir(&dir);
        let forged = dir.join("manifest.json");
        let _ =
            crate::disk::write(&forged, br#"{"images":[{"name":"workspace","id":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}]}"#);
        let e = workspace_id(&forged).err().map(|e| e.message).unwrap_or_default();
        assert!(e.contains("Prüfsumme"), "{e}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
