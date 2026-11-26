//! Request shapes against a scripted HTTP server on 127.0.0.1.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_core::ErrorKind;
use aiscraft_voice::{collect_audio, encode_wav, speak_ais, speak_openai, transcribe_openai};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// The raw request as received (head lower-cased for easy matching, body as bytes).
struct Seen {
    head: String,
    body: Vec<u8>,
}

impl Seen {
    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Decode a chunked body (enough for tests).
fn dechunk(mut b: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = find(b, b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&b[..line_end]).ok()?.trim(), 16).ok()?;
        b = &b[line_end + 2..];
        if size == 0 {
            return Some(out);
        }
        if b.len() < size + 2 {
            return None;
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

/// Serve one request with a fixed raw response; report what was received.
async fn serve_once(response: Vec<Vec<u8>>) -> (String, mpsc::UnboundedReceiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else { return };
        let mut buf = Vec::new();
        let mut tmp = vec![0u8; 65536];
        let seen = loop {
            let n = sock.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break None;
            }
            buf.extend_from_slice(&tmp[..n]);
            let Some(h) = find(&buf, b"\r\n\r\n") else { continue };
            let head = String::from_utf8_lossy(&buf[..h]).to_ascii_lowercase();
            let rest = &buf[h + 4..];
            if head.contains("transfer-encoding: chunked") {
                if let Some(body) = dechunk(rest) {
                    break Some(Seen { head, body });
                }
                continue;
            }
            let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
            if rest.len() >= len {
                break Some(Seen { head, body: rest[..len].to_vec() });
            }
        };
        if let Some(s) = seen {
            let _ = tx.send(s);
        }
        for part in response {
            let _ = sock.write_all(&part).await;
            let _ = sock.flush().await;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let _ = sock.shutdown().await;
    });
    (format!("http://{addr}"), rx)
}

fn fixed(status: &str, ctype: &str, extra: &str, body: &[u8]) -> Vec<Vec<u8>> {
    let mut r = format!("HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n", body.len()).into_bytes();
    r.extend_from_slice(body);
    vec![r]
}

