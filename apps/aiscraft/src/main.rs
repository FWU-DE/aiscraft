//! AIS.ChatCraft desktop app.
//!
//! `aiscraft` opens the window. `aiscraft --control [port]` also starts the JSON control
//! channel on 127.0.0.1 (default 7979), so agents and tests can drive the running app: one
//! JSON object per line, `{"token": …, "id": 1, "method": "chat.send", "params": {…}}`, answered
//! with `{"id": 1, "ok": true, "result": …}`. The token is in `control-token` in the data folder
//! (readable only by you), so a web page that posts to localhost cannot send commands. Framing and
//! the token check are the Craft suite's (craft-control-protocol): every request carries the
//! token, a wrong one or a line that is not a request closes the connection.
//! Besides every engine command: `ui.screenshot {path}` and `ui.show {what}`.
//!
//! `aiscraft --quick [--screenshot] [Frage …]` is for a desktop shortcut (#35): it hands the
//! question to the running app through the control channel (`chat.quick_ask`), or starts the app
//! with it when none is running. No global keyboard hooks.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::{AisCraftApp, UiRequest};
use serde_json::{Value, json};

const DEFAULT_PORT: u16 = 7979;
/// Longest request line the control channel reads.
const MAX_CONTROL_LINE: usize = 16 * 1024 * 1024;

/// The HTML preview (`aiscraft-preview`, its own process with a webview) next to this program.
/// Without it, learning apps open in the system browser.
#[cfg(feature = "preview")]
fn preview_program() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name(if cfg!(windows) { "aiscraft-preview.exe" } else { "aiscraft-preview" });
    p.is_file().then_some(p)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    // craft-update's restart helper (an update on Windows) runs this program with its own
    // arguments: finish the update, start the new copy, and end here.
    let raw: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if let Some(done) = aiscraft_engine::finish_update_helper(&raw) {
        if let Err(e) = done {
            eprintln!("Update nicht abgeschlossen: {e}");
            std::process::exit(1);
        }
        return;
    }
    let args: Vec<String> = raw.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "AIS.ChatCraft {}\n\n  aiscraft                 Fenster öffnen\n  aiscraft --control [port] mit JSON-Steuerkanal auf 127.0.0.1 (Standard {DEFAULT_PORT})\n  aiscraft --quick [--screenshot] [Frage]  Schnelleingabe (für ein Tastenkürzel des Desktops)",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("aiscraft {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let mut control_port = args.iter().position(|a| a == "--control").map(|i| args.get(i + 1).and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT));

    let opts = Options::default_dirs();
    let data_dir = opts.data_dir.clone();
    // Quick entry: to the running app; without one, this start opens with it.
    let quick = args.iter().any(|a| a == "--quick").then(|| quick_params(&args));
    if let Some(params) = &quick {
        match send_quick(data_dir.as_deref(), control_port.unwrap_or(DEFAULT_PORT), params) {
            Ok(()) => return,
            Err(e) => {
                log::info!("Schnelleingabe: keine laufende App ({e}), starte sie");
                control_port.get_or_insert(DEFAULT_PORT);
            }
        }
    }
    let engine = match Engine::open(opts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("AIS.ChatCraft kann nicht starten: {e}");
            std::process::exit(1);
        }
    };

    #[cfg(feature = "preview")]
    engine.set_preview_program(preview_program());

    if let Some(params) = quick {
        engine.dispatch("chat.quick_ask", params);
    }

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

/// `chat.quick_ask` params from `--quick [--screenshot] [Frage …]`.
fn quick_params(args: &[String]) -> Value {
    let screenshot = args.iter().any(|a| a == "--screenshot");
    let start = args.iter().position(|a| a == "--quick").map(|i| i + 1).unwrap_or(args.len());
    let mut words = Vec::new();
    let mut skip_port = false;
    for a in args.get(start..).unwrap_or_default() {
        if skip_port {
            skip_port = false;
            if a.parse::<u16>().is_ok() {
                continue;
            }
        }
        match a.as_str() {
            "--screenshot" => {}
            "--control" => skip_port = true,
            _ => words.push(a.as_str()),
        }
    }
    json!({"text": words.join(" "), "screenshot": screenshot})
}

/// Hand a quick entry to the running app through its control channel. `Err` only when no app
/// took it (no token, nobody listening): then this start opens the window itself. Once the
/// request is out, the running app has it, whatever the reply says or how long it takes.
fn send_quick(data_dir: Option<&std::path::Path>, port: u16, params: &Value) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader};
    let dir = data_dir.map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let token = std::fs::read_to_string(dir.join("control-token"))?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    let line = json!({"token": token.trim(), "id": 1, "method": "chat.quick_ask", "params": params});
    stream.write_all(format!("{line}\n").as_bytes())?;
    // The screenshot and the answer can take a while; wait a little for an error to show.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut reply = String::new();
    if BufReader::new(stream).read_line(&mut reply).is_ok()
        && let Ok(v) = serde_json::from_str::<Value>(&reply)
        && v.get("ok").and_then(Value::as_bool) != Some(true)
    {
        // A wrong token closes the connection without a reply; any other error the app shows too.
        let msg = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("unbekannter Fehler");
        eprintln!("Schnelleingabe: {msg}");
    }
    Ok(())
}

