//! Computer use (#97) with a fake desktop: off by default, the hard block list and „Hier nie“,
//! opening for one request, a trust check with the window content for every change, the window
//! read again before the step, no background work. No real window is read or touched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use aiscraft_desktop::Act;
use aiscraft_desktop::fake::{FakeDesktop, control};
use serde_json::{Value, json};

use crate::computer::{KEY, LIST, OPEN, PRESS, READ, TYPE};
use crate::trust::{Evidence, OwnerMessage};
use crate::trust_tests::{Seen, is_trust_check, model, text};
use crate::{Engine, Options};

const WORD: &str = r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE";

fn call(name: &str, args: Value) -> aiscraft_core::ToolCall {
    aiscraft_core::ToolCall { id: "c".into(), name: name.into(), arguments: args, outcome: None }
}

fn ev(turn: &str, words: &str) -> Evidence {
    Evidence { messages: vec![OwnerMessage { n: 1, text: words.into(), answered: None }], turn: turn.into(), from_view: false }
}

/// An engine with a scripted model for the trust check and a fake desktop with one Word window.
/// The checker covers opening Word („Öffne Word“) and nothing else.
fn setup() -> (Engine, Arc<FakeDesktop>, Seen, String) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = model(&engine, |body| {
        if is_trust_check(body) && body.contains("computer_open") && body.contains("Öffne Word") {
            return text(
                &json!({"covered": true, "source": "message:1", "terms": [{"term": "Word", "value": "Word", "match": true}], "reason": "genannt"}).to_string(),
            );
        }
        text(&json!({"covered": false, "source": "", "terms": [], "reason": "Nicht ausdrücklich verlangt."}).to_string())
    });
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    let desk = Arc::new(FakeDesktop::new());
    let word = desk.add_window(
        WORD,
        "Elternbrief.docx – Word",
        vec![
            control("1", "Dokument", "Textbereich", Some("Liebe Eltern,"), &[Act::Type]),
            control("2", "Textfeld", "Kennwort", Some(""), &[Act::Type]),
            control("3", "Schaltfläche", "Speichern", None, &[Act::Press]),
        ],
    );
    engine.ctx().0.computer.set_desktop(desk.clone());
    (engine, desk, seen, word)
}

fn run(engine: &Engine, c: aiscraft_core::ToolCall, ev: &Evidence, interactive: bool) -> (String, bool, &'static str) {
    engine.block_on(crate::tools::run(engine.ctx(), "conv", &c, ev, interactive))
}

/// Run in the background; the test answers the question meanwhile.
fn run_async(engine: &Engine, c: aiscraft_core::ToolCall, ev: Evidence) -> std::sync::mpsc::Receiver<(String, bool, &'static str)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let engine = engine.clone();
    std::thread::spawn(move || {
        let _ = tx.send(engine.block_on(crate::tools::run(engine.ctx(), "conv", &c, &ev, true)));
    });
    rx
}

fn approval(engine: &Engine) -> crate::Approval {
    assert!(engine.wait_for(Duration::from_secs(30), |s| !s.approvals.is_empty()), "a question was asked");
    engine.state().approvals[0].clone()
}

fn checks(seen: &Seen) -> Vec<String> {
    seen.lock().unwrap().iter().filter(|b| is_trust_check(b)).cloned().collect()
}

/// Gate: without switching it on, no tool is offered and no window is listed or read; the
/// settings commands switch it and keep the person's own list.
#[test]
fn computer_use_is_off_until_switched_on_and_reads_nothing() {
    let (engine, desk, seen, word) = setup();
    let ctx = engine.ctx().clone();
    assert!(!engine.state().computer.enabled, "off by default");
    assert!(!crate::tools::specs(&ctx).iter().any(|t| t.name.starts_with("computer_")));
    let e = ev("t1", "Öffne Word und lies den Brief");
    for c in
        [call(LIST, json!({})), call(OPEN, json!({"app": "Word"})), call(READ, json!({"window": word})), call(KEY, json!({"window": word, "keys": "Strg+S"}))]
    {
        let (text, err, outcome) = run(&engine, c, &e, true);
        assert!(err && outcome == "denied" && text.contains("ist aus"), "{text}");
    }
    assert_eq!(desk.reads(), 0, "no window listed or read while it is off");
    assert!(engine.state().approvals.is_empty() && checks(&seen).is_empty(), "nothing asked, nothing checked");

    engine.call("computer.set", json!({"enabled": true})).unwrap();
    assert!(engine.state().computer.enabled);
    let names: Vec<String> = crate::tools::specs(&ctx).into_iter().map(|t| t.name).collect();
    for n in [LIST, OPEN, READ, PRESS, TYPE, KEY] {
        assert!(names.iter().any(|x| x == n), "{n} on offer");
    }
    // „Hier nie“: added once, removed only by name.
    engine.call("computer.never_add", json!({"name": "Outlook"})).unwrap();
    engine.call("computer.never_add", json!({"name": "outlook"})).unwrap();
    assert_eq!(engine.state().computer.never, ["Outlook"]);
    assert!(engine.call("computer.never_add", json!({"name": "  "})).is_err());
    assert!(engine.call("computer.never_remove", json!({"name": "Excel"})).is_err());
    engine.call("computer.never_remove", json!({"name": "Outlook"})).unwrap();
    assert!(engine.state().computer.never.is_empty());
    engine.call("computer.set", json!({"enabled": false})).unwrap();
    assert!(!crate::tools::specs(&ctx).iter().any(|t| t.name.starts_with("computer_")));
}

