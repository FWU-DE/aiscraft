//! Helpers shared by the UI tests: wait for conditions, not for a fixed number of frames (under
//! heavy load a frame count is sometimes not enough, issue #71).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use aiscraft_ui::AisCraftApp;
use egui_kittest::kittest::{NodeT, Queryable};

pub type Harness = egui_kittest::Harness<'static, AisCraftApp>;

/// The app as the tests open it: no Omarchy theme, whatever the desktop has, unless a test gives
/// one. Each app has its own theme source, so tests running side by side can't change each
/// other's screen (#71).
pub fn app(
    cc: &eframe::CreationContext<'_>,
    engine: &aiscraft_engine::Engine,
    control: Option<std::sync::mpsc::Receiver<aiscraft_ui::UiRequest>>,
    omarchy: aiscraft_ui::theme::OmarchyTheme,
) -> AisCraftApp {
    AisCraftApp::new(cc, engine.clone(), control).with_omarchy_theme(omarchy)
}

/// Whether the control with this label is the chosen one (selected or toggled on).
pub fn chosen(h: &egui_kittest::Harness<'static, AisCraftApp>, label: &str) -> bool {
    h.query_all(
        egui_kittest::kittest::By::new().label(label).predicate(|n| n.is_selected() == Some(true) || n.toggled() == Some(egui::accesskit::Toggled::True)),
    )
    .next()
    .is_some()
}

