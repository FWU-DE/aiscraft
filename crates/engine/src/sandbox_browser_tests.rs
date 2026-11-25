//! The browser in the private computer (#107): started only when needed and locked down on every
//! start, Nod's rule for its steps (reading runs, changing steps go through the trust check with
//! page, form target and values, background work refuses), vault values entered by the app,
//! downloads only through the explicit step, the take-over that pauses the model, „Namen
//! schützen“ on page content. A fake machine plays Nod's worker: no test starts a computer,
//! container or browser.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::sandbox::browser::{CLICK, DOWNLOAD, OPEN, SCREENSHOT, SNAPSHOT, TAKEOVER, TYPE, WAIT};
use crate::sandbox::container as k;
use crate::sandbox::runtime::{FAKE_FORM, FAKE_PNG, Fake, FakeCall};
use crate::sandbox_tests::{answer, call, direct, installed, run_async};
use crate::{Engine, Options};

/// Installed and on, with the browser switched on.
fn with_browser() -> (Engine, Arc<Fake>) {
    let (engine, fake) = installed();
    engine.call("sandbox.set", json!({"browser": true})).unwrap();
    assert!(engine.state().sandbox.browser);
    (engine, fake)
}

/// Every `docker run` of the browser image.
fn browser_starts(fake: &Fake) -> Vec<Vec<String>> {
    let image = crate::sandbox::runtime::Workspace::windows_browser().image;
    fake.container_starts().into_iter().filter(|a| a.contains(&image)).collect()
}

/// The actions the fake browser was asked to do (`act`), in order.
fn actions(fake: &Fake) -> Vec<String> {
    fake.browser_requests().iter().filter(|r| r["method"] == "act").map(|r| r["input"]["action"].as_str().unwrap_or("").to_string()).collect()
}

