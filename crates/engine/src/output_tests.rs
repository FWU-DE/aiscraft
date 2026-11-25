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

/// „Erstellt mit KI“ (#57): on by default for AIS.chat, on every export of an answer and of
/// what was made from it (documents, worksheets, learning apps, H5P), with the prompt only on
/// request; off for other services unless set to `all`; „Mit Kennzeichnung kopieren“ always.
#[test]
fn exports_are_labelled_as_ai_generated() {
    use base64::Engine as _;
    let (engine, _) = super::ais_engine();
    engine.call("chat.send", json!({"text": "Elternbrief zum Wandertag"})).unwrap();
    let mid = engine.state().messages[1].id.clone();
    let md = |e: &crate::Engine, mid: &str| {
        e.call("message.export", json!({"message_id": mid, "format": "markdown"})).unwrap()["text"].as_str().unwrap().to_string()
    };
    let text = md(&engine, &mid);
    assert!(text.starts_with("Ich bin Goethe.") && text.contains("*Erstellt mit KI (AIS.chat, Modell GPT), "), "{text}");
    assert!(!text.contains("Wandertag"), "no prompt unless asked for");
    let chat = engine.call("chat.export", json!({})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(chat.trim_end().ends_with(".*") && chat.contains("*Erstellt mit KI (AIS.chat, Modell GPT), "), "the chat export too: {chat}");
    engine.call("settings.set", json!({"key": "ai_label_prompt", "value": true})).unwrap();
    assert!(md(&engine, &mid).contains("**Prompt:**\n\n> Elternbrief zum Wandertag"));
    // Word and PDF carry it too.
    let docx = engine.call("message.export", json!({"message_id": mid, "format": "docx"})).unwrap();
    let docx = base64::engine::general_purpose::STANDARD.decode(docx["base64"].as_str().unwrap()).unwrap();
    assert!(super::zip_part(&docx, "word/document.xml").contains("Erstellt mit KI"));

    // What was made in this chat: document, worksheet, learning app, H5P.
    let doc = engine.call("artifact.create", json!({"kind": "document", "from_message": mid})).unwrap()["id"].as_str().unwrap().to_string();
    let out = engine.call("artifact.export", json!({"id": doc, "format": "markdown"})).unwrap();
    assert!(out["text"].as_str().unwrap().contains("Erstellt mit KI (AIS.chat"), "{out}");
    let sheet = json!({"title": "Wandertag", "tasks": [{"type": "true_false", "prompt": "Wir wandern.", "answer": true}]});
    let ws = engine.call("worksheet.create", json!({"worksheet": sheet})).unwrap()["id"].as_str().unwrap().to_string();
    let out = engine.call("worksheet.export", json!({"id": ws, "format": "markdown"})).unwrap();
    let ws_md = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(out["base64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(ws_md.contains("Erstellt mit KI (AIS.chat"), "{ws_md}");
    let json_out = engine.call("worksheet.export", json!({"id": ws, "format": "json"})).unwrap();
    let ws_json = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(json_out["base64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(!ws_json.contains("Erstellt mit KI"), "the editable JSON stays clean");
    let app = engine.call("learning_app.create", json!({"html": "<!doctype html><html><body><h1>Wandern</h1></body></html>"})).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let html = engine.call("artifact.export", json!({"id": app, "format": "html"})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(html.contains("data-aiscraft=\"ki-hinweis\"") && html.find("ki-hinweis") < html.find("</body>"), "{html}");
    let h5p = json!({"title": "Quiz", "tasks": [{"type": "true_false", "prompt": "Ja?", "answer": true}]});
    let hid = engine.call("h5p.create", json!({"h5p": h5p})).unwrap()["id"].as_str().unwrap().to_string();
    let out = engine.call("h5p.export", json!({"id": hid})).unwrap();
    let pkg = base64::engine::general_purpose::STANDARD.decode(out["base64"].as_str().unwrap()).unwrap();
    let meta: serde_json::Value = serde_json::from_str(&super::zip_part(&pkg, "h5p.json")).unwrap();
    assert!(meta["authorComments"].as_str().unwrap().starts_with("Erstellt mit KI (AIS.chat"), "{meta}");

    // Off: nothing, but copying with the label still works.
    engine.call("settings.set", json!({"key": "ai_label", "value": "off"})).unwrap();
    assert_eq!(md(&engine, &mid), "Ich bin Goethe.");
    let copied = engine.call("message.copy_labeled", json!({"message_id": mid})).unwrap();
    assert!(copied["text"].as_str().unwrap().starts_with("Ich bin Goethe.\n\n---\n\n*Erstellt mit KI (AIS.chat"), "{copied}");
    assert!(engine.call("message.copy_labeled", json!({"message_id": engine.state().messages[0].id})).is_err(), "only answers");
    assert!(engine.call("settings.set", json!({"key": "ai_label", "value": "manchmal"})).is_err());

    // Another service: unlabelled by default, labelled with `all`.
    let (other, _) = super::engine_with_recording("Antwort.");
    other.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let mid = other.state().messages[1].id.clone();
    assert_eq!(md(&other, &mid), "Antwort.");
    other.call("settings.set", json!({"key": "ai_label", "value": "all"})).unwrap();
    assert!(md(&other, &mid).contains("Erstellt mit KI ("), "with `all`");
}
