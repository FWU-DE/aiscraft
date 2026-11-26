//! Sign in to AIS.chat in a real browser and take the session cookie from it.
//!
//! AIS.ChatCraft has no webview. Instead it starts an installed Chromium-based browser (Chrome,
//! Chromium, Edge, Brave, Vivaldi) with its own profile and the DevTools protocol on localhost,
//! opens the AIS.chat login there, and reads the session cookie with `Storage.getCookies` as soon
//! as the person has signed in (VIDIS, school account or Microsoft 365). The profile stays, so a
//! later sign-in can usually run headless and silently while the identity provider's session holds.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use aiscraft_core::{Error, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::aischat::SESSION_COOKIE;

/// Browsers to try, most common first, per platform. `AISCRAFT_BROWSER` overrides.
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(b) = std::env::var("AISCRAFT_BROWSER")
        && !b.is_empty()
    {
        out.push(PathBuf::from(b));
    }
    if cfg!(target_os = "macos") {
        for app in ["Google Chrome", "Chromium", "Microsoft Edge", "Brave Browser", "Vivaldi"] {
            out.push(PathBuf::from(format!("/Applications/{app}.app/Contents/MacOS/{app}")));
        }
    } else if cfg!(windows) {
        let roots: Vec<String> = ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"].iter().filter_map(|v| std::env::var(v).ok()).collect();
        for r in roots {
            for rel in [
                r"Google\Chrome\Application\chrome.exe",
                r"Microsoft\Edge\Application\msedge.exe",
                r"BraveSoftware\Brave-Browser\Application\brave.exe",
                r"Chromium\Application\chrome.exe",
            ] {
                out.push(Path::new(&r).join(rel));
            }
        }
    } else {
        for name in
            ["chromium", "chromium-browser", "google-chrome-stable", "google-chrome", "brave-browser", "brave", "microsoft-edge-stable", "vivaldi-stable"]
        {
            if let Some(p) = which(name) {
                out.push(p);
            }
        }
    }
    out.into_iter().filter(|p| p.is_file()).collect()
}

fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(name)).find(|p| p.is_file()))
}

