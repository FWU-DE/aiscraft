//! A scripted Google for tests (also for the engine, with the feature `test-support`): token
//! endpoint, model catalog, token counts, Claude's rawPredict and the OpenAI-compatible endpoint.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A throwaway RSA key made for these tests only (PEM body; the armour lines are added here).
pub fn test_pem(pkcs1: bool) -> String {
    let (body, kind) = if pkcs1 {
        (include_str!("../tests-data/vertex-test-key-pkcs1.b64"), "RSA PRIVATE KEY")
    } else {
        (include_str!("../tests-data/vertex-test-key.b64"), "PRIVATE KEY")
    };
    format!("-----BEGIN {kind}-----\n{}\n-----END {kind}-----\n", body.trim())
}

pub fn key_json(token_uri: &str, pkcs1: bool) -> String {
    let mut v = json!({
        "project_id": "schule-test",
        "private_key_id": "kid1",
        "private_key": test_pem(pkcs1),
        "client_email": "chatcraft@schule-test.iam.gserviceaccount.com",
        "token_uri": token_uri,
    });
    v["type"] = json!("service_account");
    v.to_string()
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = sock.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf).to_string();
        if let Some(h) = text.find("\r\n\r\n") {
            let len = text[..h]
                .lines()
                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                .unwrap_or(0);
            if buf.len() >= h + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// A scripted Google: token endpoint, catalog, token counts, Claude's rawPredict and the
/// OpenAI-compatible endpoint. Returns its address and the requests it saw.
pub async fn fake_google() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let public_key =
        crate::vertex::signer(&crate::vertex::parse_service_account(&key_json(&format!("{addr}/token"), false)).unwrap()).unwrap().public().as_ref().to_vec();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let req = read_request(&mut sock).await;
            seen2.lock().unwrap().push(req.clone());
            let line = req.lines().next().unwrap_or_default().to_string();
            let body = req.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
            let authed = req.to_ascii_lowercase().contains("authorization: bearer ya29.test");
            let (status, ctype, out) = if line.starts_with("POST /token") {
                // The assertion is signed with the key and names the account and scope.
                let jwt = body.split("assertion=").nth(1).unwrap_or_default();
                let parts: Vec<&str> = jwt.split('.').collect();
                let ok = parts.len() == 3 && {
                    let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[2]).unwrap_or_default();
                    let input = format!("{}.{}", parts[0], parts[1]);
                    let claims: Value =
                        serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).unwrap_or_default()).unwrap_or_default();
                    ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PKCS1_2048_8192_SHA256, &public_key).verify(input.as_bytes(), &sig).is_ok()
                        && claims["iss"] == "chatcraft@schule-test.iam.gserviceaccount.com"
                        && claims["scope"] == crate::vertex::SCOPE
                        && body.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer")
                };
                if ok {
                    (200, "application/json", json!({"access_token": "ya29.test", "expires_in": 3599, "token_type": "Bearer"}).to_string())
                } else {
                    (400, "application/json", json!({"error": "invalid_grant", "error_description": "Invalid JWT Signature."}).to_string())
                }
            } else if !authed {
                (401, "application/json", json!({"error": {"message": "no token"}}).to_string())
            } else if line.starts_with("GET /v1beta1/publishers/anthropic/models") {
                (200, "application/json", json!({"publisherModels": [{"name": "publishers/anthropic/models/claude-mythos-5"}]}).to_string())
            } else if line.starts_with("GET /v1beta1/publishers/google/models") {
                (500, "application/json", "{}".to_string())
            } else if line.contains("/publishers/anthropic/models/count-tokens:rawPredict") {
                if body.contains("\"claude-opus-5-5\"") {
                    (200, "application/json", json!({"input_tokens": 3}).to_string())
                } else if body.contains("\"claude-sonnet-5-5\"") {
                    (403, "application/json", json!({"error": {"message": "Permission denied"}}).to_string())
                } else {
                    (404, "application/json", json!({"error": {"message": "not found"}}).to_string())
                }
            } else if line.contains("/publishers/google/models/gemini-2.5-flash:countTokens") {
                (200, "application/json", json!({"totalTokens": 2}).to_string())
            } else if line.contains("countTokens") {
                (404, "application/json", json!({"error": {"message": "not found"}}).to_string())
            } else if line.starts_with("POST /v1/projects/schule-test/locations/global/publishers/anthropic/models/claude-opus-5-5:streamRawPredict") {
                let events = [
                    json!({"type": "message_start", "message": {"usage": {"input_tokens": 4}}}),
                    json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
                    json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hallo von Claude"}}),
                    json!({"type": "content_block_stop", "index": 0}),
                    json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 3}}),
                    json!({"type": "message_stop"}),
                ];
                (200, "text/event-stream", events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap())).collect())
            } else if line.starts_with("POST /v1/projects/schule-test/locations/global/endpoints/openapi/chat/completions") {
                let chunk = json!({"choices": [{"index": 0, "delta": {"content": "Hallo von Gemini"}}]});
                (200, "text/event-stream", format!("data: {chunk}\n\ndata: [DONE]\n\n"))
            } else {
                (404, "application/json", json!({"error": {"message": "unbekannt"}}).to_string())
            };
            let head = format!("HTTP/1.1 {status} X\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", out.len());
            let _ = sock.write_all(format!("{head}{out}").as_bytes()).await;
        }
    });
    (addr, seen)
}

pub fn fake_key(addr: &str) -> String {
    key_json(&format!("{addr}/token"), false)
}
