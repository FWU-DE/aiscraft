//! Render the AIS.ChatCraft window offscreen (no window, no focus stealing) and save a PNG.
//!
//! ```sh
//! cargo run -p aiscraft-ui --example snapshot -- --out shot.png --size 1280x800 --demo \
//!     --script '[["chat.send", {"text": "Erkläre Photosynthese"}], ["ui.show", {"what": "settings"}]]'
//! ```
//!
//! `"$last"` in params is the id the previous command returned; `["ui.click", {"label": "Teilen"}]`
//! clicks a widget by its label.
//!
//! `--demo` starts a local OpenAI-compatible server that streams a Markdown answer, adds it as a
//! backend and fills projects, notes and a few chats (a prompt with "Formel" gets formulas and
//! Mermaid diagrams, "Arbeitsblatt Bruchrechnung" a worksheet block, "Lern-App" an html
//! block; `worksheet.request`/`learning_app.request` go through the tools; "Linkliste" one working and one dead link). `--demo-ais` adds a local stand-in for
//! AIS.chat (assistants, characters, learning scenarios, image generation, a plot answer for
//! prompts with "Parabel"), with a catalogue of placeholder portraits for the "Entdecken" page
//! (`["ui.show", {"what": "personas"}]`, `{"what": "persona:character:c1"}` for one entry;
//! `persona.favorite` works against it;
//! `["ui.wait", {"ms": 1500}]` lets the pictures arrive; `["ui.drop", {"paths": [..]}]` drops
//! files on the window). `--theme modern|glas|system|…` (see `aiscraft_engine::THEMES`),
//! `--system light|dark` for what "Wie das System" follows.
//! `--demo-update` hands the update check a made-up newer release (no request to GitHub) and
//! checks, so the hint shows (`["ui.show", {"what": "about"}]` for the settings part).
//! The engine runs in memory; nothing touches your real data.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::mpsc;
use std::time::Duration;

use aiscraft_engine::{Engine, Options};
use aiscraft_ui::{AisCraftApp, UiRequest};
use egui_kittest::kittest::Queryable as _;
use serde_json::{Value, json};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

const ANSWER: &str = "Die **Photosynthese** ist der Vorgang, mit dem Pflanzen aus Licht Energie gewinnen.\n\n## Was passiert?\n\n1. Blätter nehmen **Kohlenstoffdioxid** aus der Luft auf.\n2. Die Wurzeln liefern **Wasser**.\n3. Mit Sonnenlicht entsteht daraus **Traubenzucker** und **Sauerstoff**.\n\n> 6 CO₂ + 6 H₂O → C₆H₁₂O₆ + 6 O₂\n\n| Stoff | Rolle |\n|---|---|\n| Chlorophyll | fängt das Licht ein |\n| Wasser | liefert Elektronen |\n\n```python\n# Sauerstoff pro Stunde (vereinfacht)\nblaetter = 120\nprint(blaetter * 0.5, \"ml\")\n```\n\nSoll ich daraus ein Arbeitsblatt für Klasse 7 machen?";

/// The answer for prompts with "Formel": formulas in the text and on their own line, a Mermaid
/// flowchart and a mind map.
const MATH_ANSWER: &str = "Die **Mitternachtsformel** löst $ax^2 + bx + c = 0$ für $a \\neq 0$:\n\n$$x_{1,2} = \\frac{-b \\pm \\sqrt{b^2 - 4ac}}{2a}$$\n\nDer Ausdruck unter der Wurzel heißt Diskriminante \\(D = b^2 - 4ac\\). Außerdem gilt\n\n\\[\n\\sum_{i=1}^{n} i = \\frac{n(n+1)}{2}, \\quad \\int_0^1 x^2\\,dx = \\frac{1}{3}, \\quad A = \\begin{pmatrix} 1 & 2 \\\\ 3 & 4 \\end{pmatrix}\n\\]\n\nSo gehst du vor:\n\n```mermaid\ngraph TD\n  A[Gleichung aufstellen] --> B{D berechnen}\n  B -->|D > 0| C(Zwei Lösungen)\n  B -->|D = 0| D(Eine Lösung)\n  B -->|D < 0| E([Keine reelle Lösung])\n  C & D --> F[/Probe machen/]\n```\n\n```mermaid\nmindmap\n  root((Quadratische Gleichungen))\n    Formen\n      Normalform\n      Scheitelpunktform\n    Lösen\n      Mitternachtsformel\n      pq-Formel\n      Faktorisieren\n    Graph\n      Parabel\n      Nullstellen\n```\n\nSoll ich Übungsaufgaben dazu erstellen?";

