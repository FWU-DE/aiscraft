//! OAuth 2.0 sign-in for connected apps (#26): Authorization Code with PKCE (RFC 7636) and a
//! loopback redirect (RFC 8252 §7.3), the way native apps sign in to Microsoft 365 and Google.
//!
//! [`start`] binds `127.0.0.1` on a random port and builds the address the system browser opens;
//! [`Pending::finish`] waits for the browser's redirect, checks `state`, answers the browser with
//! a short page and trades the code (with the PKCE verifier) for tokens. [`refresh`] renews an
//! access token. Nothing here opens a browser: the engine does that, and tests request the
//! loopback address themselves.

use std::time::Duration;

use aiscraft_core::{Error, Result};
use base64::Engine as _;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::http::{client, net_err, read_capped, status_error};

/// The longest request line and headers read from the browser.
const MAX_REQUEST: usize = 16 * 1024;
/// The path the browser comes back to.
pub const CALLBACK_PATH: &str = "/callback";

/// Where and as whom to sign in.
#[derive(Debug, Clone)]
pub struct Client {
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    /// Google's desktop clients have a secret that is not secret; Microsoft's public clients none.
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    /// More parameters for the sign-in page (`access_type=offline`, `prompt=consent`).
    pub extra: Vec<(String, String)>,
}

/// What the token endpoint gave.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch; 0 when the server said nothing.
    #[serde(default)]
    pub expires_at: i64,
    #[serde(default)]
    pub scope: Option<String>,
}

impl Tokens {
    /// Whether the access token is (nearly) used up at `now_ms`.
    pub fn expired(&self, now_ms: i64) -> bool {
        self.expires_at > 0 && now_ms + 60_000 >= self.expires_at
    }
}

/// Percent-encoding for query strings and form bodies (RFC 3986 unreserved characters stay).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `a=1&b=2`, encoded.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

fn random(bytes: usize) -> Result<String> {
    use ring::rand::SecureRandom;
    let mut buf = vec![0u8; bytes];
    ring::rand::SystemRandom::new().fill(&mut buf).map_err(|_| Error::storage("Kein Zufall verfügbar"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf))
}

/// The S256 challenge of a PKCE verifier.
pub fn challenge(verifier: &str) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.as_ref())
}

/// A sign-in that waits for the browser.
pub struct Pending {
    listener: tokio::net::TcpListener,
    url: String,
    redirect_uri: String,
    state: String,
    verifier: String,
}

/// Bind the loopback port and build the sign-in address.
pub async fn start(c: &Client) -> Result<Pending> {
    if !(c.auth_url.starts_with("https://") || c.auth_url.starts_with("http://127.0.0.1") || c.auth_url.starts_with("http://localhost")) {
        return Err(Error::invalid("Die Anmeldeadresse muss mit https:// beginnen"));
    }
    if c.client_id.trim().is_empty() {
        return Err(Error::invalid("Client-ID fehlt"));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| Error::network(format!("Kein freier Port für die Anmeldung: {e}")))?;
    let port = listener.local_addr().map_err(|e| Error::network(e.to_string()))?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let state = random(24)?;
    let verifier = random(48)?;
    let scope = c.scopes.join(" ");
    let mut pairs: Vec<(&str, &str)> = vec![
        ("response_type", "code"),
        ("client_id", c.client_id.trim()),
        ("redirect_uri", &redirect_uri),
        ("scope", &scope),
        ("state", &state),
        ("code_challenge_method", "S256"),
    ];
    let ch = challenge(&verifier);
    pairs.push(("code_challenge", &ch));
    for (k, v) in &c.extra {
        pairs.push((k, v));
    }
    let sep = if c.auth_url.contains('?') { '&' } else { '?' };
    let url = format!("{}{sep}{}", c.auth_url, form(&pairs));
    Ok(Pending { listener, url, redirect_uri, state, verifier })
}

/// The browser's answer, read from its request line.
#[derive(Debug, PartialEq)]
enum Callback {
    Code {
        code: String,
        state: String,
    },
    Denied {
        why: String,
        state: String,
    },
    /// Something else (a favicon): answered 404, keep waiting.
    Other,
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(v) = hex {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The query pairs of `a=1&b=2`, decoded.
pub fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, ""))).map(|(k, v)| (decode(k), decode(v))).collect()
}

fn parse_request(head: &str) -> Callback {
    let target = head.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|l| l.split(' ').next()).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Callback::Other;
    }
    let pairs = query_pairs(query);
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    if let Some(e) = get("error") {
        return Callback::Denied { why: get("error_description").unwrap_or(e).chars().take(300).collect(), state: get("state").unwrap_or_default() };
    }
    match (get("code"), get("state")) {
        (Some(code), Some(state)) if !code.is_empty() => Callback::Code { code, state },
        _ => Callback::Other,
    }
}

async fn reply(sock: &mut tokio::net::TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>AIS.ChatCraft</title><body style=\"font-family:sans-serif;margin:3em\"><h1>AIS.ChatCraft</h1><p>{text}</p></body>"
    );
    let _ = sock
        .write_all(
            format!("HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len())
                .as_bytes(),
        )
        .await;
    let _ = sock.shutdown().await;
}

