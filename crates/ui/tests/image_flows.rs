//! Generated images: „KI-Kennzeichnung“ under the picture, on by default as in AIS.chat; the
//! switch decides what „Bild speichern“ writes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::theme::OmarchyTheme;
use egui_kittest::kittest::Queryable;
use serde_json::json;

fn picture() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(600, 400, image::Rgb([90, 160, 230]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A scripted AIS.chat that draws one picture per prompt.
fn scripted_ais(engine: &Engine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let png = std::sync::Arc::new(picture());
    engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let png = png.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let s = String::from_utf8_lossy(&buf);
                        if let Some(end) = s.find("\r\n\r\n") {
                            let len = s[..end]
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    if req.starts_with("GET /files/demo.png") {
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", png.len());
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(&png).await;
                        return;
                    }
                    let id = |n: u8| format!("{n:02x}").repeat(21);
                    let page = |chunk: &str, row: &str| {
                        format!(
                            r#"<script src="/_next/static/chunks/{chunk}.js"></script><script>self.__next_f.push([1,{}])</script>"#,
                            serde_json::to_string(&format!("{row}\n")).unwrap()
                        )
                    };
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    let models = r#"3:{"usedBudget":1,"maxBudget":1000,"models":[{"id":"m-gpt","name":"gpt","displayName":"GPT-5","priceMetadata":{"type":"text"}},{"id":"m-img","name":"img","displayName":"Bildmodell","priceMetadata":{"type":"image"}}]}"#;
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin"},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("application/json", r#"{"conversations":[]}"#.to_string())
                    } else if req.starts_with("GET /image-generation ") {
                        ("text/html", page("images", models))
                    } else if req.starts_with("GET /_next/static/chunks/images.js") {
                        ("application/javascript", reference(0x44, "generateImageAction"))
                    } else if req.starts_with("POST / ") && action == id(0x44) {
                        (
                            "text/x-component",
                            flight(r#"{"success":true,"value":{"imageUrl":"/files/demo.png","conversationId":"srv-img","fileId":"file_1","assistantMessageId":"am1","userMessageId":"um1"}}"#),
                        )
                    } else if req.starts_with("GET /assistants ") || req.starts_with("GET /characters ") || req.starts_with("GET /learning-scenarios ") {
                        ("text/html", page("none", "1:{}"))
                    } else if req.starts_with("GET / ") {
                        ("text/html", page("app", models))
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        ("application/javascript", reference(0x7f, "sendChatMessageAction"))
                    } else {
                        ("text/plain", String::new())
                    };
                    let head = format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len());
                    let _ = sock.write_all(head.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    })
}

/// The pixel just inside the top of the badge in a saved picture.
fn badge_pixel(path: &std::path::Path) -> [u8; 3] {
    let img = image::open(path).unwrap().to_rgb8();
    let (cx, cy, r) = aiscraft_engine::ai_badge::geometry(img.width(), img.height());
    img.get_pixel(cx as u32, (cy - r * 0.85) as u32).0
}

#[test]
fn ai_label_switch_under_a_generated_image_reaches_the_save() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let base = scripted_ais(&engine);
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).unwrap();
    engine.set_picture_origin(Some(&base)).unwrap();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "demo"})).unwrap();
    engine.call("chat.new_image", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Ein Fuchs im Schnee"})).unwrap();
    let answer = engine.state().messages.last().cloned().unwrap();
    assert_eq!(answer.attachments.len(), 1, "{answer:?}");

    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, None, OmarchyTheme::none()));
    h.run_steps(4);
    for close in ["Später", "Fertig"] {
        if let Some(b) = h.query_by_label(close) {
            b.click();
            h.run_steps(4);
        }
    }
    common::step_until(&mut h, "the switch under the picture", |h| h.query_by_label("KI-Kennzeichnung").is_some());
    assert!(h.query_by_label("Prompt kopieren").is_some() && h.query_by_label("Bild kopieren").is_some());
    assert!(common::chosen(&h, "KI-Kennzeichnung"), "on by default");
    let dir = std::env::temp_dir().join(format!("aiscraft-ki-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bild.png");
    let params = h.state().image_save_params(&answer.id, 0, &path.display().to_string());
    assert_eq!(params["badge"], true);
    engine.call("chat.save_image", params).unwrap();
    assert_eq!(badge_pixel(&path), [0, 0, 0], "saved with the badge");

    common::click_until(&mut h, "KI-Kennzeichnung", "the switch is off", |h| !common::chosen(h, "KI-Kennzeichnung"));
    let params = h.state().image_save_params(&answer.id, 0, &path.display().to_string());
    assert_eq!(params["badge"], false);
    engine.call("chat.save_image", params).unwrap();
    assert_eq!(badge_pixel(&path), [90, 160, 230], "the original");
    let _ = std::fs::remove_dir_all(&dir);
}