/// The worksheet for prompts with "Arbeitsblatt Bruchrechnung" (as a block) and for
/// `worksheet.request` (through the `worksheet_create` tool).
const DEMO_SHEET: &str = r#"{"format":"aiscraft-worksheet","version":1,"title":"Bruchrechnung: Brüche vergleichen","subject":"Mathematik","grade":"6","instructions":"Bearbeite die Aufgaben der Reihe nach. Kürze, wo es geht.",
"tasks":[
{"id":"a1","type":"choice","prompt":"Welcher Bruch ist am größten?","points":1,"hint":"Bring die Brüche auf einen gemeinsamen Nenner.","feedback":"3/4 = 9/12 ist größer als 2/3 = 8/12 und 1/2 = 6/12.","options":[{"text":"1/2"},{"text":"2/3"},{"text":"3/4","correct":true}]},
{"id":"a2","type":"true_false","prompt":"2/4 und 1/2 sind gleich groß.","points":1,"answer":true,"feedback":"2/4 lässt sich mit 2 kürzen."},
{"id":"a3","type":"cloze","prompt":"Ergänze die Lücken.","points":2,"text":"Kürzt man 6/8 mit 2, erhält man [[3/4]]. Erweitert man 1/3 mit 4, erhält man [[4/12]]."},
{"id":"a4","type":"matching","prompt":"Ordne jedem Bruch die Dezimalzahl zu.","points":3,"pairs":[{"left":"1/4","right":"0,25"},{"left":"1/5","right":"0,2"},{"left":"3/10","right":"0,3"}]},
{"id":"a5","type":"ordering","prompt":"Ordne der Größe nach, den kleinsten zuerst.","points":2,"items":["1/8","1/3","5/6"]},
{"id":"a6","type":"numeric","prompt":"Wie viel ist 3/4 von 20 €?","points":1,"answer":15,"unit":"€"},
{"id":"a7","type":"free_text","prompt":"Erkläre einem Mitschüler, wie man zwei Brüche vergleicht.","points":2,"sample":"Man bringt beide Brüche auf denselben Nenner und vergleicht dann die Zähler.","lines":4}
]}"#;

/// The learning app for prompts with "Lern-App".
const DEMO_APP: &str = "<!doctype html>\n<html lang=\"de\">\n<head>\n<meta charset=\"utf-8\">\n<title>Bruch-Trainer</title>\n<style>\n  body { font-family: system-ui, sans-serif; max-width: 40rem; margin: 2rem auto; }\n  .bar { height: 2rem; background: #e2e8f0; border-radius: .5rem; }\n  .fill { height: 100%; background: #107fad; border-radius: .5rem; }\n</style>\n</head>\n<body>\n<h1>Bruch-Trainer</h1>\n<p>Welcher Bruch ist größer? Tippe auf deine Antwort.</p>\n<div id=\"aufgabe\" role=\"group\" aria-label=\"Aufgabe\"></div>\n<p id=\"rueckmeldung\" aria-live=\"polite\"></p>\n<button id=\"neu\">Neue Aufgabe</button>\n<script>\n  const paare = [[1, 2, 2, 3], [3, 4, 5, 8], [2, 5, 1, 3]];\n  let i = 0;\n  function zeige() {\n    const [a, b, c, d] = paare[i % paare.length];\n    document.getElementById('aufgabe').innerHTML =\n      `<button data-w=\"${a / b}\">${a}/${b}</button> <button data-w=\"${c / d}\">${c}/${d}</button>`;\n  }\n  document.getElementById('aufgabe').addEventListener('click', (e) => {\n    const w = Number(e.target.dataset.w);\n    const alle = [...document.querySelectorAll('#aufgabe button')].map((x) => Number(x.dataset.w));\n    document.getElementById('rueckmeldung').textContent = w === Math.max(...alle) ? 'Richtig!' : 'Noch einmal: Erweitere beide Brüche.';\n  });\n  document.getElementById('neu').addEventListener('click', () => { i++; zeige(); });\n  zeige();\n</script>\n</body>\n</html>";