/// Quality gate (AGENTS.md §6): the browser is off until the person switches it on, starts only
/// with the first browser step (never with the computer, never with a workspace command), every
/// start is locked down like the workspace (no capabilities, no new privileges, user 1000, a
/// read-only root, limits), shares no folder with the workspace, has its own network kept to
/// public addresses, and loads only the pinned image; it ends when idle and starts again on use.
#[test]
fn the_browser_starts_only_when_needed_and_every_start_is_locked_down() {
    let (engine, fake) = installed();
    let ctx = engine.ctx().clone();
    // Off by default: no tools, a call is refused, nothing starts.
    assert!(!engine.state().sandbox.browser);
    assert!(!crate::tools::specs(&ctx).iter().any(|t| t.name.starts_with("browser_")));
    let (text, err, _) = direct(&engine, OPEN, json!({"url": "https://lernen.example.org"}));
    assert!(err && text.contains("Browser") && text.contains("aus"), "{text}");
    engine.call("sandbox.set", json!({"browser": true})).unwrap();
    let names: Vec<String> = crate::tools::specs(&ctx).into_iter().map(|t| t.name).filter(|n| n.starts_with("browser_")).collect();
    assert_eq!(names, [OPEN, SNAPSHOT, CLICK, TYPE, WAIT, SCREENSHOT, DOWNLOAD, TAKEOVER]);
    // A workspace command does not start it.
    direct(&engine, crate::sandbox::LIST, json!({}));
    assert!(browser_starts(&fake).is_empty() && !engine.state().sandbox.browser_running, "switched on is not started");
    // Reading before anything is open says so and starts nothing.
    let (text, err, _) = direct(&engine, SNAPSHOT, json!({}));
    assert!(err && text.contains("browser_open"), "{text}");
    assert!(browser_starts(&fake).is_empty());
    // Private addresses are refused before anything starts.
    for bad in [
        "http://192.168.1.10/",
        "http://localhost:8080",
        "http://10.0.0.1",
        "http://[::1]/",
        "file:///etc/passwd",
        "http://schulserver/",
        "https://user:pw@example.org",
    ] {
        let (text, err, _) = direct(&engine, OPEN, json!({"url": bad}));
        assert!(err, "{bad}: {text}");
    }
    assert!(browser_starts(&fake).is_empty(), "nothing started for a refused address");

    // The first open starts it: image checked and loaded, network kept to public addresses.
    let (text, err, outcome) = direct(&engine, OPEN, json!({"url": "https://lernen.example.org/login"}));
    assert!(!err && outcome == "ok", "{text}");
    assert!(text.starts_with("<untrusted_tool_result tool=\"browser_open\">") && text.contains("Anmeldung Lernplattform"), "{text}");
    assert!(engine.state().sandbox.browser_running);
    let prepared: Vec<Vec<String>> = fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Exec(a) if a.get(2).map(String::as_str) == Some(k::BROWSER_PREPARE_SCRIPT) => Some(a),
            _ => None,
        })
        .collect();
    let b = crate::sandbox::runtime::Workspace::windows_browser();
    assert_eq!(prepared.len(), 1);
    assert_eq!(&prepared[0][4..], [b.image.clone(), b.tar.to_string(), b.tar_sha256.to_string()], "only the pinned image, checked");
    for need in [
        "172.16.0.0/12",
        "10.0.0.0/8",
        "192.168.0.0/16",
        "169.254.0.0/16",
        "127.0.0.0/8",
        "DOCKER-USER -i aiscraft-br0 -j AISCRAFT-BROWSER",
        "enable_icc=false",
    ] {
        assert!(k::BROWSER_PREPARE_SCRIPT.contains(need), "{need}");
    }
    // The workspace's preparation keeps the browser image (it used to delete it).
    let ws_prep = fake.calls().into_iter().rev().find_map(|c| match c {
        FakeCall::Exec(a) if a.get(2).map(String::as_str) == Some(k::PREPARE_SCRIPT) => Some(a),
        _ => None,
    });
    assert_eq!(ws_prep.unwrap().last(), Some(&b.image), "the workspace's preparation keeps the browser's image");
    assert!(!k::PREPARE_SCRIPT.contains("browser.tar.zst"));

    // Reading, waiting and more steps reuse the running browser.
    assert!(!direct(&engine, SNAPSHOT, json!({})).1);
    assert!(!direct(&engine, WAIT, json!({"load": "networkidle"})).1);
    assert_eq!(browser_starts(&fake).len(), 1, "started once");

    // Idle: it ends (not before the time), and the next step starts it again.
    let ctx2 = ctx.clone();
    assert!(!engine.block_on(async move { crate::sandbox::browser::reap(&ctx2, Duration::from_secs(3600)).await }), "used just now");
    let ctx2 = ctx.clone();
    assert!(engine.block_on(async move { crate::sandbox::browser::reap(&ctx2, Duration::ZERO).await }));
    assert!(!engine.state().sandbox.browser_running);
    assert!(fake.calls().iter().any(|c| matches!(c, FakeCall::Exec(a) if a.join(" ") == "docker rm -f aiscraft-browser")));
    assert!(direct(&engine, SNAPSHOT, json!({})).1, "nothing is kept when it ends");
    assert!(!direct(&engine, OPEN, json!({"url": "https://lernen.example.org/login"})).1);
    assert_eq!(browser_starts(&fake).len(), 2);
    // `browser.stop` ends it now; switching the browser off ends it too.
    assert_eq!(engine.call("browser.stop", json!({})).unwrap()["stopped"], true);
    assert_eq!(engine.call("browser.stop", json!({})).unwrap()["stopped"], false);
    assert!(!direct(&engine, OPEN, json!({"url": "https://lernen.example.org/login"})).1);
    engine.call("sandbox.set", json!({"browser": false})).unwrap();
    assert!(!engine.state().sandbox.browser_running && !crate::tools::specs(&ctx).iter().any(|t| t.name == OPEN));

    // Every start: locked down, no mount, its own network, the pinned image.
    let starts = browser_starts(&fake);
    assert_eq!(starts.len(), 3);
    for argv in &starts {
        let j = argv.join(" ");
        for need in [
            "--name aiscraft-browser",
            "--network aiscraft-browser",
            "--cap-drop ALL",
            "--security-opt no-new-privileges",
            "--user 1000:1000",
            "--read-only",
            "--memory 2g",
            "--memory-swap 2g",
            "--cpus 2",
            "--pids-limit 768",
            "--init",
        ] {
            assert!(j.contains(need), "{need} missing: {j}");
        }
        assert!(!argv.iter().any(|a| a == "--mount" || a == "-v" || a == "--volume"), "no shared folder: {j}");
        assert!(!j.contains("/work") && !j.contains("--privileged") && !j.contains("docker.sock") && !j.contains("aiscraft-net"), "{j}");
        assert_eq!(argv.last(), Some(&b.image));
    }
    // Every call into it runs as its unprivileged user.
    for c in fake.calls() {
        if let FakeCall::Exec(a) = c
            && a.get(1).map(String::as_str) == Some("exec")
            && a.get(5).map(String::as_str) == Some(k::BROWSER_NAME)
        {
            assert_eq!(&a[2..5], ["-i", "--user", "node"], "{a:?}");
        }
    }
}

