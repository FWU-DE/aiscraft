//! The private Linux computer the workspace runs in (#94), behind one trait: WSL2 on Windows
//! (`wsl.rs`), Lima on macOS and Linux later ([`Unsupported`] until then), and a fake for tests
//! ([`Fake`]) that never starts anything.
//!
//! A runtime knows one machine (on Windows the distribution `aiscraft-<profil>`) and can create
//! it from an image, run a program in it as its root, stop and remove it. What runs in the
//! machine (Docker, the locked-down container) is decided in `container.rs`, the same for every
//! runtime.

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::future::BoxFuture;

/// What a program in the machine gave back.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Output {
    /// Exit code; `None` when it was stopped (time limit) or ended by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The time limit ended it.
    pub timed_out: bool,
    /// More output than kept; the rest was dropped.
    pub truncated: bool,
}

impl Output {
    pub(crate) fn ok(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// stderr, else stdout, as one short line for an error message.
    pub(crate) fn why(&self) -> String {
        let text = if self.stderr.is_empty() { &self.stdout } else { &self.stderr };
        let s = String::from_utf8_lossy(text);
        let s: String = s.trim().chars().take(400).collect();
        if s.is_empty() { format!("Exit-Code {}", self.code.map(|c| c.to_string()).unwrap_or_else(|| "–".into())) } else { s }
    }
}

/// One private computer.
pub(crate) trait SandboxRuntime: Send + Sync {
    /// How people call it („WSL2“).
    fn kind(&self) -> &'static str;
    /// Whether this system has a way to run it at all (false: macOS and Linux until Lima).
    fn supported(&self) -> bool {
        true
    }
    /// The machine's name (`aiscraft-<profil>`).
    fn machine(&self) -> String;
    /// Whether this computer can run it at all: `Err` names what is missing and the next step.
    /// Only asks the system (no machine is started).
    fn probe(&self) -> BoxFuture<'static, std::result::Result<(), String>>;
    /// Whether the machine exists.
    fn exists(&self) -> BoxFuture<'static, bool>;
    /// Create the machine from the checked image `image`, its disk in `dir`.
    fn create(&self, image: PathBuf, dir: PathBuf) -> BoxFuture<'static, Result<()>>;
    /// Run `argv` in the machine as its root, `stdin` fed in, at most `timeout`, at most
    /// `max_output` bytes kept per stream.
    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, timeout: Duration, max_output: usize) -> BoxFuture<'static, Result<Output>>;
    /// Stop the machine (everything in it ends; files stay).
    fn stop(&self) -> BoxFuture<'static, Result<()>>;
    /// Remove the machine with its disk.
    fn destroy(&self) -> BoxFuture<'static, Result<()>>;
}

pub(crate) type Shared = Arc<dyn SandboxRuntime>;

/// The runtime for this system, for the real app.
pub(crate) fn for_this_system(machine: String) -> Shared {
    #[cfg(windows)]
    {
        Arc::new(super::wsl::Wsl::new(machine))
    }
    #[cfg(not(windows))]
    {
        Arc::new(Unsupported { machine })
    }
}

/// macOS and Linux until the Lima runtime exists.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) struct Unsupported {
    #[cfg_attr(windows, allow(dead_code))]
    pub(crate) machine: String,
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) const NOT_YET: &str = "Auf macOS und Linux ist der eigene Rechner noch nicht verfügbar (geplant über Lima). Unter Windows läuft er über WSL2.";

