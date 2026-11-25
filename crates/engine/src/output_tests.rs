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

fn tmp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("aiscraft-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const LETTER: &str = "## Elternbrief\n\nLiebe Eltern, der Wandertag ist am **15. Juni**.\n\n| Name | Note | Anteil |\n|---|---|---|\n| Ada | 1,5 | 20 % |\n| Bo | 2 | =SUMME(A1) |";

/// An answer as Word, PDF, Excel (its tables) or Markdown (#46): to a path or as bytes;
/// questions, unknown formats and answers without a table for Excel are refused.
#[test]
fn an_answer_is_saved_as_word_pdf_excel_or_markdown() {
    use base64::Engine as _;
    let (engine, _) = super::engine_with_recording(LETTER);
    engine.call("chat.send", json!({"text": "Schreib einen Elternbrief"})).unwrap();
    let s = engine.state();
    let (question, answer) = (s.messages[0].id.clone(), s.messages[1].id.clone());
    let dir = tmp_dir("export");
    for (format, magic) in [("docx", &b"PK"[..]), ("pdf", &b"%PDF"[..]), ("xlsx", &b"PK"[..])] {
        let path = dir.join(format!("brief.{format}"));
        let out = engine.call("message.export", json!({"message_id": answer, "format": format, "path": path.display().to_string()})).unwrap();
        assert_eq!(out["path"], path.display().to_string());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(magic), "{format}");
    }
    // The workbook holds the table, numbers as numbers, „=…“ as text.
    let out = engine.call("message.export", json!({"message_id": answer, "format": "xlsx"})).unwrap();
    assert_eq!(out["file_name"], "Elternbrief.xlsx");
    let bytes = base64::engine::general_purpose::STANDARD.decode(out["base64"].as_str().unwrap()).unwrap();
    let all = format!("{}{}", super::zip_part(&bytes, "xl/sharedStrings.xml"), super::zip_part(&bytes, "xl/worksheets/sheet1.xml"));
    assert!(all.contains("Ada") && all.contains("<v>1.5</v>") && all.contains("=SUMME(A1)") && !all.contains("<f>"), "{all}");
    let out = engine.call("message.export", json!({"message_id": answer, "format": "markdown", "title": "Brief"})).unwrap();
    assert_eq!((out["text"].as_str().unwrap(), out["file_name"].as_str().unwrap()), (LETTER, "Brief.md"));
    assert!(engine.call("message.export", json!({"message_id": question, "format": "docx"})).is_err(), "only answers");
    assert!(engine.call("message.export", json!({"message_id": answer, "format": "odt"})).is_err());
    assert!(engine.call("message.export", json!({"message_id": "nichts", "format": "pdf"})).is_err());
    let (plain, _) = super::engine_with_recording("Nur Text, keine Tabelle.");
    plain.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let mid = plain.state().messages[1].id.clone();
    let err = plain.call("message.export", json!({"message_id": mid, "format": "xlsx"})).unwrap_err();
    assert!(err.message.contains("keine Tabelle"), "{err:?}");
}
