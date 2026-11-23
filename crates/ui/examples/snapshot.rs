//! Render the AIS.ChatCraft window offscreen (no window, no focus stealing) and save a PNG.
//!
//! ```sh
//! cargo run -p aiscraft-ui --example snapshot -- --out shot.png --size 1280x800 --demo \
//!     --script '[["chat.send", {"text": "Erkläre Photosynthese"}], ["ui.show", {"what": "settings"}]]'
//! ```
//!
//! `--demo` starts a local OpenAI-compatible server that streams a Markdown answer, adds it as a
//! backend and fills projects, notes and a few chats. `--demo-ais` adds a local stand-in for
//! AIS.chat (assistants, characters, learning scenarios, image generation, a plot answer for
//! prompts with "Parabel"). `--theme light|dark|omarchy`.
//! The engine runs in memory; nothing touches your real data.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::mpsc;
use std::time::Duration;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::{AisCraftApp, UiRequest};
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

const ANSWER: &str = "Die **Photosynthese** ist der Vorgang, mit dem Pflanzen aus Licht Energie gewinnen.\n\n## Was passiert?\n\n1. Blätter nehmen **Kohlenstoffdioxid** aus der Luft auf.\n2. Die Wurzeln liefern **Wasser**.\n3. Mit Sonnenlicht entsteht daraus **Traubenzucker** und **Sauerstoff**.\n\n> 6 CO₂ + 6 H₂O → C₆H₁₂O₆ + 6 O₂\n\n| Stoff | Rolle |\n|---|---|\n| Chlorophyll | fängt das Licht ein |\n| Wasser | liefert Elektronen |\n\n```python\n# Sauerstoff pro Stunde (vereinfacht)\nblaetter = 120\nprint(blaetter * 0.5, \"ml\")\n```\n\nSoll ich daraus ein Arbeitsblatt für Klasse 7 machen?";

