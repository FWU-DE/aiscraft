//! Local files (#25): blocklist, links that lead out, and the questions in the chat.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::local_files::{LIST, Places, READ, WRITE, refusal, resolve_existing, resolve_target};
use crate::{Engine, Options};

/// A home folder of its own: Downloads with a text file, secrets in the usual places, and links
/// that lead out of it.
fn home() -> (PathBuf, PathBuf) {
    let base = std::env::temp_dir().join(format!("aiscraft-local-{}-{}", std::process::id(), crate::new_id()));
    let home = base.join("home");
    let outside = base.join("outside");
    for d in [home.join("Downloads"), home.join("Dokumente"), home.join(".ssh"), home.join(".mozilla/profil"), home.join("Library/Keychains"), outside.clone()]
    {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(home.join("Downloads/brief.txt"), "Liebe Eltern, am Freitag ist Projekttag.").unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "PRIVATE KEY").unwrap();
    std::fs::write(home.join(".mozilla/profil/logins.json"), "{}").unwrap();
    std::fs::write(home.join("Dokumente/.env"), "TOKEN=geheim").unwrap();
    std::fs::write(home.join("Dokumente/server.pem"), "KEY").unwrap();
    std::fs::write(outside.join("fremd.txt"), "außerhalb").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.join("fremd.txt"), home.join("Downloads/raus.txt")).unwrap();
        std::os::unix::fs::symlink(home.join(".ssh/id_ed25519"), home.join("Downloads/schluessel.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, home.join("Dokumente/Ablage")).unwrap();
    }
    (base, std::fs::canonicalize(&home).unwrap())
}

fn places(home: &Path) -> Places {
    Places { roots: vec![home.to_path_buf()], refused: vec![], home: Some(home.to_path_buf()) }
}

#[test]
fn secrets_hidden_folders_and_places_outside_are_always_refused() {
    let (base, home) = home();
    let p = places(&home);
    assert!(resolve_existing(&p, "~/Downloads/brief.txt").is_ok());
    assert!(resolve_existing(&p, &home.join("Downloads").display().to_string()).is_ok());
    for bad in [
        "~/.ssh/id_ed25519",
        "~/.mozilla/profil/logins.json",
        "~/Dokumente/.env",
        "~/Dokumente/server.pem",
        "~/Library/Keychains",
        // An absolute path outside the home folder (`/etc/passwd` is not absolute on Windows).
        if cfg!(windows) { r"C:\Windows\win.ini" } else { "/etc/passwd" },
        "~/../outside/fremd.txt",
    ] {
        let e = resolve_existing(&p, bad).unwrap_err();
        assert_eq!(e.kind, aiscraft_core::ErrorKind::Refused, "{bad}: {}", e.message);
    }
    assert!(resolve_existing(&p, "Downloads/brief.txt").is_err(), "relative paths are not guessed");
    // Writing: never a hidden file, a secret, a program, outside, or into a missing folder.
    for bad in ["~/.bashrc", "~/Dokumente/neu.key", "~/Downloads/start.sh", "~/Downloads/x.desktop", "/tmp/aiscraft-x.txt", "~/Fehlt/x.txt"] {
        assert!(resolve_target(&p, bad).is_err(), "{bad}");
    }
    let (target, exists) = resolve_target(&p, "~/Dokumente/Bericht.md").unwrap();
    assert!(!exists && target.ends_with("Dokumente/Bericht.md"));
    // The app's own data and folders kept out.
    let kept = Places { refused: vec![home.join("Dokumente")], ..places(&home) };
    assert!(refusal(&kept, &home.join("Dokumente/Bericht.md")).is_some());
    let _ = std::fs::remove_dir_all(base);
}

#[cfg(unix)]
#[test]
fn links_are_followed_and_checked_where_they_lead() {
    let (base, home) = home();
    let p = places(&home);
    // A link out of the home folder and a link to a key both count as their targets.
    assert_eq!(resolve_existing(&p, "~/Downloads/raus.txt").unwrap_err().kind, aiscraft_core::ErrorKind::Refused);
    assert_eq!(resolve_existing(&p, "~/Downloads/schluessel.txt").unwrap_err().kind, aiscraft_core::ErrorKind::Refused);
    // A folder link that leads out: nothing inside it, and no file is written through it.
    assert!(resolve_existing(&p, "~/Dokumente/Ablage/fremd.txt").is_err());
    assert!(resolve_target(&p, "~/Dokumente/Ablage/neu.txt").is_err());
    // Writing onto a link is refused (the link would be replaced, or its target written).
    assert!(resolve_target(&p, "~/Downloads/raus.txt").is_err());
    let _ = std::fs::remove_dir_all(base);
}

fn call(name: &str, args: Value) -> aiscraft_core::ToolCall {
    aiscraft_core::ToolCall { id: "c".into(), name: name.into(), arguments: args, outcome: None }
}

/// Run a call in the background; the test answers the question meanwhile.
fn run_async(engine: &Engine, c: aiscraft_core::ToolCall, interactive: bool) -> std::sync::mpsc::Receiver<(String, bool, &'static str)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = engine.clone();
    std::thread::spawn(move || {
        let ctx = engine.ctx().clone();
        let r = engine.block_on(crate::tools::run(&ctx, "conv", &c, &Default::default(), interactive));
        let _ = tx.send(r);
    });
    rx
}

fn answer(engine: &Engine, allow: bool) -> crate::Approval {
    assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()), "a question was asked");
    let a = engine.state().approvals[0].clone();
    engine.call("tool.answer", json!({"id": a.id, "allow": allow})).unwrap();
    a
}