fn demo_server(engine: &Engine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 1 << 16];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    if req.starts_with("HEAD /weg") || req.starts_with("GET /weg") {
                        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nconnection: close\r\ncontent-length: 0\r\n\r\n").await;
                        return;
                    }
                    if req.starts_with("HEAD /ferien") {
                        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nconnection: close\r\ncontent-length: 0\r\n\r\n").await;
                        return;
                    }
                    if req.starts_with("GET /ferien") {
                        let body = "<html><head><title>Ferientermine Berlin 2027</title></head><body><p>Sommerferien: 8. Juli bis 21. August 2027.</p></body></html>";
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    if req.starts_with("GET") {
                        let body = r#"{"data":[{"id":"demo-sonnet","name":"Demo Sonnet","context_length":200000},{"id":"demo-haiku","name":"Demo Haiku"}]}"#;
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n").await;
                    // "Ferien" + the page reader: read the demo's own page, then answer with it as the source.
                    if req.contains("Ferien") && req.contains("\"web_fetch\"") {
                        let c = if req.contains("\"role\":\"tool\"") {
                            json!({"choices":[{"delta":{"content":"Die Sommerferien in Berlin dauern 2027 vom **8. Juli bis 21. August**."}}]}).to_string()
                        } else {
                            let args = json!({"url": format!("http://{addr}/ferien")}).to_string();
                            let c1 = json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"w1","function":{"name":"web_fetch","arguments":args}}]}}]});
                            format!("{c1}\n\ndata: {}", json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}))
                        };
                        let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                        return;
                    }
                    // What the person wrote last (tool descriptions mention "Notenliste" and "Lern-App" too).
                    let user_text = req
                        .split_once("\r\n\r\n")
                        .and_then(|(_, body)| serde_json::from_str::<Value>(body).ok())
                        .and_then(|v| v["messages"].as_array().and_then(|m| m.iter().rev().find(|x| x["role"] == "user").map(|x| x["content"].to_string())))
                        .unwrap_or_default();
                    // Learning apps and worksheets: through the tools when the prompt helpers ask for
                    // them, as blocks for plain prompts ("Arbeitsblatt Bruchrechnung", "Lern-App").
                    let tool_result = req.contains("\"role\":\"tool\"");
                    let tool = [("Werkzeug worksheet_create", "worksheet_create"), ("Werkzeug learning_app_create", "learning_app_create")].into_iter().find(|(k, _)| req.contains(k));
                    if let Some((_, name)) = tool {
                        let c = if tool_result {
                            json!({"choices":[{"delta":{"content":"Fertig, es ist rechts geöffnet."}}]}).to_string()
                        } else {
                            let args = if name == "worksheet_create" {
                                json!({"worksheet": serde_json::from_str::<Value>(DEMO_SHEET).unwrap()})
                            } else {
                                json!({"title": "Bruch-Trainer", "html": DEMO_APP})
                            };
                            let c1 = json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"l1","function":{"name":name,"arguments":args.to_string()}}]}}]});
                            format!("{c1}\n\ndata: {}", json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}))
                        };
                        let _ = sock.write_all(format!("data: {c}\n\ndata: [DONE]\n\n").as_bytes()).await;
                        return;
                    }
                    let learn = if user_text.contains("Lern-App") {
                        Some(format!("Hier ist eine kleine Lern-App zum Vergleichen von Brüchen:\n\n```html\n{DEMO_APP}\n```\n\nDie Kinder wählen den größeren Bruch und bekommen sofort eine Rückmeldung."))
                    } else if user_text.contains("Arbeitsblatt Bruchrechnung") {
                        Some(format!("Gern, hier ist ein Arbeitsblatt mit sieben Aufgaben:\n\n```aiscraft-worksheet\n{DEMO_SHEET}\n```\n\nDu kannst es rechts bearbeiten, lösen lassen und drucken."))
                    } else {
                        None
                    };
                    if let Some(answer) = learn {
                        for chunk in answer.split_inclusive('\n') {
                            let c = json!({"choices":[{"delta":{"content":chunk}}]});
                            let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                        }
                        let _ = sock.write_all(b"data: [DONE]\n\n").await;
                        return;
                    }
                    // "Notenliste": create a spreadsheet and a deck with the file tools (download cards).
                    if user_text.contains("Notenliste") {
                        let (c1, c2) = if req.contains("\"role\":\"tool\"") {
                            (json!({"choices":[{"delta":{"content":"Hier sind die Notenliste und die Folien für den Elternabend."}}]}), json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
                        } else {
                            let sheet = json!({"title": "Notenliste 7b", "sheets": [{"name": "Mathe", "columns": ["Name", "Note"], "rows": [["Anna", 2], ["Ben", 1], ["Schnitt", "=AVERAGE(B2:B3)"]]}]}).to_string();
                            let deck = json!({"title": "Elternabend 7b", "slides": [{"title": "Themen", "bullets": ["Noten", "Klassenfahrt"]}]}).to_string();
                            (
                                json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"f1","function":{"name":"spreadsheet_create","arguments":sheet}},{"index":1,"id":"f2","function":{"name":"slides_create","arguments":deck}}]}}]}),
                                json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
                            )
                        };
                        let _ = sock.write_all(format!("data: {c1}\n\ndata: {c2}\n\ndata: [DONE]\n\n").as_bytes()).await;
                        return;
                    }
                    // "Kollegium" + a connector tool named *__notify: ask to call it (approval demo).
                    if req.contains("Kollegium") && !req.contains("\"role\":\"tool\"")
                        && let Some(name) = req.split('"').find(|p| p.ends_with("__notify"))
                    {
                        {
                            let c1 = json!({"choices":[{"delta":{"content":"Ich schicke die Einladung ans Kollegium.","tool_calls":[{"index":0,"id":"c1","function":{"name":name,"arguments":"{\"an\":\"kollegium@schule.de\",\"betreff\":\"Konferenz am 4. Dezember\"}"}}]}}]});
                            let c2 = json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]});
                            let _ = sock.write_all(format!("data: {c1}\n\ndata: {c2}\n\ndata: [DONE]\n\n").as_bytes()).await;
                            return;
                        }
                    }
                    // "Linkliste": an answer with one working and one dead link (for "Links prüfen").
                    let links;
                    let answer = if user_text.contains("Linkliste") {
                        links = format!("Zwei Seiten zum Thema:\n\n- [Ferientermine](http://{addr}/ferien)\n- [Arbeitsblätter Photosynthese](http://{addr}/weg/photosynthese)\n\nPrüfe die Links, bevor du sie weitergibst.");
                        links.as_str()
                    } else if user_text.contains("Formel") {
                        MATH_ANSWER
                    } else {
                        ANSWER
                    };
                    for chunk in answer.split_inclusive(' ') {
                        let c = json!({"choices":[{"delta":{"content":chunk}}]});
                        let _ = sock.write_all(format!("data: {c}\n\n").as_bytes()).await;
                    }
                    let _ = sock.write_all(b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":412,\"completion_tokens\":168}}\n\ndata: [DONE]\n\n").await;
                });
            }
        });
        format!("http://{addr}/v1")
    })
}