impl SandboxRuntime for Unsupported {
    fn supported(&self) -> bool {
        false
    }
    fn kind(&self) -> &'static str {
        "Lima"
    }
    fn machine(&self) -> String {
        self.machine.clone()
    }
    fn probe(&self) -> BoxFuture<'static, std::result::Result<(), String>> {
        Box::pin(async { Err(NOT_YET.to_string()) })
    }
    fn exists(&self) -> BoxFuture<'static, bool> {
        Box::pin(async { false })
    }
    fn create(&self, _image: PathBuf, _dir: PathBuf) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { Err(Error::unsupported(NOT_YET)) })
    }
    fn exec(&self, _argv: Vec<String>, _stdin: Vec<u8>, _timeout: Duration, _max: usize) -> BoxFuture<'static, Result<Output>> {
        Box::pin(async { Err(Error::unsupported(NOT_YET)) })
    }
    fn stop(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn destroy(&self) -> BoxFuture<'static, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

/// What the fake saw: every call, in order.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FakeCall {
    Probe,
    Create(PathBuf),
    Exec(Vec<String>),
    Stop,
    Destroy,
}

#[cfg(test)]
/// A machine for tests: nothing is started. It understands the scripts of `container.rs` well
/// enough to keep written files per chat and to answer commands with what it was asked.
#[derive(Default)]
pub(crate) struct Fake {
    pub(crate) calls: Mutex<Vec<FakeCall>>,
    /// The probe's answer (`Err`: WSL missing).
    pub(crate) missing: Mutex<Option<String>>,
    exists: Mutex<bool>,
    /// Files by machine path (`/work/<chat>/<path>`).
    files: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

#[cfg(test)]
impl Fake {
    pub(crate) fn new() -> Arc<Fake> {
        Arc::new(Fake::default())
    }

    pub(crate) fn calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Every `docker run` the fake was asked to start.
    pub(crate) fn container_starts(&self) -> Vec<Vec<String>> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                FakeCall::Exec(argv) if argv.first().map(String::as_str) == Some("docker") && argv.get(1).map(String::as_str) == Some("run") => Some(argv),
                _ => None,
            })
            .collect()
    }

    pub(crate) fn file(&self, path: &str) -> Option<Vec<u8>> {
        self.files.lock().unwrap_or_else(PoisonError::into_inner).get(path).cloned()
    }

    fn record(&self, c: FakeCall) {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).push(c);
    }

    fn answer(&self, argv: &[String], stdin: &[u8]) -> Output {
        use super::container as k;
        let ok = |out: &[u8]| Output { code: Some(0), stdout: out.to_vec(), ..Default::default() };
        if argv.get(2).map(String::as_str) == Some(k::CLEAR_SCRIPT) {
            let prefix = format!("/work/{}/", argv.get(4).map(String::as_str).unwrap_or("?"));
            self.files.lock().unwrap_or_else(PoisonError::into_inner).retain(|p, _| !p.starts_with(&prefix));
            return ok(b"");
        }
        // `docker run … IMAGE /bin/sh -c SCRIPT aiscraft ARGS…`
        let Some(at) = argv.iter().position(|a| a == k::IMAGE) else {
            return ok(b"");
        };
        let mount = argv.iter().find_map(|a| a.strip_prefix("type=bind,src=")).and_then(|m| m.split(',').next()).unwrap_or("/work/x").to_string();
        let script = argv.get(at + 3).map(String::as_str).unwrap_or("");
        let args: Vec<&str> = argv.iter().skip(at + 5).map(String::as_str).collect();
        let arg = |i: usize| args.get(i).copied().unwrap_or("");
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        match script {
            k::RUN_SCRIPT => {
                let cmd = arg(1);
                if cmd.contains("sleep") {
                    return Output { code: None, timed_out: true, stdout: b"angefangen\n".to_vec(), ..Default::default() };
                }
                if let Some(rest) = cmd.strip_prefix("exit ") {
                    return Output { code: rest.trim().parse().ok(), stderr: b"fehlgeschlagen".to_vec(), ..Default::default() };
                }
                // As the real Docker under WSL2: a warning on every start.
                let warning =
                    b"WARNING: Your kernel does not support swap limit capabilities or the cgroup is not mounted. Memory limited without swap.\n".to_vec();
                Output { stderr: warning, ..ok(format!("ausgeführt in {}: {cmd}\n", arg(0)).as_bytes()) }
            }
            k::WRITE_SCRIPT => {
                files.insert(format!("{mount}/{}", arg(0)), stdin.to_vec());
                ok(b"")
            }
            k::READ_SCRIPT => match files.get(&format!("{mount}/{}", arg(0))) {
                Some(b) => {
                    use base64::Engine as _;
                    ok(base64::engine::general_purpose::STANDARD.encode(b).as_bytes())
                }
                None => Output { code: Some(2), stderr: format!("Datei fehlt: {}", arg(0)).into_bytes(), ..Default::default() },
            },
            k::CONVERT_SCRIPT => {
                let (src, to) = (arg(0), arg(1));
                match files.get(&format!("{mount}/{src}")).cloned() {
                    Some(b) => {
                        let stem = src.rsplit_once('.').map(|(s, _)| s).unwrap_or(src);
                        let mut pdf = b"%PDF-1.7 umgewandelt\n".to_vec();
                        pdf.extend_from_slice(&b);
                        files.insert(format!("{mount}/{stem}.{to}"), pdf);
                        ok(b"convert ok")
                    }
                    None => Output { code: Some(1), stderr: b"Error: source file could not be loaded".to_vec(), ..Default::default() },
                }
            }
            k::LIST_SCRIPT => {
                let prefix = format!("{mount}/");
                let rows: Vec<String> = files.iter().filter_map(|(p, b)| p.strip_prefix(&prefix).map(|rel| format!("f {} {rel}", b.len()))).collect();
                ok(rows.join("\n").as_bytes())
            }
            _ => ok(b""),
        }
    }
}

