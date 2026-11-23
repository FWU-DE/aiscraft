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
    sock.send(WsMessage::Text(json!({"id": 1, "method": "Storage.getCookies"}).to_string().into()))
        .await
        .map_err(|e| Error::network(format!("DevTools: {e}")))?;
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

async fn wait_for_port(profile: &Path, timeout: Duration) -> Result<u16> {
    let file = profile.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(s) = std::fs::read_to_string(&file)
            && let Some(port) = s.lines().next().and_then(|l| l.trim().parse::<u16>().ok())
        {
            return Ok(port);
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
    let _ = std::fs::remove_file(opts.profile_dir.join("DevToolsActivePort"));
    let login_url = format!("{}/login", opts.base_url.trim_end_matches('/'));
    let mut cmd = tokio::process::Command::new(&browser);
    cmd.arg(format!("--user-data-dir={}", opts.profile_dir.display()))
        .args(["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "--no-first-run", "--no-default-browser-check"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if opts.headless {
        cmd.args(["--headless=new", &login_url]);
    } else {
        cmd.arg(format!("--app={login_url}")).arg("--window-size=520,760");
    }
    let mut child = cmd.spawn().map_err(|e| Error::not_found(format!("Browser {} lässt sich nicht starten: {e}", browser.display())))?;
    let result = async {
        let port = wait_for_port(&opts.profile_dir, Duration::from_secs(20)).await?;
        let mut alive = || matches!(child.try_wait(), Ok(None));
        wait_for_cookie(port, &host, opts.timeout, &mut alive).await
    }
    .await;
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
