//! Plug-ins in the Agent Plugins format (#28): loaded from a folder, tools follow the approval
//! rules, skills reach recall, broken parts are left out, plug-ins dropped into the folder by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::tests::{engine, scratch};

const PLUGIN: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
const MCP: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A sample plug-in: one working server, one that points out of the plug-in, a skill, a broken
/// skill, Nod's display name and a prohibition.
fn sample(dir: &Path, server: &[u8]) -> PathBuf {
    let root = dir.join("schulkalender");
    write(
        &root.join("plugin.json"),
        &json!({
            "$schema": PLUGIN,
            "name": "schulkalender",
            "version": "0.2.0",
            "description": "Termine und Elternbriefe",
            "author": {"name": "Beispiel-Schule"},
            "keywords": ["schule"],
            "zusatz": true,
            "extensions": {
                "io.n8n.nod": {"app": "./io.n8n.nod/app.json"},
                "ai.uds.aischatcraft": {"rules": [{"kind": "forbid", "text": "nie Termine am Wochenende"}]}
            }
        })
        .to_string(),
    );
    write(&root.join("io.n8n.nod/app.json"), &json!({"displayName": "Schulkalender"}).to_string());
    write(
        &root.join("mcp.json"),
        &json!({
            "$schema": MCP,
            "mcpServers": {
                "kalender": {"type": "stdio", "command": "./bin/fake_server", "args": ["--daten", "${PLUGIN_DATA}"], "env": {"FAKE_MCP_MODE": ""}},
                "ausbruch": {"type": "stdio", "command": "/bin/sh", "args": ["-c", "echo"]},
                "alt": {"type": "sse", "url": "https://example.org/sse"}
            }
        })
        .to_string(),
    );
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(root.join("bin/fake_server"), server).unwrap();
    write(
        &root.join("skills/elternbrief/SKILL.md"),
        "---\nname: elternbrief\ndescription: Elternbrief zu einem Schultermin schreiben\n---\n\n1. Termin im Kalender nachsehen\n2. Brief in einfacher Sprache\n",
    );
    write(&root.join("skills/kaputt/SKILL.md"), "---\nname: anders\ndescription: x\n---\n");
    root
}

fn call(name: &str, args: Value) -> aiscraft_core::ToolCall {
    aiscraft_core::ToolCall { id: "c".into(), name: name.into(), arguments: args, outcome: None }
}

