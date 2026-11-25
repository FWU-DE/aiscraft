//! The private Linux computer the workspace runs in (#94, #102), behind one trait: WSL2 on
//! Windows (`wsl.rs`), Lima with QEMU/KVM on Linux (`lima.rs`), not yet on macOS
//! ([`Unsupported`]), and a fake for tests ([`Fake`]) that never starts anything.
//!
//! A runtime knows one machine (`aiscraft-<profil>`) and can create it from the checked
//! downloads, run a program in it as its root, stop and remove it. What runs in the machine
//! (Docker, the locked-down container) is decided in `container.rs`, the same for every runtime.

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

/// The workspace image a machine runs: what `docker run` names, where its tar lies in the
/// machine, and the tar's pinned checksum (checked before it is loaded).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Workspace {
    pub image: String,
    pub tar: &'static str,
    pub tar_sha256: &'static str,
}

impl Workspace {
    /// Nod's Windows runtime (also what the fake runs).
    pub(crate) fn windows() -> Workspace {
        Workspace { image: super::image::WORKSPACE_ID.into(), tar: super::image::WORKSPACE_TAR, tar_sha256: super::image::WORKSPACE_TAR_SHA256 }
    }

    /// Nod's browser image in the Windows runtime (#107; also what the fake runs).
    pub(crate) fn windows_browser() -> Workspace {
        Workspace { image: super::image::BROWSER_ID.into(), tar: super::image::BROWSER_TAR, tar_sha256: super::image::BROWSER_TAR_SHA256 }
    }
}

/// One private computer.
pub(crate) trait SandboxRuntime: Send + Sync {
    /// How people call it („WSL2“, „Lima“).
    fn kind(&self) -> &'static str;
    /// Whether this system has a way to run it at all (false: macOS until Lima with `vz`).
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
    /// Create the machine from the checked downloads `files` (in the order of the source's
    /// parts), its disk in `dir`.
    fn create(&self, files: Vec<PathBuf>, dir: PathBuf) -> BoxFuture<'static, Result<()>>;
    /// The workspace image this machine runs.
    fn workspace(&self) -> Result<Workspace> {
        Ok(Workspace::windows())
    }
    /// Nod's browser image in this machine (#107), loaded only when the model first needs it.
    fn browser(&self) -> Result<Workspace> {
        Ok(Workspace::windows_browser())
    }
    /// Run `argv` in the machine as its root, `stdin` fed in, at most `timeout`, at most
    /// `max_output` bytes kept per stream.
    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, timeout: Duration, max_output: usize) -> BoxFuture<'static, Result<Output>>;
    /// Start `argv` in the machine as its root and keep its stdin and stdout open (the relay of
    /// a run with network, `net.rs`); stderr is dropped.
    fn stream(&self, _argv: Vec<String>) -> BoxFuture<'static, Result<Pipe>> {
        Box::pin(async { Err(Error::unsupported("Dieser Rechner hat kein Netz für den Arbeitsbereich.")) })
    }
    /// Stop the machine (everything in it ends; files stay).
    fn stop(&self) -> BoxFuture<'static, Result<()>>;
    /// Remove the machine with its disk.
    fn destroy(&self) -> BoxFuture<'static, Result<()>>;
}

pub(crate) type Shared = Arc<dyn SandboxRuntime>;

/// A program in the machine that keeps running: its stdin and stdout, and what keeps it alive
/// (dropping `guard` ends it).
pub(crate) struct Pipe {
    pub reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    pub writer: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    pub guard: Box<dyn std::any::Any + Send>,
}

/// The runtime for this system, for the real app. `dir` is `<data>/sandbox`.
pub(crate) fn for_this_system(machine: String, dir: PathBuf) -> Shared {
    #[cfg(windows)]
    {
        let _ = dir;
        Arc::new(super::wsl::Wsl::new(machine))
    }
    #[cfg(target_os = "linux")]
    {
        Arc::new(super::lima::Lima::new(machine, dir))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = dir;
        Arc::new(Unsupported { machine })
    }
}

/// macOS until the Lima runtime with Apple's virtualization (`vz`) exists.
#[cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]
pub(crate) struct Unsupported {
    pub(crate) machine: String,
}

