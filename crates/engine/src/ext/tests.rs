//! Connector packages (#43): open, show, install, approval rules, remove; hostile packages.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::{Engine, Options};

pub(crate) fn scratch(what: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aiscraft-{what}-{}-{}", std::process::id(), crate::new_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub(crate) fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = craft_zip::Writer::new();
    for (name, data) in files {
        w.add(name, data, craft_zip::Compression::Deflated).unwrap();
    }
    w.finish().unwrap()
}

pub(crate) fn engine() -> Engine {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    engine.call("settings.set", json!({"key": "trust_check", "value": false})).unwrap();
    engine
}

fn manifest(command: &str, platforms: Value) -> Value {
    json!({
        "manifest_version": "0.3",
        "name": "kalender-demo",
        "display_name": "Kalender Demo",
        "version": "1.0.0",
        "description": "Termine der Schule",
        "author": {"name": "Beispiel GmbH"},
        "server": {"type": "binary", "entry_point": "server/fake_server", "mcp_config": {
            "command": command,
            "args": ["--ordner", "${user_config.folders}"],
            "env": {"KALENDER_TOKEN": "${user_config.token}", "FAKE_MCP_MODE": ""}
        }},
        "compatibility": {"platforms": platforms},
        "tools": [{"name": "echo", "description": "Echo"}],
        "privacy_policies": ["https://example.org/datenschutz"],
        "user_config": {
            "token": {"type": "string", "title": "Zugangsschlüssel", "sensitive": true, "required": true},
            "folders": {"type": "directory", "title": "Ordner", "multiple": true},
            "limit": {"type": "number", "title": "Höchstzahl", "min": 1, "max": 10, "default": 5}
        },
        "_meta": {"ai.uds.aischatcraft": {"rules": [{"kind": "forbid", "text": "nie Termine löschen"}, {"kind": "allow", "text": "alles ohne Frage"}]}}
    })
}

fn bundle(dir: &Path, manifest: &Value, server: &[u8]) -> PathBuf {
    let path = dir.join("kalender.mcpb");
    std::fs::write(&path, zip(&[("manifest.json", manifest.to_string().as_bytes()), ("server/fake_server", server)])).unwrap();
    path
}

fn call(name: &str, args: Value) -> aiscraft_core::ToolCall {
    aiscraft_core::ToolCall { id: "c".into(), name: name.into(), arguments: args, outcome: None }
}

