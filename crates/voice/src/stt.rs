//! Speech to text over the OpenAI-compatible `/audio/transcriptions` route (OpenAI, speaches,
//! faster-whisper-server, LocalAI, …).

use std::time::Duration;

use aiscraft_core::{Error, Result};
use reqwest::multipart::{Form, Part};
use serde_json::Value;

use crate::http::{base, client, net_err, read_capped, status_error};

/// OpenAI's upload limit; local servers accept the same.
pub const MAX_UPLOAD: usize = 25 * 1024 * 1024;
const ANSWER_CAP: usize = 4 * 1024 * 1024;

/// Transcribe a WAV recording. `base_url` is the API root including `/v1`
/// (e.g. `https://api.openai.com/v1`); `language` is an ISO 639-1 code like `de`.
pub async fn transcribe_openai(base_url: &str, api_key: Option<&str>, model: &str, wav: Vec<u8>, language: Option<&str>) -> Result<String> {
    let base = base(base_url)?;
    if wav.is_empty() {
        return Err(Error::invalid("Die Aufnahme ist leer."));
    }
    if wav.len() > MAX_UPLOAD {
        return Err(Error::invalid("Die Aufnahme ist zu lang für die Spracherkennung (höchstens 25 MB)."));
    }
    if model.trim().is_empty() {
        return Err(Error::invalid("Kein Modell für die Spracherkennung angegeben."));
    }
    let file = Part::bytes(wav).file_name("aufnahme.wav").mime_str("audio/wav").map_err(|e| Error::invalid(format!("Aufnahme: {e}")))?;
    let mut form = Form::new().part("file", file).text("model", model.trim().to_string()).text("response_format", "json");
    if let Some(lang) = language.map(str::trim).filter(|l| !l.is_empty()) {
        form = form.text("language", lang.to_string());
    }
    let mut rb = client(true)?.post(format!("{base}/audio/transcriptions")).timeout(Duration::from_secs(300)).multipart(form);
    if let Some(k) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
        rb = rb.bearer_auth(k);
    }
    let resp = rb.send().await.map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(status_error(resp, "Spracherkennung").await);
    }
    let body = read_capped(resp, ANSWER_CAP).await?;
    parse_transcript(&String::from_utf8_lossy(&body))
}

/// `{"text": "..."}`, or plain text from servers that ignore `response_format`.
pub fn parse_transcript(body: &str) -> Result<String> {
    let trimmed = body.trim();
    if trimmed.starts_with('{') {
        let v: Value = serde_json::from_str(trimmed).map_err(|e| Error::protocol(format!("Spracherkennung: Antwort nicht lesbar ({e})")))?;
        if let Some(err) = v.get("error") {
            let msg = err.get("message").and_then(Value::as_str).or_else(|| err.as_str()).unwrap_or("Fehler des Servers");
            return Err(Error::refused(format!("Spracherkennung: {msg}")));
        }
        return v.get("text").and_then(Value::as_str).map(|t| t.trim().to_string()).ok_or_else(|| Error::protocol("Spracherkennung: Antwort ohne Text"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_answers() {
        assert_eq!(parse_transcript(r#"{"text":" Hallo Welt. "}"#).unwrap(), "Hallo Welt.");
        assert_eq!(parse_transcript("Hallo\n").unwrap(), "Hallo");
        assert!(parse_transcript(r#"{"error":{"message":"quota"}}"#).is_err());
        assert!(parse_transcript(r#"{"foo":1}"#).is_err());
        assert!(parse_transcript("{kaputt").is_err());
    }
}