impl Pending {
    /// The address to open in the browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Wait up to `timeout` for the browser, then trade the code for tokens.
    pub async fn finish(self, c: &Client, timeout: Duration) -> Result<Tokens> {
        let code = tokio::time::timeout(timeout, self.wait())
            .await
            .map_err(|_| Error::busy("Die Anmeldung im Browser kam nicht rechtzeitig zurück. Bitte erneut verbinden."))??;
        exchange(c, &code, &self.verifier, &self.redirect_uri).await
    }

    /// Accept connections until one brings this sign-in's `state`. Every connection is handled on
    /// its own with one total deadline, so a slow or silent one blocks nobody. Callbacks with
    /// another (or no) state are answered and ignored, also an `error=`: anything on this computer
    /// can reach the port, and only the real browser knows the state.
    async fn wait(&self) -> Result<String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<String>>(1);
        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let Ok((sock, _)) = accepted else { continue };
                    tokio::spawn(handle_callback(sock, self.state.clone(), tx.clone()));
                }
                Some(done) = rx.recv() => return done,
            }
        }
    }
}

/// How long one connection to the loopback port may take, from accept to answer.
const CONNECTION_DEADLINE: Duration = Duration::from_secs(10);

async fn handle_callback(mut sock: tokio::net::TcpStream, expected: String, done: tokio::sync::mpsc::Sender<Result<String>>) {
    let Ok(head) = tokio::time::timeout(CONNECTION_DEADLINE, read_head(&mut sock)).await else { return };
    let outcome = match parse_request(&head) {
        Callback::Other => None,
        // Constant-time is not needed: the state is single-use and random.
        Callback::Code { code, state } if state == expected => Some(Ok(code)),
        Callback::Denied { why, state } if state == expected => Some(Err(Error::auth(format!("Anmeldung abgebrochen: {why}")))),
        Callback::Code { .. } | Callback::Denied { .. } => {
            let text = "Diese Rückmeldung passt nicht zur Anmeldung von AIS.ChatCraft und wurde ignoriert.";
            let _ = tokio::time::timeout(CONNECTION_DEADLINE, reply(&mut sock, "400 Bad Request", text)).await;
            return;
        }
    };
    let (status, text) = match &outcome {
        None => ("404 Not Found", "Nicht gefunden."),
        Some(Ok(_)) => ("200 OK", "Angemeldet. Du kannst dieses Fenster schließen und zu AIS.ChatCraft zurückkehren."),
        Some(Err(_)) => ("200 OK", "Die Anmeldung wurde abgebrochen. Du kannst dieses Fenster schließen."),
    };
    let _ = tokio::time::timeout(CONNECTION_DEADLINE, reply(&mut sock, status, text)).await;
    if let Some(o) = outcome {
        let _ = done.send(o).await;
    }
}

/// The request line and headers, at most [`MAX_REQUEST`] bytes.
async fn read_head(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = vec![0u8; MAX_REQUEST];
    let mut n = 0;
    while let Some(room) = buf.get_mut(n..) {
        if room.is_empty() {
            break;
        }
        let k = sock.read(room).await.unwrap_or(0);
        n += k;
        if k == 0 || buf.get(..n).is_some_and(|b| b.windows(4).any(|w| w == b"\r\n\r\n")) {
            break;
        }
    }
    String::from_utf8_lossy(buf.get(..n).unwrap_or_default()).into_owned()
}

impl Pending {
    #[cfg(test)]
    fn state(&self) -> &str {
        &self.state
    }
}

fn tokens_from(v: &Value, old_refresh: Option<&str>, now_ms: i64) -> Result<Tokens> {
    let access_token =
        v.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| Error::protocol("Die Anmeldung lieferte kein Zugriffstoken"))?;
    let expires_in = v.get("expires_in").and_then(|e| e.as_i64().or_else(|| e.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0).clamp(0, 400 * 24 * 3600);
    Ok(Tokens {
        access_token: access_token.chars().take(16_384).collect(),
        refresh_token: v.get("refresh_token").and_then(Value::as_str).map(|s| s.chars().take(16_384).collect()).or_else(|| old_refresh.map(str::to_string)),
        expires_at: if expires_in > 0 { now_ms + expires_in * 1000 } else { 0 },
        scope: v.get("scope").and_then(Value::as_str).map(|s| s.chars().take(4000).collect()),
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)).unwrap_or(0)
}

