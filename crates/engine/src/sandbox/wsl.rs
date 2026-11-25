//! The private computer on Windows (#94): an own WSL2 distribution `aiscraft-<profil>`, imported
//! with `wsl --import … --version 2` from the checked image. Only this distribution is ever
//! touched: never the default distribution, never `.wslconfig` or other global WSL settings.
//!
//! The distribution mounts no Windows drive and cannot start Windows programs: `/etc/wsl.conf`
//! turns automount and interop off. The image ships that file, and [`Wsl::create`] writes it
//! again and checks after a restart that no drive is mounted, so it does not depend on the image.
//!
//! `wsl.exe` comes from `%SystemRoot%\System32` (never from `PATH`), runs without a console
//! window, with `WSL_UTF8=1` (its own messages in UTF-8 instead of UTF-16). Arguments travel
//! base64-encoded and are decoded by a fixed script in the distribution, so no quoting rule of
//! the Windows command line can change what a command means.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use super::runtime::{Output, SandboxRuntime};

/// What the distribution's `/etc/wsl.conf` says: no Windows drives, no Windows programs, root.
pub(crate) const WSL_CONF: &str = "[automount]\nenabled = false\nmountFsTab = false\n\n[interop]\nenabled = false\nappendWindowsPath = false\n\n[user]\ndefault = root\n\n[boot]\nsystemd = false\n";

/// Decodes the base64 arguments and runs them (`$0` is `aiscraft`).
const DECODER: &str = r#"for a do set -- "$@" "$(printf %s "$a" | base64 -d)"; shift; done; exec "$@""#;

/// Fails when a Windows drive is mounted (`/mnt/c` and the like).
const NO_DRIVES: &str = r#"! grep -Eq '^[^ ]+ /mnt/[a-zA-Z] ' /proc/mounts"#;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(crate) const MISSING: &str = "WSL fehlt auf diesem Rechner. Eine Administratorin oder ein Administrator richtet es einmal ein: Eingabeaufforderung als Administrator öffnen, „wsl --install --no-distribution“ ausführen und den Rechner neu starten. Danach hier noch einmal „Einrichten“.";
pub(crate) const TOO_OLD: &str = "Die WSL-Version auf diesem Rechner ist zu alt. Eine Administratorin oder ein Administrator führt einmal „wsl --update“ aus; danach hier noch einmal „Einrichten“.";
const NO_VIRTUALIZATION: &str = "WSL2 kann hier keinen virtuellen Rechner starten: Die Windows-Funktion „VM-Plattform“ fehlt oder die Virtualisierung ist im BIOS/UEFI aus. Eine Administratorin oder ein Administrator schaltet beides ein (z. B. „wsl --install --no-distribution“) und startet neu.";

pub(crate) struct Wsl {
    distro: String,
    exe: PathBuf,
}

impl Wsl {
    pub(crate) fn new(distro: String) -> Wsl {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        Wsl { distro, exe: root.join("System32").join("wsl.exe") }
    }
}

/// Run a program: stdin fed in, both streams capped, ended at the time limit.
pub(crate) async fn run_program(exe: &Path, args: &[String], stdin: Vec<u8>, timeout: Duration, max: usize) -> Result<Output> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args).env("WSL_UTF8", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child =
        cmd.spawn().map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Error::unsupported(MISSING) } else { Error::io(exe.display(), &e) })?;
    let mut input = child.stdin.take();
    let out = child.stdout.take();
    let err = child.stderr.take();
    let feed = async move {
        if let Some(i) = input.as_mut() {
            let _ = i.write_all(&stdin).await;
            let _ = i.shutdown().await;
        }
        drop(input);
    };
    let work = async {
        let (_, (o, ocut), (e, ecut)) = tokio::join!(feed, read_capped(out, max), read_capped(err, max));
        let status = child.wait().await;
        (o, ocut || ecut, e, status)
    };
    match tokio::time::timeout(timeout, work).await {
        Ok((stdout, truncated, stderr, status)) => {
            let status = status.map_err(|e| Error::io(exe.display(), &e))?;
            Ok(Output { code: status.code(), stdout, stderr, timed_out: false, truncated })
        }
        // The child is dropped and killed with the future.
        Err(_) => Ok(Output { code: None, timed_out: true, ..Default::default() }),
    }
}