/// The session cookie as a `Cookie` header, from a `Storage.getCookies` result.
pub fn cookie_from_cdp(result: &Value, host: &str) -> Option<String> {
    let host = host.trim_start_matches('.');
    let mut parts: Vec<(String, String)> = result
        .get("cookies")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?;
            let value = c.get("value")?.as_str()?;
            let domain = c.get("domain").and_then(Value::as_str).unwrap_or("").trim_start_matches('.');
            (name.starts_with(SESSION_COOKIE) && !value.is_empty() && (domain == host || host.ends_with(&format!(".{domain}"))))
                .then(|| (name.to_string(), value.to_string()))
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    parts.sort();
    Some(parts.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; "))
}

/// Ask a DevTools endpoint for cookies until the session cookie appears or `timeout` passes.
pub async fn wait_for_cookie(port: u16, host: &str, timeout: Duration, alive: &mut (dyn FnMut() -> bool + Send)) -> Result<String> {
    let deadline = Instant::now() + timeout;
    let http = crate::http::client(false)?;
    let mut last_err = String::new();
    while Instant::now() < deadline {
        if !alive() {
            return Err(Error::cancelled());
        }
        match read_cookies(&http, port).await {
            Ok(result) => {
                if let Some(c) = cookie_from_cdp(&result, host) {
                    return Ok(c);
                }
            }
            Err(e) => last_err = e.message,
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
    }
    Err(Error::auth(if last_err.is_empty() {
        "Anmeldung nicht abgeschlossen (Zeit abgelaufen)".into()
    } else {
        format!("Anmeldung nicht abgeschlossen: {last_err}")
    }))
}

async fn read_cookies(http: &reqwest::Client, port: u16) -> Result<Value> {
    cdp_call(http, port, "Storage.getCookies").await
}

/// One DevTools call on the browser target; returns its `result`.
async fn cdp_call(http: &reqwest::Client, port: u16, method: &str) -> Result<Value> {
    let v: Value = http
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
        .map_err(crate::http::net_err)?
        .json()
        .await
        .map_err(|e| Error::protocol(format!("DevTools: {e}")))?;
    let ws = v.get("webSocketDebuggerUrl").and_then(Value::as_str).ok_or_else(|| Error::protocol("DevTools: keine Adresse"))?;
    if !ws.starts_with("ws://127.0.0.1:") && !ws.starts_with("ws://localhost:") {
        return Err(Error::protocol("DevTools-Adresse ist nicht lokal"));
    }
    let (mut sock, _) = tokio::time::timeout(Duration::from_secs(5), tokio_tungstenite::connect_async(ws))
        .await
        .map_err(|_| Error::network("DevTools antwortet nicht"))?
        .map_err(|e| Error::network(format!("DevTools: {e}")))?;
    sock.send(WsMessage::Text(json!({"id": 1, "method": method}).to_string().into())).await.map_err(|e| Error::network(format!("DevTools: {e}")))?;
    let reply = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(m) = sock.next().await {
            if let Ok(WsMessage::Text(t)) = m
                && let Ok(v) = serde_json::from_str::<Value>(&t)
                && v.get("id").and_then(Value::as_u64) == Some(1)
            {
                return Some(v);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
    .ok_or_else(|| Error::protocol("DevTools: keine Antwort"))?;
    let _ = sock.close(None).await;
    if let Some(e) = reply.get("error") {
        return Err(Error::protocol(format!("DevTools: {e}")));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

fn read_port(profile: &Path) -> Option<u16> {
    std::fs::read_to_string(profile.join("DevToolsActivePort")).ok()?.lines().next()?.trim().parse().ok()
}

/// A browser from an earlier sign-in that still holds the profile (the app was ended while its
/// window was open) would take over the new start: Chromium hands the window to the running
/// instance and the new process ends. Close it through DevTools while it still answers.
async fn close_leftover_browser(profile: &Path) {
    let Some(port) = read_port(profile) else { return };
    let Ok(http) = crate::http::client(false) else { return };
    if cdp_call(&http, port, "Browser.close").await.is_err() {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && http.get(format!("http://127.0.0.1:{port}/json/version")).send().await.is_ok() {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    // The profile lock goes a moment after the endpoint.
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// A start handed over to another browser that holds the profile. The sign-in ends such leftovers
/// itself and tries again; this text only stays when even that did not help.
const LEFTOVER: &str = "Ein anderer Browser hält das Anmeldeprofil von AIS.ChatCraft und ließ sich nicht beenden. Bitte den Rechner neu anmelden oder im Task-Manager Browser-Prozesse ohne Fenster beenden, dann erneut anmelden.";

/// Whether a process's command line uses exactly this profile folder (`--user-data-dir=<dir>`).
/// Only browsers started with the app's own AIS.chat profile match, never the person's browser.
fn uses_profile(cmd: &[std::ffi::OsString], profile: &Path) -> bool {
    let want = profile.to_string_lossy().trim_end_matches(['/', '\\']).to_lowercase();
    if want.is_empty() {
        return false;
    }
    cmd.iter().any(|a| {
        let a = a.to_string_lossy();
        a.strip_prefix("--user-data-dir=").is_some_and(|v| v.trim_matches('"').trim_end_matches(['/', '\\']).to_lowercase() == want)
    })
}

/// Ends browser processes without a window (a hidden sign-in from before, a background process
/// after the app ended) that still hold the app's own profile, so a new sign-in can start.
/// Returns how many were ended.
fn end_profile_processes(profile: &Path) -> usize {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always));
    let mut ended = 0;
    for p in sys.processes().values() {
        if uses_profile(p.cmd(), profile) && p.kill() {
            ended += 1;
        }
    }
    ended
}

/// How long a browser that relaunched itself (Edge on Windows starts a second process and the
/// first one ends at once) gets to write its port file after the started process ended.
const RELAUNCH_GRACE: Duration = Duration::from_secs(10);

/// Waits for the port file of the browser just started. `alive` says whether that process still
/// runs; when it ended and no port file came within [`RELAUNCH_GRACE`], another browser has the
/// profile.
async fn wait_for_port(profile: &Path, timeout: Duration, grace: Duration, alive: &mut (dyn FnMut() -> bool + Send)) -> Result<u16> {
    let deadline = Instant::now() + timeout;
    let mut ended_at: Option<Instant> = None;
    while Instant::now() < deadline {
        if let Some(port) = read_port(profile) {
            return Ok(port);
        }
        if !alive() {
            // The started process ended: a browser that relaunched itself writes the port file
            // a moment later; only when none comes did another browser take the start over.
            let since = *ended_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= grace {
                return read_port(profile).ok_or_else(|| Error::protocol(LEFTOVER));
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    Err(Error::protocol("Der Browser hat seine DevTools-Schnittstelle nicht geöffnet"))
}

pub struct LoginOptions {
    pub base_url: String,
    /// The browser profile AIS.ChatCraft keeps for AIS.chat.
    pub profile_dir: PathBuf,
    /// Without a window: only works while the identity provider still knows the person.
    pub headless: bool,
    pub timeout: Duration,
    pub browser: Option<PathBuf>,
}

/// Open the AIS.chat login in a browser and return the session cookie header.
pub async fn login(opts: LoginOptions) -> Result<String> {
    let browser = opts.browser.clone().or_else(|| candidates().into_iter().next()).ok_or_else(|| {
        Error::not_found(
            "Kein Chromium-Browser gefunden (Chrome, Chromium, Edge, Brave, Vivaldi). Den Cookie stattdessen von Hand einfügen oder AISCRAFT_BROWSER setzen.",
        )
    })?;
    let host =
        reqwest::Url::parse(&opts.base_url).ok().and_then(|u| u.host_str().map(str::to_string)).ok_or_else(|| Error::invalid("Ungültige AIS.chat-Adresse"))?;
    std::fs::create_dir_all(&opts.profile_dir).map_err(|e| Error::storage(format!("Browser-Profil: {e}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&opts.profile_dir, std::fs::Permissions::from_mode(0o700));
    }
    free_profile(&opts.profile_dir).await;
    // Already signed in (the profile still holds a valid AIS.chat session): a short hidden look
    // finds the cookie, and no window opens at all.
    if !opts.headless {
        let quick = run(&browser, &opts.profile_dir, &opts.base_url, &host, true, SIGNED_IN_LOOK).await;
        free_profile(&opts.profile_dir).await;
        if let Ok(cookie) = quick {
            return Ok(cookie);
        }
    }
    match run(&browser, &opts.profile_dir, &opts.base_url, &host, opts.headless, opts.timeout).await {
        // A browser without a window still held the profile: end it and try once more.
        Err(e) if e.message == LEFTOVER => {
            free_profile(&opts.profile_dir).await;
            run(&browser, &opts.profile_dir, &opts.base_url, &host, opts.headless, opts.timeout).await
        }
        other => other,
    }
}

/// How long the hidden look for an existing session may take before the window opens.
const SIGNED_IN_LOOK: Duration = Duration::from_secs(8);

/// Closes a browser from before that still holds the profile (politely over DevTools, then by
/// ending its processes) and removes its port file.
async fn free_profile(profile: &Path) {
    close_leftover_browser(profile).await;
    if end_profile_processes(profile) > 0 {
        // The profile lock goes a moment after the processes.
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
}

/// One browser start with the profile: hidden or as a small app window on the login page; the
/// session cookie as soon as it is there.
async fn run(browser: &Path, profile: &Path, base_url: &str, host: &str, headless: bool, timeout: Duration) -> Result<String> {
    let login_url = format!("{}/login", base_url.trim_end_matches('/'));
    let mut cmd = tokio::process::Command::new(browser);
    cmd.arg(format!("--user-data-dir={}", profile.display()))
        .args(["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "--no-first-run", "--no-default-browser-check"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if headless {
        cmd.args(["--headless=new", &login_url]);
    } else {
        cmd.arg(format!("--app={login_url}")).arg("--window-size=520,760");
    }
    let mut child = cmd.spawn().map_err(|e| Error::not_found(format!("Browser {} lässt sich nicht starten: {e}", browser.display())))?;
    let result = async {
        let mut started = || matches!(child.try_wait(), Ok(None));
        let port = wait_for_port(profile, Duration::from_secs(20), RELAUNCH_GRACE, &mut started).await?;
        // The window is open as long as its DevTools endpoint answers, also when the browser
        // relaunched itself and the process started here has long ended (Edge on Windows).
        let mut open = || {
            matches!(child.try_wait(), Ok(None))
                || std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
        };
        wait_for_cookie(port, host, timeout, &mut open).await
    }
    .await;
    // A relaunched browser is not our child: close it over DevTools as well.
    close_leftover_browser(profile).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    match result {
        Err(e) if e.kind == aiscraft_core::ErrorKind::Cancelled => Err(Error::auth("Das Anmeldefenster wurde geschlossen, bevor die Anmeldung fertig war.")),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_only_the_session_cookie_of_the_host() {
        let r = json!({"cookies": [
            {"name": SESSION_COOKIE, "value": "abc", "domain": "app.ais-chat.schule"},
            {"name": "__Secure-ais-chat-app.authjs.csrf-token", "value": "x", "domain": "app.ais-chat.schule"},
            {"name": SESSION_COOKIE, "value": "other", "domain": "evil.example"},
            {"name": format!("{SESSION_COOKIE}.1"), "value": "b2", "domain": ".ais-chat.schule"},
            {"name": format!("{SESSION_COOKIE}.0"), "value": "b1", "domain": ".ais-chat.schule"},
        ]});
        assert_eq!(cookie_from_cdp(&r, "app.ais-chat.schule").unwrap(), format!("{SESSION_COOKIE}=abc; {SESSION_COOKIE}.0=b1; {SESSION_COOKIE}.1=b2"));
        assert_eq!(cookie_from_cdp(&json!({"cookies": []}), "x"), None);
        assert_eq!(cookie_from_cdp(&json!("garbage"), "x"), None);
    }

    /// A start handed to a browser that already holds the profile ends at once without a port
    /// file: say so right away instead of waiting 20 s for nothing.
    #[tokio::test]
    async fn a_start_taken_over_by_a_leftover_browser_is_named_at_once() {
        let dir = std::env::temp_dir().join(format!("aiscraft-port-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("DevToolsActivePort"));
        let started = Instant::now();
        let err = wait_for_port(&dir, Duration::from_secs(20), Duration::from_millis(600), &mut || false).await.unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(err.message, LEFTOVER);

        std::fs::write(dir.join("DevToolsActivePort"), "9222\n/devtools/browser/x\n").unwrap();
        assert_eq!(wait_for_port(&dir, Duration::from_secs(1), Duration::ZERO, &mut || false).await.unwrap(), 9222);
        // Nothing answers on that port: closing a leftover is a no-op, never an error.
        close_leftover_browser(&dir).await;
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Edge on Windows relaunches itself: the started process ends at once and the real one
    /// writes the port file a moment later. That is a normal start, not a leftover.
    #[tokio::test]
    async fn a_browser_that_relaunches_itself_is_waited_for() {
        let dir = std::env::temp_dir().join(format!("aiscraft-relaunch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join("DevToolsActivePort"));
        let file = dir.join("DevToolsActivePort");
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let _ = std::fs::write(file, "9333\n/devtools/browser/y\n");
        });
        let port = wait_for_port(&dir, Duration::from_secs(10), Duration::from_secs(5), &mut || false).await.unwrap();
        assert_eq!(port, 9333);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Only browsers started with exactly the app's profile count as leftovers, never the
    /// person's own browser or another profile that merely starts the same way.
    #[test]
    fn only_processes_with_the_apps_profile_are_leftovers() {
        let os = |v: &[&str]| v.iter().map(std::ffi::OsString::from).collect::<Vec<_>>();
        let profile = Path::new(r"C:\Users\x\AppData\Roaming\uds\aiscraft\data\browser-profile");
        assert!(uses_profile(&os(&["msedge.exe", r"--user-data-dir=C:\Users\x\AppData\Roaming\uds\aiscraft\data\browser-profile", "--headless=new"]), profile));
        assert!(uses_profile(&os(&["chrome", r#"--user-data-dir="c:\users\x\appdata\roaming\uds\aiscraft\data\browser-profile\""#]), profile));
        assert!(!uses_profile(&os(&["msedge.exe"]), profile), "the person's browser");
        assert!(!uses_profile(&os(&["chrome", r"--user-data-dir=C:\Users\x\AppData\Roaming\uds\aiscraft\data\browser-profile-2"]), profile));
        assert!(!uses_profile(&os(&["chrome", r"--profile-directory=browser-profile"]), profile));
        assert!(!uses_profile(&os(&["chrome", "--user-data-dir="]), Path::new("")));
    }

    /// A fake DevTools endpoint: the cookie appears on the third ask.
    #[tokio::test]
    async fn waits_until_the_cookie_appears() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let asks = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let asks2 = asks.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let asks = asks2.clone();
                tokio::spawn(async move {
                    let mut peek = [0u8; 4];
                    let _ = sock.peek(&mut peek).await;
                    let mut buf = vec![0u8; 4096];
                    let n = sock.peek(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    if head.starts_with("GET /json/version") {
                        let _ = sock.read(&mut buf).await;
                        let body = format!(r#"{{"webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/browser/x"}}"#);
                        let _ = sock
                            .write_all(
                                format!(
                                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}",
                                    body.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                        return;
                    }
                    let Ok(mut ws) = tokio_tungstenite::accept_async(sock).await else { return };
                    while let Some(Ok(WsMessage::Text(t))) = ws.next().await {
                        let v: Value = serde_json::from_str(&t).unwrap_or(Value::Null);
                        let n = asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let cookies = if n >= 2 { json!([{"name": SESSION_COOKIE, "value": "tok", "domain": "app.ais-chat.schule"}]) } else { json!([]) };
                        let _ = ws.send(WsMessage::Text(json!({"id": v["id"], "result": {"cookies": cookies}}).to_string().into())).await;
                    }
                });
            }
        });
        let mut alive = || true;
        let c = wait_for_cookie(port, "app.ais-chat.schule", Duration::from_secs(20), &mut alive).await.unwrap();
        assert_eq!(c, format!("{SESSION_COOKIE}=tok"));
        assert!(asks.load(std::sync::atomic::Ordering::SeqCst) >= 3);
        // A closed window ends the wait.
        let mut dead = || false;
        assert_eq!(wait_for_cookie(port, "x", Duration::from_secs(5), &mut dead).await.unwrap_err().kind, aiscraft_core::ErrorKind::Cancelled);
    }

    /// The real thing, headless (no window): Chromium signs in at a local page that sets the
    /// session cookie. `cargo test -p aiscraft-backends -- --ignored real_browser_login`
    #[tokio::test]
    #[ignore]
    async fn real_browser_login() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        if candidates().is_empty() {
            eprintln!("skipped: no Chromium-based browser");
            return;
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = sock.read(&mut buf).await;
                    let body = "<html><body>angemeldet</body></html>";
                    let _ = sock
                        .write_all(format!("HTTP/1.1 200 OK\r\nset-cookie: {SESSION_COOKIE}=live-token; Path=/; Secure; HttpOnly\r\ncontent-type: text/html\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes())
                        .await;
                });
            }
        });
        let profile = std::env::temp_dir().join(format!("aiscraft-browser-test-{}", std::process::id()));
        let cookie = login(LoginOptions {
            base_url: format!("http://localhost:{port}"),
            profile_dir: profile.clone(),
            headless: true,
            timeout: Duration::from_secs(30),
            browser: None,
        })
        .await
        .unwrap();
        assert_eq!(cookie, format!("{SESSION_COOKIE}=live-token"));
        let _ = std::fs::remove_dir_all(profile);
    }
}
