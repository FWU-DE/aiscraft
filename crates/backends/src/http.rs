use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::StreamExt;
use reqwest::{RequestBuilder, Response, StatusCode};

pub const USER_AGENT: &str = concat!("AIS.ChatCraft/", env!("CARGO_PKG_VERSION"));

pub fn client(follow_redirects: bool) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        // No total timeout: answers stream for minutes. Idle connections are cut by the server.
        .redirect(if follow_redirects { reqwest::redirect::Policy::limited(5) } else { reqwest::redirect::Policy::none() })
        .build()
        .map_err(|e| Error::network(format!("HTTP-Client: {e}")))
}

/// Adds the headers OpenRouter and the Vercel AI Gateway use to name AIS.ChatCraft, when `url`
/// points there ([`aiscraft_core::attribution_headers`]). Other providers get only the User-Agent.
pub fn attributed(rb: RequestBuilder, url: &str) -> RequestBuilder {
    aiscraft_core::attribution_headers(url).iter().fold(rb, |rb, (k, v)| rb.header(*k, *v))
}

pub fn net_err(e: reqwest::Error) -> Error {
    if e.is_timeout() {
        return Error::busy("Zeitüberschreitung beim Server");
    }
    // A server on this computer that refuses or drops the connection has stopped (Ollama, LM
    // Studio, the demo server): say which one and what to do.
    if let Some(url) = e.url()
        && let Some(host) = url.host_str()
        && aiscraft_core::is_loopback_host(host)
        && (e.is_connect() || aiscraft_core::is_connection_lost(&e))
    {
        return Error::local_service_gone(host.trim_start_matches('[').trim_end_matches(']'), url.port_or_known_default());
    }
    if e.is_connect() {
        Error::network(format!("Server nicht erreichbar: {}", e.url().map(|u| u.host_str().unwrap_or("").to_string()).unwrap_or_default()))
    } else {
        Error::network(e.to_string())
    }
}

/// Turn a non-success response into an error a person can act on, using the body's message.
pub async fn status_error(resp: Response, what: &str) -> Error {
    let status = resp.status();
    let body = read_capped(resp, 16 * 1024).await.unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).or_else(|| v.get("message")).and_then(|m| m.as_str().map(str::to_string)))
        .unwrap_or_else(|| body.chars().take(300).collect());
    let msg = if detail.trim().is_empty() { format!("{what}: HTTP {status}") } else { format!("{what}: {detail}") };
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::auth(msg),
        StatusCode::NOT_FOUND => Error::not_found(msg),
        // Overloaded, rate-limited, out of quota or a server error: the model's, not the content's
        // (another model may answer, #66).
        StatusCode::TOO_MANY_REQUESTS | StatusCode::PAYMENT_REQUIRED => Error::busy(msg),
        s if s.is_server_error() || s.as_u16() == 529 => Error::busy(msg),
        s if s.is_client_error() => Error::invalid(msg),
        _ => Error::protocol(msg),
    }
}

/// Read at most `cap` bytes of a body as text.
pub async fn read_capped(resp: Response, cap: usize) -> Result<String> {
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(net_err)?;
        let room = cap.saturating_sub(out.len());
        out.extend_from_slice(chunk.get(..chunk.len().min(room)).unwrap_or_default());
        if out.len() >= cap {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Read a whole body as bytes; more than `cap` bytes is an error.
pub async fn read_bytes_capped(resp: Response, cap: usize, what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(net_err)?;
        if out.len() + chunk.len() > cap {
            return Err(Error::protocol(format!("{what} ist größer als {} MB", cap / (1024 * 1024))));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Bodies we parse whole (model lists, pages) are capped so a hostile server cannot exhaust memory.
pub const PAGE_CAP: usize = 16 * 1024 * 1024;

pub async fn json(resp: Response, what: &str) -> Result<serde_json::Value> {
    let text = read_capped(resp, PAGE_CAP).await?;
    serde_json::from_str(&text).map_err(|e| Error::protocol(format!("{what}: keine gültige JSON-Antwort ({e})")))
}