async fn token_request(c: &Client, mut pairs: Vec<(&str, &str)>, old_refresh: Option<&str>) -> Result<Tokens> {
    pairs.push(("client_id", c.client_id.trim()));
    if let Some(s) = c.client_secret.as_deref().filter(|s| !s.is_empty()) {
        pairs.push(("client_secret", s));
    }
    let resp = client(false)?
        .post(&c.token_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .timeout(Duration::from_secs(30))
        .body(form(&pairs))
        .send()
        .await
        .map_err(net_err)?;
    if !resp.status().is_success() {
        let e = status_error(resp, "Anmeldung").await;
        // An expired or withdrawn grant: sign in again.
        return Err(if e.message.contains("invalid_grant") { Error::auth(format!("{} – bitte neu anmelden", e.message)) } else { e });
    }
    let text = read_capped(resp, 64 * 1024).await?;
    let v: Value = serde_json::from_str(&text).map_err(|_| Error::protocol("Die Anmeldung lieferte keine gültige JSON-Antwort"))?;
    tokens_from(&v, old_refresh, now_ms())
}

/// Trade an authorization code for tokens.
pub async fn exchange(c: &Client, code: &str, verifier: &str, redirect_uri: &str) -> Result<Tokens> {
    token_request(c, vec![("grant_type", "authorization_code"), ("code", code), ("code_verifier", verifier), ("redirect_uri", redirect_uri)], None).await
}

/// A new access token from a refresh token; keeps the old refresh token when none comes back.
pub async fn refresh(c: &Client, refresh_token: &str) -> Result<Tokens> {
    let scope = c.scopes.join(" ");
    let mut pairs = vec![("grant_type", "refresh_token"), ("refresh_token", refresh_token)];
    if !scope.is_empty() {
        pairs.push(("scope", &scope));
    }
    token_request(c, pairs, Some(refresh_token)).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn encoding_and_callback_parsing() {
        assert_eq!(encode("a b/ä"), "a%20b%2F%C3%A4");
        assert_eq!(decode("a%20b+c%C3%A4%zz"), "a b cä%zz");
        assert_eq!(parse_request("GET /callback?code=x%2By&state=s1 HTTP/1.1\r\nHost: x\r\n\r\n"), Callback::Code { code: "x+y".into(), state: "s1".into() });
        assert_eq!(parse_request("GET /favicon.ico HTTP/1.1\r\n\r\n"), Callback::Other);
        assert_eq!(
            parse_request("GET /callback?error=access_denied&state=s1 HTTP/1.1\r\n\r\n"),
            Callback::Denied { why: "access_denied".into(), state: "s1".into() }
        );
        assert_eq!(parse_request("POST /callback?code=1&state=2 HTTP/1.1"), Callback::Other);
        assert_eq!(parse_request(""), Callback::Other);
        // RFC 7636 appendix B.
        assert_eq!(challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        let t = tokens_from(&serde_json::json!({"access_token": "a", "expires_in": "3600"}), Some("r"), 1000).unwrap();
        assert_eq!((t.refresh_token.as_deref(), t.expires_at), (Some("r"), 3_601_000));
        assert!(!t.expired(1000) && t.expired(3_560_000));
        assert!(tokens_from(&serde_json::json!({"error": "x"}), None, 0).is_err());
    }

    async fn request(port: u16, path: &str) -> String {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        let _ = s.read_to_string(&mut out).await;
        out
    }

    /// Anything on this computer can reach the loopback port: callbacks without this sign-in's
    /// state are ignored (also `error=`), and a connection that never sends blocks nobody.
    #[tokio::test]
    async fn only_the_right_state_ends_the_sign_in() {
        let c = Client {
            auth_url: "https://login.example/authorize".into(),
            token_url: "https://login.example/token".into(),
            client_id: "cid".into(),
            client_secret: None,
            scopes: vec![],
            extra: vec![],
        };
        let p = start(&c).await.unwrap();
        let port: u16 = p.redirect_uri().trim_start_matches("http://127.0.0.1:").trim_end_matches(CALLBACK_PATH).parse().unwrap();
        let state = encode(p.state());
        let waiting = tokio::spawn(async move { p.wait().await });
        // A silent connection stays open the whole time.
        let _silent = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        assert!(request(port, "/callback?code=FALSCH&state=fremd").await.starts_with("HTTP/1.1 400"));
        assert!(request(port, "/callback?error=access_denied").await.starts_with("HTTP/1.1 400"));
        assert!(request(port, "/callback?error=access_denied&state=fremd").await.starts_with("HTTP/1.1 400"));
        assert!(request(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(!waiting.is_finished(), "none of these ends the sign-in");
        assert!(request(port, &format!("/callback?code=RICHTIG&state={state}")).await.starts_with("HTTP/1.1 200"));
        let code = tokio::time::timeout(Duration::from_secs(5), waiting).await.unwrap().unwrap().unwrap();
        assert_eq!(code, "RICHTIG");

        // A cancel with the right state ends it as cancelled.
        let p = start(&c).await.unwrap();
        let port: u16 = p.redirect_uri().trim_start_matches("http://127.0.0.1:").trim_end_matches(CALLBACK_PATH).parse().unwrap();
        let state = encode(p.state());
        let waiting = tokio::spawn(async move { p.wait().await });
        request(port, &format!("/callback?error=access_denied&error_description=abgebrochen&state={state}")).await;
        let e = tokio::time::timeout(Duration::from_secs(5), waiting).await.unwrap().unwrap().unwrap_err();
        assert!(e.message.contains("abgebrochen"), "{}", e.message);
    }
}
