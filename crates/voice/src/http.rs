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
        return Error::network("Zeitüberschreitung beim Server");
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
    let retry_after = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
    let body = read_capped(resp, 16 * 1024).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    status_error_from(status, retry_after, &body, what)
}

/// [`status_error`] without the network: the service's own reason always stays in the message.
/// A rate limit (429) is a „wait and try again“ (`Busy`), never a missing credit; missing credit
/// (402, or OpenAI's 429 with `insufficient_quota`) says so, for AIS.chat as its token points
/// (Nod #1334).
pub fn status_error_from(status: StatusCode, retry_after: Option<u64>, body: &str, what: &str) -> Error {
    let json = serde_json::from_str::<serde_json::Value>(body).ok();
    let detail = json
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("error"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("detail"))
                .and_then(|m| m.as_str().map(str::to_string))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    let detail: String = detail.trim().chars().take(300).collect();
    let code = json.as_ref().and_then(|v| v.pointer("/error/code").or_else(|| v.pointer("/error/type")).or_else(|| v.get("code"))).and_then(|c| c.as_str());
    let reason = |text: String| if detail.is_empty() { format!("{text} (HTTP {status})") } else { format!("{text} Der Dienst meldet: {detail}") };
    let no_credit = status == StatusCode::PAYMENT_REQUIRED
        || (status == StatusCode::TOO_MANY_REQUESTS && code.is_some_and(|c| c == "insufficient_quota" || c == "TokenPointsExceededError"));
    if no_credit {
        let credit = if what.contains("AIS.chat") {
            "Die Token-Punkte bei AIS.chat sind aufgebraucht."
        } else {
            "Das Guthaben oder Kontingent beim Dienst ist aufgebraucht."
        };
        return Error::refused(reason(format!("{what}: {credit}")));
    }
    match status {
        StatusCode::TOO_MANY_REQUESTS => {
            let wait = match retry_after {
                Some(s) if s > 0 && s <= 3600 => format!("Bitte etwa {s} s warten und es dann noch einmal versuchen."),
                _ => "Bitte kurz warten und es dann noch einmal versuchen.".into(),
            };
            Error::busy(reason(format!("{what}: Zu viele Anfragen in kurzer Zeit. {wait}")))
        }
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Error::auth(reason_or_status(what, &detail, status)),
        StatusCode::NOT_FOUND => Error::not_found(reason_or_status(what, &detail, status)),
        s if s.is_client_error() => Error::invalid(reason_or_status(what, &detail, status)),
        _ => Error::protocol(reason_or_status(what, &detail, status)),
    }
}

fn reason_or_status(what: &str, detail: &str, status: StatusCode) -> String {
    if detail.is_empty() { format!("{what}: HTTP {status}") } else { format!("{what}: {detail}") }
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

    /// Nod #1334: a rate limit is „wait and retry“, missing credit says so; both keep the reason.
    #[test]
    fn a_rate_limit_is_not_missing_credit() {
        use aiscraft_core::ErrorKind;
        let limit = status_error_from(
            StatusCode::TOO_MANY_REQUESTS,
            Some(20),
            r#"{"error":{"message":"Rate limit reached for tts-1","code":"rate_limit_exceeded"}}"#,
            "Sprachausgabe",
        );
        assert_eq!(limit.kind, ErrorKind::Busy);
        assert!(limit.message.contains("20 s warten") && limit.message.contains("Rate limit reached for tts-1"), "{}", limit.message);
        assert!(!limit.message.contains("Guthaben") && !limit.message.contains("Token-Punkte") && !limit.message.contains("Kontingent"), "{}", limit.message);
        let bare = status_error_from(StatusCode::TOO_MANY_REQUESTS, None, "", "Spracherkennung");
        assert_eq!(bare.kind, ErrorKind::Busy);
        assert!(bare.message.contains("kurz warten") && bare.message.contains("429"), "{}", bare.message);

        let paid = status_error_from(StatusCode::PAYMENT_REQUIRED, None, r#"{"error":"Insufficient credits: add more at billing"}"#, "Sprachausgabe");
        assert_eq!(paid.kind, ErrorKind::Refused);
        assert!(paid.message.contains("Guthaben") && paid.message.contains("Insufficient credits: add more at billing"), "{}", paid.message);
        let quota = status_error_from(
            StatusCode::TOO_MANY_REQUESTS,
            None,
            r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota"}}"#,
            "Sprachausgabe",
        );
        assert_eq!(quota.kind, ErrorKind::Refused, "OpenAI's quota error comes as 429 but is missing credit");
        assert!(quota.message.contains("You exceeded your current quota"));
        let ais = status_error_from(StatusCode::PAYMENT_REQUIRED, None, r#"{"message":"Budget erschöpft"}"#, "AIS.chat-Sprachausgabe");
        assert!(ais.message.contains("Token-Punkte") && ais.message.contains("Budget erschöpft"), "{}", ais.message);
        let ais_limit = status_error_from(StatusCode::TOO_MANY_REQUESTS, None, r#"{"message":"Bitte langsamer"}"#, "AIS.chat-Sprachausgabe");
        assert!(ais_limit.kind == ErrorKind::Busy && !ais_limit.message.contains("Token-Punkte") && ais_limit.message.contains("Bitte langsamer"));
        // Others as before.
        assert_eq!(status_error_from(StatusCode::UNAUTHORIZED, None, r#"{"error":{"message":"bad key"}}"#, "Sprachausgabe").message, "Sprachausgabe: bad key");
        assert_eq!(status_error_from(StatusCode::BAD_GATEWAY, None, "", "Sprachausgabe").kind, ErrorKind::Protocol);
    }

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
