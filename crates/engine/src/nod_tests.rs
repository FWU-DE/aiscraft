//! Features taken from n8n Nod (AGENTS.md §5): #20, #21, #27, #66, #68, #69.

use std::time::Duration;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tests::engine_with_recording;
use crate::{Engine, Options};

/// What the scripted server does with one chat request.
enum Reply {
    /// Stream this text.
    Text(&'static str),
    /// Answer with this HTTP status and JSON body.
    Status(u16, &'static str),
    /// Stream this text once the flag is set (checked every few milliseconds).
    After(Arc<AtomicBool>, &'static str),
}

/// An OpenAI-compatible server: `/models` lists `models`, each chat request is answered as
/// `reply` decides from its body; every body is recorded.
fn scripted(engine: &Engine, models: &'static str, reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> (String, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen2 = seen.clone();
    let reply = Arc::new(reply);
    let url = engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (seen, reply) = (seen2.clone(), reply.clone());
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
                    if req.starts_with("GET") {
                        let head =
                            format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", models.len());
                        let _ = sock.write_all(format!("{head}{models}").as_bytes()).await;
                        return;
                    }
                    seen.lock().unwrap().push(req.clone());
                    let text = match reply(&req) {
                        Reply::Status(code, body) => {
                            let head =
                                format!("HTTP/1.1 {code} X\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                            let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                            return;
                        }
                        Reply::Text(t) => t,
                        Reply::After(flag, t) => {
                            while !flag.load(Ordering::SeqCst) {
                                tokio::time::sleep(Duration::from_millis(5)).await;
                            }
                            t
                        }
                    };
                    let c = json!({"choices":[{"delta":{"content": text}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    });
    (url, seen)
}

/// An engine with one scripted service selected (model `m`); returns the service's id.
fn engine_with(models: &'static str, reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> (Engine, String, Arc<Mutex<Vec<String>>>) {
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    let (url, seen) = scripted(&engine, models, reply);
    let id = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Erster Dienst"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": id})).unwrap();
    engine.call("chat.select_model", json!({"backend": id, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    (engine, id, seen)
}

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

/// 8:10 on the Monday `weeks` weeks before last week's Monday, local time.
fn monday_at_8(weeks: i64) -> i64 {
    use chrono::{Datelike, Local, TimeZone};
    let today = Local::now().date_naive();
    let last_monday = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()) + 7);
    let day = last_monday - chrono::Duration::weeks(weeks);
    Local.from_local_datetime(&day.and_hms_opt(8, 10, 0).unwrap()).earliest().unwrap().timestamp_millis()
}

/// Sends `text` in a new chat and moves it to `at`.
fn ask_at(engine: &crate::Engine, text: &str, at: i64) {
    engine.call("chat.new", json!({})).unwrap();
    engine.call("chat.send", json!({"text": text})).unwrap();
    let id = engine.state().messages.iter().find(|m| m.role == aiscraft_core::Role::User).unwrap().id.clone();
    engine.backdate_message(&id, at).unwrap();
}

/// #68: the same request on three Mondays becomes a schedule idea; tool text does not count, a
/// declined idea does not come back, an accepted one becomes a schedule with the owner's changes.
#[test]
fn recurring_requests_become_schedule_ideas() {
    let (engine, _) = engine_with_recording("ok");
    for (w, text) in
        ["Fass mir die Mails der Schulleitung zusammen", "Bitte fass die Mails der Schulleitung zusammen", "Fass mir die Mails der Schulleitung zusammen"]
            .iter()
            .enumerate()
    {
        ask_at(&engine, text, monday_at_8(w as i64));
    }
    let ideas = engine.call("schedule.ideas", json!({})).unwrap();
    let ideas = ideas.as_array().unwrap();
    assert_eq!(ideas.len(), 1, "{ideas:?}");
    let idea = &ideas[0];
    assert_eq!(idea["cron"], "0 8 * * 1");
    assert_eq!(idea["rhythm"], "montags um 8 Uhr");
    assert_eq!(idea["model_id"], "m");
    assert_eq!(idea["prompt"], "Fass mir die Mails der Schulleitung zusammen");
    let id = idea["id"].as_str().unwrap().to_string();

    // The hints announce it once.
    engine.block_on(crate::proactive::check(engine.ctx(), true)).unwrap();
    let announced = |e: &crate::Engine| {
        e.state().notices.iter().filter(|n| n.action.as_ref().is_some_and(|a| a.command == "schedule.idea_accept" && a.params["id"] == id.as_str())).count()
    };
    assert_eq!(announced(&engine), 1);
    engine.block_on(crate::proactive::check(engine.ctx(), true)).unwrap();
    assert_eq!(announced(&engine), 1, "never twice");

    // Declined: gone for good.
    engine.call("schedule.idea_dismiss", json!({"id": id})).unwrap();
    assert!(engine.state().schedule_ideas.is_empty());
    assert!(engine.call("schedule.ideas", json!({})).unwrap().as_array().unwrap().is_empty(), "a declined idea does not come back");
    assert!(engine.call("schedule.idea_accept", json!({"id": id})).is_err());

    // Accepted with changes: a schedule with that model; then no idea for it any more.
    let (engine, _) = engine_with_recording("ok");
    for w in 0..3 {
        ask_at(&engine, "Fass mir die Mails der Schulleitung zusammen", monday_at_8(w));
    }
    let id = engine.call("schedule.ideas", json!({})).unwrap()[0]["id"].as_str().unwrap().to_string();
    engine.call("schedule.idea_accept", json!({"id": id, "title": "Mails Schulleitung", "cron": "30 7 * * 1"})).unwrap();
    let s = engine.state().schedules.last().cloned().unwrap();
    assert_eq!((s.title.as_str(), s.cron.as_deref(), s.model_id.as_str()), ("Mails Schulleitung", Some("30 7 * * 1"), "m"));
    assert!(engine.call("schedule.ideas", json!({})).unwrap().as_array().unwrap().is_empty());
    // The model per schedule.
    engine.call("schedule.update", json!({"id": s.id, "model": "m2"})).unwrap();
    assert_eq!(engine.state().schedules.last().unwrap().model_id, "m2");
    assert!(engine.call("schedule.update", json!({"id": s.id, "backend": "gibtsnicht"})).is_err());

    // Tool results with the same words on three Mondays are no requests of the owner.
    let (engine, _) = engine_with_recording("ok");
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let conv = engine.state().current.clone().unwrap();
    for w in 0..3 {
        let mut m = engine.state().messages[0].clone();
        m.id = crate::new_id();
        m.role = aiscraft_core::Role::Tool;
        m.content = "Fass mir die Mails der Schulleitung zusammen".into();
        m.created_at = monday_at_8(w);
        m.conversation_id = conv.clone();
        engine.ctx().store(|st| st.save_message(&m)).unwrap();
    }
    assert!(engine.call("schedule.ideas", json!({})).unwrap().as_array().unwrap().is_empty(), "tool text does not count");
}

/// #69: a running task is in the status line until it ends; its result comes back as a card that
/// leads to the task's chat. Schedules show there while they run.
#[test]
fn running_tasks_show_in_the_status_line_and_end_as_a_card() {
    use crate::{JobKind, RunningJob, running_summary};
    let job = |kind, title: &str| RunningJob { id: title.into(), kind, title: title.into(), started_at: 0 };
    assert_eq!(running_summary(&[]), None);
    assert_eq!(running_summary(&[job(JobKind::Task, "Elternbrief prüfen"), job(JobKind::Task, "Noten")]).unwrap(), "2 Aufgaben laufen · Elternbrief prüfen …");
    assert_eq!(running_summary(&[job(JobKind::Task, "A"), job(JobKind::Watch, "Plan")]).unwrap(), "1 Aufgabe läuft · 1 Beobachtung läuft · A …");

    let release = Arc::new(AtomicBool::new(false));
    let r2 = release.clone();
    let (engine, _, _) = engine_with(r#"{"data":[{"id":"m"}]}"#, move |body| {
        if body.contains("Aufgabe") {
            Reply::After(r2.clone(), "**Ergebnis**\nDer Elternbrief ist geprüft, zwei Tippfehler sind korrigiert.")
        } else {
            Reply::Text("ok")
        }
    });
    engine.call("chat.send", json!({"text": "Hallo"})).unwrap();
    let origin = engine.state().current.clone().unwrap();
    let task =
        engine.call("task.start", json!({"title": "Elternbrief prüfen", "instructions": "Prüfe den Elternbrief"})).unwrap()["id"].as_str().unwrap().to_string();
    let s = engine.state();
    assert_eq!(s.running.len(), 1);
    assert_eq!((s.running[0].id.as_str(), s.running[0].kind), (task.as_str(), JobKind::Task));
    assert!(!s.messages.iter().any(|m| m.content.contains("Elternbrief ist geprüft")), "no progress in the chat");
    release.store(true, Ordering::SeqCst);
    let o = origin.clone();
    assert!(engine.wait_for(Duration::from_secs(20), move |s| s.running.is_empty()
        && s.current.as_deref() == Some(o.as_str())
        && s.messages.iter().any(|m| m.activity.iter().any(|a| a.label == crate::TASK_RESULT))));
    let s = engine.state();
    let m = s.messages.iter().find(|m| m.activity.iter().any(|a| a.label == crate::TASK_RESULT)).unwrap();
    let card = crate::task_card(&s, m).unwrap();
    assert_eq!((card.task_id.as_str(), card.title.as_str(), card.ok), (task.as_str(), "Elternbrief prüfen", true));
    assert_eq!(card.line, "Der Elternbrief ist geprüft, zwei Tippfehler sind korrigiert.", "the first line that is no heading");

    // A schedule is listed while it runs, and gone afterwards.
    let r3 = Arc::new(AtomicBool::new(false));
    let r4 = r3.clone();
    let (engine, _, _) = engine_with(r#"{"data":[{"id":"m"}]}"#, move |_| Reply::After(r4.clone(), "Heute nichts Neues"));
    let id = engine.call("schedule.create", json!({"title": "Vertretungsplan", "prompt": "Prüfe den Plan", "cron": "0 7 * * *", "watch": true})).unwrap()["id"]
        .clone();
    engine.dispatch("schedule.run", json!({"id": id}));
    assert!(engine.wait_for(Duration::from_secs(20), |s| s.running.iter().any(|j| j.kind == JobKind::Watch && j.title == "Vertretungsplan")));
    r3.store(true, Ordering::SeqCst);
    assert!(engine.wait_for(Duration::from_secs(20), |s| s.running.is_empty()));
}

/// #66: a backup model steps in when the model fails (503), never for a content error, and never
/// at another service without the owner's consent.
#[test]
fn a_backup_model_steps_in_when_the_model_fails() {
    let (engine, first, seen) = engine_with(r#"{"data":[{"id":"m"},{"id":"b"},{"id":"f"}]}"#, |body| {
        if body.contains(r#""model":"m""#) {
            Reply::Status(503, r#"{"error":{"message":"Das Modell ist überlastet"}}"#)
        } else if body.contains(r#""model":"f""#) {
            Reply::Status(400, r#"{"error":{"message":"Vom Inhaltsfilter blockiert"}}"#)
        } else {
            Reply::Text("Antwort vom Ersatzmodell")
        }
    });
    let last = |e: &Engine| e.state().messages.last().cloned().unwrap();
    // Without backup models the error stays.
    assert!(engine.call("chat.send", json!({"text": "Hallo"})).is_err());
    assert!(last(&engine).error.unwrap().contains("überlastet"));

    engine.call("fallback.set", json!({"models": [{"backend": first, "model": "b"}]})).unwrap();
    engine.call("chat.send", json!({"text": "Noch einmal bitte"})).unwrap();
    let a = last(&engine);
    assert_eq!(a.content, "Antwort vom Ersatzmodell");
    assert!(a.error.is_none());
    assert_eq!(a.model.as_deref(), Some("b"));
    let note = a.activity.iter().find(|s| s.label == crate::FALLBACK).and_then(|s| s.detail.clone()).unwrap();
    assert!(note.contains("Ersatzmodell b") && note.contains("überlastet"), "{note}");
    let models: Vec<&str> = seen.lock().unwrap().iter().rev().take(2).map(|r| if r.contains(r#""model":"b""#) { "b" } else { "m" }).collect();
    assert_eq!(models, ["b", "m"], "first the chosen model, then the backup");

    // A content error never switches.
    engine.call("chat.select_model", json!({"backend": first, "model": "f"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    let before = seen.lock().unwrap().len();
    assert!(engine.call("chat.send", json!({"text": "Hallo"})).is_err());
    assert_eq!(seen.lock().unwrap().len(), before + 1, "no second request");
    assert!(last(&engine).activity.iter().all(|s| s.label != crate::FALLBACK));

    // Another service gets the chat only with consent.
    let (url, seen2) = scripted(&engine, r#"{"data":[{"id":"x"}]}"#, |_| Reply::Text("Antwort vom anderen Dienst"));
    let second = engine.call("backend.add", json!({"kind": "openai", "base_url": url, "name": "Zweiter Dienst"})).unwrap()["id"].as_str().unwrap().to_string();
    engine.call("backend.check", json!({"id": second})).unwrap();
    engine.call("fallback.set", json!({"models": [{"backend": second, "model": "x"}]})).unwrap();
    engine.call("chat.select_model", json!({"backend": first, "model": "m"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
    assert!(engine.call("chat.send", json!({"text": "Hallo"})).is_err());
    assert!(seen2.lock().unwrap().is_empty(), "the other service got nothing");
    engine.call("fallback.set", json!({"other_services": true})).unwrap();
    engine.call("chat.send", json!({"text": "Und jetzt?"})).unwrap();
    let a = last(&engine);
    assert_eq!(a.content, "Antwort vom anderen Dienst");
    assert!(a.activity.iter().any(|s| s.detail.as_deref().is_some_and(|d| d.contains("(Zweiter Dienst)"))));

    // Checks on the list.
    assert!(engine.call("fallback.set", json!({"models": [{"backend": "nix", "model": "x"}]})).is_err());
    assert!(engine.call("fallback.set", json!({"models": [{"backend": first, "model": ""}]})).is_err());
    assert!(engine.call("settings.set", json!({"key": "fallback_other_services", "value": true})).is_err(), "only through fallback.set");
}
