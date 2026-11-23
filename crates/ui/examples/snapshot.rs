//! Render the AISCraft window offscreen (no window, no focus stealing) and save a PNG.
//!
//! ```sh
//! cargo run -p aiscraft-ui --example snapshot -- --out shot.png --size 1280x800 --demo \
//!     --script '[["chat.send", {"text": "Erkläre Photosynthese"}], ["ui.show", {"what": "settings"}]]'
//! ```
//!
//! `--demo` starts a local OpenAI-compatible server that streams a Markdown answer, adds it as a
//! backend and fills projects, notes and a few chats. `--theme light|dark|omarchy`.
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
                        let body = r#"{"data":[{"id":"demo-sonnet","name":"Demo Sonnet"},{"id":"demo-haiku","name":"Demo Haiku"}]}"#;
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
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