/// Gate: blocked programs (hard list, the person's own list) are refused without a question and
/// never listed; background work refuses everything.
#[test]
fn blocked_programs_and_background_work_are_refused_without_a_question() {
    let (engine, desk, seen, word) = setup();
    engine.call("computer.set", json!({"enabled": true})).unwrap();
    engine.call("computer.never_add", json!({"name": "Outlook"})).unwrap();
    let cmd = desk.add_window(r"C:\Windows\System32\cmd.exe", "Eingabeaufforderung", vec![control("1", "Textfeld", "", Some("C:\\>"), &[Act::Type])]);
    let explorer = desk.add_window(r"C:\Windows\explorer.exe", "Dokumente", vec![]);
    let keepass = desk.add_window(r"C:\Program Files\KeePassXC\KeePassXC.exe", "Passwörter.kdbx", vec![]);
    let outlook = desk.add_window(r"C:\Program Files\Microsoft Office\root\Office16\OUTLOOK.EXE", "Posteingang", vec![]);
    let settings = desk.add_window(r"C:\Windows\ImmersiveControlPanel\SystemSettings.exe", "Einstellungen", vec![]);
    desk.install(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe", "PowerShell");
    let e = ev("t1", "Öffne die Eingabeaufforderung, PowerShell, Outlook und die Einstellungen");

    let (list, err, _) = run(&engine, call(LIST, json!({})), &e, true);
    assert!(!err && list.contains(&word) && list.contains("5 gesperrte Fenster"), "{list}");
    for hidden in [&cmd, &explorer, &keepass, &outlook, &settings] {
        assert!(!list.contains(hidden.as_str()), "{hidden} is not listed: {list}");
    }
    for app in ["cmd", "Eingabeaufforderung", cmd.as_str(), "powershell", explorer.as_str(), keepass.as_str(), "Outlook", settings.as_str()] {
        let (text, err, outcome) = run(&engine, call(OPEN, json!({"app": app})), &e, true);
        assert!(err && text.contains("Abgelehnt") || text.contains("nicht zu finden"), "{app}: {text}");
        let _ = outcome;
    }
    for w in [&cmd, &keepass] {
        let (text, err, outcome) = run(&engine, call(TYPE, json!({"window": w, "control": "1", "text": "del *"})), &e, true);
        assert!(err && outcome == "denied" && text.contains("gesperrt"), "{text}");
    }
    assert!(engine.state().approvals.is_empty() && checks(&seen).is_empty(), "refused without a question or a check");
    assert!(desk.log().is_empty(), "nothing started, nothing typed: {:?}", desk.log());

    // Background work: refused, nothing read.
    let reads = desk.reads();
    for c in [call(LIST, json!({})), call(OPEN, json!({"app": "Word"})), call(READ, json!({"window": word}))] {
        let (text, err, outcome) = run(&engine, c, &e, false);
        assert!(err && outcome == "denied" && text.contains("Hintergrund"), "{text}");
    }
    assert_eq!(desk.reads(), reads);
}

/// Gate: opening needs the owner's words or a yes and lasts for this request; every change has a
/// trust check with the window content; the window is read again before the step and a change
/// since the check stops it; passwords are never typed into.
#[test]
fn every_change_is_checked_with_the_window_content_and_reread_before_it_runs() {
    let (engine, desk, seen, word) = setup();
    engine.call("computer.set", json!({"enabled": true})).unwrap();
    let e = ev("t1", "Öffne Word und schreib eine Einkaufsliste");

    // Not opened yet: no reading.
    let (text, err, _) = run(&engine, call(READ, json!({"window": word})), &e, true);
    assert!(err && text.contains("computer_open"), "{text}");
    // The owner named Word: opened without a question (the program runs already, nothing starts).
    let (text, err, outcome) = run(&engine, call(OPEN, json!({"app": "Word"})), &e, true);
    assert!(!err && outcome == "ok" && text.contains(&word), "{text}");
    assert!(engine.state().approvals.is_empty());
    let (outline, err, _) = run(&engine, call(READ, json!({"window": word})), &e, true);
    assert!(!err && outline.contains("untrusted_tool_result") && outline.contains("[1] Dokument „Textbereich“ = „Liebe Eltern,“"), "{outline}");

    // Typing: nobody asked for this text, so a question with the window content.
    let rx = run_async(&engine, call(TYPE, json!({"window": word, "control": "1", "text": " Milch, Brot", "replace": false})), e.clone());
    let a = approval(&engine);
    assert!(a.window.as_deref().unwrap_or("").contains("Liebe Eltern,") && a.window.as_deref().unwrap_or("").contains("← dieses Steuerelement"), "{a:?}");
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("Erledigt") && text.contains("Milch, Brot"), "{text}");
    assert_eq!(desk.value(&word, "1").as_deref(), Some("Liebe Eltern, Milch, Brot"));

    // The person types in the window while the question is open: the step does not run.
    let rx = run_async(&engine, call(PRESS, json!({"window": word, "control": "3"})), e.clone());
    let a = approval(&engine);
    desk.set_value(&word, "1", "Ganz anderer Text");
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    let (text, err, outcome) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(err && outcome == "denied" && text.contains("geändert"), "{text}");
    assert!(!desk.log().iter().any(|l| l.starts_with("drücken")), "{:?}", desk.log());

    // Never into a password field; no keys while one has the focus.
    desk.focus(&word, "2", true);
    let (text, err, outcome) = run(&engine, call(TYPE, json!({"window": word, "control": "2", "text": "geheim"})), &e, true);
    assert!(err && outcome == "denied" && text.contains("Passwortfeld"), "{text}");
    let (text, err, outcome) = run(&engine, call(KEY, json!({"window": word, "keys": "Strg+V"})), &e, true);
    assert!(err && outcome == "denied" && text.contains("Passwortfeld"), "{text}");
    let (text, err, _) = run(&engine, call(KEY, json!({"window": word, "keys": "Win+R"})), &e, true);
    assert!(err && text.contains("Windows-Taste"), "{text}");

    // Every check of a change saw the window content as data, never as the owner's words.
    let all = checks(&seen);
    let steps: Vec<&String> = all.iter().filter(|b| b.contains("computer_type") || b.contains("computer_press")).collect();
    assert!(steps.len() >= 2, "{all:#?}");
    for b in steps {
        assert!(b.contains("fensterinhalt") && b.contains("Elternbrief.docx"), "{b}");
    }

    // The next request: Word is no longer open to the model.
    let next = ev("t2", "Danke");
    let (text, err, _) = run(&engine, call(READ, json!({"window": word})), &next, true);
    assert!(err && text.contains("nicht geöffnet"), "{text}");
}