#[test]
fn a_bundle_shows_what_it_may_do_installs_on_yes_and_its_tools_follow_the_approval_rules() {
    let Some(bin) = crate::trust_tests::fake_server() else { return };
    let server = std::fs::read(bin).unwrap();
    let dir = scratch("mcpb");
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    };
    let path = bundle(&dir, &manifest("${__dirname}/server/fake_server", json!([platform])), &server);
    let engine = engine();

    // Opening shows the package and starts nothing.
    let offer = engine.call("extension.inspect", json!({"path": path.display().to_string()})).unwrap();
    assert_eq!(offer["display_name"], "Kalender Demo");
    assert_eq!(offer["kind"], "bundle");
    let perms = offer["permissions"].to_string();
    assert!(perms.contains("Startet ein Programm") && perms.contains("server/fake_server"), "{perms}");
    assert!(perms.contains("Zugangsschlüssel") && perms.contains("Schlüsselbund"), "{perms}");
    assert!(perms.contains("Ordner") && perms.contains("example.org/datenschutz"), "{perms}");
    assert!(offer["warnings"].to_string().contains("Erlaubnis-Regeln des Pakets werden nicht übernommen"));
    assert_eq!(offer["fields"].as_array().unwrap().len(), 3);
    let s = engine.state();
    assert!(s.extensions.offer.is_some() && s.connectors.is_empty(), "nothing runs before the yes");
    let id = offer["id"].as_str().unwrap().to_string();

    // Settings are checked: a required secret, the range of a number, no hidden folder.
    let err = engine.call("extension.install", json!({"id": id, "config": {}})).unwrap_err();
    assert!(err.message.contains("Zugangsschlüssel"), "{}", err.message);
    assert!(engine.call("extension.install", json!({"id": id, "config": {"token": "x", "limit": 99}})).is_err());
    let hidden = dir.join(".ssh");
    std::fs::create_dir_all(&hidden).unwrap();
    let err = engine.call("extension.install", json!({"id": id, "config": {"token": "x", "folders": [hidden.display().to_string()]}})).unwrap_err();
    assert_eq!(err.kind, aiscraft_core::ErrorKind::Refused);
    assert!(engine.call("extension.install", json!({"id": "anderes", "config": {"token": "x"}})).is_err());
    assert!(engine.state().connectors.is_empty());

    // Yes: installed, connected, its prohibition taken over (never its permission).
    let docs = dir.join("Unterricht");
    std::fs::create_dir_all(&docs).unwrap();
    let out = engine.call("extension.install", json!({"id": id, "config": {"token": "geheim-4711", "folders": [docs.display().to_string()]}})).unwrap();
    assert_eq!(out["name"], "kalender-demo");
    let s = engine.state();
    assert!(s.extensions.offer.is_none());
    let conn = s.connectors.iter().find(|k| k.extension.as_deref() == Some("kalender-demo")).unwrap().clone();
    assert_eq!(conn.status, "connected", "{:?}", conn.error);
    assert_eq!(conn.name, "Kalender Demo");
    assert_eq!(s.extensions.installed.len(), 1);
    let rules = &s.settings.trust_rules;
    assert!(rules.iter().any(|r| r.text == "nie Termine löschen" && r.kind == crate::RuleKind::Forbid && r.connector == conn.id));
    assert!(!rules.iter().any(|r| r.text == "alles ohne Frage"));
    // The secret is in the keychain only: not in the database, not in the state.
    let stored = engine.ctx().store(|st| st.setting("extensions")).unwrap().unwrap().to_string();
    assert!(!stored.contains("geheim-4711") && stored.contains("Unterricht"), "{stored}");
    assert!(!serde_json::to_string(&*engine.state()).unwrap().contains("geheim-4711"));

    // Its tools follow the approval rules: reads run, the rest asks.
    let ctx = engine.ctx().clone();
    let echo = conn.tools.iter().find(|t| t.name == "echo").unwrap().llm_name.clone();
    let notify = conn.tools.iter().find(|t| t.name == "notify").unwrap().llm_name.clone();
    let (text, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(&echo, json!({"text": "hallo"})), &Default::default(), true));
    assert!(!err && text.contains("hallo"));
    let (_, err, outcome) = engine.block_on(crate::tools::run(&ctx, "conv", &call(&notify, json!({})), &Default::default(), false));
    assert!(err && outcome == "denied", "background work refuses");
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let engine = engine.clone();
        let notify = notify.clone();
        std::thread::spawn(move || {
            let ctx = engine.ctx().clone();
            let _ = tx.send(engine.block_on(crate::tools::run(&ctx, "conv", &call(&notify, json!({"x": 1})), &Default::default(), true)));
        });
    }
    assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()));
    let a = engine.state().approvals[0].clone();
    assert!(a.label.starts_with("Kalender Demo"), "{}", a.label);
    engine.call("tool.answer", json!({"id": a.id, "allow": false})).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "denied");

    // Off and on again.
    engine.call("extension.toggle", json!({"name": "kalender-demo", "enabled": false})).unwrap();
    assert_eq!(engine.state().connectors.iter().find(|k| k.id == conn.id).unwrap().status, "off");
    engine.call("extension.toggle", json!({"name": "kalender-demo", "enabled": true})).unwrap();
    assert_eq!(engine.state().connectors.iter().find(|k| k.id == conn.id).unwrap().status, "connected");

    // Removing takes its folder, connector and rules along.
    let folder = engine.ctx().0.ext.dir.join("kalender-demo");
    assert!(folder.join("manifest.json").is_file());
    engine.call("extension.remove", json!({"name": "kalender-demo"})).unwrap();
    let s = engine.state();
    assert!(s.connectors.is_empty() && s.extensions.installed.is_empty());
    assert!(!s.settings.trust_rules.iter().any(|r| r.text == "nie Termine löschen"));
    assert!(!folder.exists());
    assert!(engine.call("extension.remove", json!({"name": "kalender-demo"})).is_err());

    // Cancelling drops the staged copy.
    engine.call("extension.inspect", json!({"path": path.display().to_string()})).unwrap();
    engine.call("extension.cancel", json!({})).unwrap();
    assert!(engine.state().extensions.offer.is_none());
    let staging = engine.ctx().0.ext.dir.join(".staging");
    assert!(std::fs::read_dir(&staging).map(|d| d.count()).unwrap_or(0) == 0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hostile_bundles_are_refused_before_anything_runs() {
    let engine = engine();
    let dir = scratch("mcpb-bad");
    let inspect = |bytes: Vec<u8>| {
        let p = dir.join(format!("{}.mcpb", crate::new_id()));
        std::fs::write(&p, bytes).unwrap();
        engine.call("extension.inspect", json!({"path": p.display().to_string()}))
    };
    let good = manifest("${__dirname}/server/fake_server", json!(["darwin", "win32", "linux"])).to_string();
    // A path that climbs out of the package, an absolute one, a program outside it.
    for name in ["../ausbruch.txt", "/tmp/aiscraft-absolut.txt", "server/../../x"] {
        let err = inspect(zip(&[("manifest.json", good.as_bytes()), (name, b"x")])).unwrap_err();
        assert!(err.message.contains("unzulässiger Pfad"), "{name}: {}", err.message);
    }
    assert!(!dir.join("ausbruch.txt").exists() && !std::env::temp_dir().join("ausbruch.txt").exists());
    for command in ["/bin/sh", "${__dirname}/../../bin/sh", "sh -c 'rm -rf ~'"] {
        let m = manifest(command, json!(["darwin", "win32", "linux"])).to_string();
        let err = inspect(zip(&[("manifest.json", m.as_bytes()), ("server/fake_server", b"x")])).unwrap_err();
        assert!(err.message.contains("Programm"), "{command}: {}", err.message);
    }
    let other = if cfg!(target_os = "linux") { "darwin" } else { "linux" };
    let m = manifest("${__dirname}/server/fake_server", json!([other])).to_string();
    assert_eq!(inspect(zip(&[("manifest.json", m.as_bytes()), ("server/fake_server", b"x")])).unwrap_err().kind, aiscraft_core::ErrorKind::Unsupported);
    assert!(inspect(b"kein zip".to_vec()).is_err());
    assert!(inspect(zip(&[("manifest.json", b"{kaputt")])).is_err());
    assert!(inspect(zip(&[("readme.txt", b"x")])).is_err());
    let mut m = manifest("${__dirname}/server/fake_server", json!(["darwin", "win32", "linux"]));
    m["manifest_version"] = json!("9.9");
    assert!(inspect(zip(&[("manifest.json", m.to_string().as_bytes()), ("server/fake_server", b"x")])).is_err());
    assert!(engine.call("extension.inspect", json!({"path": "/gibt/es/nicht.mcpb"})).is_err());
    let s = engine.state();
    assert!(s.extensions.offer.is_none() && s.connectors.is_empty());
    let staging = engine.ctx().0.ext.dir.join(".staging");
    assert!(std::fs::read_dir(&staging).map(|d| d.count()).unwrap_or(0) == 0, "refused packages leave nothing behind");
    let _ = std::fs::remove_dir_all(dir);
}
