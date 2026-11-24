//! HTTP helpers, in the style of `aiscraft-backends` (copied so this crate stays small).

use std::time::Duration;

use aiscraft_core::{Error, Result};
use futures::StreamExt;
use reqwest::{RequestBuilder, Response, StatusCode};

pub const USER_AGENT: &str = concat!("AIS.ChatCraft/", env!("CARGO_PKG_VERSION"));

pub fn client(follow_redirects: bool) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
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
        Error::network("Zeitüberschreitung beim Server")
    } else if e.is_connect() {
        Error::network(format!("Server nicht erreichbar: {}", e.url().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default()))
    } else {
        Error::network(e.to_string())
    }
}

/// `https://host/v1/` → `https://host/v1`.
pub fn base(url: &str) -> Result<String> {
    let b = url.trim().trim_end_matches('/');
    if !(b.starts_with("http://") || b.starts_with("https://")) {
        return Err(Error::invalid(format!("Ungültige Server-Adresse: {url}")));
    }
    Ok(b.to_string())
}

/// Turn a non-success response into an error a person can act on, using the body's message.
pub async fn status_error(resp: Response, what: &str) -> Error {
    let status = resp.status();
    let body = read_capped(resp, 16 * 1024).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let detail = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let msg = if detail.trim().is_empty() { format!("{what}: HTTP {status}") } else { format!("{what}: {detail}") };
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::auth(msg),
        StatusCode::NOT_FOUND => Error::not_found(msg),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::PAYMENT_REQUIRED => Error::refused(msg),
        s if s.is_client_error() => Error::invalid(msg),
        _ => Error::protocol(msg),
    }
}

/// Read at most `cap` bytes of a body.
pub async fn read_capped(resp: Response, cap: usize) -> Result<Vec<u8>> {
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
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech requests (`/audio/speech`, `/audio/transcriptions`) carry the attribution headers
    /// for OpenRouter and the Vercel AI Gateway, next to the key; other providers do not.
    #[test]
    fn speech_requests_name_the_app_where_the_provider_reads_it() {
        let http = client(true).unwrap();
        for (url, expected) in [
            ("https://openrouter.ai/api/v1/audio/speech", true),
            ("https://ai-gateway.vercel.sh/v1/audio/transcriptions", true),
            ("https://api.openai.com/v1/audio/speech", false),
        ] {
            let req = attributed(http.post(url), url).bearer_auth("sk-test").build().unwrap();
            let h = req.headers();
            assert_eq!(h.get("authorization").and_then(|v| v.to_str().ok()), Some("Bearer sk-test"), "{url}");
            assert_eq!(h.get("http-referer").and_then(|v| v.to_str().ok()), expected.then_some("https://github.com/FWU-DE/aiscraft"), "{url}");
            assert_eq!(h.get("x-title").and_then(|v| v.to_str().ok()), expected.then_some("AIS.ChatCraft"), "{url}");
        }
    }
}
