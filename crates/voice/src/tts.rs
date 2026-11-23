//! Text to speech: the OpenAI-compatible `/audio/speech` route and AIS.chat's
//! `/api/v1/speech/stream` (signed-in web session, chunked MP3).

use std::time::Duration;

use aiscraft_core::{Error, Result};
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use reqwest::header::{CONTENT_TYPE, LOCATION};
use reqwest::{RequestBuilder, Response};
use serde_json::{Value, json};

use crate::http::{base, client, net_err, status_error};

/// Audio chunks as they arrive from the server.
pub type AudioStream = BoxStream<'static, Result<Bytes>>;

/// A single speech answer is cut off beyond this size.
pub const AUDIO_CAP: usize = 32 * 1024 * 1024;
/// OpenAI's input limit per request; `speakable_chunks` stays well below it.
pub const MAX_INPUT_CHARS: usize = 4096;

/// The AIS.chat session cookie name (as in `aiscraft-backends`).
pub const SESSION_COOKIE: &str = "__Secure-ais-chat-app.authjs.session-token";

/// Read a whole audio stream into memory (capped at [`AUDIO_CAP`]).
pub async fn collect_audio(mut stream: AudioStream) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk?);
    }
    if out.is_empty() {
        return Err(Error::protocol("Die Sprachausgabe hat keine Audiodaten geliefert."));
    }
    Ok(out)
}

fn check_text(text: &str) -> Result<&str> {
    let t = text.trim();
    if t.is_empty() {
        return Err(Error::invalid("Kein Text zum Vorlesen."));
    }
    if t.chars().count() > MAX_INPUT_CHARS {
        return Err(Error::invalid(format!("Text zu lang zum Vorlesen am Stück (höchstens {MAX_INPUT_CHARS} Zeichen). Bitte in Abschnitte teilen.")));
    }
    Ok(t)
}

/// Turn the body into a capped stream, after refusing answers that are not audio.
async fn audio_stream(resp: Response, what: &'static str) -> Result<AudioStream> {
    let ctype = resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    if ctype.starts_with("application/json") || ctype.starts_with("text/") {
        let body = crate::http::read_capped(resp, 16 * 1024).await.unwrap_or_default();
        let text = String::from_utf8_lossy(&body);
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.pointer("/error/message").or_else(|| v.get("error")).or_else(|| v.get("message")).and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(Error::protocol(format!("{what}: keine Audiodaten erhalten ({ctype}): {detail}")));
    }
    let mut seen = 0usize;
    let s = resp.bytes_stream().map_err(net_err).and_then(move |chunk| {
        seen = seen.saturating_add(chunk.len());
        let r = if seen > AUDIO_CAP { Err(Error::refused(format!("{what}: Audioantwort zu groß"))) } else { Ok(chunk) };
        futures::future::ready(r)
    });
    // Stop after the first error.
    let s = s.scan(false, |failed, item| {
        if *failed {
            return futures::future::ready(None);
        }
        *failed = item.is_err();
        futures::future::ready(Some(item))
    });
    Ok(s.boxed())
}

/// The OpenAI-compatible speech request body.
pub fn openai_speech_body(model: &str, voice: &str, text: &str) -> Value {
    json!({"model": model, "voice": voice, "input": text, "response_format": "mp3"})
}

/// `POST {base}/audio/speech`, MP3. `base_url` includes `/v1`. Voices on OpenAI: alloy, ash,
/// coral, echo, fable, nova, onyx, sage, shimmer, …
pub async fn speak_openai(base_url: &str, api_key: Option<&str>, model: &str, voice: &str, text: &str) -> Result<AudioStream> {
    let base = base(base_url)?;
    let text = check_text(text)?;
    if model.trim().is_empty() || voice.trim().is_empty() {
        return Err(Error::invalid("Modell und Stimme für die Sprachausgabe angeben."));
    }
    let mut rb = client(true)?.post(format!("{base}/audio/speech")).json(&openai_speech_body(model.trim(), voice.trim(), text)).header("Accept", "audio/mpeg");
    if let Some(k) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        rb = rb.bearer_auth(k);
    }
    let resp = send(rb).await?;
    if !resp.status().is_success() {
        return Err(status_error(resp, "Sprachausgabe").await);
    }
    audio_stream(resp, "Sprachausgabe").await
}

