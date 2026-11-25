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

/// #20: the writing style comes only from the owner's own messages (no answers, no prompts the app
/// wrote), leaves without names and numbers, keeps none, and goes along only with writing requests.
#[test]
fn style_is_learned_from_own_words_only() {
    // The model tries to slip a greeted name and a number into the note.
    let (engine, seen) = engine_with_recording("- Grüßt mit „Liebe Frau Kaya“\n- Schreibt 3 kurze Sätze, duzt Kolleginnen");
    for text in [
        "Liebe Frau Kaya, danke für die Unterlagen zur Klassenfahrt.",
        "Hallo Jonas, kannst du mir die Liste bis 12 Uhr schicken?",
        "Schreib mir bitte eine kurze Mail an das Kollegium wegen Freitag",
        "Ich brauche drei Ideen für den Wandertag",
        "Meine Nummer ist 0170 1234567, ruf mich gern an",
    ] {
        engine.call("chat.send", json!({"text": text})).unwrap();
    }
    // A prompt the app wrote is not the owner's: a schedule run in its own chat.
    let sched =
        engine.call("schedule.create", json!({"title": "Plan", "prompt": "Liebe Frau Yilmaz, was steht heute an im Plan?", "cron": "0 7 * * *"})).unwrap();
    engine.call("schedule.run", json!({"id": sched["id"]})).unwrap();

    let out = engine.call("style.learn", json!({})).unwrap();
    assert_eq!(out["messages"], 5, "five own messages, not the schedule's prompt");
    let req = seen.lock().unwrap().iter().rev().find(|r| r.contains("nachrichten_der_person")).cloned().unwrap();
    for gone in ["Kaya", "Jonas", "Yilmaz", "0170", "12 Uhr", "Grüßt mit"] {
        assert!(!req.contains(gone), "{gone} left the device: {req}");
    }
    assert!(req.contains("Wandertag"), "the owner's own words went along");
    let note = engine.state().memory.iter().find(|n| n.area == aiscraft_core::MemoryArea::Style && !n.archived).cloned().unwrap();
    assert!(!note.text.contains("Kaya") && !note.text.chars().any(|c| c.is_ascii_digit()), "no names or numbers kept: {}", note.text);
    assert!(note.text.contains("duzt Kolleginnen"));
    assert!(note.source_quote.contains("eigenen Nachrichten"), "the note says where it comes from");

    // Only with writing requests, and each chat can switch it off.
    let style_sent = |seen: &std::sync::Mutex<Vec<String>>| seen.lock().unwrap().last().is_some_and(|r| r.contains("Schreibstil der Person"));
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": "Was ist Photosynthese?"})).unwrap();
    assert!(!style_sent(&seen), "not with a question");
    engine.call("chat.send", json!({"text": "Schreib eine Mail an die Eltern der 7b zum Wandertag"})).unwrap();
    assert!(style_sent(&seen), "with a writing request");
    engine.call("chat.set_sources", json!({"own_style": false})).unwrap();
    engine.call("chat.send", json!({"text": "Schreib noch eine Mail an die Eltern"})).unwrap();
    assert!(!style_sent(&seen), "switched off for this chat");

    // Learning again replaces the old note.
    engine.call("style.learn", json!({})).unwrap();
    let styles: Vec<_> = engine.state().memory.iter().filter(|n| n.area == aiscraft_core::MemoryArea::Style).cloned().collect();
    assert_eq!(styles.iter().filter(|n| !n.archived).count(), 1);
    assert!(styles.iter().any(|n| n.archived && n.replaced_by.is_some()));

    // Too few own messages: it says so.
    let (fresh, _) = engine_with_recording("x");
    let err = fresh.call("style.learn", json!({})).unwrap_err();
    assert!(err.message.contains("zu wenige"), "{}", err.message);
}

/// #27: briefing templates create a schedule with one call, and say what is missing instead.
#[test]
fn briefing_templates_say_what_is_missing() {
    let (engine, _) = engine_with_recording("ok");
    let list = engine.call("schedule.templates", json!({})).unwrap();
    let by_id = |id: &str| list.as_array().unwrap().iter().find(|t| t["id"] == id).cloned().unwrap();
    let missing = by_id("morgen")["missing"].as_array().unwrap().clone();
    assert_eq!(missing.len(), 2, "{missing:?}");
    assert!(by_id("vertretung")["missing"].as_array().unwrap().is_empty(), "this model reads pages with the built-in reader");
    assert_eq!(by_id("vertretung")["needs_url"], true);

    let err = engine.call("schedule.from_template", json!({"template": "morgen"})).unwrap_err();
    assert!(err.message.contains("Kalender") && err.message.contains("E-Mail") && err.message.contains("Konnektoren"), "{}", err.message);
    assert!(engine.state().schedules.is_empty(), "nothing half-working is created");

    // A connector for calendar and mail (set up, even if not reachable now) meets both needs.
    engine.call("connector.add", json!({"name": "Outlook Kalender und Mail", "url": "http://127.0.0.1:9/mcp"})).unwrap();
    engine.call("schedule.from_template", json!({"template": "morgen"})).unwrap();
    let s = engine.state().schedules.last().cloned().unwrap();
    assert_eq!((s.title.as_str(), s.cron.as_deref()), ("Morgenüberblick", Some("0 7 * * 1-5")));

    // The page watch needs the page; it becomes a watch.
    assert!(engine.call("schedule.from_template", json!({"template": "vertretung"})).unwrap_err().message.contains("Adresse"));
    assert!(engine.call("schedule.from_template", json!({"template": "vertretung", "url": "kein link"})).is_err());
    engine.call("schedule.from_template", json!({"template": "vertretung", "url": "https://schule.example/plan"})).unwrap();
    let w = engine.state().schedules.last().cloned().unwrap();
    assert!(w.watch && w.prompt.contains("https://schule.example/plan"));
    // The owner's changes count.
    engine.call("schedule.from_template", json!({"template": "woche", "title": "Meine Woche", "cron": "0 17 * * 0"})).unwrap();
    let mine = engine.state().schedules.last().cloned().unwrap();
    assert_eq!((mine.title.as_str(), mine.cron.as_deref()), ("Meine Woche", Some("0 17 * * 0")));
    assert!(engine.call("schedule.from_template", json!({"template": "gibtsnicht"})).is_err());

    // Without tools no page can be read: the template says so.
    engine.call("settings.set", json!({"key": "tools", "value": false})).unwrap();
    let list = engine.call("schedule.templates", json!({})).unwrap();
    assert!(!list.as_array().unwrap().iter().find(|t| t["id"] == "vertretung").unwrap()["missing"].as_array().unwrap().is_empty());
}