#[cfg(test)]
impl SandboxRuntime for Fake {
    fn kind(&self) -> &'static str {
        "Test"
    }
    fn machine(&self) -> String {
        "aiscraft-test".into()
    }
    fn probe(&self) -> BoxFuture<'static, std::result::Result<(), String>> {
        self.record(FakeCall::Probe);
        let r = match self.missing.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            Some(why) => Err(why),
            None => Ok(()),
        };
        Box::pin(async move { r })
    }
    fn exists(&self) -> BoxFuture<'static, bool> {
        let e = *self.exists.lock().unwrap_or_else(PoisonError::into_inner);
        Box::pin(async move { e })
    }
    fn create(&self, image: PathBuf, _dir: PathBuf) -> BoxFuture<'static, Result<()>> {
        self.record(FakeCall::Create(image));
        *self.exists.lock().unwrap_or_else(PoisonError::into_inner) = true;
        Box::pin(async { Ok(()) })
    }
    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, _timeout: Duration, _max: usize) -> BoxFuture<'static, Result<Output>> {
        if !*self.exists.lock().unwrap_or_else(PoisonError::into_inner) {
            return Box::pin(async { Err(Error::not_found("Kein eigener Rechner eingerichtet")) });
        }
        self.record(FakeCall::Exec(argv.clone()));
        let out = self.answer(&argv, &stdin);
        Box::pin(async move { Ok(out) })
    }
    fn stop(&self) -> BoxFuture<'static, Result<()>> {
        self.record(FakeCall::Stop);
        Box::pin(async { Ok(()) })
    }
    fn destroy(&self) -> BoxFuture<'static, Result<()>> {
        self.record(FakeCall::Destroy);
        *self.exists.lock().unwrap_or_else(PoisonError::into_inner) = false;
        self.files.lock().unwrap_or_else(PoisonError::into_inner).clear();
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_and_linux_say_not_yet() {
        let u = Unsupported { machine: "aiscraft-default".into() };
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap_or_else(|_| std::process::abort());
        let why = rt.block_on(u.probe()).err().unwrap_or_default();
        assert!(why.contains("noch nicht verfügbar"), "{why}");
        assert!(rt.block_on(u.exec(vec!["echo".into()], Vec::new(), Duration::from_secs(1), 10)).is_err());
        assert_eq!(u.machine(), "aiscraft-default");
        assert!(!u.supported());
    }
}
