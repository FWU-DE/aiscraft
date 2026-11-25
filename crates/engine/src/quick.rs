//! Quick entry (#35): a question from anywhere, through the control channel
//! (`aiscraft --quick`, which a desktop shortcut runs), optionally with a screenshot. There are
//! no global keyboard hooks: the desktop's own shortcut settings start the command.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;

/// The last quick entry, for the window: it comes to the front, a new chat is open, the
/// screenshot waits in the composer when there was no text to send.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QuickEntry {
    /// Counts up with every quick entry.
    pub seq: u64,
    /// Files for the composer (the screenshot).
    pub files: Vec<String>,
}

/// Longest wait for a screenshot program.
const SHOT_TIMEOUT: Duration = Duration::from_secs(20);

fn screenshots_dir(c: &Ctx) -> PathBuf {
    c.0.attachments_dir.join("screenshots")
}

/// Screenshots of an earlier run that were never sent.
pub(crate) fn clean_up(c: &Ctx) {
    let _ = std::fs::remove_dir_all(screenshots_dir(c));
}

/// The programs that take a screenshot of the whole screen into `out`, in the order they are
/// tried: the desktop's own tool first.
fn shot_commands(out: &Path) -> Vec<(String, Vec<String>)> {
    let o = out.display().to_string();
    let c = |p: &str, a: &[&str]| (p.to_string(), a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    if cfg!(target_os = "macos") {
        return vec![c("screencapture", &["-x", &o])];
    }
    if cfg!(windows) {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; $b=[System.Windows.Forms.SystemInformation]::VirtualScreen; \
             $p=New-Object System.Drawing.Bitmap $b.Width,$b.Height; $g=[System.Drawing.Graphics]::FromImage($p); \
             $g.CopyFromScreen($b.Left,$b.Top,0,0,$p.Size); $p.Save('{}')",
            o.replace('\'', "''")
        );
        return vec![c("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])];
    }
    let mut v = Vec::new();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && !desktop.contains("gnome") && !desktop.contains("kde") {
        // Hyprland, Sway and other wlroots compositors.
        v.push(c("grim", &[&o]));
    }
    if desktop.contains("kde") {
        v.push(c("spectacle", &["-b", "-n", "-f", "-o", &o]));
    }
    v.push(c("gnome-screenshot", &["-f", &o]));
    v.push(c("grim", &[&o]));
    v.push(c("spectacle", &["-b", "-n", "-f", "-o", &o]));
    v.push(c("scrot", &["-o", &o]));
    v.push(c("import", &["-window", "root", &o]));
    v
}

/// A screenshot of the whole screen as a PNG file. Only the app itself takes one: engines without
/// a data folder (tests, scripts) refuse, like the browser and the microphone.
pub(crate) async fn screenshot(c: &Ctx) -> Result<PathBuf> {
    if !c.0.browser_allowed {
        return Err(Error::refused("Bildschirmfotos macht nur die App selbst, nicht in Tests oder Skripten"));
    }
    let dir = screenshots_dir(c);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io("Ordner für Bildschirmfotos", &e))?;
    let out = dir.join(format!("Bildschirmfoto-{}.png", chrono::Local::now().format("%Y-%m-%d-%H%M%S")));
    let mut tried = Vec::new();
    for (prog, args) in shot_commands(&out) {
        if tried.contains(&prog) {
            continue;
        }
        tried.push(prog.clone());
        let run = tokio::process::Command::new(&prog)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match tokio::time::timeout(SHOT_TIMEOUT, run).await {
            Ok(Ok(status)) if status.success() && std::fs::metadata(&out).is_ok_and(|m| m.len() > 0) => return Ok(out),
            // Not installed, failed or hung: the next one.
            _ => continue,
        }
    }
    Err(Error::unsupported(
        "Kein Programm für Bildschirmfotos gefunden. Unter Linux grim (Hyprland, Sway), gnome-screenshot oder spectacle installieren; macOS und Windows haben eins dabei.",
    ))
}

/// `chat.quick_ask`: a new chat, brought to the front; with `text` the question is sent at once,
/// with `screenshot` a picture of the screen goes along (or waits in the composer).
pub(crate) async fn run(c: Ctx, p: Value) -> Result<Value> {
    let text = match p.get("text") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => return Err(Error::invalid("text muss ein Text sein")),
    };
    let shot = match p.get("screenshot") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(Error::invalid("screenshot muss true oder false sein")),
    };
    // The screenshot first: the window must not be in front yet.
    let file = if shot { Some(screenshot(&c).await?) } else { None };
    c.run("chat.new", json!({})).await?;
    let files: Vec<String> = file.iter().map(|f| f.display().to_string()).collect();
    let send_now = !text.is_empty();
    c.update(|s| {
        let seq = s.quick.as_ref().map(|q| q.seq).unwrap_or(0) + 1;
        s.quick = Some(QuickEntry { seq, files: if send_now { Vec::new() } else { files.clone() } });
    });
    if !send_now {
        return Ok(json!({"sent": false, "files": files}));
    }
    let result = crate::chat::send(c.clone(), text, files.clone()).await;
    // The chat keeps its own copy of the picture.
    for f in &files {
        let _ = std::fs::remove_file(f);
    }
    let mut out = result?;
    if let Some(o) = out.as_object_mut() {
        o.insert("sent".into(), json!(true));
    }
    Ok(out)
}
