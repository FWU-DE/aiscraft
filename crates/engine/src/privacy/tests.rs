//! „Namen schützen“ (#55) end to end: class lists, preview, sending, translating back.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::detect::{Found, Kind};
use super::*;
use crate::{Engine, Options};

/// An OpenAI-compatible server that records every request and streams `chunks` as the answer,
/// so placeholders can be cut across chunk boundaries.
fn chunked_openai(engine: &Engine, chunks: &'static [&'static str]) -> (String, Arc<Mutex<Vec<String>>>) {
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
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..k]);
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
                    for part in chunks {
                        let c = json!({"choices":[{"delta":{"content": part}}]});
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                    }
                    let _ = sock.write_all(b"data: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

fn engine_with(chunks: &'static [&'static str]) -> (Engine, Arc<Mutex<Vec<String>>>) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = chunked_openai(&engine, chunks);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, seen)
}

fn posted(seen: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    seen.lock().unwrap().iter().filter(|r| r.starts_with("POST")).cloned().collect()
}

/// Real names, contact data and the bank account of the made-up class.
const SECRETS: &[&str] =
    &["Lea", "Müller", "Klaus", "Weber", "Jonas", "Schmidt", "lea.mueller@example.org", "0171 2345678", "DE89 3704 0044 0532 0130 00", "Lindenstraße 5"];

