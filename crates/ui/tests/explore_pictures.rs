//! Quality gate: the "Entdecken" page asks only for the pictures of the cards in view (and a row
//! ahead), more while scrolling, and never one twice. Counts requests at a scripted AIS.chat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::sync::{Arc, Mutex};

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::AisCraftApp;
use aiscraft_ui::theme::OmarchyTheme;
use egui_kittest::kittest::Queryable;
use serde_json::json;

const COUNT: usize = 60;

fn avatar() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([40, 120, 200]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A scripted AIS.chat with `COUNT` assistants, each with a picture; returns the base URL and
/// the assistant numbers whose picture was asked for, in order.
fn scripted_ais(engine: &Engine) -> (String, Arc<Mutex<Vec<usize>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let asked = Arc::new(Mutex::new(Vec::new()));
    let rec = asked.clone();
    let assistants: Vec<_> = (1..=COUNT)
        .map(|n| {
            json!({"id": format!("a{n}"), "name": name(n), "description": "Hilft beim Unterricht.", "userId": "x", "isFavorite": false,
                   "maybeSignedPictureUrl": format!("https://bilder.example/avatars/{n}.png?X-Amz-Signature=t")})
        })
        .collect();
    let assistants = Arc::new(serde_json::Value::from(assistants).to_string());
    let png = Arc::new(avatar());
    let base = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (rec, assistants, png) = (rec.clone(), assistants.clone(), png.clone());
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
                    if let Some(rest) = req.strip_prefix("GET /avatars/") {
                        let n = rest.split('.').next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        rec.lock().unwrap().push(n);
                        let _ = sock
                            .write_all(
                                format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", png.len()).as_bytes(),
                            )
                            .await;
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
                    let models =
                        r#"3:{"usedBudget":1,"maxBudget":1000,"models":[{"id":"m-gpt","name":"gpt","displayName":"GPT-5","priceMetadata":{"type":"text"}}]}"#;
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin"},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("application/json", r#"{"conversations":[]}"#.to_string())
                    } else if req.starts_with("GET /assistants ") {
                        ("text/html", page("assistants", "1:{}"))
                    } else if req.starts_with("GET /characters ") {
                        ("text/html", page("characters", "1:{}"))
                    } else if req.starts_with("GET /learning-scenarios ") {
                        ("text/html", page("scenarios", "1:{}"))
                    } else if req.starts_with("GET / ") {
                        ("text/html", page("app", models))
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        ("application/javascript", reference(0x7f, "sendChatMessageAction"))
                    } else if req.starts_with("GET /_next/static/chunks/assistants.js") {
                        ("application/javascript", reference(0x11, "getAssistantsByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/characters.js") {
                        ("application/javascript", reference(0x22, "getCharactersByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/scenarios.js") {
                        ("application/javascript", reference(0x33, "getLearningScenariosByFilterAction"))
                    } else if req.starts_with("POST / ") && action == id(0x11) {
                        ("text/x-component", flight(&assistants))
                    } else if req.starts_with("POST / ") && (action == id(0x22) || action == id(0x33)) {
                        ("text/x-component", flight("[]"))
                    } else {
                        ("text/plain", String::new())
                    };
                    let _ = sock
                        .write_all(
                            format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes(),
                        )
                        .await;
                });
            }
        });
        format!("http://{addr}")
    });
    (base, asked)
}

/// The card label of assistant `n` (zero-padded: the page sorts by name).
fn name(n: usize) -> String {
    format!("Assistent {n:02}")
}

/// The cards in the window, and those in it or at most `AHEAD` points below or above.
fn cards_in_view(h: &egui_kittest::Harness<'static, AisCraftApp>) -> (Vec<usize>, Vec<usize>) {
    const AHEAD: f32 = 320.0 + 1.0;
    let (mut seen, mut near) = (Vec::new(), Vec::new());
    for n in 1..=COUNT {
        let label = name(n);
        let Some(card) = h.query_by_label(&label) else { continue };
        let r = card.rect();
        let (top, bottom) = (r.min.y, r.max.y);
        if bottom > 0.0 && top < 800.0 {
            seen.push(n);
        }
        if bottom > -AHEAD && top < 800.0 + AHEAD {
            near.push(n);
        }
    }
    (seen, near)
}

/// Draws frames until no new picture was asked for during 40 frames in a row and none is open.
fn settle(h: &mut egui_kittest::Harness<'static, AisCraftApp>, engine: &Engine, asked: &Arc<Mutex<Vec<usize>>>) -> Vec<usize> {
    let mut quiet = 0;
    let mut last = usize::MAX;
    for _ in 0..600 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(15));
        let now = asked.lock().unwrap().clone();
        let arrived = engine.state().persona_pictures.len();
        if now.len() == last && arrived >= now.len() {
            quiet += 1;
            if quiet >= 40 {
                return now;
            }
        } else {
            quiet = 0;
            last = now.len();
        }
    }
    panic!("pictures did not settle: {:?}", asked.lock().unwrap());
}

/// Quality gate (#lazy pictures): opening Entdecken asks only for the pictures of the cards in
/// view and about a row ahead; jumping to the end asks for the last cards and skips those in
/// between; scrolling back asks for nothing again.
#[test]
fn explore_asks_only_for_pictures_in_view() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (base, asked) = scripted_ais(&engine);
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).unwrap();
    engine.set_picture_origin(Some(&base)).unwrap();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "demo"})).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let e = engine.clone();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, Some(rx), OmarchyTheme::none()));
    h.run_steps(4);
    for close in ["Später", "Fertig"] {
        if let Some(b) = h.query_by_label(close) {
            b.click();
            h.run_steps(4);
        }
    }
    let (reply, _answer) = std::sync::mpsc::channel();
    tx.send(aiscraft_ui::UiRequest::Show { what: "personas".into(), reply }).unwrap();
    common::step_until(&mut h, "the cards are there", |h| h.query_by_label(&name(1)).is_some());

    let first = settle(&mut h, &engine, &asked);
    let (seen, near) = cards_in_view(&h);
    assert!(!seen.is_empty(), "some cards are in view");
    for n in &seen {
        assert!(first.contains(n), "card {n} is in view and asked for its picture: {first:?}");
    }
    for n in &first {
        assert!(near.contains(n), "card {n} asked though it is not in view or a row ahead: {first:?}");
    }
    assert!(first.len() < COUNT / 2, "opening the page does not ask for all {COUNT}: {first:?}");

    // Jump to the end: the last cards ask, those jumped over do not.
    h.get_by_label(&name(COUNT)).scroll_to_me();
    let after = settle(&mut h, &engine, &asked);
    let (seen, near_end) = cards_in_view(&h);
    assert!(after.contains(&COUNT), "the last card asks once it is in view: {after:?}");
    for n in &seen {
        assert!(after.contains(n), "card {n} is in view and asked for its picture: {after:?}");
    }
    for n in &after {
        assert!(near.contains(n) || near_end.contains(n), "card {n} was never near the view and still asked: {after:?}");
    }
    assert!(after.len() < COUNT, "jumping over cards does not ask for their pictures: {after:?}");

    // Back to the top: nothing is asked for twice.
    h.get_by_label(&name(1)).scroll_to_me();
    let back = settle(&mut h, &engine, &asked);
    assert_eq!(back.len(), after.len(), "pictures already there are not asked for again: {back:?}");
    let mut unique = back.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), back.len(), "no picture asked for twice: {back:?}");
}