/// Nod's rule for browser steps (AGENTS.md 2.7): opening and reading run without a question;
/// clicking and typing ask with the page, the form's target and the value, a no does nothing;
/// background work refuses; vault values are entered by the app and never reach the model;
/// downloads land in the working folder only through the explicit step.
#[test]
fn changing_browser_steps_go_through_the_trust_check() {
    let (engine, fake) = with_browser();
    let ctx = engine.ctx().clone();
    // Background work: refused without a question, nothing starts.
    let (text, err, outcome) =
        engine.block_on(crate::tools::run(&ctx, "conv", &call(OPEN, json!({"url": "https://lernen.example.org"})), &Default::default(), false));
    assert!(err && outcome == "denied" && text.contains("Hintergrund"), "{text}");
    assert!(browser_starts(&fake).is_empty());

    // Reading runs without asking.
    assert!(!direct(&engine, OPEN, json!({"url": "https://lernen.example.org/login"})).1);
    assert!(engine.state().approvals.is_empty());

    // Typing asks, with the page, where the form sends and the value; a no types nothing.
    let rx = run_async(&engine, call(TYPE, json!({"element": "@e2", "eingabe": "frau.ada", "seite": "https://gefaelscht.example"})));
    let a = answer(&engine, false);
    assert_eq!(a.arguments["seite"], "https://lernen.example.org/login", "the app names the page, not the model");
    assert_eq!(a.arguments["formularziel"], format!("POST {FAKE_FORM}"));
    let shown: Vec<(String, String)> = a.highlights.iter().map(|h| (h.label.clone(), h.value.clone())).collect();
    for want in [("Seite", "https://lernen.example.org/login".to_string()), ("Formularziel", format!("POST {FAKE_FORM}")), ("Eingabe", "frau.ada".to_string())]
    {
        assert!(shown.contains(&(want.0.to_string(), want.1.clone())), "{want:?} in {shown:?}");
    }
    assert!(a.label.contains("Browser") && a.label.contains("Eingeben"), "{}", a.label);
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "denied");
    assert!(!actions(&fake).contains(&"fill".to_string()), "a no types nothing");
    // Yes: it types.
    let rx = run_async(&engine, call(TYPE, json!({"element": "@e2", "eingabe": "frau.ada"})));
    answer(&engine, true);
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("Eingegeben in @e2"), "{text}");
    let fill = fake.browser_requests().into_iter().rfind(|r| r["input"]["action"] == "fill").unwrap();
    assert_eq!((fill["input"]["selector"].as_str(), fill["input"]["text"].as_str()), (Some("@e2"), Some("frau.ada")));
    // Clicking asks too, with page and form target.
    let rx = run_async(&engine, call(CLICK, json!({"element": "@e4"})));
    let a = answer(&engine, true);
    assert_eq!(a.arguments["formularziel"], format!("POST {FAKE_FORM}"));
    assert!(!rx.recv_timeout(Duration::from_secs(30)).unwrap().1);
    assert!(actions(&fake).contains(&"click".to_string()));

    // A vault password: entered by the app through Nod's credential entry; the model sees no value.
    engine.call("vault.add", json!({"name": "Lernplattform", "username": "frau.ada", "password": "geheim-123"})).unwrap();
    let rx = run_async(&engine, call(TYPE, json!({"element": "@e3", "eingabe": "{{vault:lernplattform}}", "absenden": true})));
    let a = answer(&engine, true);
    assert!(!a.arguments.to_string().contains("geheim-123"), "the question shows the placeholder");
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && !text.contains("geheim-123") && text.contains("abgeschickt"), "{text}");
    let inject = fake.browser_requests().into_iter().find(|r| r["method"] == "injectCredentials").expect("entered by the app");
    assert_eq!((inject["input"]["kind"].as_str(), inject["input"]["password"].as_str()), (Some("password"), Some("geheim-123")));
    assert!(!fake.browser_requests().iter().any(|r| r["method"] == "act" && r.to_string().contains("geheim-123")), "never typed as text");
    // The user name of a vault entry goes in as the name.
    let rx = run_async(&engine, call(TYPE, json!({"element": "@e2", "eingabe": "{{vault:lernplattform.username}}"})));
    answer(&engine, true);
    assert!(!rx.recv_timeout(Duration::from_secs(30)).unwrap().1);
    assert!(fake.browser_requests().iter().any(|r| r["method"] == "injectCredentials" && r["input"]["kind"] == "identifier"));

    // A download lands in /work/downloads only through the explicit step, after the yes.
    let rx = run_async(&engine, call(DOWNLOAD, json!({"element": "@e9"})));
    answer(&engine, true);
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("/work/downloads/stundenplan.pdf"), "{text}");
    assert_eq!(fake.file("/work/conv/downloads/stundenplan.pdf").unwrap(), b"%PDF-1.7 Stundenplan");
    // A picture for the person: an attachment of the answer.
    let (text, err, _) = direct(&engine, SCREENSHOT, json!({}));
    assert!(!err && text.contains("Anhang"), "{text}");
    let files = crate::tools::take_files(&ctx, "conv");
    assert_eq!(files.len(), 1);
    assert_eq!(std::fs::read(files[0].local_path.as_ref().unwrap()).unwrap(), FAKE_PNG);

    // Every step is in the action log.
    let log = engine.call("log.list", json!({})).unwrap();
    assert!(log["entries"].as_array().unwrap().iter().any(|e| e["name"] == "Browser · Eingeben"), "{log}");
    assert!(!log.to_string().contains("geheim-123"));
}