#[cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]
pub(crate) const NOT_YET: &str =
    "Auf macOS ist der eigene Rechner noch nicht verfügbar (geplant über Lima). Unter Windows läuft er über WSL2, unter Linux über Lima mit QEMU/KVM.";

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
    fn create(&self, _files: Vec<PathBuf>, _dir: PathBuf) -> BoxFuture<'static, Result<()>> {
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
    Create(Vec<PathBuf>),
    Exec(Vec<String>),
    /// A program kept running (the relay).
    Stream(Vec<String>),
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
    /// The working folders are over their limit (the prepare script says so).
    pub(crate) full: Mutex<bool>,
    exists: Mutex<bool>,
    /// Files by machine path (`/work/<chat>/<path>`).
    files: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
    /// The container side of the running relay: what a fake `curl` in a run talks through.
    relay: Arc<tokio::sync::Mutex<Option<tokio::io::DuplexStream>>>,
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

    /// Every relay the fake was asked to start.
    pub(crate) fn relay_starts(&self) -> Vec<Vec<String>> {
        self.calls().into_iter().filter_map(|c| if let FakeCall::Stream(a) = c { Some(a) } else { None }).collect()
    }

    fn record(&self, c: FakeCall) {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).push(c);
    }

    fn answer(&self, argv: &[String], stdin: &[u8]) -> Output {
        use super::container as k;
        let ok = |out: &[u8]| Output { code: Some(0), stdout: out.to_vec(), ..Default::default() };
        // A call into the browser (#107): Nod's worker, played.
        if argv.get(1).map(String::as_str) == Some("exec") && argv.get(8).map(String::as_str) == Some(k::BRIDGE_JS) {
            let req: serde_json::Value = argv.get(9).and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
            let extra: serde_json::Value = argv.get(10).and_then(|r| serde_json::from_str(r).ok()).unwrap_or_default();
            return ok(fake_browser(&req, &extra).as_bytes());
        }
        if argv.get(2).map(String::as_str) == Some(k::PREPARE_SCRIPT) && *self.full.lock().unwrap_or_else(PoisonError::into_inner) {
            return Output { code: Some(4), stderr: "Der Arbeitsordner dieses Chats ist voll (über 2048 MB).".as_bytes().to_vec(), ..Default::default() };
        }
        if argv.get(2).map(String::as_str) == Some(k::CLEAR_SCRIPT) {
            let prefix = format!("/work/{}/", argv.get(4).map(String::as_str).unwrap_or("?"));
            self.files.lock().unwrap_or_else(PoisonError::into_inner).retain(|p, _| !p.starts_with(&prefix));
            return ok(b"");
        }
        // `docker run … IMAGE /bin/sh -c SCRIPT aiscraft ARGS…`
        let image = Workspace::windows().image;
        let Some(at) = argv.iter().position(|a| *a == image) else {
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
            k::READ_SCRIPT if arg(0).contains("riesig") => Output { code: Some(0), stdout: b"QUJD".to_vec(), truncated: true, ..Default::default() },
            k::READ_SCRIPT => match files.get(&format!("{mount}/{}", arg(0))) {
                Some(b) => {
                    use base64::Engine as _;
                    ok(base64::engine::general_purpose::STANDARD.encode(b).as_bytes())
                }
                None => Output { code: Some(2), stderr: format!("Datei fehlt: {}", arg(0)).into_bytes(), ..Default::default() },
            },
            k::CONVERT_SCRIPT if arg(0).contains("langsam") => Output { code: None, timed_out: true, ..Default::default() },
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

/// The page the fake browser shows: a sign-in form of a school platform with a name on it.
#[cfg(test)]
pub(crate) const FAKE_PAGE: &str = "- heading \"Anmeldung Lernplattform\" [ref=e1]\n- textbox \"Benutzername\" [ref=e2]\n- textbox \"Passwort\" [ref=e3]\n- button \"Anmelden\" [ref=e4]\nZuletzt angemeldet: Lea Müller";
/// Where the fake page's form sends.
#[cfg(test)]
pub(crate) const FAKE_FORM: &str = "https://lernen.example.org/anmelden";
/// The fake browser's picture (PNG signature and a little more).
#[cfg(test)]
pub(crate) const FAKE_PNG: &[u8] = b"\x89PNG\r\n\x1a\nein Bild der Seite";

/// What Nod's worker answers to one request (and the input of a take-over).
#[cfg(test)]
fn fake_browser(frame: &serde_json::Value, extra: &serde_json::Value) -> String {
    use base64::Engine as _;
    use serde_json::json;
    let req = frame.get("request").cloned().unwrap_or_default();
    let id = req.get("id").and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    let input = req.get("input").cloned().unwrap_or_default();
    let page = "https://lernen.example.org/login";
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let answer = |result: serde_json::Value| json!({"id": id, "ok": true, "result": result, "metadata": {"status": "Workspace browser is ready."}}).to_string();
    match req.get("method").and_then(serde_json::Value::as_str).unwrap_or("") {
        "act" => {
            let url = input.get("url").and_then(serde_json::Value::as_str).unwrap_or(page);
            let text = match input.get("action").and_then(serde_json::Value::as_str).unwrap_or("") {
                "snapshot" | "open" => FAKE_PAGE.to_string(),
                "fill" => "Feld ausgefüllt".into(),
                "click" => "Angeklickt".into(),
                other => format!("{other} erledigt"),
            };
            answer(json!({"text": text, "url": url}))
        }
        "currentPage" => answer(json!({"origin": "https://lernen.example.org", "url": page, "submits": {"method": "post", "to": FAKE_FORM}})),
        "screenshot" => answer(json!(b64(FAKE_PNG))),
        "download" => {
            let chunk = json!({"id": id, "chunk": b64(b"%PDF-1.7 Stundenplan")}).to_string();
            format!("{chunk}\n{}", answer(json!({"name": "stundenplan.pdf", "mimeType": "application/pdf"})))
        }
        "view" => {
            let frame = json!({"id": id, "frame": b64(FAKE_PNG)}).to_string();
            let input_id = extra.get("inputId").and_then(serde_json::Value::as_str).unwrap_or("");
            format!("{frame}\n{}", json!({"id": id, "inputId": input_id, "ok": true}))
        }
        _ => answer(serde_json::Value::Null),
    }
}

#[cfg(test)]
impl Fake {
    /// Every request the fake browser got (Nod's request objects), in order.
    pub(crate) fn browser_requests(&self) -> Vec<serde_json::Value> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                FakeCall::Exec(argv) if argv.get(1).map(String::as_str) == Some("exec") => {
                    argv.get(9).and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()).and_then(|f| f.get("request").cloned())
                }
                _ => None,
            })
            .collect()
    }
}