/// A chunked response sent in separate writes, like a streaming TTS server.
fn chunked(ctype: &str, parts: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut out = vec![format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n").into_bytes()];
    for p in parts {
        let mut c = format!("{:x}\r\n", p.len()).into_bytes();
        c.extend_from_slice(p);
        c.extend_from_slice(b"\r\n");
        out.push(c);
    }
    out.push(b"0\r\n\r\n".to_vec());
    out
}

#[tokio::test]
async fn transcribe_sends_multipart() {
    let (base, mut rx) = serve_once(fixed("200 OK", "application/json", "", br#"{"text":" Hallo Welt. "}"#)).await;
    let wav = encode_wav(&[0, 100, -100, 0], 16_000, 1).unwrap();
    let text = transcribe_openai(&format!("{base}/v1/"), Some(" sk-test "), "whisper-1", wav.clone(), Some("de")).await.unwrap();
    assert_eq!(text, "Hallo Welt.");
    let seen = rx.recv().await.unwrap();
    assert!(seen.head.starts_with("post /v1/audio/transcriptions http/1.1"), "{}", seen.head);
    assert!(seen.head.contains("authorization: bearer sk-test"));
    assert!(seen.head.contains("content-type: multipart/form-data; boundary="));
    let body = seen.body_text();
    assert!(body.contains("name=\"file\"; filename=\"aufnahme.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF"), "{body}");
    assert!(find(&seen.body, &wav).is_some(), "WAV bytes sent unchanged");
    assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1\r\n"));
    assert!(body.contains("name=\"language\"\r\n\r\nde\r\n"));
    assert!(body.contains("name=\"response_format\"\r\n\r\njson\r\n"));
}

#[tokio::test]
async fn transcribe_without_key_or_language() {
    let (base, mut rx) = serve_once(fixed("200 OK", "text/plain", "", b"nur Text\n")).await;
    let text = transcribe_openai(&base, None, "Systran/faster-whisper-small", encode_wav(&[1], 16_000, 1).unwrap(), None).await.unwrap();
    assert_eq!(text, "nur Text");
    let seen = rx.recv().await.unwrap();
    assert!(seen.head.starts_with("post /audio/transcriptions "));
    assert!(!seen.head.contains("authorization:"));
    assert!(!seen.body_text().contains("name=\"language\""));
}

#[tokio::test]
async fn transcribe_errors() {
    let (base, _rx) = serve_once(fixed("401 Unauthorized", "application/json", "", br#"{"error":{"message":"Incorrect API key"}}"#)).await;
    let e = transcribe_openai(&base, Some("bad"), "whisper-1", encode_wav(&[1], 16_000, 1).unwrap(), None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Auth);
    assert!(e.message.contains("Incorrect API key"), "{}", e.message);

    let e = transcribe_openai("http://127.0.0.1:9", None, "whisper-1", encode_wav(&[1], 16_000, 1).unwrap(), None).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Network);
    assert_eq!(transcribe_openai("ftp://x", None, "m", vec![1], None).await.unwrap_err().kind, ErrorKind::Invalid);
    assert_eq!(transcribe_openai("http://x", None, "m", Vec::new(), None).await.unwrap_err().kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn openai_speech_streams_mp3() {
    let (base, mut rx) = serve_once(chunked("audio/mpeg", &[b"ID3\x04", b"\xff\xfbframe1", b"frame2"])).await;
    let stream = speak_openai(&format!("{base}/v1"), Some("sk"), "gpt-4o-mini-tts", "alloy", " Hallo. ").await.unwrap();
    let audio = collect_audio(stream).await.unwrap();
    assert_eq!(audio, b"ID3\x04\xff\xfbframe1frame2");
    let seen = rx.recv().await.unwrap();
    assert!(seen.head.starts_with("post /v1/audio/speech "));
    assert!(seen.head.contains("authorization: bearer sk"));
    assert!(seen.head.contains("content-type: application/json"));
    let v: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
    assert_eq!(v, serde_json::json!({"model": "gpt-4o-mini-tts", "voice": "alloy", "input": "Hallo.", "response_format": "mp3"}));
}

#[tokio::test]
async fn openai_speech_json_instead_of_audio_is_err() {
    let (base, _rx) = serve_once(fixed("200 OK", "application/json", "", br#"{"error":{"message":"voice not found"}}"#)).await;
    let e = speak_openai(&base, None, "tts-1", "nobody", "Hallo").await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Protocol);
    assert!(e.message.contains("voice not found"), "{}", e.message);

    let (base, _rx) = serve_once(fixed("429 Too Many Requests", "application/json", "", br#"{"error":{"message":"quota"}}"#)).await;
    let e = speak_openai(&base, None, "tts-1", "alloy", "Hallo").await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Busy, "a rate limit: wait and retry");
    assert!(e.message.contains("quota") && e.message.contains("warten"), "{}", e.message);
}

#[tokio::test]
async fn ais_speech_sends_cookie_and_json() {
    let (base, mut rx) = serve_once(chunked("audio/mpeg", &[b"\xff\xf3aaa", b"bbb"])).await;
    let stream = speak_ais(&base, "tok123", "Guten Tag.", Some("nova"), Some("conv-1")).await.unwrap();
    assert_eq!(collect_audio(stream).await.unwrap(), b"\xff\xf3aaabbb");
    let seen = rx.recv().await.unwrap();
    assert!(seen.head.starts_with("post /api/v1/speech/stream "), "{}", seen.head);
    assert!(seen.head.contains("cookie: __secure-ais-chat-app.authjs.session-token=tok123"));
    assert!(seen.head.contains("content-type: application/json"));
    let v: serde_json::Value = serde_json::from_slice(&seen.body).unwrap();
    assert_eq!(v, serde_json::json!({"text": "Guten Tag.", "voice": "nova", "conversationId": "conv-1"}));
}

#[tokio::test]
async fn ais_speech_optional_fields_are_left_out() {
    let (base, mut rx) = serve_once(fixed("200 OK", "audio/mpeg", "", b"\xff\xf3x")).await;
    let cookie = "Cookie: __Secure-ais-chat-app.authjs.session-token.0=a; __Secure-ais-chat-app.authjs.session-token.1=b";
    collect_audio(speak_ais(&base, cookie, "Hallo", None, None).await.unwrap()).await.unwrap();
    let seen = rx.recv().await.unwrap();
    assert!(seen.head.contains("cookie: __secure-ais-chat-app.authjs.session-token.0=a; __secure-ais-chat-app.authjs.session-token.1=b"));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&seen.body).unwrap(), serde_json::json!({"text": "Hallo"}));
}

#[tokio::test]
async fn ais_redirect_to_login_is_auth() {
    let (base, _rx) = serve_once(fixed("307 Temporary Redirect", "text/plain", "location: /login?callbackUrl=%2F\r\n", b"")).await;
    let e = speak_ais(&base, "expired", "Hallo", None, None).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Auth);

    let (base, _rx) = serve_once(fixed("307 Temporary Redirect", "text/plain", "location: /terms\r\n", b"")).await;
    assert_eq!(speak_ais(&base, "tok", "Hallo", None, None).await.err().unwrap().kind, ErrorKind::Protocol);

    assert_eq!(speak_ais("http://127.0.0.1:9", "  ", "Hallo", None, None).await.err().unwrap().kind, ErrorKind::Auth);
    assert_eq!(speak_ais("http://127.0.0.1:9", "tok", "   ", None, None).await.err().unwrap().kind, ErrorKind::Invalid);
}

#[tokio::test]
async fn ais_html_page_is_err() {
    let (base, _rx) = serve_once(fixed("200 OK", "text/html; charset=utf-8", "", b"<html>Anmelden</html>")).await;
    assert_eq!(speak_ais(&base, "tok", "Hallo", None, None).await.err().unwrap().kind, ErrorKind::Protocol);
}