/// Opening a program nobody named asks; the yes starts it and opens its new window.
#[test]
fn opening_a_program_nobody_named_asks_and_starts_it() {
    let (engine, desk, _seen, _word) = setup();
    engine.call("computer.set", json!({"enabled": true})).unwrap();
    desk.install(r"C:\Windows\System32\notepad.exe", "Unbenannt – Editor");
    let e = ev("t1", "Schreib mir eine Notiz");
    let rx = run_async(&engine, call(OPEN, json!({"app": "Editor"})), e.clone());
    let a = approval(&engine);
    assert!(a.window.as_deref().unwrap_or("").contains("notepad.exe"), "{a:?}");
    engine.call("tool.answer", json!({"id": a.id, "allow": true})).unwrap();
    let (text, err, _) = rx.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(!err && text.contains("gestartet") && text.contains("Unbenannt"), "{text}");
    assert_eq!(desk.log(), [r"launch C:\Windows\System32\notepad.exe"]);
    let id = desk.windows_now().into_iter().find(|w| w.title.contains("Editor")).unwrap().id;
    let (outline, err, _) = run(&engine, call(READ, json!({"window": id})), &e, true);
    assert!(!err && outline.contains("Textbereich"), "{outline}");
    // A program that is not installed: no question.
    let (text, err, _) = run(&engine, call(OPEN, json!({"app": "Paint"})), &e, true);
    assert!(err && text.contains("nicht zu finden"), "{text}");
    // A „no“ starts nothing.
    desk.install(r"C:\Program Files\Microsoft Office\root\Office16\EXCEL.EXE", "Mappe1 – Excel");
    let rx = run_async(&engine, call(OPEN, json!({"app": "Excel"})), e);
    let a = approval(&engine);
    engine.call("tool.answer", json!({"id": a.id, "allow": false})).unwrap();
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)).unwrap().2, "denied");
    assert_eq!(desk.log().len(), 1, "{:?}", desk.log());
}