/// Take-over (#107): the model hands the browser to the teacher; the task waits, the panel gets
/// live pictures, the teacher's input goes to that browser, every other browser step is refused
/// while the teacher has it, and only „Zurück an AIS.ChatCraft“ lets the model go on.
#[test]
fn the_takeover_pauses_the_model_until_the_teacher_returns() {
    let (engine, fake) = with_browser();
    assert!(engine.call("browser.input", json!({"type": "text", "text": "a"})).is_err(), "no input without a take-over");
    assert_eq!(engine.call("browser.return", json!({})).unwrap()["returned"], false);
    assert!(direct(&engine, TAKEOVER, json!({"grund": "Bitte anmelden"})).1, "nothing open, nothing to hand over");
    direct(&engine, OPEN, json!({"url": "https://lernen.example.org/login"}));

    let rx = run_async(&engine, call(TAKEOVER, json!({"grund": "Bitte melde dich mit deinem Schulkonto an."})));
    assert!(engine.wait_for(Duration::from_secs(30), |s| s.sandbox.takeover.as_ref().is_some_and(|t| t.frame > 0)), "the panel gets a picture");
    let t = engine.state().sandbox.takeover.clone().unwrap();
    assert_eq!((t.conversation_id.as_str(), t.url.as_str()), ("conv", "https://lernen.example.org/login"));
    assert!(t.reason.contains("Schulkonto"));
    let (n, bytes) = engine.browser_frame().unwrap();
    assert!(n >= 1 && bytes.as_slice() == FAKE_PNG);
    assert!(engine.state().approvals.is_empty(), "handing over asks nothing");
    assert!(fake.browser_requests().iter().any(|r| r["method"] == "hold" && r["held"] == true), "the browser is held for the teacher");

    // The teacher's input reaches the browser through Nod's view.
    engine.call("browser.input", json!({"type": "mouse", "event": "down", "x": 0.5, "y": 0.25})).unwrap();
    engine.call("browser.input", json!({"type": "text", "text": "frau.ada"})).unwrap();
    engine.call("browser.input", json!({"type": "key", "key": "Enter"})).unwrap();
    engine.call("browser.input", json!({"type": "wheel", "dx": 0, "dy": 300})).unwrap();
    for bad in [json!({"type": "mouse", "event": "down", "x": 2, "y": 0}), json!({"type": "key", "key": "Enter; rm"}), json!({"type": "nope"})] {
        assert!(engine.call("browser.input", bad.clone()).is_err(), "{bad}");
    }
    let inputs: Vec<Value> = fake
        .calls()
        .into_iter()
        .filter_map(|c| match c {
            FakeCall::Exec(a) if a.get(1).map(String::as_str) == Some("exec") && a.get(10).is_some_and(|x| !x.is_empty()) => serde_json::from_str(&a[10]).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(inputs.len(), 4);
    assert_eq!(inputs[0]["input"], json!({"type": "mouse", "event": "down", "x": 0.5, "y": 0.25, "button": "left"}));
    assert_eq!(inputs[1]["input"], json!({"type": "text", "text": "frau.ada"}));

    // While the teacher has control the model does nothing in the browser: every step is refused.
    let acts = actions(&fake).len();
    for (name, args) in [(SNAPSHOT, json!({})), (OPEN, json!({"url": "https://example.org"})), (CLICK, json!({"element": "@e4"}))] {
        let (text, err, _) = direct(&engine, name, args);
        assert!(err && text.contains("steuert gerade den Browser"), "{name}: {text}");
    }
    assert_eq!(actions(&fake).len(), acts);
    assert!(engine.call("browser.stop", json!({})).is_err(), "not ended under the teacher's hands");
    let ctx = engine.ctx().clone();
    assert!(!engine.block_on(async move { crate::sandbox::browser::reap(&ctx, Duration::ZERO).await }), "not idle while held");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the task still waits");

    // The teacher gives it back: the model goes on and looks again.
    assert_eq!(engine.call("browser.return", json!({})).unwrap()["returned"], true);
    let (text, err, outcome) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && outcome == "ok" && text.contains("zurückgegeben") && text.contains("browser_snapshot"), "{text}");
    assert!(engine.state().sandbox.takeover.is_none() && engine.browser_frame().is_none());
    assert!(fake.browser_requests().iter().any(|r| r["method"] == "hold" && r["held"] == false));
    assert!(!direct(&engine, SNAPSHOT, json!({})).1);

    // The test hook the UI gate uses shows the panel without a computer; only in memory.
    engine.show_browser_takeover("conv", "Captcha lösen", "https://example.org", FAKE_PNG.to_vec()).unwrap();
    assert!(engine.state().sandbox.takeover.is_some());
    engine.call("browser.return", json!({})).unwrap();
}

/// An OpenAI-compatible server: on „Schau auf der Lernplattform“ the model opens the page, then
/// answers; every request is recorded.
fn browsing_model(engine: &Engine) -> (String, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 1 << 16];
                    loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let s = String::from_utf8_lossy(&buf).to_string();
                        if let Some(h) = s.find("\r\n\r\n") {
                            let len = s[..h]
                                .lines()
                                .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= h + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    seen.lock().unwrap().push(req.clone());
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"m","context_length":100000}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let events = if req.contains("\"role\":\"tool\"") {
                        vec![json!({"choices":[{"delta":{"content": "Gelesen."}}]})]
                    } else if req.contains("Schau auf der Lernplattform") {
                        vec![
                            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"browser_open","arguments":"{\"url\":\"https://lernen.example.org/login\"}"}}]}}]}),
                            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
                        ]
                    } else {
                        vec![json!({"choices":[{"delta":{"content": "Hallo."}}]})]
                    };
                    for e in events {
                        let _ = sock.write_all(format!("data: {e}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

/// Page content is untrusted data and reaches the model through „Namen schützen“ like every tool
/// result: a name on the page becomes its placeholder before the model sees it.
#[test]
fn page_content_passes_name_protection() {
    let (engine, _fake) = with_browser();
    let (url, seen) = browsing_model(&engine);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("privacy.list_add", json!({"name": "7b", "text": "Lea Müller"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Schau auf der Lernplattform nach"})).unwrap();
    let posts: Vec<String> = seen.lock().unwrap().iter().filter(|r| r.starts_with("POST")).cloned().collect();
    let with_result = posts.iter().find(|r| r.contains("\"role\":\"tool\"")).expect("the page went back to the model");
    assert!(with_result.contains("Anmeldung Lernplattform") && !with_result.contains("Müller"), "{with_result}");
    assert!(with_result.contains("untrusted_tool_result"), "marked as data");
}

/// An engine in memory never starts a browser: without a machine every browser step refuses.
#[test]
fn an_engine_in_memory_starts_no_browser() {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    assert!(engine.call("sandbox.set", json!({"browser": true})).is_ok());
    let (text, err, _) = direct(&engine, OPEN, json!({"url": "https://example.org"}));
    assert!(err && !text.is_empty());
    assert_eq!(engine.call("browser.stop", json!({})).unwrap()["stopped"], false);
}

/// The live check on a real computer: create it from the downloads, load and start the browser
/// exactly as the app does, open a public page, read its snapshot, check the container runs as
/// user 1000 without capabilities, cannot write its root and reaches no private address, then
/// remove the computer again.
#[cfg(any(windows, target_os = "linux"))]
fn live_browser(machine: &dyn crate::sandbox::runtime::SandboxRuntime, files: Vec<std::path::PathBuf>, dir: &std::path::Path) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let t = Duration::from_secs(900);
        machine.probe().await.expect("the computer can run it");
        if machine.exists().await {
            machine.destroy().await.unwrap();
        }
        machine.create(files, dir.join("machine")).await.expect("created");
        let ws = machine.workspace().unwrap();
        let b = machine.browser().unwrap();
        let prep = machine.exec(k::machine_script(k::PREPARE_SCRIPT, &[&ws.image, "", ws.tar, ws.tar_sha256, "0", &b.image]), Vec::new(), t, 65536).await.unwrap();
        assert!(prep.ok(), "{prep:?}");
        let bprep = machine.exec(k::machine_script(k::BROWSER_PREPARE_SCRIPT, &[&b.image, b.tar, b.tar_sha256]), Vec::new(), t, 65536).await.unwrap();
        assert!(bprep.ok(), "{bprep:?}");
        let run = machine.exec(k::browser_run(&b.image), Vec::new(), t, 65536).await.unwrap();
        assert!(run.ok(), "{run:?}");
        let ready = machine.exec(k::machine_script(k::BROWSER_READY_SCRIPT, &[k::READY_JS]), Vec::new(), t, 65536).await.unwrap();
        assert!(ready.ok(), "{ready:?}");
        let ns = format!("sk-{}", "0".repeat(24));
        let ask = |action: Value| {
            let id = crate::new_id();
            let line = json!({"type": "request", "request": {"id": id, "namespace": ns, "method": "act", "threadId": "live", "input": action}}).to_string();
            (id, k::browser_exec(&line, "", ""))
        };
        let (id, argv) = ask(json!({"action": "open", "url": "https://example.org/", "locale": "de-DE"}));
        let out = machine.exec(argv, Vec::new(), Duration::from_secs(120), 1 << 24).await.unwrap();
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        eprintln!("{text}{}", String::from_utf8_lossy(&out.stderr));
        assert!(text.contains(&id) && text.contains("\"ok\":true"), "{out:?}");
        let (_, argv) = ask(json!({"action": "snapshot"}));
        let out = machine.exec(argv, Vec::new(), Duration::from_secs(120), 1 << 24).await.unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("Example Domain"), "{out:?}");
        // No rights, a read-only root, no private address.
        let probe = "id -u; grep CapEff /proc/self/status; grep -q ' / overlay ro,' /proc/mounts || echo SCHREIBBAR; node -e 'require(\"net\").connect(80,\"192.168.0.1\").on(\"connect\",()=>{console.log(\"PRIVAT\");process.exit()}).on(\"error\",()=>process.exit()).setTimeout(3000,()=>process.exit())'";
        let who = machine
            .exec(vec!["docker".into(), "exec".into(), k::BROWSER_NAME.into(), "sh".into(), "-c".into(), probe.into()], Vec::new(), Duration::from_secs(60), 65536)
            .await
            .unwrap();
        let w = String::from_utf8_lossy(&who.stdout).to_string();
        assert!(w.starts_with("1000\n") && w.contains("CapEff:\t0000000000000000"), "{w}");
        assert!(!w.contains("SCHREIBBAR") && !w.contains("PRIVAT"), "{w}");
        machine.destroy().await.unwrap();
    });
}

/// Live (Windows, not in CI): a real WSL2 distribution `aiscraft-livetest` from Nod's runtime,
/// the browser started as the app does, a page opened and read. Downloads (1.3 GB) and installs:
/// run only on request, with `AISCRAFT_SANDBOX_IMAGE` set to an already downloaded
/// `nod-Runtime-win32-x64.tar.gz` or unset to download it.
///
/// `cargo test -p aiscraft-engine real_browser_opens_a_page_without_rights -- --ignored --nocapture`
#[cfg(windows)]
#[test]
#[ignore]
fn real_browser_opens_a_page_without_rights() {
    let dir = std::env::temp_dir().join(format!("aiscraft-browser-live-{}", crate::new_id()));
    let files = match std::env::var_os("AISCRAFT_SANDBOX_IMAGE") {
        Some(p) => vec![std::path::PathBuf::from(p)],
        None => crate::sandbox_tests::live_downloads(&dir),
    };
    live_browser(&crate::sandbox::wsl::Wsl::new("aiscraft-livetest".into()), files, &dir);
    let _ = std::fs::remove_dir_all(dir);
}

/// Live (Linux with KVM, QEMU and ssh, not in CI): the same in a real Lima VM; with
/// `AISCRAFT_SANDBOX_LIMA` and `AISCRAFT_SANDBOX_NOD` set to downloaded files nothing is
/// downloaded.
///
/// `cargo test -p aiscraft-engine real_browser_opens_a_page_without_rights -- --ignored --nocapture`
#[cfg(target_os = "linux")]
#[test]
#[ignore]
fn real_browser_opens_a_page_without_rights() {
    let dir = std::env::temp_dir().join(format!("aiscraft-browser-live-{}", crate::new_id()));
    let files = match (std::env::var_os("AISCRAFT_SANDBOX_LIMA"), std::env::var_os("AISCRAFT_SANDBOX_NOD")) {
        (Some(l), Some(n)) => vec![l.into(), n.into()],
        _ => crate::sandbox_tests::live_downloads(&dir),
    };
    live_browser(&crate::sandbox::lima::Lima::new("aiscraft-livetest".into(), dir.clone()), files, &dir);
    let _ = std::fs::remove_dir_all(dir);
}
