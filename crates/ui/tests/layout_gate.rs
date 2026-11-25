//! Quality gate (layout), AGENTS.md §6: the main views, headless, at 100 % (1280×800) and 150 %
//! (1333×889 points, a 2000×1333 laptop), in Modern, Glas and Omarchy (from a fake theme
//! directory, never the real one). For every view:
//!
//! - no panel grows from frame to frame (a row that overflows widens its panel a little every
//!   frame; at 150 % that once made the artifact panel cover the chat),
//! - the sidebar and the artifact panel keep their default width,
//! - no control lies outside the window or reaches into a neighbouring panel.
//!
//! The gate waits for conditions (the layout settles, a label shows up), not for a fixed number of
//! frames, so it holds under load (issue #71). Each app reads Omarchy's colours from its own
//! directory (`OmarchyTheme`), never the desktop's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::theme::OmarchyTheme;
use aiscraft_ui::{PanelLayout, UiRequest};
use common::Harness;
use egui::accesskit::Role;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};

/// 100 % and 150 %: window size in points and pixels per point.
const SCREENS: [(f32, f32, f32); 2] = [(1280.0, 800.0, 1.0), (1333.0, 889.0, 1.5)];

/// How many frames a settled view must stay put.
const STAY_FRAMES: usize = 40;

/// An answer that tests wrapping: a wide table, a long code line, a long word and a long URL.
const LONG_ANSWER: &str = "## Photosynthese im Überblick\n\nPflanzen bauen aus Licht, Wasser und Kohlenstoffdioxid Traubenzucker auf. Ein sehr langes Wort: Donaudampfschifffahrtsgesellschaftskapitänsmützenabzeichenherstellungsbetrieb. Eine lange Adresse: https://example.org/unterricht/biologie/klasse-7/photosynthese/arbeitsmaterialien/versuche/blattfarbstoffe-chromatographie-anleitung-fuer-schuelerinnen-und-schueler.html\n\n| Stoff | Rolle | Wo | Wann | Menge | Hinweis für den Unterricht |\n|---|---|---|---|---|---|\n| Chlorophyll | fängt das Licht ein | Chloroplasten | tagsüber | viel | grüner Blattfarbstoff, mit Chromatographie sichtbar machen |\n| Wasser | liefert Elektronen | Wurzeln, Leitbündel | immer | sehr viel | Versuch mit gefärbtem Wasser und Selleriestangen |\n\n```python\n# Eine absichtlich sehr lange Codezeile, die nicht umbrochen werden darf und seitlich scrollen muss\nsauerstoff_pro_stunde = blaetter * flaeche_pro_blatt_in_quadratzentimetern * lichtstaerke_in_lux * wirkungsgrad / 1000\n```\n\n1. Blätter nehmen **Kohlenstoffdioxid** auf.\n2. Die Wurzeln liefern **Wasser**.\n\n> 6 CO2 + 6 H2O → C6H12O6 + 6 O2\n\nSoll ich daraus ein Arbeitsblatt machen?";