fn demo_server(engine: &Engine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 16];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"demo-sonnet","name":"Demo Sonnet","context_length":200000},{"id":"demo-haiku","name":"Demo Haiku"}]}"#;
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    // "Kollegium" + a connector tool named *__notify: ask to call it (approval demo).
                    if req.contains("Kollegium") && !req.contains("\"role\":\"tool\"") {
                        if let Some(name) = req.split('"').find(|p| p.ends_with("__notify")) {
                            let c1 = json!({"choices":[{"delta":{"content":"Ich schicke die Einladung ans Kollegium.","tool_calls":[{"index":0,"id":"c1","function":{"name":name,"arguments":"{\"an\":\"kollegium@schule.de\",\"betreff\":\"Konferenz am 4. Dezember\"}"}}]}}]});
                            let c2 = json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]});
                            let _ = sock.write_all(format!("data: {c1}\n\ndata: {c2}\n\ndata: [DONE]\n\n").as_bytes()).await;
                            return;
                        }
                    }
                    for chunk in ANSWER.split_inclusive(' ') {
                        let c = json!({"choices":[{"delta":{"content":chunk}}]});
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":412,\"completion_tokens\":168}}\n\ndata: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// A local stand-in for AIS.chat (`--demo-ais`): signs in, lists assistants, characters and
/// learning scenarios and answers chats. Nothing goes to the real app.
fn demo_ais(engine: &Engine) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // A drawn "generated image": sky, sun, hills.
    let png: std::sync::Arc<Vec<u8>> = {
        let img = image::RgbImage::from_fn(768, 512, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let hill = 360.0 + 40.0 * (fx / 90.0).sin();
            if (fx - 560.0).hypot(fy - 130.0) < 60.0 {
                image::Rgb([255, 214, 90])
            } else if fy > hill {
                image::Rgb([60, (140.0 + (fy - hill) * 0.3) as u8, 70])
            } else {
                image::Rgb([(120.0 + fy * 0.15) as u8, (170.0 + fy * 0.1) as u8, 235])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        std::sync::Arc::new(out.into_inner())
    };
    let base = engine.block_on(async {
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
                            let len = s[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    if req.starts_with("GET /files/demo.png") {
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", png.len()).as_bytes()).await;
                        let _ = sock.write_all(&png).await;
                        return;
                    }
                    let id = |n: u8| format!("{n:02x}").repeat(21);
                    let page = |chunk: &str, row: &str| format!(r#"<script src="/_next/static/chunks/{chunk}.js"></script><script>self.__next_f.push([1,{}])</script>"#, serde_json::to_string(&format!("{row}\n")).unwrap());
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    let models = r#"3:{"usedBudget":120,"maxBudget":1000,"models":[{"id":"m-gpt","name":"gpt","displayName":"GPT-5","priceMetadata":{"type":"text"}},{"id":"m-img","name":"img","displayName":"Bildmodell","priceMetadata":{"type":"image"}}]}"#;
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin","school":{"name":"Gesamtschule Nord"}},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("application/json", r#"{"conversations":[]}"#.to_string())
                    } else if req.starts_with("GET /image-generation ") {
                        ("text/html", page("images", models))
                    } else if req.starts_with("GET /_next/static/chunks/images.js") {
                        ("application/javascript", reference(0x44, "generateImageAction"))
                    } else if req.starts_with("POST / ") && action == id(0x44) {
                        ("text/x-component", flight(r#"{"success":true,"value":{"imageUrl":"/files/demo.png","conversationId":"srv-img","fileId":"file_1","assistantMessageId":"am1","userMessageId":"um1"}}"#))
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
                        ("text/x-component", flight(r#"[{"id":"a1","name":"Mathe-Coach","userId":"me","description":"Übt Bruchrechnung Schritt für Schritt","promptSuggestions":["Erkläre 3/4 + 1/8","Gib mir eine Übungsaufgabe"]},{"id":"a2","name":"Elternbrief-Helfer","userId":"x","description":"Formuliert freundliche Elternbriefe"}]"#))
                    } else if req.starts_with("POST / ") && action == id(0x22) {
                        ("text/x-component", flight(r#"[{"id":"c1","name":"Johann Wolfgang von Goethe","userId":"x","description":"Dichter und Naturforscher aus Weimar","initialMessage":"Seid gegrüßt! Was möchtet Ihr über mein Leben und Werk erfahren?"},{"id":"c2","name":"Marie Curie","userId":"me","description":"Physikerin und Chemikerin"}]"#))
                    } else if req.starts_with("POST / ") && action == id(0x33) {
                        ("text/x-component", flight(r#"[{"id":"l1","name":"Photosynthese erforschen","userId":"me","description":"Klasse 7, Biologie","studentExercise":"Finde heraus, was eine Pflanze zum Wachsen braucht, und erkläre es in drei Sätzen."}]"#))
                    } else if req.starts_with("POST / ") && action == id(0x7f) && req.contains("Parabel") {
                        // A create_plot answer, as AIS.chat streams it: activity, then text with the spec.
                        let activity = serde_json::to_string(&format!("\u{1e}{}", json!({"type": "ai_activity", "steps": [
                            {"kind": "tool", "id": "1", "tool": "web_scraper", "links": [{"title": "Lehrplan Mathematik", "url": "https://schule.example/plan"}, {"title": "", "url": "https://kaputt.example/", "failed": true}]},
                            {"kind": "tool", "id": "2", "tool": "create_plot", "detail": "Parabel y = a·x² mit Schieberegler a, Scheitel markiert", "detailPlacement": "below", "result": "ok"},
                            {"kind": "done"}
                        ]}))).unwrap();
                        let plot = r##"{"board":{"boundingBox":[-5,9,5,-2],"grid":true},"elements":[["slider",[0,1,3],{"name":"a"}],["functiongraph",["a*x^2"],{"name":"f","strokeColor":"#2563eb"}],["functiongraph",["2*x+3"],{"strokeColor":"#dc2626","dash":2}],["point",[0,0],{"name":"S"}],["text",[1.2,7.5,"y = a·x²"]]]}"##;
                        let text = serde_json::to_string(&format!("Hier ist die Parabel mit dem Schieberegler *a*:\n\n```jsxgraph-json\n{plot}\n```\n\nDie rote Gerade schneidet sie zweimal.")).unwrap();
                        ("text/x-component", format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{{\"stream\":\"$2\",\"messageId\":\"$undefined\"}}\n2:R\n2:{activity}\n2:{text}\n2:C\n"))
                    } else if req.starts_with("POST / ") && action == id(0x7f) {
                        let text = serde_json::to_string(ANSWER).unwrap();
                        ("text/x-component", format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{{\"stream\":\"$2\",\"messageId\":\"$undefined\"}}\n2:R\n2:{text}\n2:C\n"))
                    } else {
                        ("text/plain", String::new())
                    };
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    });
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).unwrap();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "demo"})).unwrap();
    engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m-gpt"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
}

fn seed(engine: &Engine) {
    let url = demo_server(engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "name": "Demo", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    let bid = id.clone();
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.backend(&bid).is_some_and(|b| !b.models.is_empty())));
    engine.call("chat.select_model", json!({"backend": id, "model": "demo-sonnet"})).unwrap();
    engine.call("project.create", json!({"name": "Biologie 7b", "instructions": "Antworte für Siebtklässler."})).unwrap();
    engine.call("project.create", json!({"name": "Elternabend", "instructions": ""})).unwrap();
    engine.call("memory.add", json!({"text": "Ich unterrichte Biologie und Chemie an einer Gesamtschule", "area": "about"})).unwrap();
    engine.call("memory.add", json!({"text": "Antworte bitte kurz und mit Beispielen", "area": "preferences"})).unwrap();
    for t in ["Elternbrief zur Klassenfahrt", "Quiz: Zellbestandteile", "Bewertungsraster Referat"] {
        engine.call("chat.new", json!({})).unwrap();
        engine.call("chat.send", json!({"text": t})).unwrap();
    }
    engine.call("chat.new", json!({})).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").unwrap_or_else(|| "snapshot.png".into());
    let (w, h) = arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))).unwrap_or((1280.0, 800.0));
    let scale: f32 = arg(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let script: Vec<(String, Value)> = arg(&args, "--script").map(|s| serde_json::from_str(&s).expect("--script: [[cmd, params], …]")).unwrap_or_default();

    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    if let Some(theme) = arg(&args, "--theme") {
        engine.call("settings.set", json!({"key": "theme", "value": theme})).unwrap();
    }
    if args.iter().any(|a| a == "--demo") {
        seed(&engine);
    }
    if args.iter().any(|a| a == "--demo-ais") {
        demo_ais(&engine);
    }
    let (tx, rx) = mpsc::channel::<UiRequest>();
    let app_engine = engine.clone();
    let mut rx = Some(rx);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(scale)
        .with_max_steps(64)
        // Answers carry arbitrary characters (→, ₂ …); the app draws boxes for missing ones too.
        .allow_missing_glyphs()
        .wgpu()
        .build_eframe(move |cc| AisCraftApp::new(cc, app_engine.clone(), rx.take()));
    harness.run_steps(4);
    for (cmd, params) in script {
        if cmd == "ui.show" {
            let (rtx, _rrx) = mpsc::channel();
            tx.send(UiRequest::Show { what: params["what"].as_str().unwrap_or("").into(), reply: rtx }).unwrap();
        } else if cmd == "ui.key" {
            let key = egui::Key::from_name(params["key"].as_str().unwrap_or("")).expect("key name");
            let modifiers = if params["ctrl"].as_bool().unwrap_or(false) { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
            harness.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            harness.step();
            harness.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
        } else if cmd == "ui.type" {
            harness.event(egui::Event::Text(params["text"].as_str().unwrap_or("").into()));
        } else if let Some(c) = cmd.strip_prefix("dispatch:") {
            engine.dispatch(c, params.clone());
            std::thread::sleep(Duration::from_millis(800));
        } else if let Err(e) = engine.call(&cmd, params.clone()) {
            eprintln!("{cmd}: {e}");
        }
        harness.run_steps(6);
    }
    for _ in 0..20 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let img = harness.render().expect("render");
    img.save(&out).expect("save png");
    println!("wrote {out} ({}×{})", img.width(), img.height());
}