#[test]
fn a_sample_plugin_loads_and_its_tool_follows_the_approval_rules() {
    let Some(bin) = crate::trust_tests::fake_server() else { return };
    let server = std::fs::read(bin).unwrap();
    let dir = scratch("plugin");
    let root = sample(&dir, &server);
    #[cfg(unix)]
    {
        // A link that leads out of the plug-in is left out, never copied.
        std::fs::write(dir.join("geheim.txt"), "geheim").unwrap();
        std::os::unix::fs::symlink(dir.join("geheim.txt"), root.join("skills/elternbrief/notiz.txt")).unwrap();
    }
    let engine = engine();
    let offer = engine.call("extension.inspect", json!({"path": root.display().to_string()})).unwrap();
    assert_eq!(offer["kind"], "plugin");
    assert_eq!(offer["display_name"], "Schulkalender", "Nod's display name");
    assert_eq!(offer["skills"], json!(["elternbrief"]));
    let warnings = offer["warnings"].to_string();
    for w in ["ausbruch", "alt", "kaputt", "zusatz"] {
        assert!(warnings.contains(w), "{w} named: {warnings}");
    }
    #[cfg(unix)]
    assert!(warnings.contains("Verknüpfungen nach außerhalb"), "{warnings}");
    assert!(engine.state().connectors.is_empty(), "nothing runs before the yes");
    engine.call("extension.install", json!({"id": offer["id"]})).unwrap();
    let installed = engine.ctx().0.ext.dir.join("schulkalender");
    assert!(installed.join("skills/elternbrief/SKILL.md").is_file());
    assert!(!installed.join("skills/elternbrief/notiz.txt").exists());

    let s = engine.state();
    assert_eq!(s.connectors.len(), 1, "only the working server");
    let conn = s.connectors[0].clone();
    assert_eq!(conn.status, "connected", "{:?}", conn.error);
    assert!(s.settings.trust_rules.iter().any(|r| r.text == "nie Termine am Wochenende" && r.connector == conn.id));
    assert_eq!(s.extensions.installed[0].skills, vec!["elternbrief"]);

    // The tool follows the rules: a read runs, a change asks and a no keeps it from running.
    let ctx = engine.ctx().clone();
    let echo = conn.tools.iter().find(|t| t.name == "echo").unwrap().llm_name.clone();
    let notify = conn.tools.iter().find(|t| t.name == "notify").unwrap().llm_name.clone();
    assert!(crate::tools::specs(&ctx).iter().any(|t| t.name == notify && t.description.contains("Freigabe")));
    let (text, err, _) = engine.block_on(crate::tools::run(&ctx, "conv", &call(&echo, json!({"text": "Projektwoche"})), &Default::default(), true));
    assert!(!err && text.contains("Projektwoche"));
    let (tx, rx) = std::sync::mpsc::channel();
    {
        let engine = engine.clone();
        std::thread::spawn(move || {
            let ctx = engine.ctx().clone();
            let _ = tx.send(engine.block_on(crate::tools::run(&ctx, "conv", &call(&notify, json!({"termin": "Sportfest"})), &Default::default(), true)));
        });
    }
    assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()));
    let a = engine.state().approvals[0].clone();
    assert!(a.label.starts_with("Schulkalender"));
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "ok");

    // Its skill is a hint for fitting requests, marked as coming from the plug-in.
    let hits = crate::skills::relevant(&ctx, "Schreib einen Elternbrief zum Sportfest", 3);
    let hit = hits.iter().find(|s| s.title == "elternbrief").expect("plug-in skill in recall");
    assert!(hit.steps.contains("Termin im Kalender") && hit.steps.contains("Plug-in"), "{}", hit.steps);
    // Switched off, it is gone from recall and tools.
    engine.call("extension.toggle", json!({"name": "schulkalender", "enabled": false})).unwrap();
    assert!(crate::skills::relevant(&ctx, "Elternbrief schreiben", 3).iter().all(|s| s.title != "elternbrief"));
    assert!(!crate::tools::specs(&ctx).iter().any(|t| t.name.starts_with(&conn.id)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_plugin_dropped_into_the_folder_waits_until_it_is_switched_on() {
    let Some(bin) = crate::trust_tests::fake_server() else { return };
    let server = std::fs::read(bin).unwrap();
    let engine = engine();
    let folder = engine.ctx().0.ext.dir.clone();
    sample(&folder, &server);
    engine.call("extension.rescan", json!({})).unwrap();
    let s = engine.state();
    let found = s.extensions.installed.iter().find(|x| x.name == "schulkalender").unwrap();
    assert!(found.found && !found.enabled);
    assert!(s.connectors.is_empty(), "a dropped-in plug-in does not start by itself");
    engine.call("extension.toggle", json!({"name": "schulkalender", "enabled": true})).unwrap();
    let s = engine.state();
    assert_eq!(s.connectors.len(), 1);
    assert_eq!(s.connectors[0].status, "connected", "{:?}", s.connectors[0].error);
    assert!(!s.extensions.installed[0].found);
}

#[test]
fn broken_plugins_are_refused() {
    let engine = engine();
    let dir = scratch("plugin-bad");
    let try_manifest = |m: Value| {
        let root = dir.join(crate::new_id());
        write(&root.join("plugin.json"), &m.to_string());
        write(&root.join("skills/ok/SKILL.md"), "---\nname: ok\ndescription: gut\n---\nText");
        engine.call("extension.inspect", json!({"path": root.display().to_string()}))
    };
    assert!(try_manifest(json!({"$schema": PLUGIN, "name": "ok-plugin"})).is_ok());
    engine.call("extension.cancel", json!({})).unwrap();
    for bad in [
        json!({"name": "ok-plugin"}),
        json!({"$schema": PLUGIN, "name": "Böse Name"}),
        json!({"$schema": PLUGIN, "name": "a--b"}),
        json!({"$schema": PLUGIN, "name": "ok", "author": {"name": "x", "token": "y"}}),
        json!({"$schema": PLUGIN, "name": "ok", "version": 2}),
        json!([1, 2]),
    ] {
        assert!(try_manifest(bad.clone()).is_err(), "{bad}");
    }
    // Neither tools nor skills: nothing to load.
    let root = dir.join("leer");
    write(&root.join("plugin.json"), &json!({"$schema": PLUGIN, "name": "leer"}).to_string());
    assert!(engine.call("extension.inspect", json!({"path": root.display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