/// A local OpenAI-compatible server that always streams `answer`; a request that says „langsam“
/// never gets one (a task that keeps running, #69).
fn answer_backend(engine: &Engine, answer: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
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
                    if buf.starts_with(b"GET") {
                        let body = r#"{"data":[{"id":"m"}]}"#;
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", body.len());
                        let _ = sock.write_all(format!("{head}{body}").as_bytes()).await;
                        return;
                    }
                    if String::from_utf8_lossy(&buf).contains("langsam") {
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        return;
                    }
                    // A request for the week's dates calls the connector tool with a view (#74).
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let board = text.split('"').find(|w| w.ends_with("__board") && w.len() < 64).map(str::to_string);
                    if let Some(board) = board.filter(|_| text.contains("Termine der Woche") && !text.contains(r#""role":"tool""#)) {
                        let c = json!({"choices":[{"delta":{"tool_calls": [{"index": 0, "id": "c0", "function": {"name": board, "arguments": "{}"}}]}}]});
                        let done = json!({"choices":[{"delta":{}, "finish_reason": "tool_calls"}]});
                        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                        let _ = sock.write_all(format!("data: {c}\n\ndata: {done}\n\ndata: [DONE]\n\n").as_bytes()).await;
                        return;
                    }
                    let c = json!({"choices":[{"delta":{"content": answer}}]});
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// Opens the app on `engine` at a size and scale, with a control channel for `UiRequest::Show`.
fn open(engine: &Engine, (w, h, ppp): (f32, f32, f32), omarchy: &OmarchyTheme) -> (Harness, std::sync::mpsc::Sender<UiRequest>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let e = engine.clone();
    let mut rx = Some(rx);
    let omarchy = omarchy.clone();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(ppp)
        .allow_missing_glyphs()
        .build_eframe(move |cc| common::app(cc, &e, rx.take(), omarchy.clone()));
    harness.run_steps(2);
    (harness, tx)
}

fn show(h: &mut Harness, tx: &std::sync::mpsc::Sender<UiRequest>, what: &str) {
    let (reply, _r) = std::sync::mpsc::channel();
    tx.send(UiRequest::Show { what: what.into(), reply }).unwrap();
    h.step();
}

/// The controls a person can operate (buttons, fields, tabs …).
fn interactive(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::Link
            | Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::CheckBox
            | Role::RadioButton
            | Role::Switch
            | Role::Tab
            | Role::ComboBox
            | Role::Slider
            | Role::SpinButton
            | Role::MenuItem
    )
}

#[derive(Debug, PartialEq)]
struct Control {
    label: String,
    rect: egui::Rect,
    /// An empty text field: its accessibility rect is as wide as its hint text, also where the
    /// field shows the hint shortened, so it does not count for overlaps.
    empty_field: bool,
}

/// The panels plus every control's name and rect: what must not move once a view has settled.
fn snapshot(h: &Harness) -> (PanelLayout, Vec<Control>) {
    let controls = h
        .query_all(By::new().predicate(|n| interactive(n.role())))
        .map(|n| {
            let a = n.accesskit_node();
            let field = matches!(a.role(), Role::TextInput | Role::MultilineTextInput | Role::SearchInput);
            Control { label: a.label().unwrap_or_default(), rect: n.rect(), empty_field: field && a.value().is_none_or(|v| v.is_empty()) }
        })
        .collect();
    (h.state().panel_layout(), controls)
}

/// Waits until the view is there (`ready`) and the layout stops changing; a panel that keeps
/// growing never settles, so this fails with the widths it went through.
fn settle(h: &mut Harness, what: &str, ready: impl Fn(&Harness) -> bool) {
    let mut last = None;
    let mut same = 0;
    let mut widths = Vec::new();
    for _ in 0..500 {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
        if !ready(h) {
            same = 0;
            last = None;
            continue;
        }
        let now = snapshot(h);
        widths.push((now.0.sidebar.map(|r| r.width()), now.0.artifact.map(|r| r.width())));
        if last.as_ref() == Some(&now) {
            same += 1;
            if same >= 5 {
                return;
            }
        } else {
            same = 0;
        }
        last = Some(now);
    }
    let tail: Vec<_> = widths.iter().rev().take(8).rev().collect();
    panic!("{what}: the layout never settled (sidebar, artifact widths, last frames: {tail:?})");
}

/// The checks of the gate for the view on screen.
fn check(h: &mut Harness, what: &str) {
    let (before, _) = snapshot(h);
    for _ in 0..STAY_FRAMES {
        h.step();
    }
    let (layout, controls) = snapshot(h);
    let grew = |a: Option<egui::Rect>, b: Option<egui::Rect>| match (a, b) {
        (Some(a), Some(b)) => (a.width() - b.width()).abs() > 0.5 || (a.left() - b.left()).abs() > 0.5,
        (a, b) => a.is_some() != b.is_some(),
    };
    assert!(!grew(before.sidebar, layout.sidebar), "{what}: the sidebar changed within {STAY_FRAMES} frames: {:?} → {:?}", before.sidebar, layout.sidebar);
    assert!(
        !grew(before.artifact, layout.artifact),
        "{what}: the artifact panel changed within {STAY_FRAMES} frames: {:?} → {:?}",
        before.artifact,
        layout.artifact
    );
    if let Some(s) = layout.sidebar {
        assert!(
            (s.width() - layout.sidebar_default).abs() <= 2.0,
            "{what}: the sidebar is {:.0} wide, its default is {:.0}",
            s.width(),
            layout.sidebar_default
        );
    }
    if let Some(a) = layout.artifact {
        assert!(
            (a.width() - layout.artifact_default).abs() <= 2.0,
            "{what}: the artifact panel is {:.0} wide, its default is {:.0} (a row inside overflows)",
            a.width(),
            layout.artifact_default
        );
    }
    assert!(!controls.is_empty(), "{what}: no controls found");
    let win = layout.window;
    for Control { label, rect: r, empty_field } in &controls {
        // Scrolled content may sit above or below the window; sideways nothing may stick out.
        assert!(r.left() >= win.left() - 0.5 && r.right() <= win.right() + 0.5, "{what}: „{label}“ at {r:?} lies outside the window {win:?}");
        // Two controls of one row on top of each other: a field that took the buttons' room.
        // Only the same row (scrolled content may pass under a bar, clipped, and that is fine),
        // and not under a dialog (the panels behind it keep their controls).
        if !layout.overlay && !empty_field && win.y_range().contains(r.center().y) {
            for Control { label: other, rect: o, .. } in controls.iter().filter(|c| !c.empty_field) {
                let both = r.intersect(*o);
                let nested = r.contains_rect(*o) || o.contains_rect(*r);
                assert!(
                    nested || both.width() <= 4.0 || both.height() <= 4.0 || (r.center().y - o.center().y).abs() > 2.0,
                    "{what}: „{label}“ at {r:?} lies on „{other}“ at {o:?}"
                );
            }
        }
        if layout.overlay {
            continue;
        }
        // A control belongs to the panel its centre is in and must stay inside it.
        for panel in [layout.sidebar, layout.artifact].into_iter().flatten() {
            let inside = panel.x_range().contains(r.center().x);
            let crosses = if inside {
                r.left() < panel.left() - 0.5 || r.right() > panel.right() + 0.5
            } else {
                r.x_range().intersects(panel.shrink2(egui::vec2(0.5, 0.0)).x_range())
            };
            assert!(!crosses, "{what}: „{label}“ at {r:?} reaches across the edge of the panel at {panel:?}");
        }
    }
}

fn label(h: &Harness, l: &str) -> bool {
    h.query_by_label(l).is_some()
}

fn call(engine: &Engine, cmd: &str, params: Value) -> Value {
    engine.call(cmd, params).unwrap_or_else(|e| panic!("{cmd}: {}", e.message))
}

/// All views of one look at one screen size.
fn gate(theme: &str, screen: (f32, f32, f32), omarchy: &OmarchyTheme) {
    let at = |view: &str| format!("{theme} {}×{} @{}: {view}", screen.0, screen.1, screen.2);
    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    call(&engine, "settings.set", json!({"key": "theme", "value": theme}));
    call(&engine, "settings.set", json!({"key": "look_chosen", "value": true}));
    let url = answer_backend(&engine, LONG_ANSWER);
    let id = call(&engine, "backend.add", json!({"kind": "openai", "base_url": url, "name": "Test", "secret": "sk-test"}))["id"].as_str().unwrap().to_string();
    call(&engine, "backend.check", json!({"id": id}));
    call(&engine, "chat.select_model", json!({"backend": id, "model": "m"}));
    call(&engine, "chat.new", json!({}));
    call(&engine, "chat.send", json!({"text": "Erkläre Photosynthese ausführlich"}));
    assert!(engine.wait_for(std::time::Duration::from_secs(10), |s| s.messages.iter().any(|m| m.content.contains("Donaudampf"))), "the answer arrived");
    let chat = engine.state().current.clone().unwrap();
    let (mut h, tx) = open(&engine, screen, omarchy);

    // Start page.
    call(&engine, "chat.close", json!({}));
    settle(&mut h, &at("Startseite"), |h| label(h, "Stundenentwurf"));
    check(&mut h, &at("Startseite"));

    // Chat with a long Markdown answer.
    call(&engine, "chat.open", json!({"id": chat}));
    settle(&mut h, &at("Chat"), |h| label(h, "Kopieren"));
    check(&mut h, &at("Chat"));

    // Background work (#69): a finished task's card in the chat, a running one in the status line.
    call(&engine, "task.start", json!({"title": "Elternbrief für die Klassenfahrt der 7b prüfen", "instructions": "Prüfe den Elternbrief"}));
    assert!(
        engine.wait_for(std::time::Duration::from_secs(10), |s| s.messages.iter().any(|m| m.activity.iter().any(|a| a.label == aiscraft_engine::TASK_RESULT))),
        "the task's card arrived"
    );
    call(&engine, "task.start", json!({"title": "Noten der Klassenarbeit in die Liste eintragen und Durchschnitt berechnen", "instructions": "langsam"}));
    settle(&mut h, &at("Chat, Aufgaben"), |h| label(h, "Details") && h.query_by_label_contains("1 Aufgabe läuft").is_some());
    check(&mut h, &at("Chat, Aufgaben"));
    // Research in the background (#37, this one finds nothing) and the voice mode (#40) above
    // the composer.
    call(&engine, "research.set_search", json!({"url": url.trim_end_matches("/v1")}));
    call(&engine, "research.start", json!({"question": "Wie leben Igel im Winter und was fressen sie im Herbst, bevor sie schlafen gehen?"}));
    assert!(engine.wait_for(std::time::Duration::from_secs(20), |s| s.research.runs.iter().all(|r| r.status != "running")), "the research ends");
    engine.use_test_audio().unwrap();
    call(&engine, "voice.mode", json!({}));
    settle(&mut h, &at("Chat, Recherche und Sprachmodus"), |h| label(h, "Fertig gesprochen") && label(h, "×"));
    check(&mut h, &at("Chat, Recherche und Sprachmodus"));
    call(&engine, "voice.mode", json!({"on": false}));
    let run = engine.state().research.runs[0].id.clone();
    call(&engine, "research.dismiss", json!({"id": run}));
    settle(&mut h, &at("Recherche ausgeblendet"), |h| !label(h, "×") && !label(h, "Fertig gesprochen"));

    // The same chat with the search open (Strg+F, #41) and a hit marked.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    settle(&mut h, &at("Chat, Suche"), |h| label(h, "Im Chat suchen"));
    call(&engine, "chat.find", json!({"query": "Photosynthese"}));
    settle(&mut h, &at("Chat, Suche"), |h| h.query_by_label_contains(" von ").is_some());
    check(&mut h, &at("Chat, Suche"));
    h.key_press(egui::Key::Escape);
    settle(&mut h, &at("Chat, Suche geschlossen"), |h| !label(h, "Im Chat suchen"));

    // A connector tool's result with a view (#74): the card under it.
    let fake = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/examples/fake_server");
    if fake.is_file() {
        call(
            &engine,
            "connector.add",
            json!({"name": "Kalender der Gesamtschule am Stadtpark", "command": fake.display().to_string(), "env": "FAKE_MCP_MODE=apps"}),
        );
        call(&engine, "chat.new", json!({}));
        call(&engine, "chat.send", json!({"text": "Zeig mir die Termine der Woche"}));
        settle(&mut h, &at("Chat mit Ansicht"), |h| label(h, "Ansicht öffnen"));
        check(&mut h, &at("Chat mit Ansicht"));
        call(&engine, "chat.open", json!({"id": chat}));
        settle(&mut h, &at("Chat"), |h| label(h, "Kopieren"));
    } else {
        eprintln!("skipped the view card: build it with `cargo build -p aiscraft-mcp --example fake_server`");
    }

    // Chat next to each kind of panel.
    let panels: [(&str, &str, Value); 6] = [
        ("Dokument", "artifact.create", json!({"kind": "document", "content": LONG_ANSWER})),
        (
            "H5P",
            "h5p.create",
            json!({"h5p": {"format": "aiscraft-h5p", "version": 1, "title": "Igel – was weißt du über die stacheligen Nachtschwärmer im Garten?", "intro": "Teste dein Wissen.", "tasks": [
                {"type": "choice", "prompt": "Was frisst der Igel am liebsten, wenn er nachts durch den Garten streift?", "options": [{"text": "Insekten, Würmer und Schnecken", "correct": true}, {"text": "Gras"}]},
                {"type": "true_false", "prompt": "Igel halten Winterschlaf.", "answer": true},
                {"type": "cloze", "prompt": "Ergänze.", "text": "Der Igel hat [[Stacheln]]."}
            ]}}),
        ),
        (
            "Lern-App",
            "learning_app.create",
            json!({"html": "<!doctype html><html lang=\"de\"><head><meta charset=\"utf-8\"><title>Bruch-Trainer für die Klasse 6 mit vielen Aufgaben</title></head><body><h1>Bruch-Trainer</h1><button>Neue Aufgabe</button></body></html>"}),
        ),
        ("Whiteboard", "artifact.create", json!({"kind": "board"})),
        (
            "Arbeitsblatt",
            "worksheet.create",
            json!({"worksheet": {"title": "Photosynthese – So stellen Pflanzen Nahrung her", "tasks": [
                {"type": "choice", "prompt": "Welche Zellbestandteile führen die Photosynthese durch?", "options": [{"text": "Chloroplasten", "correct": true}, {"text": "Mitochondrien"}]},
                {"type": "true_false", "prompt": "Pflanzen nehmen Sauerstoff auf.", "answer": false, "hints": ["Welches Gas geben Pflanzen ab?"]},
                {"type": "categorize", "prompt": "Sortiere", "categories": [{"name": "Säugetiere", "items": ["Wal"]}, {"name": "Fische", "items": ["Hai"]}]}
            ]}}),
        ),
        (
            "Arbeitsblatt ohne Punkte",
            "worksheet.create",
            json!({"worksheet": {"title": "Übungsblatt Brüche", "points": false, "tasks": [
                {"type": "choice", "prompt": "Welcher Bruch ist größer?", "options": [{"text": "drei Viertel", "correct": true}, {"text": "zwei Drittel"}]},
                {"type": "cloze", "prompt": "Ergänze.", "text": "Ein Halb sind [[zwei Viertel]].", "word_bank": true}
            ]}}),
        ),
    ];
    for (view, cmd, params) in panels {
        call(&engine, cmd, params);
        let artifact = engine.state().open_artifact.clone();
        assert!(artifact.is_some(), "{}: opened", at(view));
        let what = at(&format!("Chat + {view}"));
        settle(&mut h, &what, |h| label(h, "Im Chat") && h.state().panel_layout().artifact.is_some() && !h.state().panel_layout().overlay);
        check(&mut h, &what);
        if view == "Dokument" {
            // The link field in the composer, narrow next to the panel.
            let what = at("Chat + Dokument, Link-Feld offen");
            let globe = "Webseite anhängen (bis zu 5 Links)";
            common::click_until(&mut h, globe, "the link field opens", |h| label(h, "Abbrechen"));
            settle(&mut h, &what, |h| label(h, "Abbrechen"));
            check(&mut h, &what);
            common::click_until(&mut h, "Abbrechen", "the link field closes", |h| !label(h, "Abbrechen"));
        }
        // The same content in its own window (#75), drawn as the whole screen (kittest hosts no
        // second native window): as wide as the window, nothing sticking out. Learning apps stay
        // in the preview window.
        if view != "Lern-App" {
            let id = artifact.clone().unwrap_or_default();
            call(&engine, "artifact.detach", json!({"id": id}));
            h.state_mut().show_as_window(Some(id.clone()));
            let what = at(&format!("{view} im eigenen Fenster"));
            settle(&mut h, &what, |h| label(h, "Zurück ins Panel") && h.state().panel_layout().artifact.is_some() && !h.state().panel_layout().overlay);
            check(&mut h, &what);
            h.state_mut().show_as_window(None);
            call(&engine, "artifact.attach", json!({"id": id}));
            settle(&mut h, &at("zurück im Panel"), |h| h.state().panel_layout().artifact.is_some() && label(h, "In eigenem Fenster"));
        }
        call(&engine, "artifact.close", json!({}));
        settle(&mut h, &at("Panel geschlossen"), |h| h.state().panel_layout().artifact.is_none());
    }

    // What the settings show once used (#20, #66, #68): a style note, a backup model, a schedule
    // idea from a request made on three earlier weeks.
    call(
        &engine,
        "memory.add",
        json!({"text": "Kurze Sätze, duzt Kolleginnen, grüßt mit „Viele Grüße“, keine Emojis, neue Absätze nach zwei oder drei Sätzen", "area": "style"}),
    );
    call(&engine, "fallback.set", json!({"models": [{"backend": id, "model": "m"}]}));
    for weeks in 1..=3 {
        call(&engine, "chat.new", json!({}));
        call(&engine, "chat.send", json!({"text": "Fass mir die Mails der Schulleitung von dieser Woche zusammen"}));
        let user = engine.state().messages.iter().find(|m| m.role == aiscraft_core::Role::User).map(|m| m.id.clone()).unwrap();
        engine.backdate_message(&user, aiscraft_engine::now_ms() - weeks * 7 * 86_400_000).unwrap();
    }
    assert_eq!(call(&engine, "schedule.ideas", json!({})).as_array().map(Vec::len), Some(1), "a schedule idea to show");
    call(&engine, "chat.open", json!({"id": chat}));

    // Settings, every tab.
    show(&mut h, &tx, "settings");
    settle(&mut h, &at("Einstellungen"), |h| h.query(By::new().label("Dienste").role(Role::Tab)).is_some());
    for tab in ["Dienste", "Konnektoren", "Zeitpläne", "Fähigkeiten", "Tresor", "Namen schützen", "Nutzung", "Gedächtnis", "Darstellung", "Über"] {
        let what = at(&format!("Einstellungen › {tab}"));
        let on = |h: &Harness| h.query(By::new().label(tab).role(Role::Tab).predicate(|n| n.toggled() == Some(egui::accesskit::Toggled::True))).is_some();
        for _ in 0..5 {
            if on(&h) {
                break;
            }
            h.get(By::new().label(tab).role(Role::Tab)).click();
            for _ in 0..50 {
                h.step();
                if on(&h) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        settle(&mut h, &what, on);
        check(&mut h, &what);
    }
    show(&mut h, &tx, "none");

    // Installing a connector package (#43): what it may do and its settings, before it runs.
    let pkg = std::env::temp_dir().join(format!("aiscraft-gate-paket-{}-{}", std::process::id(), theme));
    std::fs::create_dir_all(pkg.join("server")).unwrap();
    std::fs::write(pkg.join("server/index.js"), "// leer").unwrap();
    let manifest = json!({
        "manifest_version": "0.3", "name": "stundenplan", "display_name": "Stundenplan und Vertretungen der Gesamtschule am Stadtpark",
        "version": "1.2.0", "description": "Liest Stundenplan und Vertretungsplan aus dem Schulverwaltungsprogramm und trägt Termine ein.",
        "author": {"name": "Beispiel-Schulträger"},
        "server": {"type": "node", "entry_point": "server/index.js", "mcp_config": {"command": "node", "args": ["${__dirname}/server/index.js", "${user_config.folders}"], "env": {"TOKEN": "${user_config.token}"}}},
        "user_config": {
            "token": {"type": "string", "title": "Zugangsschlüssel des Schulverwaltungsprogramms", "sensitive": true, "required": true},
            "folders": {"type": "directory", "title": "Ordner", "multiple": true},
            "limit": {"type": "number", "title": "Höchstzahl", "default": 5},
            "read_only": {"type": "boolean", "title": "Nur lesen", "default": true}
        }
    });
    std::fs::write(pkg.join("manifest.json"), manifest.to_string()).unwrap();
    call(&engine, "extension.inspect", json!({"path": pkg.display().to_string()}));
    settle(&mut h, &at("Paket installieren"), |h| label(h, "Installieren und starten"));
    check(&mut h, &at("Paket installieren"));
    call(&engine, "extension.cancel", json!({}));
    settle(&mut h, &at("Paket abgebrochen"), |h| !label(h, "Installieren und starten"));
    show(&mut h, &tx, "none");
    let _ = std::fs::remove_dir_all(&pkg);

    // Templates, connector results, research, project knowledge (#54, #53, #37, #32).
    let pid = call(&engine, "project.create", json!({"name": "Biologie 7b mit einem recht langen Namen"}))["id"].as_str().unwrap().to_string();
    let file = std::env::temp_dir().join(format!("aiscraft-gate-lehrplan-{}-{}.md", std::process::id(), theme));
    std::fs::write(
        &file,
        "# Lehrplan Biologie Klasse 7

Igel, Winterschlaf und Fotosynthese.",
    )
    .unwrap();
    call(&engine, "project.file_add", json!({"project": pid, "path": file.display().to_string()}));
    let tpl = engine.state().templates.iter().find(|t| t.title == "Unterrichtsstunde planen").map(|t| t.id.clone()).unwrap();
    for (view, what, ready) in [
        ("Vorlage ausfüllen", format!("template:{tpl}"), "Ins Eingabefeld"),
        ("Neue Vorlage", "template_new".to_string(), "Speichern"),
        ("Recherche", "research".to_string(), "Recherche starten"),
        ("Konnektor-Ergebnis", "connector_insert".to_string(), "Konnektor-Ergebnis einfügen"),
        ("Projekt mit Projektwissen", format!("project:{pid}"), "Datei hinzufügen …"),
        ("Chats übernehmen", "import_chats".to_string(), "Exportdatei wählen …"),
    ] {
        show(&mut h, &tx, &what);
        settle(&mut h, &at(view), |h| label(h, ready));
        check(&mut h, &at(view));
        show(&mut h, &tx, "none");
    }
    let _ = std::fs::remove_file(&file);

    // Entdecken (nobody signed in to AIS.chat: the page explains it).
    show(&mut h, &tx, "personas");
    settle(&mut h, &at("Entdecken"), |h| label(h, "Nicht bei AIS.chat angemeldet"));
    check(&mut h, &at("Entdecken"));

    // The welcome a fresh profile gets.
    let fresh = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    call(&fresh, "settings.set", json!({"key": "theme", "value": theme}));
    let (mut h, _tx) = open(&fresh, screen, omarchy);
    settle(&mut h, &at("Willkommen"), |h| label(h, "Willkommen bei AIS.ChatCraft"));
    check(&mut h, &at("Willkommen"));
}

#[test]
fn layout_gate_modern() {
    for screen in SCREENS {
        gate("modern-light", screen, &OmarchyTheme::none());
    }
}

#[test]
fn layout_gate_glas() {
    for screen in SCREENS {
        gate("glas-dark", screen, &OmarchyTheme::none());
    }
}

#[test]
fn layout_gate_omarchy() {
    let dir = std::env::temp_dir().join(format!("aiscraft-layout-gate-omarchy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("colors.toml"), "background = \"#1e1e2e\"\nforeground = \"#cdd6f4\"\naccent = \"#89b4fa\"\n").unwrap();
    for screen in SCREENS {
        gate("omarchy", screen, &OmarchyTheme::at(&dir));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
