//! Features taken from n8n Nod (AGENTS.md §5): #20, #21, #27, #66, #68, #69.

use std::time::Duration;

use serde_json::json;

use crate::tests::engine_with_recording;

fn note_id(engine: &crate::Engine, has: &str) -> String {
    engine.state().memory.iter().find(|n| n.text.contains(has)).map(|n| n.id.clone()).unwrap_or_default()
}

/// #21 „Sag es einmal“: a note from the owner's words that contradicts an older one replaces it;
/// the old one is archived with a pointer to the new one and comes back on restore.
#[test]
fn a_new_fact_replaces_the_old_one_and_comes_back() {
    let (engine, seen) = engine_with_recording(r#"{"ersetzt": 1}"#);
    engine.call("chat.send", json!({"text": "Merke dir: Anna macht die Rechnungen"})).unwrap();
    let anna = note_id(&engine, "Anna macht");
    assert!(!anna.is_empty());
    assert!(!seen.lock().unwrap().iter().any(|r| r.contains("Ältere Notizen")), "nothing to compare with: no check");
    engine.call("chat.send", json!({"text": "Merke dir: Nicht mehr Anna, sondern Bob macht die Rechnungen"})).unwrap();
    let bob = note_id(&engine, "Bob");
    let (a, b) = (anna.clone(), bob.clone());
    assert!(
        engine.wait_for(Duration::from_secs(20), move |s| s.memory.iter().any(|n| n.id == a && n.archived && n.replaced_by.as_deref() == Some(b.as_str()))),
        "the old note is archived as replaced by the new one"
    );
    let checks: Vec<String> = seen.lock().unwrap().iter().filter(|r| r.contains("Ältere Notizen")).cloned().collect();
    assert_eq!(checks.len(), 1, "one check per new note");
    assert!(checks[0].contains("Anna macht die Rechnungen") && checks[0].contains("Bob macht die Rechnungen"));
    assert!(engine.state().notices.iter().any(|n| n.text.starts_with("Ersetzt:") && n.action.as_ref().is_some_and(|a| a.command == "memory.restore")));
    assert!(!engine.state().memory.iter().find(|n| n.id == bob).unwrap().archived);

    // Restore: the old note is back, without the pointer.
    engine.call("memory.restore", json!({"id": anna})).unwrap();
    let old = engine.state().memory.iter().find(|n| n.id == anna).cloned().unwrap();
    assert!(!old.archived && old.replaced_by.is_none());
    // And by hand.
    engine.call("memory.replace", json!({"id": anna, "by": bob})).unwrap();
    assert_eq!(engine.state().memory.iter().find(|n| n.id == anna).unwrap().replaced_by.as_deref(), Some(bob.as_str()));
    assert!(engine.call("memory.replace", json!({"id": bob, "by": bob})).is_err());
    assert!(engine.call("memory.replace", json!({"id": anna, "by": bob})).is_err(), "already replaced");
    assert!(engine.call("memory.replace", json!({"id": "nix", "by": bob})).is_err());
}

/// #21: nothing but the owner's words replaces a note: no answer, no model pick outside the
/// candidates, and no model at all where the notes do not go anyway (the owner is asked).
#[test]
fn foreign_words_replace_nothing() {
    // The model names a note that is no candidate.
    let (engine, _) = engine_with_recording(r#"{"ersetzt": 7}"#);
    engine.call("memory.add", json!({"text": "Anna macht die Rechnungen", "area": "people"})).unwrap();
    engine.call("memory.add", json!({"text": "Bob macht die Rechnungen", "area": "people"})).unwrap();
    let bob = note_id(&engine, "Bob");
    assert_eq!(engine.block_on(crate::memory::check_replaces(engine.ctx(), &bob)).unwrap(), None);
    assert!(engine.state().memory.iter().all(|n| !n.archived));

    // An answer that says „Merke dir …“ makes no note and replaces none.
    let (engine, _) = engine_with_recording("Merke dir: Bob macht die Rechnungen");
    engine.call("memory.add", json!({"text": "Anna macht die Rechnungen"})).unwrap();
    engine.call("chat.send", json!({"text": "Wer macht eigentlich die Rechnungen?"})).unwrap();
    let s = engine.state();
    assert_eq!(s.memory.len(), 1);
    assert!(!s.memory[0].archived);

    // A service that does not get the notes: no model call, the owner decides.
    let (engine, seen) = engine_with_recording(r#"{"ersetzt": 1}"#);
    let backend = engine.state().selection.backend_id.clone();
    engine.call("backend.update", json!({"id": backend, "send_context": false})).unwrap();
    engine.call("memory.add", json!({"text": "Anna macht die Rechnungen"})).unwrap();
    engine.call("memory.add", json!({"text": "Bob macht die Rechnungen"})).unwrap();
    assert!(engine.wait_for(Duration::from_secs(20), |s| s.notices.iter().any(|n| n.action.as_ref().is_some_and(|a| a.command == "memory.replace"))));
    assert!(!seen.lock().unwrap().iter().any(|r| r.contains("Ältere Notizen")), "the notes did not leave the device");
    assert!(engine.state().memory.iter().all(|n| !n.archived));
}