#[test]
fn off_by_default_switched_on_by_a_question_in_the_chat_never_in_the_background() {
    let (base, home) = home();
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("settings.set", json!({"key": "trust_check", "value": false})).unwrap();
    engine.ctx().0.ext.set_local_roots(vec![home.clone()]);
    let ctx = engine.ctx().clone();
    assert!(!engine.state().local_files.enabled, "off by default");
    // The tools are on offer, so the model can ask.
    assert!(crate::tools::specs(&ctx).iter().any(|t| t.name == LIST));

    // Background work: refused, nothing asked, still off.
    let (text, err, outcome) = engine.block_on(crate::tools::run(&ctx, "conv", &call(LIST, json!({"path": "~/Downloads"})), &Default::default(), false));
    assert!(err && outcome == "denied" && text.contains("Hintergrund"), "{text}");
    assert!(engine.state().approvals.is_empty() && !engine.state().local_files.enabled);

    // In the chat: one question; no keeps it off.
    let rx = run_async(&engine, call(LIST, json!({"path": "~/Downloads"})), true);
    let a = answer(&engine, false);
    assert!(a.note.as_deref().unwrap_or("").contains("Lokale Dateien sind aus"), "{a:?}");
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "denied");
    assert!(!engine.state().local_files.enabled);

    // Yes switches them on and lists the folder: the text file, never the link out or to a key.
    let rx = run_async(&engine, call(LIST, json!({"path": "~/Downloads"})), true);
    answer(&engine, true);
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("brief.txt") && text.contains("untrusted_tool_result"), "{text}");
    assert!(!text.contains("raus.txt") && !text.contains("schluessel"), "{text}");
    assert!(engine.state().local_files.enabled);

    // Once on, listing runs without a question; background work still refuses.
    let (text, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(LIST, json!({"path": "~"})), &Default::default(), true));
    assert!(!err && text.contains("Downloads/") && !text.contains(".ssh"), "{text}");
    let (_, err, outcome) = engine.block_on(crate::tools::run(&ctx, "conv", &call(READ, json!({"path": "~/Downloads/brief.txt"})), &Default::default(), false));
    assert!(err && outcome == "denied");

    // Refused paths end without a question.
    // The two links exist only where the test can make symlinks (Unix).
    let links = if cfg!(unix) { vec![json!({"path": "~/Downloads/schluessel.txt"}), json!({"path": "~/Downloads/raus.txt"})] } else { vec![] };
    for bad in std::iter::once(json!({"path": "~/.ssh/id_ed25519"})).chain(links) {
        let (text, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(READ, bad), &Default::default(), true));
        assert!(err && (text.contains("gesperrt") || text.contains("außerhalb")), "{text}");
    }
    assert!(engine.state().approvals.is_empty());

    // Reading asks (nobody's words cover it here) and gives the text as untrusted data.
    let rx = run_async(&engine, call(READ, json!({"path": "~/Downloads/brief.txt"})), true);
    let a = answer(&engine, true);
    assert_eq!(a.label, "Lokale Dateien · Datei lesen");
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("Projekttag") && text.contains("untrusted_tool_result"), "{text}");

    // Writing: a no writes nothing, a yes writes; replacing is destructive and asks again.
    let report = home.join("Dokumente/Bericht.md");
    let rx = run_async(&engine, call(WRITE, json!({"path": "~/Dokumente/Bericht.md", "text": "# Bericht"})), true);
    answer(&engine, false);
    rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!report.exists());
    let rx = run_async(&engine, call(WRITE, json!({"path": "~/Dokumente/Bericht.md", "text": "# Bericht"})), true);
    let a = answer(&engine, true);
    assert!(!a.destructive);
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "ok");
    assert_eq!(std::fs::read_to_string(&report).unwrap(), "# Bericht");
    let (text, err, _) =
        engine.block_on(crate::tools::run(&ctx, "conv", &call(WRITE, json!({"path": "~/Dokumente/Bericht.md", "text": "neu"})), &Default::default(), true));
    assert!(err && text.contains("replace"), "an existing file is not replaced unasked: {text}");
    let rx = run_async(&engine, call(WRITE, json!({"path": "~/Dokumente/Bericht.md", "text": "neu", "replace": true})), true);
    assert!(answer(&engine, true).destructive);
    rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(std::fs::read_to_string(&report).unwrap(), "neu");
    for bad in [json!({"path": "~/.bashrc", "text": "x"}), json!({"path": "~/Downloads/los.sh", "text": "x"})] {
        let (_, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(WRITE, bad), &Default::default(), true));
        assert!(err);
    }
    assert!(engine.state().approvals.is_empty());

    // Folders kept out: refused until the person takes them off the list.
    let kept = engine.call("files.keep_out_add", json!({"path": home.join("Downloads").display().to_string()})).unwrap();
    let (_, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(LIST, json!({"path": "~/Downloads"})), &Default::default(), true));
    assert!(err);
    engine.call("files.keep_out_remove", json!({"path": kept["path"]})).unwrap();
    assert!(engine.call("files.keep_out_remove", json!({"path": "/nirgends"})).is_err());
    let (_, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(LIST, json!({"path": "~/Downloads"})), &Default::default(), true));
    assert!(!err);

    // The person switches them off in the settings.
    engine.call("files.local_set", json!({"enabled": false})).unwrap();
    assert!(!engine.state().local_files.enabled);
    assert!(engine.call("files.local_set", json!({})).is_err());
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn engines_in_memory_never_see_the_real_home_folder() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("files.local_set", json!({"enabled": true})).unwrap();
    let ctx = engine.ctx().clone();
    let (text, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(LIST, json!({"path": "~"})), &Default::default(), true));
    assert!(err, "{text}");
}