/// Draws frames until `done` holds (at most about 10 s). Engine commands run in the background,
/// so under load a fixed number of frames is sometimes not enough.
pub fn step_until(h: &mut egui_kittest::Harness<'static, AisCraftApp>, what: &str, done: impl Fn(&egui_kittest::Harness<'static, AisCraftApp>) -> bool) {
    for _ in 0..500 {
        if done(h) {
            return;
        }
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("timed out waiting for: {what}: {}", screen(h));
}

/// Draws frames until a widget with this label is there and has stopped moving, so a click hits
/// it and not where it was a frame earlier.
pub fn settled(h: &mut egui_kittest::Harness<'static, AisCraftApp>, label: &str) {
    let mut last = None;
    for _ in 0..500 {
        let now = h.query_by_label(label).map(|n| n.rect());
        if now.is_some() && now == last {
            return;
        }
        last = now;
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("„{label}“ did not settle: {}", screen(h));
}

/// Clicks `label` until `done` holds. Under heavy load (many test processes at once) a simulated
/// click is now and then not taken; what the click must achieve is still checked.
pub fn click_until(
    h: &mut egui_kittest::Harness<'static, AisCraftApp>,
    label: &str,
    what: &str,
    done: impl Fn(&egui_kittest::Harness<'static, AisCraftApp>) -> bool,
) {
    for _ in 0..5 {
        settled(h, label);
        h.get_by_label(label).click();
        for _ in 0..100 {
            h.step();
            if done(h) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    panic!("clicking „{label}“ did not lead to: {what}: {}", screen(h));
}

/// Every labelled node with its rect, for failure messages: what was on screen instead.
pub fn screen(h: &egui_kittest::Harness<'static, AisCraftApp>) -> String {
    let labels: Vec<String> = h
        .query_all(egui_kittest::kittest::By::new().predicate(|n| n.label().is_some()))
        .map(|n| format!("{:?}@{:?}", n.accesskit_node().label().unwrap_or_default(), n.rect()))
        .collect();
    labels.join(" | ")
}

/// A scripted AIS.chat signed in as `me` (school `s1`, sharing within schools on) that lists
/// `lists` (assistants, characters, learning scenarios) for `get…ByFilterAction("all")`.
/// Returns the base URL; nothing leaves the machine.
pub fn scripted_ais_lists(engine: &aiscraft_engine::Engine, lists: [serde_json::Value; 3]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let lists = std::sync::Arc::new(lists.map(|v| v.to_string()));
    engine.block_on(async move {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let lists = lists.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        let s = String::from_utf8_lossy(&buf);
                        if let Some(end) = s.find("\r\n\r\n") {
                            let len = s[..end]
                                .lines()
                                .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let id = |n: u8| format!("{n:02x}").repeat(21);
                    let page = |chunk: &str, row: &str| {
                        format!(
                            r#"<script src="/_next/static/chunks/{chunk}.js"></script><script>self.__next_f.push([1,{}])</script>"#,
                            serde_json::to_string(&format!("{row}\n")).unwrap()
                        )
                    };
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    let models = r#"3:{"usedBudget":1,"maxBudget":1000,"models":[{"id":"m-gpt","name":"gpt","displayName":"GPT-5","priceMetadata":{"type":"text"}}]}"#;
                    let session = r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin","schoolIds":["s1"],"federalState":{"featureToggles":{"isShareTemplateWithSchoolEnabled":true}}},"expires":"2030-01-01"}"#;
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", session.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("application/json", r#"{"conversations":[]}"#.to_string())
                    } else if req.starts_with("GET /assistants ") {
                        ("text/html", page("assistants", "1:{}"))
                    } else if req.starts_with("GET /characters ") {
                        ("text/html", page("characters", "1:{}"))
                    } else if req.starts_with("GET /learning-scenarios ") {
                        ("text/html", page("scenarios", "1:{}"))
                    } else if req.starts_with("GET / ") {
                        ("text/html", page("app", models))
                    } else if req.starts_with("GET /_next/static/chunks/app.js") {
                        ("application/javascript", reference(0x7f, "sendChatMessageAction"))
                    } else if req.starts_with("GET /_next/static/chunks/assistants.js") {
                        ("application/javascript", reference(0x11, "getAssistantsByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/characters.js") {
                        ("application/javascript", reference(0x22, "getCharactersByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/scenarios.js") {
                        ("application/javascript", reference(0x33, "getLearningScenariosByFilterAction"))
                    } else if req.starts_with("POST / ") && action == id(0x11) {
                        ("text/x-component", flight(&lists[0]))
                    } else if req.starts_with("POST / ") && action == id(0x22) {
                        ("text/x-component", flight(&lists[1]))
                    } else if req.starts_with("POST / ") && action == id(0x33) {
                        ("text/x-component", flight(&lists[2]))
                    } else {
                        ("text/plain", String::new())
                    };
                    let _ = sock
                        .write_all(
                            format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes(),
                        )
                        .await;
                });
            }
        });
        format!("http://{addr}")
    })
}

/// A few assistants, characters and learning scenarios with what AIS.chat's overview filters
/// and sorts by: last change, popularity, sharing and filter values.
pub fn filter_catalog() -> [serde_json::Value; 3] {
    use serde_json::json;
    [
        json!([
            {"id": "a1", "name": "Mathe-Coach", "description": "Übt Bruchrechnung.", "userId": "me", "updatedAt": "$D2026-11-20T08:00:00.000Z", "popularity": 30,
             "filterGroup": {"school_types": ["grammar-school"], "grade_ranges": ["range-2"], "subjects": ["mathematics"]}},
            {"id": "a2", "name": "Elternbrief-Helfer", "description": "Freundliche Elternbriefe.", "userId": "land", "isGlobal": true, "updatedAt": "$D2026-11-02T08:00:00.000Z", "popularity": 120,
             "filterGroup": {"categories": ["organisation"], "languages": ["german", "english"]}},
            {"id": "a3", "name": "Korrekturhilfe Deutsch", "description": "Rückmeldung zu Aufsätzen.", "userId": "kollegin", "isSchoolShared": true, "ownerSchoolIds": ["s1"],
             "updatedAt": "$D2026-11-14T08:00:00.000Z", "popularity": 30, "filterGroup": {"school_types": ["grammar-school", "secondary-school"], "subjects": ["german"], "categories": ["feedback"]}}
        ]),
        json!([
            {"id": "c1", "name": "Johann Wolfgang von Goethe", "description": "Dichter aus Weimar.", "userId": "land", "isGlobal": true, "isFavorite": true,
             "updatedAt": "$D2026-10-11T08:00:00.000Z", "popularity": 98, "filterGroup": {"subjects": ["german"], "categories": ["historical-figures"], "grade_ranges": ["range-3"]}},
            {"id": "c2", "name": "Marie Curie", "description": "Physikerin und Chemikerin.", "userId": "x", "isCommunityShared": true, "ownerSchoolIds": ["s9"],
             "updatedAt": "$D2026-11-18T08:00:00.000Z", "popularity": 40, "filterGroup": {"subjects": ["physics", "chemistry"], "categories": ["historical-figures"]}}
        ]),
        json!([
            {"id": "l1", "name": "Photosynthese erforschen", "description": "Was Pflanzen brauchen.", "userId": "me", "updatedAt": "$D2026-11-21T08:00:00.000Z", "popularity": 45,
             "schoolType": "comprehensive-school", "gradeLevel": "range-2", "subject": "biology", "filterGroup": {}}
        ]),
    ]
}