fn start_control(engine: Engine, ui: mpsc::Sender<UiRequest>, port: u16, data_dir: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let dir = data_dir.unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    // 64 hex characters, the suite's control token form (`craft_control_protocol::valid_token`).
    let token = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
    let token_path = dir.join("control-token");
    // Mode 0600 from the first byte, replaced atomically: a reader never sees half a token.
    let private = craft_atomic::WriteOptions::default().with_new_file_permissions(craft_atomic::NewFilePermissions::Private);
    private.atomic_write(&token_path, token.as_bytes())?;
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    std::thread::Builder::new().name("aiscraft-control".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            let (engine, ui, token) = (engine.clone(), ui.clone(), token.clone());
            let Ok(read) = stream.try_clone() else { continue };
            let _ = std::thread::Builder::new().name("aiscraft-control-conn".into()).spawn(move || serve(read, stream, &engine, &ui, &token));
        }
    })?;
    Ok(token_path)
}

fn serve(read: impl Read, out: impl Write, engine: &Engine, ui: &mpsc::Sender<UiRequest>, token: &str) {
    craft_control_protocol::serve_lines(read, out, token, MAX_CONTROL_LINE, |method, params| {
        // Last-resort guard: a panic in a command must not take the app down.
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(engine, ui, &method, params)));
        Some(match res {
            Ok(Ok(v)) => json!({"ok": true, "result": v}),
            Ok(Err(e)) => json!({"ok": false, "error": e}),
            Err(_) => json!({"ok": false, "error": {"kind": "protocol", "message": "Interner Fehler"}}),
        })
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_channel_checks_the_token_on_every_request() {
        let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
        // No window: UI requests fail at once instead of waiting.
        let (ui, _) = mpsc::channel::<UiRequest>();
        let token = "ab".repeat(32);
        let input = format!(
            "{}\n{}\n{}\n{}\n",
            json!({"id": 1, "token": token, "method": "app.state"}),
            json!({"id": 2, "token": token, "method": "ui.show", "params": {"what": "settings"}}),
            json!({"id": 3, "token": "falsch", "method": "app.state"}),
            json!({"id": 4, "token": token, "method": "app.state"}),
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &engine, &ui, &token);
        let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 3, "{replies:?}");
        assert_eq!((replies[0]["id"].clone(), replies[0]["ok"].clone()), (json!(1), json!(true)));
        assert_eq!(replies[1]["error"]["message"], "Fenster ist geschlossen");
        assert_eq!(replies[2]["ok"], false, "a wrong token closes the connection");
        assert!(craft_control_protocol::valid_token(&token));
    }

    /// #35: the quick entry goes through the control channel: a new chat, the window to the front;
    /// a screenshot is refused by an engine without a data folder (no test touches the screen).
    #[test]
    fn quick_entry_through_the_control_channel() {
        let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
        let (ui, _) = mpsc::channel::<UiRequest>();
        let token = "cd".repeat(32);
        let input = format!(
            "{}\n{}\n",
            json!({"id": 1, "token": token, "method": "chat.quick_ask", "params": quick_params(&["--quick".to_string()])}),
            json!({"id": 2, "token": token, "method": "chat.quick_ask", "params": {"screenshot": true}}),
        );
        let mut out = Vec::new();
        serve(input.as_bytes(), &mut out, &engine, &ui, &token);
        let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies[0]["ok"], true, "{replies:?}");
        assert_eq!(replies[0]["result"]["sent"], false);
        assert_eq!(engine.state().quick.as_ref().map(|q| q.seq), Some(1), "the window is asked to come to the front");
        assert!(engine.state().current.is_none(), "a new chat waits for the question");
        assert_eq!(replies[1]["ok"], false);
        assert!(replies[1]["error"]["message"].as_str().unwrap().contains("Bildschirmfoto"), "{replies:?}");
        // The arguments of a desktop shortcut.
        let args: Vec<String> = ["--quick", "--screenshot", "Was", "ist", "das?"].iter().map(|s| s.to_string()).collect();
        assert_eq!(quick_params(&args), json!({"text": "Was ist das?", "screenshot": true}));
    }

    /// #35: `aiscraft --quick` reaches a running app over 127.0.0.1 with the token from the data
    /// folder; without a running app it says so, and that start opens the window itself.
    #[test]
    fn quick_entry_finds_the_running_app() {
        let dir = std::env::temp_dir().join(format!("aiscraft-quick-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let params = quick_params(&["--quick".to_string()]);
        assert!(send_quick(Some(&dir), 1, &params).is_err(), "no token: no app");
        let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
        let (ui, _rx) = mpsc::channel::<UiRequest>();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        start_control(engine.clone(), ui, port, Some(dir.clone())).unwrap();
        send_quick(Some(&dir), port, &params).unwrap();
        assert_eq!(engine.state().quick.as_ref().map(|q| q.seq), Some(1));
        let _ = std::fs::remove_dir_all(dir);
    }
}