/// A placeholder portrait for the demo: a soft gradient with a head-and-shoulders silhouette.
fn demo_avatar(seed: u32) -> Vec<u8> {
    let hue = (seed.wrapping_mul(47) % 360) as f32;
    let rgb = |h: f32, s: f32, v: f32| -> [u8; 3] {
        let c = v * s;
        let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
        let (r, g, b) = match (h / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8]
    };
    let img = image::RgbImage::from_fn(320, 320, |x, y| {
        let (fx, fy) = (x as f32, y as f32);
        let head = (fx - 160.0).hypot((fy - 130.0) * 0.9) < 62.0;
        let body = fy > 215.0 && ((fx - 160.0) / 120.0).powi(2) + ((fy - 330.0) / 120.0).powi(2) < 1.0;
        if head || body {
            image::Rgb(rgb((hue + 20.0) % 360.0, 0.35, 0.35 + 0.1 * (seed % 3) as f32))
        } else {
            image::Rgb(rgb(hue, 0.35 + fy / 1600.0, 0.95 - fy / 1100.0))
        }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The demo catalogue: (kind, id, name, description, own, favourite, with picture, extra fields).
fn demo_personas() -> [Value; 3] {
    let entry = |id: &str, name: &str, desc: &str, own: bool, fav: bool, pic: bool, extra: Value| {
        let mut v = json!({"id": id, "name": name, "description": desc, "userId": if own { "me" } else { "x" }, "isFavorite": fav});
        if pic {
            v["maybeSignedPictureUrl"] = json!(format!("https://bilder.demo.invalid/avatars/{id}.png?X-Amz-Signature=demo"));
        }
        if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        v
    };
    [
        json!([
            entry(
                "a1",
                "Mathe-Coach",
                "Übt Bruchrechnung Schritt für Schritt, gibt Tipps statt Lösungen und passt das Tempo an.",
                true,
                true,
                true,
                json!({"isWebSearchEnabled": false, "promptSuggestions": ["Erkläre 3/4 + 1/8", "Gib mir eine Übungsaufgabe", "Prüfe meinen Lösungsweg"]})
            ),
            entry(
                "a2",
                "Elternbrief-Helfer",
                "Formuliert freundliche, klare Elternbriefe in einfacher Sprache und auf Wunsch mehrsprachig.",
                false,
                false,
                true,
                json!({})
            ),
            entry(
                "a3",
                "Korrekturhilfe Deutsch",
                "Gibt Rückmeldung zu Aufsätzen nach dem Bewertungsraster der Fachschaft: Inhalt, Aufbau, Sprache.",
                false,
                true,
                true,
                json!({})
            ),
            entry(
                "a4",
                "Arbeitsblatt-Generator",
                "Erstellt differenzierte Arbeitsblätter in drei Niveaustufen zu einem Thema deiner Wahl.",
                false,
                false,
                false,
                json!({})
            ),
            entry("a5", "Inklusions-Berater", "Hilft beim Anpassen von Material für Lernende mit Förderbedarf.", false, false, true, json!({})),
            entry("a6", "Stundenplaner", "Plant Unterrichtsstunden mit Einstieg, Erarbeitung und Sicherung.", true, false, false, json!({}))
        ]),
        json!([
            entry(
                "c1",
                "Johann Wolfgang von Goethe",
                "Dichter und Naturforscher aus Weimar. Erzählt von Faust, seiner Farbenlehre und der Reise nach Italien.",
                false,
                true,
                true,
                json!({"isWebSearchEnabled": true, "initialMessage": "Seid gegrüßt! Was möchtet Ihr über mein Leben und Werk erfahren?", "promptSuggestions": ["Wie entstand der Faust?", "Was hat Sie in Italien beeindruckt?"]})
            ),
            entry(
                "c2",
                "Marie Curie",
                "Physikerin und Chemikerin, zweifache Nobelpreisträgerin. Spricht über Radioaktivität und ihren Weg in die Wissenschaft.",
                true,
                false,
                true,
                json!({"initialMessage": "Bonjour! Fragt mich nach Polonium, Radium oder dem Labor in Paris."})
            ),
            entry("c3", "Alexander von Humboldt", "Forschungsreisender, vermaß die Anden und den Orinoco.", false, false, true, json!({})),
            entry("c4", "Sophie Scholl", "Studentin und Widerstandskämpferin der Weißen Rose.", false, false, false, json!({})),
            entry("c5", "Albert Einstein", "Erklärt Relativität mit Zügen, Uhren und Lichtstrahlen.", false, false, true, json!({})),
            entry("c6", "Hildegard von Bingen", "Äbtissin, Komponistin und Heilkundige des Mittelalters.", false, false, false, json!({}))
        ]),
        json!([
            entry(
                "l1",
                "Photosynthese erforschen",
                "Klasse 7, Biologie. Die Lernenden finden im Gespräch heraus, was Pflanzen zum Wachsen brauchen.",
                true,
                true,
                true,
                json!({"studentExercise": "Finde heraus, was eine Pflanze zum Wachsen braucht, und erkläre es in drei Sätzen."})
            ),
            entry(
                "l2",
                "Bewerbungsgespräch üben",
                "Klasse 9, Berufsorientierung: ein Gespräch mit einer Personalerin, danach Rückmeldung.",
                false,
                false,
                true,
                json!({"studentExercise": "Stell dich vor und beantworte drei Fragen zu deinem Praktikumswunsch."})
            ),
            entry("l3", "Debatte: Tempolimit", "Pro und Contra mit Argumenten aus Umwelt, Sicherheit und Freiheit.", false, false, false, json!({})),
            entry("l4", "Römisches Alltagsleben", "Klasse 6, Geschichte: Ein Tag in Pompeji, erzählt von einer Händlerin.", false, false, true, json!({}))
        ]),
    ]
}

/// A local stand-in for AIS.chat (`--demo-ais`): signs in, lists assistants, characters and
/// learning scenarios and answers chats. Nothing goes to the real app.
fn demo_ais(engine: &Engine) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // A drawn "generated image": sky, sun, hills.
    let png: std::sync::Arc<Vec<u8>> = {
        let img = image::RgbImage::from_fn(768, 512, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let hill = 360.0 + 40.0 * (fx / 90.0).sin();
            if (fx - 560.0).hypot(fy - 130.0) < 60.0 {
                image::Rgb([255, 214, 90])
            } else if fy > hill {
                image::Rgb([60, (140.0 + (fy - hill) * 0.3) as u8, 70])
            } else {
                image::Rgb([(120.0 + fy * 0.15) as u8, (170.0 + fy * 0.1) as u8, 235])
            }
        });
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        std::sync::Arc::new(out.into_inner())
    };
    let [assistants, characters, scenarios] = demo_personas().map(|v| std::sync::Arc::new(v.to_string()));
    let base = engine.block_on(async {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let png = png.clone();
                let (assistants, characters, scenarios) = (assistants.clone(), characters.clone(), scenarios.clone());
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
                            let len = s[..end].lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    if req.starts_with("GET /files/demo.png") {
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", png.len()).as_bytes()).await;
                        let _ = sock.write_all(&png).await;
                        return;
                    }
                    if let Some(rest) = req.strip_prefix("GET /avatars/") {
                        let seed = rest.bytes().take_while(|b| *b != b'.').fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
                        let pic = demo_avatar(seed);
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: image/png\r\nconnection: close\r\ncontent-length: {}\r\n\r\n", pic.len()).as_bytes()).await;
                        let _ = sock.write_all(&pic).await;
                        return;
                    }
                    let id = |n: u8| format!("{n:02x}").repeat(21);
                    let page = |chunk: &str, row: &str| format!(r#"<script src="/_next/static/chunks/{chunk}.js"></script><script>self.__next_f.push([1,{}])</script>"#, serde_json::to_string(&format!("{row}\n")).unwrap());
                    let reference = |n: u8, name: &str| format!(r#"(0,a.createServerReference)("{}",a.callServer,void 0,a.findSourceMapURL,"{name}");"#, id(n));
                    let flight = |value: &str| format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{value}\n");
                    let action = req.lines().find_map(|l| l.strip_prefix("next-action: ")).unwrap_or("").to_string();
                    let models = r#"3:{"usedBudget":120,"maxBudget":1000,"models":[{"id":"m-gpt","name":"gpt","displayName":"GPT-5","priceMetadata":{"type":"text"}},{"id":"m-img","name":"img","displayName":"Bildmodell","priceMetadata":{"type":"image"}}]}"#;
                    let (ctype, body) = if req.starts_with("GET /api/auth/session") {
                        ("application/json", r#"{"user":{"id":"me","firstName":"Ada","lastName":"Lehrerin","school":{"name":"Gesamtschule Nord"}},"expires":"2030-01-01"}"#.to_string())
                    } else if req.starts_with("GET /api/conversations") {
                        ("application/json", r#"{"conversations":[]}"#.to_string())
                    } else if req.starts_with("GET /image-generation ") {
                        ("text/html", page("images", models))
                    } else if req.starts_with("GET /_next/static/chunks/images.js") {
                        ("application/javascript", reference(0x44, "generateImageAction"))
                    } else if req.starts_with("POST / ") && action == id(0x44) {
                        ("text/x-component", flight(r#"{"success":true,"value":{"imageUrl":"/files/demo.png","conversationId":"srv-img","fileId":"file_1","assistantMessageId":"am1","userMessageId":"um1"}}"#))
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
                        ("application/javascript", reference(0x11, "getAssistantsByFilterAction") + &reference(0x66, "setFavoriteAction"))
                    } else if req.starts_with("GET /_next/static/chunks/characters.js") {
                        ("application/javascript", reference(0x22, "getCharactersByFilterAction"))
                    } else if req.starts_with("GET /_next/static/chunks/scenarios.js") {
                        ("application/javascript", reference(0x33, "getLearningScenariosByFilterAction"))
                    } else if req.starts_with("POST / ") && action == id(0x66) {
                        ("text/x-component", flight(r#"{"success":true}"#))
                    } else if req.starts_with("POST / ") && action == id(0x11) {
                        ("text/x-component", flight(&assistants))
                    } else if req.starts_with("POST / ") && action == id(0x22) {
                        ("text/x-component", flight(&characters))
                    } else if req.starts_with("POST / ") && action == id(0x33) {
                        ("text/x-component", flight(&scenarios))
                    } else if req.starts_with("POST / ") && action == id(0x7f) && req.contains("Parabel") {
                        // A create_plot answer, as AIS.chat streams it: activity, then text with the spec.
                        let activity = serde_json::to_string(&format!("\u{1e}{}", json!({"type": "ai_activity", "steps": [
                            {"kind": "tool", "id": "1", "tool": "web_scraper", "links": [{"title": "Lehrplan Mathematik", "url": "https://schule.example/plan"}, {"title": "", "url": "https://kaputt.example/", "failed": true}]},
                            {"kind": "tool", "id": "2", "tool": "create_plot", "detail": "Parabel y = a·x² mit Schieberegler a, Scheitel markiert", "detailPlacement": "below", "result": "ok"},
                            {"kind": "done"}
                        ]}))).unwrap();
                        let plot = r##"{"board":{"boundingBox":[-5,9,5,-2],"grid":true},"elements":[["slider",[0,1,3],{"name":"a"}],["functiongraph",["a*x^2"],{"name":"f","strokeColor":"#2563eb"}],["functiongraph",["2*x+3"],{"strokeColor":"#dc2626","dash":2}],["point",[0,0],{"name":"S"}],["text",[1.2,7.5,"y = a·x²"]]]}"##;
                        let text = serde_json::to_string(&format!("Hier ist die Parabel mit dem Schieberegler *a*:\n\n```jsxgraph-json\n{plot}\n```\n\nDie rote Gerade schneidet sie zweimal.")).unwrap();
                        ("text/x-component", format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{{\"stream\":\"$2\",\"messageId\":\"$undefined\"}}\n2:R\n2:{activity}\n2:{text}\n2:C\n"))
                    } else if req.starts_with("POST / ") && action == id(0x7f) {
                        let text = serde_json::to_string(ANSWER).unwrap();
                        ("text/x-component", format!("0:{{\"a\":\"$@1\",\"f\":\"\",\"b\":\"b\"}}\n1:{{\"stream\":\"$2\",\"messageId\":\"$undefined\"}}\n2:R\n2:{text}\n2:C\n"))
                    } else {
                        ("text/plain", String::new())
                    };
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    });
    engine.call("backend.update", json!({"id": "ais-chat", "base_url": base})).unwrap();
    engine.set_picture_origin(Some(&base)).unwrap();
    engine.call("backend.login", json!({"id": "ais-chat", "secret": "demo"})).unwrap();
    engine.call("chat.select_model", json!({"backend": "ais-chat", "model": "m-gpt"})).unwrap();
    engine.call("chat.new", json!({})).unwrap();
}

fn seed(engine: &Engine) {
    let url = demo_server(engine);
    engine.allow_local_web().unwrap();
    let id = engine.call("backend.add", json!({"kind": "openai", "name": "Demo", "base_url": url})).unwrap()["id"].as_str().unwrap().to_string();
    let bid = id.clone();
    assert!(engine.wait_for(Duration::from_secs(5), |s| s.backend(&bid).is_some_and(|b| !b.models.is_empty())));
    engine.call("chat.select_model", json!({"backend": id, "model": "demo-sonnet"})).unwrap();
    engine.call("project.create", json!({"name": "Biologie 7b", "instructions": "Antworte für Siebtklässler."})).unwrap();
    engine.call("project.create", json!({"name": "Elternabend", "instructions": ""})).unwrap();
    engine.call("memory.add", json!({"text": "Ich unterrichte Biologie und Chemie an einer Gesamtschule", "area": "about"})).unwrap();
    engine.call("memory.add", json!({"text": "Antworte bitte kurz und mit Beispielen", "area": "preferences"})).unwrap();
    for t in ["Elternbrief zur Klassenfahrt", "Quiz: Zellbestandteile", "Bewertungsraster Referat"] {
        engine.call("chat.new", json!({})).unwrap();
        engine.call("chat.send", json!({"text": t})).unwrap();
    }
    engine.call("chat.new", json!({})).unwrap();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = arg(&args, "--out").unwrap_or_else(|| "snapshot.png".into());
    let (w, h) = arg(&args, "--size")
        .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?))))
        .unwrap_or((1280.0, 800.0));
    let scale: f32 = arg(&args, "--scale").and_then(|s| s.parse().ok()).unwrap_or(1.5);
    let script: Vec<(String, Value)> = arg(&args, "--script").map(|s| serde_json::from_str(&s).expect("--script: [[cmd, params], …]")).unwrap_or_default();

    let engine = Engine::open(Options { data_dir: None, secrets: None }).unwrap();
    if let Some(theme) = arg(&args, "--theme") {
        engine.call("settings.set", json!({"key": "theme", "value": theme})).unwrap();
    }
    if args.iter().any(|a| a == "--demo") {
        seed(&engine);
    }
    if args.iter().any(|a| a == "--demo-ais") {
        demo_ais(&engine);
    }
    if args.iter().any(|a| a == "--demo-update") {
        let release = json!([{"tag_name": "v0.2.0", "name": "AIS.ChatCraft 0.2.0", "draft": false, "prerelease": false,
            "html_url": "https://github.com/FWU-DE/aiscraft/releases/tag/v0.2.0", "body": "- Startseite\n- Bilder in AIS.chat", "assets": []}]);
        engine.set_update_feed(Ok(release.to_string())).unwrap();
        engine.call("app.check_update", json!({})).unwrap();
    }
    let (tx, rx) = mpsc::channel::<UiRequest>();
    let app_engine = engine.clone();
    let mut rx = Some(rx);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(scale)
        .with_max_steps(64)
        // Answers carry arbitrary characters (→, ₂ …); the app draws boxes for missing ones too.
        .allow_missing_glyphs()
        .wgpu()
        .build_eframe(move |cc| AisCraftApp::new(cc, app_engine.clone(), rx.take()));
    // `--system light|dark`: the scheme the system reports ("Wie das System").
    if let Some(theme) = arg(&args, "--system") {
        harness.input_mut().system_theme = Some(if theme == "light" { egui::Theme::Light } else { egui::Theme::Dark });
    }
    harness.run_steps(4);
    // "$last" in a string param stands for the id the previous command returned.
    let mut last = String::new();
    for (cmd, params) in script {
        let params: Value = serde_json::from_str(&params.to_string().replace("$last", &last)).unwrap_or(params);
        if cmd == "ui.click" {
            harness.get_by_label(params["label"].as_str().unwrap_or("")).click();
        } else if cmd == "ui.show" {
            let (rtx, _rrx) = mpsc::channel();
            tx.send(UiRequest::Show { what: params["what"].as_str().unwrap_or("").into(), reply: rtx }).unwrap();
        } else if cmd == "ui.key" {
            let key = egui::Key::from_name(params["key"].as_str().unwrap_or("")).expect("key name");
            let modifiers = if params["ctrl"].as_bool().unwrap_or(false) { egui::Modifiers::COMMAND } else { egui::Modifiers::NONE };
            harness.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            harness.step();
            harness.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
        } else if cmd == "ui.wait" {
            // Let background work (pictures, lists) land: {"ms": 1500}.
            let ms = params["ms"].as_u64().unwrap_or(1000).min(30_000);
            for _ in 0..ms / 50 {
                harness.step();
                std::thread::sleep(Duration::from_millis(50));
            }
        } else if cmd == "ui.drop" {
            // Files dropped on the window: {"paths": ["/tmp/Brief.doc"]} (attachment chips).
            for p in params["paths"].as_array().into_iter().flatten().filter_map(|p| p.as_str()) {
                harness.input_mut().dropped_files.push(std::sync::Arc::new(ScriptFile(p.into())));
            }
        } else if cmd == "ui.type" {
            harness.event(egui::Event::Text(params["text"].as_str().unwrap_or("").into()));
        } else if let Some(c) = cmd.strip_prefix("dispatch:") {
            engine.dispatch(c, params.clone());
            std::thread::sleep(Duration::from_millis(800));
        } else {
            match engine.call(&cmd, params.clone()) {
                Ok(v) => {
                    if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                        last = id.to_string();
                    }
                }
                Err(e) => eprintln!("{cmd}: {e}"),
            }
        }
        harness.run_steps(6);
    }
    for _ in 0..20 {
        harness.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    let img = harness.render().expect("render");
    img.save(&out).expect("save png");
    println!("wrote {out} ({}×{})", img.width(), img.height());
}

/// A file "dropped" by `ui.drop`.
#[derive(Debug)]
struct ScriptFile(std::path::PathBuf);

impl egui::DroppedFile for ScriptFile {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}