async fn read_capped<R: AsyncRead + Unpin>(r: Option<R>, max: usize) -> (Vec<u8>, bool) {
    let Some(mut r) = r else { return (Vec::new(), false) };
    let mut out = Vec::new();
    let mut cut = false;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = max.saturating_sub(out.len());
                out.extend_from_slice(buf.get(..n.min(room)).unwrap_or(&[]));
                // Keep reading (and dropping), so the program is not stuck on a full pipe.
                cut |= n > room;
            }
        }
    }
    (out, cut)
}

/// `wsl.exe`'s own output: UTF-8 with `WSL_UTF8`, UTF-16 from older versions.
pub(crate) fn decode(bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units).replace('\u{feff}', "")
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// Whether `list` (`wsl --list --quiet`) names `distro`.
pub(crate) fn listed(list: &str, distro: &str) -> bool {
    list.lines().any(|l| l.trim().trim_matches('\u{0}').eq_ignore_ascii_case(distro))
}

/// What a failed `wsl --import` means for the person.
pub(crate) fn import_error(out: &Output) -> String {
    let text = format!("{}\n{}", decode(&out.stdout), decode(&out.stderr));
    let t = text.to_lowercase();
    if t.contains("hcs_e_hyperv_not_installed") || t.contains("0x80370102") || t.contains("virtual machine platform") || t.contains("vm-plattform") {
        NO_VIRTUALIZATION.into()
    } else if t.contains("--version") && (t.contains("invalid") || t.contains("ungültig")) {
        TOO_OLD.into()
    } else {
        let short: String = text.trim().chars().take(300).collect();
        format!("Der eigene Rechner ließ sich nicht anlegen: {short}")
    }
}

fn b64(s: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

impl Wsl {
    fn exec_args(&self, argv: &[String]) -> Vec<String> {
        let mut a: Vec<String> =
            ["-d", &self.distro, "-u", "root", "--cd", "/", "--exec", "/bin/sh", "-c", DECODER, "aiscraft"].iter().map(|s| s.to_string()).collect();
        a.extend(argv.iter().map(|x| b64(x)));
        a
    }

    fn guard(&self) -> Result<()> {
        // Never anything but our own distribution.
        if self.distro.starts_with("aiscraft-") && self.distro.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            Ok(())
        } else {
            Err(Error::refused("Nur die eigene Distribution aiscraft-… wird verändert."))
        }
    }
}

