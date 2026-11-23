//! AIS.ChatCraft desktop app.
//!
//! `aiscraft` opens the window. `aiscraft --control [port]` also starts the JSON control
//! channel on 127.0.0.1 (default 7979), so agents and tests can drive the running app: one
//! JSON object per line, `{"token": …, "id": 1, "cmd": "chat.send", "params": {…}}`, answered
//! with `{"id": 1, "ok": true, "result": …}`. The token is in `control-token` in the data folder
//! (readable only by you), so a web page that posts to localhost cannot send commands.
//! Besides every engine command: `ui.screenshot {path}` and `ui.show {what}`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::{AisCraftApp, UiRequest};
use serde_json::{Value, json};

const DEFAULT_PORT: u16 = 7979;

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "AIS.ChatCraft {}\n\n  aiscraft                 Fenster öffnen\n  aiscraft --control [port] mit JSON-Steuerkanal auf 127.0.0.1 (Standard {DEFAULT_PORT})",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("aiscraft {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let control_port = args.iter().position(|a| a == "--control").map(|i| args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT));

    let opts = Options::default_dirs();
    let data_dir = opts.data_dir.clone();
    let engine = match Engine::open(opts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("AIS.ChatCraft kann nicht starten: {e}");
            std::process::exit(1);
        }
    };

    let (ui_tx, ui_rx) = mpsc::channel::<UiRequest>();
    if let Some(port) = control_port {
        match start_control(engine.clone(), ui_tx, port, data_dir) {
            Ok(token_path) => eprintln!("Steuerkanal: 127.0.0.1:{port} (Token in {})", token_path.display()),
            Err(e) => eprintln!("Steuerkanal nicht gestartet: {e}"),
        }
    }

    let mut viewport = egui::ViewportBuilder::default();
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../../../packaging/icons/aischatcraft-256.png")) {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    let native = eframe::NativeOptions {
        viewport: viewport.with_title("AIS.ChatCraft").with_app_id("ai.uds.aiscraft").with_inner_size([1180.0, 780.0]).with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    let result = eframe::run_native("AIS.ChatCraft", native, Box::new(move |cc| Ok(Box::new(AisCraftApp::new(cc, engine, Some(ui_rx))))));
    if let Err(e) = result {
        eprintln!("AIS.ChatCraft: {e}");
        std::process::exit(1);
    }
}

fn start_control(engine: Engine, ui: mpsc::Sender<UiRequest>, port: u16, data_dir: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let dir = data_dir.unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let token_path = dir.join("control-token");
    std::fs::write(&token_path, &token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))?;
    }
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    std::thread::Builder::new().name("aiscraft-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (engine, ui, token) = (engine.clone(), ui.clone(), token.clone());
            let _ = std::thread::Builder::new().name("aiscraft-control-conn".into()).spawn(move || serve(stream, engine, ui, token));
        }
    })?;
    Ok(token_path)
}

fn serve(stream: TcpStream, engine: Engine, ui: mpsc::Sender<UiRequest>, token: String) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Err(_) => {
                // Not JSON (for example an HTTP request from a browser): hang up.
                return;
            }
            Ok(req) => {
                let id = req.get("id").cloned().unwrap_or(Value::Null);
                if req.get("token").and_then(Value::as_str) != Some(token.as_str()) {
                    json!({"id": id, "ok": false, "error": {"kind": "auth", "message": "Falscher oder fehlender Token"}})
                } else {
                    let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or("");
                    let params = req.get("params").cloned().unwrap_or(json!({}));
                    // Last-resort guard: a panic in a command must not take the app down.
                    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&engine, &ui, cmd, params)));
                    match res {
                        Ok(Ok(v)) => json!({"id": id, "ok": true, "result": v}),
                        Ok(Err(e)) => json!({"id": id, "ok": false, "error": e}),
                        Err(_) => json!({"id": id, "ok": false, "error": {"kind": "protocol", "message": "Interner Fehler"}}),
                    }
                }
            }
        };
        if writeln!(out, "{reply}").is_err() {
            return;
        }
    }
}

fn run(engine: &Engine, ui: &mpsc::Sender<UiRequest>, cmd: &str, params: Value) -> Result<Value, Value> {
    let ui_call = |make: &dyn Fn(mpsc::Sender<Result<Value, String>>) -> UiRequest| -> Result<Value, Value> {
        let (tx, rx) = mpsc::channel();
        ui.send(make(tx)).map_err(|_| json!({"kind": "protocol", "message": "Fenster ist geschlossen"}))?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| json!({"kind": "protocol", "message": "Fenster antwortet nicht"}))?
            .map_err(|m| json!({"kind": "invalid", "message": m}))
    };
    match cmd {
        "ui.screenshot" => {
            let path = PathBuf::from(params.get("path").and_then(Value::as_str).unwrap_or("aiscraft.png"));
            ui_call(&|tx| UiRequest::Screenshot { path: path.clone(), reply: tx })
        }
        "ui.show" => {
            let what = params.get("what").and_then(Value::as_str).unwrap_or("").to_string();
            ui_call(&|tx| UiRequest::Show { what: what.clone(), reply: tx })
        }
        _ => engine.call(cmd, params).map_err(|e| serde_json::to_value(e).unwrap_or(Value::Null)),
    }
}