/// Quality gate (AGENTS.md §6): with protection on, no real name or contact detail is in any
/// byte the backend receives — not in the message, not in an attached document, not in memory,
/// not in the system prompt, not in later rounds — and the answer shows the names again.
#[test]
fn no_real_name_reaches_the_backend() {
    let (engine, seen) = engine_with(&["[S1]s Arbeit ist gut. [S", "2] [N", "2] hilft [S1] gern. Mail an [E-Mail 1]."]);
    // Without a class list protection is off by default.
    assert!(!active(engine.ctx(), &engine.state().selection.sources, None));
    let added = engine.call("privacy.list_add", json!({"name": "7b", "text": "Lea Müller\nSchmidt, Jonas\nMark Hoffmann"})).unwrap();
    assert_eq!(added["count"], 3);
    assert_eq!(engine.state().privacy.lists[0].count, 3);
    assert!(!serde_json::to_string(&*engine.state()).unwrap().contains("Müller"), "the state agents read has no names");
    engine.call("memory.add", json!({"text": "Lea Müller braucht mehr Zeit bei Tests", "area": "about"})).unwrap();
    // The downloaded model (#73) finds last names without a form of address; here a stand-in.
    engine.ctx().0.ner.set_model(Some(fake_model(&["Weber"])));
    engine.call("memory.add", json!({"text": "Weber sitzt vorne", "area": "about"})).unwrap();

    let dir = std::env::temp_dir().join(format!("aiscraft-privacy-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let notes = dir.join("notizen.txt");
    std::fs::write(&notes, "Elternkontakt Jonas Schmidt: 0171 2345678, lea.mueller@example.org, Lindenstraße 5, 10115 Berlin").unwrap();

    let text = "Schreib eine Zeugnisbemerkung für Lea Müller (geb. 03.05.2014). Leas Mitarbeit ist gut, Jonas hilft ihr. \
                Das Mark der Sache: IBAN DE89 3704 0044 0532 0130 00. Gestern war Klaus wieder unruhig.";
    // The preview shows exactly what goes out; "Mark" is unsure and replaced too (privacy first),
    // the person can take it back.
    let pv = engine.call("privacy.preview", json!({"text": text})).unwrap();
    assert_eq!(pv["active"], true);
    let out = pv["outgoing"].as_str().unwrap();
    assert!(out.starts_with("Schreib eine Zeugnisbemerkung für [S1] [N1] (geb. [Geburtsdatum 1]). [S1]s Mitarbeit ist gut, [S2] hilft ihr."), "{out}");
    assert!(out.contains("Das [S3] der Sache: IBAN [IBAN 1]."), "{out}");
    // Klaus is in no class list: the built-in first names find him (unsure, replaced) (#73).
    assert!(out.ends_with("Gestern war [Name 1] wieder unruhig."), "{out}");
    let mark = pv["hits"].as_array().unwrap().iter().find(|h| h["text"] == "Mark").unwrap();
    assert_eq!((mark["ambiguous"].as_bool(), mark["on"].as_bool()), (Some(true), Some(true)));
    assert!(engine.state().privacy.preview.as_ref().is_some_and(|p| p.outgoing == out));
    let kept = engine.call("privacy.preview", json!({"text": text, "off": [{"text": "Mark", "n": 0}]})).unwrap();
    assert!(kept["outgoing"].as_str().unwrap().contains("Das Mark der Sache"), "an unsure hit can be taken back");
    // Switching a hit off keeps it; nothing of the preview is kept in the chat.
    let off = engine.call("privacy.preview", json!({"text": text, "off": [{"text": "Jonas", "n": 0}]})).unwrap();
    assert!(off["outgoing"].as_str().unwrap().contains("Jonas hilft"));

    engine.call("chat.send", json!({"text": text, "files": [notes.display().to_string()]})).unwrap();
    engine.call("chat.send", json!({"text": "Und noch ein Satz zu Jonas Schmidt bitte. Weber war auch dabei."})).unwrap();
    let reqs = posted(&seen);
    assert_eq!(reqs.len(), 2);
    for r in &reqs {
        for s in SECRETS {
            assert!(!r.contains(s), "„{s}“ reached the backend: {}", r[r.find("\r\n\r\n").unwrap_or(0)..].chars().take(1500).collect::<String>());
        }
        assert!(r.contains("[S1]") && r.contains("Platzhalter in eckigen Klammern"), "placeholders and the note go along");
    }
    assert!(reqs[1].contains("[S2] [N2]"), "the same pupil keeps the same placeholder");

    // Stored: what the backend saw. Shown, copied, exported: the names.
    let s = engine.state();
    assert!(s.messages[0].content.contains("[S1] [N1]") && !s.messages[0].content.contains("Müller"));
    assert_eq!(
        s.messages[0].activity.iter().find(|a| a.label == PROTECTED).and_then(|a| a.detail.clone()).as_deref(),
        Some("13 Angaben ersetzt"),
        "Mark (unsure) is replaced too"
    );
    assert!(s.messages[0].attachments[0].text.as_deref().unwrap_or("").contains("[Telefon 1]"));
    let shown = engine.shown_messages(&s);
    assert!(shown[0].content.starts_with("Schreib eine Zeugnisbemerkung für Lea Müller (geb. 03.05.2014). Leas Mitarbeit"));
    assert_eq!(shown[1].content, "Leas Arbeit ist gut. Jonas Schmidt hilft Lea gern. Mail an lea.mueller@example.org.");
    let md = engine.call("chat.export", json!({"format": "markdown"})).unwrap();
    assert!(md["text"].as_str().unwrap().contains("Jonas Schmidt hilft Lea gern"));
    let raw = engine.call("chat.export", json!({"format": "json"})).unwrap();
    assert!(!raw["text"].as_str().unwrap().contains("Müller"), "the diagnostic export has no mapping");
    let map = engine.call("privacy.mapping", json!({})).unwrap();
    assert!(map["entries"].as_array().unwrap().iter().any(|e| e["placeholder"] == "[N1]" && e["original"] == "Müller"));
    assert_eq!(engine.call("privacy.reveal", json!({"text": "[S2] [N2]"})).unwrap()["text"], "Jonas Schmidt");

    // Mark is replaced anyway now; confirming it (and adding a name by hand) keeps it replaced.
    engine.call("chat.send", json!({"text": "Mark kommt auch.", "privacy": {"on": [{"text": "Mark", "n": 0}], "add": ["Frau Özdemir"]}})).unwrap();
    engine.call("chat.send", json!({"text": "Grüße an Mark und Frau Özdemir"})).unwrap();
    let last = posted(&seen).pop().unwrap();
    assert!(!last.contains("Mark kommt") && !last.contains("Özdemir") && last.contains("[Name 1]"), "{last}");

    // Switched off for this chat: the text goes out as written.
    engine.call("chat.set_sources", json!({"protect": false})).unwrap();
    engine.call("chat.send", json!({"text": "Ohne Schutz: Lea"})).unwrap();
    let unprotected = posted(&seen).pop().unwrap();
    assert!(unprotected.contains("Ohne Schutz: Lea"));
    // Memory notes come from every chat: their names stay home here too.
    assert!(unprotected.contains("braucht mehr Zeit bei Tests") && !unprotected.contains("Müller"), "{unprotected}");
    // Single calls in the background (tidy-up, check-ins) protect once a class list exists,
    // whatever the open chat's switch says.
    engine.block_on(crate::chat::complete(engine.ctx(), None, "Fasse zusammen: Lea Müller braucht mehr Zeit.")).unwrap();
    let single = posted(&seen).pop().unwrap();
    assert!(!single.contains("Lea") && !single.contains("Müller"), "{single}");
    engine.call("chat.set_sources", json!({"protect": null})).unwrap();

    // Deleting the chat keeps its mapping while it lies in the trash (#77); purging deletes it.
    let conv = engine.state().current.clone().unwrap();
    engine.call("chat.delete", json!({"id": conv})).unwrap();
    assert!(!mapping(engine.ctx(), &conv).entries.is_empty(), "restorable with its names");
    engine.call("chat.purge", json!({"id": conv})).unwrap();
    assert!(mapping(engine.ctx(), &conv).entries.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Stage c without a model (#73): a common first name is found without any class list once
/// protection is on, replaced by default and kept with a tap.
#[test]
fn common_first_names_without_a_class_list() {
    let (engine, seen) = engine_with(&["[Name 1] braucht Ruhe."]);
    engine.call("chat.set_sources", json!({"protect": true})).unwrap();
    let text = "Gestern war Klaus wieder unruhig, ich habe mit Frau Koch gesprochen. Die Rose blüht.";
    let pv = engine.call("privacy.preview", json!({"text": text})).unwrap();
    assert_eq!(pv["outgoing"], "Gestern war [Name 1] wieder unruhig, ich habe mit Frau [Name 2] gesprochen. Die Rose blüht.");
    assert!(pv["hits"].as_array().unwrap().iter().all(|h| h["ambiguous"] == true && h["on"] == true));
    let kept = engine.call("privacy.preview", json!({"text": text, "off": [{"text": "Klaus", "n": 0}]})).unwrap();
    assert!(kept["outgoing"].as_str().unwrap().starts_with("Gestern war Klaus wieder"));
    engine.call("chat.send", json!({"text": text})).unwrap();
    let req = posted(&seen).pop().unwrap();
    assert!(!req.contains("Klaus") && !req.contains("Koch") && req.contains("[Name 1]"), "{req}");
    let s = engine.state();
    assert_eq!(engine.shown_messages(&s)[1].content, "Klaus braucht Ruhe.");
}

/// Model calls outside a chat (revising a document, tidying memory) are protected too.
#[test]
fn single_calls_are_protected_and_translated_back() {
    let (engine, seen) = engine_with(&["Für [S1]: ", "alles gut."]);
    engine.call("privacy.list_add", json!({"text": "Lea Müller"})).unwrap();
    let out = engine.block_on(crate::chat::complete(engine.ctx(), Some("Du hilfst Lea Müller."), "Überarbeite: Lea war krank.")).unwrap();
    assert_eq!(out, "Für Lea: alles gut.");
    assert!(posted(&seen).iter().all(|r| !r.contains("Lea") && !r.contains("Müller")));
}

/// Answers written before protection was switched on hold names; later requests protect them
/// too, and placeholders in answers stay as they are.
#[test]
fn older_answers_are_protected_too() {
    let (engine, seen) = engine_with(&["Lea Müller hat gefehlt, [S1] nicht."]);
    engine.call("chat.send", json!({"text": "Wer hat gefehlt?"})).unwrap();
    assert!(posted(&seen).pop().unwrap().contains("Wer hat gefehlt?"));
    engine.call("privacy.list_add", json!({"text": "Lea Müller"})).unwrap();
    engine.call("chat.send", json!({"text": "Und gestern?"})).unwrap();
    let req = posted(&seen).pop().unwrap();
    assert!(!req.contains("Lea") && !req.contains("Müller"), "{req}");
    assert!(req.contains("hat gefehlt, [S1] nicht."), "{req}");
}

/// The page reader goes over the network: a web address keeps the placeholders (#55).
#[test]
fn web_addresses_keep_the_placeholders() {
    let (engine, _) = engine_with(&["ok"]);
    engine.allow_local_web().unwrap();
    let (page, pages) = chunked_openai(&engine, &["x"]);
    engine.call("privacy.list_add", json!({"text": "Lea Müller"})).unwrap();
    engine.call("chat.send", json!({"text": "Such etwas über Lea"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    assert!(mapping(engine.ctx(), &conv).entries.iter().any(|e| e.placeholder == "[S1]"));
    let call = aiscraft_core::ToolCall {
        id: "w".into(),
        name: crate::tools::WEB_FETCH.into(),
        arguments: json!({"url": format!("{}/schueler/[S1]", page.trim_end_matches("/v1"))}),
        outcome: None,
    };
    let (_, _, _) = engine.block_on(crate::tools::run(engine.ctx(), &conv, &call, &Default::default(), true));
    let reqs = pages.lock().unwrap().clone();
    let get = reqs.iter().find(|r| r.contains("/schueler/")).expect("the page was requested");
    let line = get.lines().next().unwrap_or("");
    assert!(!line.contains("Lea") && (line.contains("[S1]") || line.contains("%5BS1%5D")), "{line}");
}

/// A task's result in the chat it came from (#55): the task chat's `[S1]` is translated back and
/// protected again with the origin chat's mapping, so it never names another pupil.
#[test]
fn task_results_keep_the_right_names() {
    let (engine, _) = engine_with(&["[S1] ist fertig."]);
    engine.call("privacy.list_add", json!({"text": "Lea Müller\nJonas Schmidt"})).unwrap();
    engine.call("chat.send", json!({"text": "Lea Müller fehlt heute."})).unwrap();
    let origin = engine.state().current.clone().unwrap();
    assert_eq!(engine.call("privacy.reveal", json!({"text": "[S1]"})).unwrap()["text"], "Lea");
    let task =
        engine.call("task.start", json!({"title": "Bericht", "instructions": "Schreib über Jonas Schmidt"})).unwrap()["id"].as_str().unwrap().to_string();
    let card =
        |s: &crate::State| s.messages.iter().find(|m| m.conversation_id == origin && m.activity.iter().any(|a| a.label == crate::tasks::TASK_RESULT)).cloned();
    assert!(engine.wait_for(std::time::Duration::from_secs(30), |s| card(s).is_some()));
    assert_eq!(mapping(engine.ctx(), &task).reveal("[S1]"), "Jonas", "in the task chat [S1] is Jonas");
    let m = card(&engine.state()).unwrap();
    assert!(!m.content.contains("Jonas") && !m.content.contains("[S1] ist fertig"), "{}", m.content);
    assert!(engine.call("privacy.reveal", json!({"text": m.content})).unwrap()["text"].as_str().unwrap().contains("Jonas ist fertig."));
}

/// Class lists come from CSV, Excel, OpenDocument-like sheets and contacts; for one project or all.
#[test]
fn class_lists_import_and_remove() {
    let (engine, _) = engine_with(&["ok"]);
    let dir = std::env::temp_dir().join(format!("aiscraft-lists-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("7b.csv");
    std::fs::write(&csv, "Nachname;Vorname\nMüller;Lea\nSchmidt;Jonas\n").unwrap();
    let r = engine.call("privacy.list_import", json!({"path": csv.display().to_string()})).unwrap();
    assert_eq!(r["count"], 2);
    let vcf = dir.join("eltern.vcf");
    std::fs::write(&vcf, "BEGIN:VCARD\r\nVERSION:3.0\r\nN:Yilmaz;Emre;;;\r\nFN:Emre Yilmaz\r\nEND:VCARD\r\n").unwrap();
    let pid = engine.call("project.create", json!({"name": "Klasse 8a"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("privacy.list_import", json!({"path": vcf.display().to_string(), "project": pid})).unwrap();
    let xlsx = dir.join("8a.xlsx");
    let bytes =
        crate::office::xlsx::workbook_xlsx(&json!({"sheets": [{"columns": ["Vorname", "Nachname"], "rows": [["Ada", "Lovelace"], ["Max", "Planck"]]}]}))
            .unwrap();
    std::fs::write(&xlsx, bytes).unwrap();
    assert_eq!(engine.call("privacy.list_import", json!({"path": xlsx.display().to_string(), "name": "8a"})).unwrap()["count"], 2);
    let lists = engine.call("privacy.lists", json!({})).unwrap();
    assert_eq!(lists.as_array().unwrap().len(), 3);
    assert!(lists[0]["names"].as_array().unwrap().iter().any(|n| n == "Lea Müller"));
    // The project's list counts only in the project's chats.
    let s = engine.state();
    assert!(active(engine.ctx(), &s.selection.sources, Some(&pid)));
    let people = people_for(engine.ctx(), None);
    assert!(!people.iter().any(|p| p.last == "Yilmaz"));
    assert!(people_for(engine.ctx(), Some(&pid)).iter().any(|p| p.last == "Yilmaz"));
    assert!(engine.call("privacy.list_import", json!({"path": dir.join("x.pdf").display().to_string()})).is_err());
    std::fs::write(dir.join("leer.csv"), "\n\n").unwrap();
    assert!(engine.call("privacy.list_import", json!({"path": dir.join("leer.csv").display().to_string()})).is_err());
    for l in lists.as_array().unwrap() {
        engine.call("privacy.list_remove", json!({"id": l["id"]})).unwrap();
    }
    assert!(engine.state().privacy.lists.is_empty());
    assert!(engine.call("privacy.list_remove", json!({"id": "weg"})).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

fn found(kind: Kind, key: &str, display: &str) -> Found {
    Found { start: 0, end: 0, kind, key: key.into(), display: display.into(), ambiguous: false }
}

#[test]
fn placeholders_are_stable_and_stream_back_without_flashing() {
    let mut m = Mapping::default();
    assert_eq!(m.placeholder_for(&found(Kind::FirstName, "p:lea|müller", "Lea")), "[S1]");
    assert_eq!(m.placeholder_for(&found(Kind::FirstName, "p:jonas|schmidt", "Jonas")), "[S2]");
    assert_eq!(m.placeholder_for(&found(Kind::LastName, "p:lea|müller", "Müller")), "[N1]");
    assert_eq!(m.placeholder_for(&found(Kind::FirstName, "p:lea|müller", "Lea")), "[S1]");
    assert_eq!(m.placeholder_for(&found(Kind::Email, "a@b.de", "a@b.de")), "[E-Mail 1]");
    assert_eq!(m.reveal("[S1] [N1], [S2]; [S3] [x] [E-Mail 1]"), "Lea Müller, Jonas; [S3] [x] a@b.de");
    // A placeholder cut off at the end of a chunk waits; other brackets do not.
    assert_eq!(m.reveal_streaming("Gut, [S"), "Gut, ");
    assert_eq!(m.reveal_streaming("Gut, [E-Ma"), "Gut, ");
    assert_eq!(m.reveal_streaming("Gut, [x"), "Gut, [x");
    assert_eq!(m.reveal_streaming("Gut, [S1]"), "Gut, Lea");
    assert_eq!(m.reveal("ü[ä[S1]"), "ü[äLea");
    let v = json!({"title": "Zeugnis [S1]", "rows": [["[N1]", 2]]});
    let mut walked = v.clone();
    if let Value::Object(o) = &mut walked {
        o.insert("title".into(), json!(m.reveal("Zeugnis [S1]")));
    }
    assert_eq!(walked["title"], "Zeugnis Lea");
    let d = Decisions::from_value(Some(&json!({"off": [{"text": "LEA", "n": 1}], "add": ["x", "Frau Özdemir", 5]})));
    assert_eq!(d.off, vec![("lea".to_string(), 1)]);
    assert_eq!(d.add, vec!["Frau Özdemir".to_string()]);
}

/// Stands in for the GLiNER model: every occurrence of these words, and the words of a
/// placeholder (which the detector must never take for a name).
fn fake_model(names: &'static [&'static str]) -> ner::NameModel {
    Arc::new(move |t: &str| {
        let mut out: Vec<(usize, usize)> = names.iter().flat_map(|n| t.match_indices(n).map(|(i, w)| (i, i + w.len()))).collect();
        out.extend(t.match_indices("Name 1").map(|(i, w)| (i, i + w.len())));
        out
    })
}

/// Stage c with a model (#73): last names without a form of address, found by the downloaded
/// model, are unsure and replaced by default like the first-name list's; the same word keeps one
/// placeholder whichever source found it; placeholders are never names again.
#[test]
fn the_name_model_finds_last_names_without_a_form_of_address() {
    let (engine, seen) = engine_with(&["[Name 2] kommt morgen."]);
    // Without a class list protection stays off until switched on for the chat.
    assert!(!engine.state().protection_on());
    engine.ctx().0.ner.set_model(Some(fake_model(&["Weber", "Klaus Brandt"])));
    engine.call("chat.set_sources", json!({"protect": true})).unwrap();
    assert!(engine.state().protection_on());
    let text = "Gestern hat Weber gefehlt. Klaus Brandt und Klaus kamen zu spät.";
    let pv = engine.call("privacy.preview", json!({"text": text})).unwrap();
    assert_eq!(pv["outgoing"], "Gestern hat [Name 1] gefehlt. [Name 2] [Name 3] und [Name 2] kamen zu spät.");
    assert!(pv["hits"].as_array().unwrap().iter().all(|h| h["ambiguous"] == true && h["on"] == true));
    let kept = engine.call("privacy.preview", json!({"text": text, "off": [{"text": "Weber", "n": 0}]})).unwrap();
    assert!(kept["outgoing"].as_str().unwrap().starts_with("Gestern hat Weber gefehlt."), "a tap keeps it");
    engine.call("chat.send", json!({"text": text})).unwrap();
    engine.call("chat.send", json!({"text": "Und [Name 1]?"})).unwrap();
    assert!(posted(&seen).pop().unwrap().contains("Und [Name 1]?"), "a placeholder is no name");
    for r in posted(&seen) {
        assert!(!r.contains("Weber") && !r.contains("Brandt") && !r.contains("Klaus"), "{r}");
        assert!(!r.contains("[[Name") && !r.contains("[Name [") && !r.contains("[[S"), "a placeholder is no name: {r}");
    }
    let s = engine.state();
    assert_eq!(engine.shown_messages(&s)[1].content, "Klaus kommt morgen.");
    // Without the model only the lists, the rules and the chat's known names count.
    let pv = engine.call("privacy.preview", json!({"text": "Krause und Weber fehlten."})).unwrap();
    assert_eq!(pv["outgoing"], "Krause und [Name 1] fehlten.");
}

/// A local file server for the downloads: path → bytes, `connection: close`; counts requests.
fn file_server(engine: &Engine, files: Vec<(&'static str, Vec<u8>)>) -> (String, Arc<Mutex<usize>>) {
    let count = Arc::new(Mutex::new(0usize));
    let count2 = count.clone();
    let files = Arc::new(files);
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (files, count) = (files.clone(), count2.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = vec![0u8; 4096];
                    while !String::from_utf8_lossy(&buf).contains("\r\n\r\n") {
                        let k = sock.read(&mut chunk).await.unwrap_or(0);
                        if k == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..k]);
                    }
                    *count.lock().unwrap() += 1;
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                    match files.iter().find(|(p, _)| *p == path) {
                        Some((_, body)) => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/octet-stream\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
                                body.len()
                            );
                            let _ = sock.write_all(head.as_bytes()).await;
                            let _ = sock.write_all(body).await;
                        }
                        None => {
                            let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nconnection: close\r\ncontent-length: 0\r\n\r\n").await;
                        }
                    }
                });
            }
        });
        format!("http://{addr}")
    });
    (url, count)
}

fn sha(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// „Namenserkennung laden“ (#73): nothing is downloaded without the yes; a wrong checksum (of
/// the download or of the library inside the package) keeps nothing; the right ones install,
/// a second install loads nothing again, and removing deletes the folder. Never the internet.
#[test]
fn the_name_recognition_downloads_only_after_yes_and_checks_checksums() {
    let (engine, _) = engine_with(&["ok"]);
    let model = b"ein winziges Modell".to_vec();
    let tokenizer = br#"{"model":"klein"}"#.to_vec();
    let lib = b"\x7fELF keine echte Bibliothek".to_vec();
    let mut w = craft_zip::Writer::new();
    w.add("onnxruntime/LICENSE", b"MIT", craft_zip::Compression::Deflated).unwrap();
    w.add("onnxruntime/capi/libonnxruntime.so.1.20.1", &lib, craft_zip::Compression::Deflated).unwrap();
    let wheel = w.finish().unwrap();
    let (url, count) = file_server(&engine, vec![("/model.onnx", model.clone()), ("/tokenizer.json", tokenizer.clone()), ("/ort.whl", wheel.clone())]);
    let part = |path: &str, bytes: &[u8], file: &str| ner::Part {
        what: file.into(),
        url: format!("{url}{path}"),
        sha256: sha(bytes),
        size: bytes.len() as u64,
        file: file.into(),
        extract: None,
        file_sha256: None,
        file_size: None,
    };
    let runtime = ner::Part {
        extract: Some("onnxruntime/capi/libonnxruntime.so.1.20.1".into()),
        file_sha256: Some(sha(&lib)),
        file_size: Some(lib.len() as u64),
        ..part("/ort.whl", &wheel, ner::library_name())
    };
    let good = vec![part("/model.onnx", &model, "model.onnx"), part("/tokenizer.json", &tokenizer, "tokenizer.json"), runtime];
    let dir = std::env::temp_dir().join(format!("aiscraft-ner-{}", uuid::Uuid::new_v4()));
    let rt = &engine.ctx().0.ner;

    // An engine in memory has no source: even with a yes it downloads nothing.
    let refused = engine.call("privacy.ner_install", json!({"yes": true})).unwrap_err();
    assert!(refused.to_string().contains("lädt nichts"), "{refused}");

    // The question first: size, source, licence; nothing downloaded.
    let mut wrong = good.clone();
    wrong[0].sha256 = sha(b"etwas anderes");
    rt.set_source(dir.clone(), wrong);
    let asked = engine.call("privacy.ner_install", json!({})).unwrap();
    assert_eq!(asked["needs_yes"], true);
    assert!(asked["model"].as_str().unwrap().contains("Apache-2.0") && asked["runtime"].as_str().unwrap().contains("MIT"));
    assert_eq!(asked["size"], (model.len() + tokenizer.len() + wheel.len()) as u64);
    assert_eq!(*count.lock().unwrap(), 0, "no download without the yes");

    // A wrong checksum: refused, nothing kept, not installed.
    let e = engine.call("privacy.ner_install", json!({"yes": true})).unwrap_err();
    assert!(e.to_string().contains("Prüfsumme"), "{e}");
    assert!(!dir.join("model.onnx").exists() && !dir.join("model.onnx.part").exists());
    assert!(!engine.call("privacy.ner_status", json!({})).unwrap()["installed"].as_bool().unwrap());

    // The library inside the package does not match: refused too.
    let mut wrong_lib = good.clone();
    wrong_lib[2].file_sha256 = Some(sha(b"andere Bibliothek"));
    rt.set_source(dir.clone(), wrong_lib);
    let e = engine.call("privacy.ner_install", json!({"yes": true})).unwrap_err();
    assert!(e.to_string().contains("Bibliothek"), "{e}");
    assert!(!dir.join(ner::library_name()).exists());
    assert!(!engine.state().privacy.ner.installed);

    // The right checksums: installed, only the library taken out of the package.
    rt.set_source(dir.clone(), good);
    let ok = engine.call("privacy.ner_install", json!({"yes": true})).unwrap();
    assert_eq!(ok["installed"], true);
    assert!(engine.state().privacy.ner.installed && engine.state().privacy.ner.progress.is_none());
    assert_eq!(std::fs::read(dir.join("model.onnx")).unwrap(), model);
    assert_eq!(std::fs::read(dir.join(ner::library_name())).unwrap(), lib);
    assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));
    let before = *count.lock().unwrap();
    engine.call("privacy.ner_install", json!({"yes": true})).unwrap();
    assert_eq!(*count.lock().unwrap(), before, "checked files are not downloaded again");

    // Removing deletes the folder.
    assert_eq!(engine.call("privacy.ner_remove", json!({})).unwrap()["removed"], true);
    assert!(!dir.exists());
    assert!(!engine.call("privacy.ner_status", json!({})).unwrap()["installed"].as_bool().unwrap());
    assert_eq!(engine.call("privacy.ner_remove", json!({})).unwrap()["removed"], false);
}

/// The setup of „Namen schützen“ (#73): not done on a fresh profile, done with
/// `privacy.onboarding_done`, kept across a restart, and brought back with `done: false`.
#[test]
fn onboarding_done_is_remembered() {
    let dir = std::env::temp_dir().join(format!("aiscraft-privacy-onboarding-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let open = || Engine::open(Options { data_dir: Some(dir.clone()), secrets: Some(crate::Secrets::memory()) }).unwrap();
    let engine = open();
    assert!(!engine.state().settings.privacy_onboarded, "a fresh profile gets the setup");
    assert_eq!(engine.call("privacy.onboarding_done", json!({})).unwrap()["done"], true);
    assert!(engine.state().settings.privacy_onboarded);
    assert!(engine.call("privacy.onboarding_done", json!({"done": "ja"})).is_err(), "only true or false");
    assert!(engine.state().settings.privacy_onboarded, "a refused value changes nothing");
    drop(engine);
    let engine = open();
    assert!(engine.state().settings.privacy_onboarded, "kept across a restart");
    engine.call("privacy.onboarding_done", json!({"done": false})).unwrap();
    assert!(!engine.state().settings.privacy_onboarded);
    drop(engine);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The real download from Hugging Face and PyPI with the pinned checksums, into
/// `AISCRAFT_NER_DOWNLOAD_DIR` (about 380 MB; only by hand:
/// `AISCRAFT_NER_DOWNLOAD_DIR=<leerer Ordner> cargo test -p aiscraft-engine --features ner -- --ignored real_download`).
/// Afterwards `AISCRAFT_NER_DIR=<Ordner>/ner … -- --ignored real_model` runs the model.
#[cfg(feature = "ner")]
#[test]
#[ignore]
fn real_download_installs_the_name_recognition() {
    let dir = std::path::PathBuf::from(std::env::var("AISCRAFT_NER_DOWNLOAD_DIR").unwrap());
    let engine = Engine::open(Options { data_dir: Some(dir.clone()), secrets: Some(crate::Secrets::memory()) }).unwrap();
    // Test builds never download on their own; this test asks for the real sources.
    engine.ctx().0.ner.set_source(dir.join("ner"), super::ner::parts().expect("Namenserkennung gibt es für dieses System"));
    let out = engine.call("privacy.ner_install", json!({"yes": true})).unwrap();
    assert_eq!(out["installed"], true, "{out}");
    assert!(dir.join("ner").join("installed.json").is_file());
}

/// With the real model installed (after `real_download`, same folder): what goes out holds no
/// name, also without a class list, and the forms of address stay.
#[cfg(feature = "ner")]
#[test]
#[ignore]
fn real_model_keeps_names_on_the_device() {
    let dir = std::path::PathBuf::from(std::env::var("AISCRAFT_NER_DOWNLOAD_DIR").unwrap());
    let engine = Engine::open(Options { data_dir: Some(dir), secrets: Some(crate::Secrets::memory()) }).unwrap();
    let text = "Gestern hat Weber im Unterricht gefehlt. Ich habe mit Frau Yilmaz über Mehmet gesprochen.";
    // Without a class list protection is off until switched on for the chat.
    engine.call("chat.set_sources", json!({"protect": true})).unwrap();
    let out = engine.call("privacy.preview", json!({"text": text})).unwrap();
    let sent = out.to_string();
    assert_eq!(out["active"], true, "{out}");
    eprintln!("{out:#}");
    for name in ["Weber", "Yilmaz", "Mehmet"] {
        assert!(!out["outgoing"].as_str().unwrap_or(&sent).contains(name), "{name} ginge raus: {out}");
    }
    assert!(out["outgoing"].as_str().unwrap_or(&sent).contains("Frau"), "{out}");
}
