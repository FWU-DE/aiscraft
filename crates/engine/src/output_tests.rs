//! Functional tests for what happens with an answer after it arrived: readability (#48),
//! the AI label (#57), saving as Word, PDF or Excel (#46) and calendar files (#51). Included
//! from `tests.rs`, so `every_command_has_a_functional_test` sees them.

use serde_json::json;

/// Readability (#48): computed on the device, shown under the answer on request, hidden again;
/// plain text works without a message.
#[test]
fn readability_is_shown_and_hidden_locally() {
    let (engine, seen) = super::engine_with_recording("Der Hund ist klein. Er spielt im Garten. Die Sonne scheint.");
    engine.call("chat.send", json!({"text": "Schreib etwas Leichtes"})).unwrap();
    let requests = seen.lock().unwrap().len();
    let mid = engine.state().messages[1].id.clone();
    let out = engine.call("message.readability", json!({"message_id": mid})).unwrap();
    assert_eq!(out["readability"]["words"], 11, "{out}");
    assert!(out["summary"].as_str().unwrap().starts_with("11 Wörter · Flesch"), "{out}");
    assert!(engine.state().readability.contains_key(&mid));
    engine.call("message.readability", json!({"message_id": mid, "show": false})).unwrap();
    assert!(engine.state().readability.is_empty());
    let out = engine.call("message.readability", json!({"text": "Ein Satz. Noch einer."})).unwrap();
    assert_eq!(out["readability"]["sentences"], 2);
    assert!(engine.call("message.readability", json!({"message_id": "gibt-es-nicht"})).is_err());
    assert!(engine.call("message.readability", json!({})).is_err());
    assert_eq!(seen.lock().unwrap().len(), requests, "nothing leaves the device");
}