impl SandboxRuntime for Wsl {
    fn kind(&self) -> &'static str {
        "WSL2"
    }

    fn machine(&self) -> String {
        self.distro.clone()
    }

    fn probe(&self) -> BoxFuture<'static, std::result::Result<(), String>> {
        let exe = self.exe.clone();
        Box::pin(async move {
            if !exe.is_file() {
                return Err(MISSING.into());
            }
            // `--version` exists only in the WSL from the Store (WSL2 with `--import --version`).
            match run_program(&exe, &["--version".into()], Vec::new(), Duration::from_secs(20), 64 * 1024).await {
                Ok(o) if o.ok() => Ok(()),
                Ok(_) => Err(TOO_OLD.into()),
                Err(e) => Err(e.message),
            }
        })
    }

    fn exists(&self) -> BoxFuture<'static, bool> {
        let (exe, distro) = (self.exe.clone(), self.distro.clone());
        Box::pin(async move {
            match run_program(&exe, &["--list".into(), "--quiet".into()], Vec::new(), Duration::from_secs(20), 256 * 1024).await {
                Ok(o) => listed(&decode(&o.stdout), &distro),
                Err(_) => false,
            }
        })
    }

    fn create(&self, image: PathBuf, dir: PathBuf) -> BoxFuture<'static, Result<()>> {
        let (exe, distro, guard) = (self.exe.clone(), self.distro.clone(), self.guard());
        let exec_conf = self.exec_args(&super::container::machine_script("umask 022; cat > /etc/wsl.conf", &[]));
        let check = self.exec_args(&super::container::machine_script(NO_DRIVES, &[]));
        Box::pin(async move {
            guard?;
            crate::disk::create_dir(&dir)?;
            let args = vec!["--import".into(), distro.clone(), dir.display().to_string(), image.display().to_string(), "--version".into(), "2".into()];
            let out = run_program(&exe, &args, Vec::new(), Duration::from_secs(30 * 60), 64 * 1024).await?;
            if !out.ok() {
                return Err(Error::unsupported(import_error(&out)));
            }
            // Our own wsl.conf, whatever the image brought, then a restart so it applies.
            let conf = run_program(&exe, &exec_conf, WSL_CONF.as_bytes().to_vec(), Duration::from_secs(120), 64 * 1024).await?;
            let stop = run_program(&exe, &["--terminate".into(), distro.clone()], Vec::new(), Duration::from_secs(60), 64 * 1024).await?;
            let drives = run_program(&exe, &check, Vec::new(), Duration::from_secs(120), 64 * 1024).await?;
            if !conf.ok() || !stop.ok() || !drives.ok() {
                let _ = run_program(&exe, &["--unregister".into(), distro], Vec::new(), Duration::from_secs(120), 64 * 1024).await;
                return Err(Error::protocol(format!(
                    "Der eigene Rechner ließ sich nicht abschotten (Windows-Laufwerke nicht sicher ausgehängt: {}). Er wurde wieder entfernt.",
                    if drives.ok() { conf.why() } else { drives.why() }
                )));
            }
            Ok(())
        })
    }

    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, timeout: Duration, max_output: usize) -> BoxFuture<'static, Result<Output>> {
        let (exe, args) = (self.exe.clone(), self.exec_args(&argv));
        Box::pin(async move { run_program(&exe, &args, stdin, timeout, max_output).await })
    }

    fn stop(&self) -> BoxFuture<'static, Result<()>> {
        let (exe, distro, guard) = (self.exe.clone(), self.distro.clone(), self.guard());
        Box::pin(async move {
            guard?;
            run_program(&exe, &["--terminate".into(), distro], Vec::new(), Duration::from_secs(60), 64 * 1024).await.map(|_| ())
        })
    }

    fn destroy(&self) -> BoxFuture<'static, Result<()>> {
        let (exe, distro, guard) = (self.exe.clone(), self.distro.clone(), self.guard());
        Box::pin(async move {
            guard?;
            let list = run_program(&exe, &["--list".into(), "--quiet".into()], Vec::new(), Duration::from_secs(20), 256 * 1024).await?;
            if !listed(&decode(&list.stdout), &distro) {
                return Ok(());
            }
            let out = run_program(&exe, &["--unregister".into(), distro], Vec::new(), Duration::from_secs(300), 64 * 1024).await?;
            if out.ok() { Ok(()) } else { Err(Error::storage(format!("Der eigene Rechner ließ sich nicht entfernen: {}", out.why()))) }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_output_in_both_encodings() {
        let utf16: Vec<u8> = "\u{feff}Ubuntu\r\naiscraft-default\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert!(listed(&decode(&utf16), "aiscraft-default"));
        assert!(listed(&decode(b"Ubuntu\naiscraft-default\n"), "aiscraft-default"));
        assert!(!listed(&decode(b"Ubuntu\naiscraft-default-2\n"), "aiscraft-default"));
    }

    #[test]
    fn only_our_distribution_and_no_drives() {
        assert!(Wsl::new("Ubuntu".into()).guard().is_err());
        assert!(Wsl::new("aiscraft-default".into()).guard().is_ok());
        assert!(WSL_CONF.contains("[automount]\nenabled = false") && WSL_CONF.contains("[interop]\nenabled = false"));
        // The image ships the same file the app writes.
        assert_eq!(include_str!("../../../../packaging/sandbox/wsl.conf").replace("\r\n", "\n"), WSL_CONF);
        let a = Wsl::new("aiscraft-default".into()).exec_args(&["echo".into(), "a \"b\" $c".into()]);
        assert!(a.iter().skip(11).all(|x| x.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))), "{a:?}");
        let out = Output { code: Some(1), stderr: b"Error code: Wsl/Service/CreateInstance/HCS_E_HYPERV_NOT_INSTALLED".to_vec(), ..Default::default() };
        assert!(import_error(&out).contains("VM-Plattform"));
    }
}