/// A run of `curl URL [--sni NAME]` in the fake: what it asks the relay for.
#[cfg(test)]
fn fake_curl_args(argv: &[String]) -> Option<(String, Option<String>)> {
    if !argv.iter().any(|a| a == super::container::RUN_SCRIPT) {
        return None;
    }
    let image = Workspace::windows().image;
    let at = argv.iter().position(|a| *a == image)?;
    let cmd = argv.get(at + 6)?;
    let mut words = cmd.strip_prefix("curl ")?.split_whitespace();
    let url = words.next()?.to_string();
    let sni = match (words.next(), words.next()) {
        (Some("--sni"), Some(n)) => Some(n.to_string()),
        _ => None,
    };
    Some((url, sni))
}

/// Everything that comes back through the relay for connection `id` until it is closed (or,
/// with `until_head`, until the end of a response head).
#[cfg(test)]
async fn relay_collect(s: &mut tokio::io::DuplexStream, id: u32, until_head: bool) -> Vec<u8> {
    let mut got = Vec::new();
    while let Ok(Ok((kind, cid, data))) = tokio::time::timeout(Duration::from_secs(20), super::net::read_frame(s)).await {
        if cid != id {
            continue;
        }
        if kind == super::net::T_CLOSE {
            break;
        }
        got.extend_from_slice(&data);
        if until_head && got.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    got
}

/// What `curl` gets through the relay: without one (no network) it cannot resolve the name, as
/// with `--network none`; with one it asks the app's proxy like the real curl with `HTTPS_PROXY`.
#[cfg(test)]
async fn fake_curl(relay: Arc<tokio::sync::Mutex<Option<tokio::io::DuplexStream>>>, url: &str, sni: Option<&str>) -> Output {
    use super::net::{T_CLOSE, T_DATA, T_OPEN, frame};
    use tokio::io::AsyncWriteExt as _;
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let mut guard = relay.lock().await;
    let Some(s) = guard.as_mut() else {
        return Output { code: Some(6), stderr: b"curl: (6) Could not resolve host".to_vec(), ..Default::default() };
    };
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let (https, rest) = match url.split_once("://") {
        Some(("https", r)) => (true, r),
        Some((_, r)) => (false, r),
        None => (false, url),
    };
    let authority = rest.split('/').next().unwrap_or("");
    if s.write_all(&frame(T_OPEN, id, &[])).await.is_err() {
        return Output { code: Some(7), stderr: b"relay".to_vec(), ..Default::default() };
    }
    if https {
        let target = if authority.contains(':') && !authority.ends_with(']') { authority.to_string() } else { format!("{authority}:443") };
        let host = target.rsplit_once(':').map(|(h, _)| h).unwrap_or("").to_string();
        let _ = s.write_all(&frame(T_DATA, id, format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())).await;
        let head = relay_collect(s, id, true).await;
        let text = String::from_utf8_lossy(&head).to_string();
        if !text.starts_with("HTTP/1.1 200") {
            let rest = relay_collect(s, id, false).await;
            return Output {
                code: Some(56),
                stdout: [head, rest].concat(),
                stderr: format!("curl: (56) CONNECT tunnel failed, response {}", text.split_whitespace().nth(1).unwrap_or("?")).into_bytes(),
                ..Default::default()
            };
        }
        let _ = s.write_all(&frame(T_DATA, id, &super::net::client_hello(sni.unwrap_or(&host)))).await;
    } else {
        let _ = s.write_all(&frame(T_DATA, id, format!("GET {url} HTTP/1.1\r\nHost: {authority}\r\nUser-Agent: curl\r\n\r\n").as_bytes())).await;
    }
    let _ = s.write_all(&frame(T_CLOSE, id, &[])).await;
    let body = relay_collect(s, id, false).await;
    if body.is_empty() {
        return Output { code: Some(35), stderr: b"curl: (35) TLS connect error: the connection was closed".to_vec(), ..Default::default() };
    }
    Output { code: Some(0), stdout: body, ..Default::default() }
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
    fn create(&self, files: Vec<PathBuf>, _dir: PathBuf) -> BoxFuture<'static, Result<()>> {
        self.record(FakeCall::Create(files));
        *self.exists.lock().unwrap_or_else(PoisonError::into_inner) = true;
        Box::pin(async { Ok(()) })
    }
    fn exec(&self, argv: Vec<String>, stdin: Vec<u8>, _timeout: Duration, _max: usize) -> BoxFuture<'static, Result<Output>> {
        if !*self.exists.lock().unwrap_or_else(PoisonError::into_inner) {
            return Box::pin(async { Err(Error::not_found("Kein eigener Rechner eingerichtet")) });
        }
        self.record(FakeCall::Exec(argv.clone()));
        if let Some((url, sni)) = fake_curl_args(&argv) {
            let relay = self.relay.clone();
            return Box::pin(async move { Ok(fake_curl(relay, &url, sni.as_deref()).await) });
        }
        if argv.get(2).map(String::as_str) == Some(super::net::NET_DOWN_SCRIPT) {
            // `docker rm -f` of the relay: its line to the engine ends.
            let relay = self.relay.clone();
            return Box::pin(async move {
                *relay.lock().await = None;
                Ok(Output { code: Some(0), ..Default::default() })
            });
        }
        let out = self.answer(&argv, &stdin);
        Box::pin(async move { Ok(out) })
    }
    fn stream(&self, argv: Vec<String>) -> BoxFuture<'static, Result<Pipe>> {
        if !*self.exists.lock().unwrap_or_else(PoisonError::into_inner) {
            return Box::pin(async { Err(Error::not_found("Kein eigener Rechner eingerichtet")) });
        }
        self.record(FakeCall::Stream(argv));
        let relay = self.relay.clone();
        Box::pin(async move {
            use tokio::io::AsyncWriteExt as _;
            let (engine_end, mut far) = tokio::io::duplex(1 << 20);
            far.write_all(&super::net::frame(super::net::T_READY, 0, &[])).await.map_err(|e| Error::io("relay", &e))?;
            *relay.lock().await = Some(far);
            let (r, w) = tokio::io::split(engine_end);
            Ok(Pipe { reader: Box::new(r), writer: Box::new(w), guard: Box::new(()) })
        })
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