async fn send(rb: RequestBuilder) -> Result<Response> {
    // No overall timeout on the stream itself, but the server must start answering.
    tokio::time::timeout(Duration::from_secs(120), rb.send()).await.map_err(|_| Error::network("Zeitüberschreitung beim Server"))?.map_err(net_err)
}

/// Accept a bare token, `name=value` pairs or a whole `Cookie:` header.
pub fn cookie_header(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.strip_prefix("Cookie:").or_else(|| s.strip_prefix("cookie:")).unwrap_or(s).trim();
    if s.is_empty() || s.contains(['\r', '\n']) {
        return None;
    }
    if !s.contains('=') {
        return (!s.contains(char::is_whitespace)).then(|| format!("{SESSION_COOKIE}={s}"));
    }
    Some(s.to_string())
}

/// The AIS.chat speech request body.
pub fn ais_speech_body(text: &str, voice: Option<&str>, conversation_id: Option<&str>) -> Value {
    let mut b = json!({"text": text});
    if let Some(o) = b.as_object_mut() {
        if let Some(v) = voice.map(str::trim).filter(|v| !v.is_empty()) {
            o.insert("voice".into(), json!(v));
        }
        if let Some(c) = conversation_id.map(str::trim).filter(|c| !c.is_empty()) {
            o.insert("conversationId".into(), json!(c));
        }
    }
    b
}

/// AIS.chat read-aloud: `POST {base}/api/v1/speech/stream` with the web session cookie. The
/// voice must be one configured for the instance's speech model; unknown voices fall back to
/// the default there. `conversation_id` attributes the cost to a chat.
pub async fn speak_ais(base_url: &str, cookie: &str, text: &str, voice: Option<&str>, conversation_id: Option<&str>) -> Result<AudioStream> {
    let base = base(base_url)?;
    let text = check_text(text)?;
    let cookie = cookie_header(cookie).ok_or_else(|| Error::auth("Nicht bei AIS.chat angemeldet: Sitzungs-Cookie fehlt"))?;
    let rb = client(false)?
        .post(format!("{base}/api/v1/speech/stream"))
        .header("Cookie", cookie)
        .header("Accept", "audio/mpeg")
        .header("Origin", base.clone())
        .header("Referer", format!("{base}/"))
        .json(&ais_speech_body(text, voice, conversation_id));
    let resp = send(rb).await?;
    let status = resp.status();
    if status.is_redirection() {
        let loc = resp.headers().get(LOCATION).and_then(|l| l.to_str().ok()).unwrap_or("").to_string();
        if loc.contains("/login") || loc.contains("/api/auth") {
            return Err(Error::auth("AIS.chat-Sitzung abgelaufen. Bitte neu anmelden."));
        }
        return Err(Error::protocol(format!("AIS.chat leitet weiter nach {loc}. Bitte einmal im Browser öffnen (z. B. Nutzungsbedingungen bestätigen).")));
    }
    if !status.is_success() {
        return Err(status_error(resp, "AIS.chat-Sprachausgabe").await);
    }
    audio_stream(resp, "AIS.chat-Sprachausgabe").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_forms() {
        assert_eq!(cookie_header("abc").as_deref(), Some("__Secure-ais-chat-app.authjs.session-token=abc"));
        assert_eq!(cookie_header("Cookie: a=b; c=d").as_deref(), Some("a=b; c=d"));
        assert_eq!(cookie_header("  "), None);
        assert_eq!(cookie_header("a b"), None);
        assert_eq!(cookie_header("a=b\r\nX: y"), None);
    }

    #[test]
    fn bodies() {
        assert_eq!(ais_speech_body("Hallo", None, Some(" ")), json!({"text": "Hallo"}));
        assert_eq!(ais_speech_body("Hallo", Some("nova"), Some("c1")), json!({"text": "Hallo", "voice": "nova", "conversationId": "c1"}));
        assert_eq!(openai_speech_body("tts-1", "alloy", "Hi"), json!({"model": "tts-1", "voice": "alloy", "input": "Hi", "response_format": "mp3"}));
    }

    #[test]
    fn text_limits() {
        assert!(check_text("  ").is_err());
        assert!(check_text(&"a".repeat(MAX_INPUT_CHARS + 1)).is_err());
        assert_eq!(check_text(" ok ").unwrap(), "ok");
    }
}
