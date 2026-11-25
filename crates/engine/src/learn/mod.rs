//! Learning apps and interactive worksheets.
//!
//! A **learning app** is one self-contained HTML file a model wrote (like a claude.ai HTML
//! artifact). AIS.ChatCraft never renders it inside the main window: the panel shows the
//! source; "Öffnen" writes a copy with a strict Content-Security-Policy into the app's data
//! folder and shows it in the separate `aiscraft-preview` process (wry) or, without it, in
//! the system browser. The policy forbids every network request, so a generated app cannot
//! load scripts or phone home.
//!
//! A **worksheet** is the native `aiscraft-worksheet` JSON ([`aiscraft_core::worksheet`]),
//! edited and worked on in egui and printed as PDF or Word (`print.rs`).
//!
//! Both work without model tool calling (AIS.chat): models answer with a fenced ```` ```html ````
//! or ```` ```aiscraft-worksheet ```` block, which the chat offers to open, and the prompt
//! helpers (`learning_app.request`, `worksheet.request`) open it right away. Backends with
//! tools get `learning_app_create`/`worksheet_create` (and `…_update`). Creating an artifact
//! in the app's own data changes nothing of the person's, so these tools run without asking,
//! like `document_create`.

pub mod board;
pub mod calendar;
pub mod formats;
pub mod h5p;
pub mod h5p_player;
pub mod print;

use std::path::{Path, PathBuf};

use aiscraft_core::worksheet::{self, Response, Worksheet};
use aiscraft_core::{Artifact, ArtifactKind, Error, Result, Role, ToolSpec};
use serde_json::{Value, json};

use crate::artifacts::{doc_new, doc_replace, doc_text, get_artifact, save_state, state_of};
use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, NoticeLevel, new_id, now_ms};

/// The largest learning app (HTML file) accepted.
pub const MAX_APP_BYTES: usize = 2 * 1024 * 1024;
/// The policy every opened or saved learning app gets: inline code only, no network.
pub const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; form-action 'none'; base-uri 'none'";
const MAX_TOPIC: usize = 2000;
const MAX_BLOCKS: usize = 20;

// ---- blocks in answers ----------------------------------------------------------------------

/// Fenced code blocks: (language in lower case, body). Unclosed fences (still streaming) are
/// left out.
pub fn fenced_blocks(text: &str) -> Vec<(String, &str)> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < text.len() && out.len() < MAX_BLOCKS {
        let le = text.get(pos..).and_then(|r| r.find('\n')).map(|i| pos + i + 1).unwrap_or(text.len());
        let line = text.get(pos..le).unwrap_or("");
        let t = line.trim_start_matches(' ');
        let fence = t.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = fence
            && line.len() - t.len() <= 3
        {
            let n = t.chars().take_while(|x| *x == c).count();
            if n >= 3 {
                let lang = t.get(n..).unwrap_or("").split_whitespace().next().unwrap_or("").to_lowercase();
                // Find the closing fence.
                let mut q = le;
                let mut close = None;
                while q < text.len() {
                    let qe = text.get(q..).and_then(|r| r.find('\n')).map(|i| q + i + 1).unwrap_or(text.len());
                    let l = text.get(q..qe).unwrap_or("").trim();
                    if l.chars().take_while(|x| *x == c).count() >= n && l.chars().all(|x| x == c) && !l.is_empty() {
                        close = Some((q, qe));
                        break;
                    }
                    q = qe;
                }
                let Some((cs, ce)) = close else { break };
                out.push((lang, text.get(le..cs).unwrap_or("")));
                pos = ce;
                continue;
            }
        }
        pos = le;
    }
    out
}

/// Is this a whole HTML document (doctype or `<html>` up to `</html>`), not a snippet?
pub fn is_html_document(s: &str) -> bool {
    let t = s.trim_start();
    let head: String = t.chars().take(15).collect::<String>().to_lowercase();
    let tail: String = s.trim_end().chars().rev().take(7).collect::<String>().chars().rev().collect::<String>().to_lowercase();
    (head.starts_with("<!doctype html") || head.starts_with("<html")) && tail == "</html>" && s.len() <= MAX_APP_BYTES
}

/// The complete HTML documents in an answer's ```` ```html ```` blocks.
pub fn html_documents(answer: &str) -> Vec<&str> {
    fenced_blocks(answer)
        .into_iter()
        .filter(|(l, b)| matches!(l.as_str(), "html" | "htm" | "xhtml" | "") && is_html_document(b))
        .map(|(_, b)| b.trim())
        .collect()
}

/// The ```` ```aiscraft-worksheet ```` blocks in an answer.
pub fn worksheet_blocks(answer: &str) -> Vec<&str> {
    fenced_blocks(answer).into_iter().filter(|(l, _)| l == worksheet::FENCE || l == "aiscraft-arbeitsblatt").map(|(_, b)| b).collect()
}

/// The `<title>` of an HTML document.
pub fn html_title(html: &str) -> Option<String> {
    let lower = html.get(..html.len().min(64 * 1024)).unwrap_or("").to_ascii_lowercase();
    let a = lower.find("<title")?;
    let start = lower.get(a..)?.find('>')? + a + 1;
    let end = lower.get(start..)?.find("</title>")? + start;
    let t = html.get(start..end)?.trim();
    let t: String = t.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").chars().take(80).collect();
    (!t.is_empty()).then_some(t)
}

/// The HTML with our Content-Security-Policy as the first thing the browser reads in the
/// document, and meta refreshes (navigation without a click) disarmed. A policy the app
/// brings itself stays, but cannot widen ours: browsers enforce every policy.
pub fn with_csp(html: &str) -> String {
    let meta = format!("<meta http-equiv=\"Content-Security-Policy\" content=\"{CSP}\">");
    let lower = html.to_ascii_lowercase();
    // Meta refresh would leave the page; neutralise the attribute value (same length, so the
    // offsets in `lower` stay valid).
    let mut body = html.to_string();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("http-equiv")).map(|i| i + from) {
        let rest = lower.get(i..i + 40).unwrap_or("");
        if rest.contains("refresh")
            && let Some(j) = rest.find("refresh")
        {
            body.replace_range(i + j..i + j + 7, "nothing");
        }
        from = i + 10;
    }
    // After the <html …> tag, else after the doctype, else at the very start.
    let insert_at = match lower.find("<html") {
        Some(i) => lower.get(i..).and_then(|r| r.find('>')).map(|j| i + j + 1),
        None => lower.find("<!doctype").and_then(|i| lower.get(i..).and_then(|r| r.find('>')).map(|j| i + j + 1)),
    }
    .unwrap_or(0);
    let mut out = String::with_capacity(body.len() + meta.len());
    out.push_str(body.get(..insert_at).unwrap_or(""));
    out.push_str(&meta);
    out.push_str(body.get(insert_at..).unwrap_or(""));
    out
}

// ---- prompts ---------------------------------------------------------------------------------

fn topic_line(p: &Value) -> Result<String> {
    let topic: String = opt_str(p, "topic").or_else(|| opt_str(p, "text")).unwrap_or_default().trim().chars().take(MAX_TOPIC).collect();
    // A stray character (a quote, a dash) is no topic: the model would invent one.
    if topic.chars().filter(|ch| ch.is_alphanumeric()).count() < 2 {
        return Err(Error::invalid("Schreib erst das Thema ins Eingabefeld, z. B. „Brüche vergleichen, Klasse 6“"));
    }
    let mut line = topic;
    if let Some(s) = opt_str(p, "subject").filter(|s| !s.trim().is_empty()) {
        line.push_str(&format!(" (Fach: {})", s.trim().chars().take(80).collect::<String>()));
    }
    if let Some(g) = opt_str(p, "grade").filter(|s| !s.trim().is_empty()) {
        line.push_str(&format!(" (Klasse: {})", g.trim().chars().take(40).collect::<String>()));
    }
    Ok(line)
}

/// What a learning app must be: the format description for the prompt (see `formats.rs`).
pub const APP_RULES: &str = "Anforderungen:\n- genau eine eigenständige HTML-Datei (<!doctype html> bis </html>) mit eingebettetem CSS und JavaScript und einem passenden <title>\n- keine externen Skripte, Stylesheets, Schriften, Bilder oder CDNs; kein Netzwerkzugriff (kein fetch, keine Formulare an Server), kein Tracking, keine Cookies\n- funktioniert offline in jedem aktuellen Browser\n- barrierefrei: lang=\"de\", semantisches HTML, beschriftete Bedienelemente, Tastaturbedienung, guter Kontrast\n- alle Texte auf Deutsch, Schülerinnen und Schüler in Du-Form ansprechen\n- interaktiv: ausprobieren, sofortige Rückmeldung, ein Neustart-Knopf\n- Grafiken nur als SVG im Code oder als data:-URL";

/// The request for a learning app. `tools`: the backend can call `learning_app_create`.
pub fn app_prompt(topic: &str, tools: bool) -> String {
    let deliver = if tools {
        "Lege die App mit dem Werkzeug learning_app_create an (Titel und vollständiges HTML) und gib das HTML nicht zusätzlich im Chat aus. Sag danach in zwei Sätzen, wie die App funktioniert."
    } else {
        "Gib die Datei vollständig in genau einem ```html-Codeblock aus, ohne weitere Codeblöcke. Sag danach in zwei Sätzen, wie die App funktioniert."
    };
    format!("{}{topic}\n\n{APP_RULES}\n\n{deliver}", formats::APP_INTRO)
}

/// The request for a worksheet; `points`: the sheet counts points (setting „Arbeitsblätter mit
/// Punkten“).
pub fn worksheet_prompt(topic: &str, count: usize, tools: bool, points: bool) -> String {
    let deliver = if tools {
        "Lege das Arbeitsblatt mit dem Werkzeug worksheet_create an und gib das JSON nicht zusätzlich im Chat aus."
    } else {
        "Antworte mit genau einem ```aiscraft-worksheet-Codeblock (gültiges JSON) und danach höchstens zwei Sätzen."
    };
    format!(
        "{}{topic}\n\nEtwa {count} Aufgaben, verschiedene Aufgabentypen, vom Leichten zum Schweren. Jede Aufgabe mit Lösung{}, wo es hilft mit Tipp (hint) und Erklärung (feedback). Deutsch, Du-Form.\n\n{}\n\n{deliver}",
        formats::WORKSHEET_INTRO,
        if points { " und Punkten" } else { " (ohne Punkte)" },
        formats::worksheet_description(&formats::PromptOptions { worksheet_points: points })
    )
}

fn repair_system(points: bool) -> String {
    format!(
        "Du reparierst ein Arbeitsblatt im JSON-Format aiscraft-worksheet. Behalte Inhalt und Aufgaben bei, behebe nur die genannten Probleme. Antworte nur mit dem korrigierten JSON in einem ```aiscraft-worksheet-Codeblock.\n\n{}",
        worksheet::prompt_schema_for(points)
    )
}

/// The setting „Arbeitsblätter mit Punkten“ for new sheets.
fn points_default(c: &Ctx) -> bool {
    c.state().settings.worksheet_points
}

// ---- creating artifacts ---------------------------------------------------------------------

fn insert_artifact(c: &Ctx, kind: ArtifactKind, title: &str, content: &str, conversation: Option<String>) -> Result<String> {
    let bytes = doc_new(content)?;
    let now = now_ms();
    let state = c.state();
    let a = Artifact {
        id: new_id(),
        kind,
        title: title.chars().take(80).collect(),
        conversation_id: conversation.or_else(|| state.current.clone()),
        project_id: state.selection.project_id.clone(),
        created_at: now,
        updated_at: now,
    };
    c.store(|st| {
        st.save_artifact(&a, Some(&bytes))?;
        st.add_artifact_version(&a.id, "Erstellt", &bytes, now)?;
        Ok(())
    })?;
    let id = a.id.clone();
    c.update(|s| {
        s.artifacts.insert(0, a);
        s.open_artifact = Some(id.clone());
        s.artifact_rev = s.artifact_rev.wrapping_add(1);
    });
    Ok(id)
}

fn message_text(c: &Ctx, mid: &str) -> Result<String> {
    c.state().messages.iter().find(|m| m.id == mid).map(|m| m.content.clone()).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))
}

/// The HTML of a learning app from `html` (a document, or an answer with an html block).
fn app_html(src: &str) -> Result<String> {
    if src.len() > MAX_APP_BYTES * 2 {
        return Err(Error::invalid("Die Lern-App ist größer als 2 MB"));
    }
    let html = if is_html_document(src) { Some(src.trim()) } else { html_documents(src).into_iter().next() };
    let html = html.ok_or_else(|| Error::invalid("Keine vollständige HTML-Datei gefunden (<!doctype html> bis </html>)"))?;
    if html.len() > MAX_APP_BYTES {
        return Err(Error::invalid("Die Lern-App ist größer als 2 MB"));
    }
    Ok(html.to_string())
}

pub(crate) fn create_app(c: &Ctx, html: &str, title: Option<String>, conversation: Option<String>) -> Result<String> {
    let html = app_html(html)?;
    let title = title.filter(|t| !t.trim().is_empty()).or_else(|| html_title(&html)).unwrap_or_else(|| "Lern-App".into());
    insert_artifact(c, ArtifactKind::App, &title, &html, conversation)
}

/// Every new worksheet comes through here: with the setting „Arbeitsblätter mit Punkten“ off,
/// it has no points, whatever the model or the imported JSON said.
pub(crate) fn create_worksheet(c: &Ctx, ws: &Worksheet, title: Option<String>, conversation: Option<String>) -> Result<String> {
    let mut ws = ws.clone();
    if !points_default(c) {
        ws.points = false;
    }
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        ws.title = t.chars().take(200).collect();
    }
    insert_artifact(c, ArtifactKind::Worksheet, &ws.title.clone(), &worksheet::to_json(&ws), conversation)
}

/// Parse, and when that fails or tasks were lost, ask the model once to repair it.
async fn parse_or_repair(c: &Ctx, src: &str, repair: bool) -> Result<(Worksheet, Vec<String>)> {
    let first = worksheet::parse_lenient(src);
    let needs = match &first {
        Err(_) => true,
        Ok((_, problems)) => !problems.is_empty(),
    };
    if !repair || !needs {
        return first.map_err(Error::invalid);
    }
    let problems = match &first {
        Err(e) => vec![e.clone()],
        Ok((_, p)) => p.clone(),
    };
    let src_capped: String = src.chars().take(worksheet::MAX_JSON_BYTES).collect();
    let prompt = format!("Probleme:\n- {}\n\n<arbeitsblatt>\n{src_capped}\n</arbeitsblatt>", problems.join("\n- "));
    let points = match &first {
        Ok((ws, _)) => ws.points && points_default(c),
        Err(_) => points_default(c),
    };
    match crate::chat::complete(c, Some(&repair_system(points)), &prompt).await {
        Ok(out) => {
            let block = worksheet_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
            match (worksheet::parse_lenient(&block), first) {
                (Ok(fixed), Ok(orig)) => Ok(if fixed.1.len() <= orig.1.len() { fixed } else { orig }),
                (Ok(fixed), Err(_)) => Ok(fixed),
                (Err(_), Ok(orig)) => Ok(orig),
                (Err(_), Err(e)) => Err(Error::invalid(e)),
            }
        }
        Err(_) => first.map_err(Error::invalid),
    }
}

fn tools_on(c: &Ctx) -> bool {
    let s = c.state();
    let Ok(b) = c.backend(&s.selection.backend_id) else { return false };
    let caps = b.capabilities();
    let conv_tools = s.current.as_ref().and_then(|id| s.conversation(id)).map(|x| x.sources.tools).unwrap_or(s.selection.sources.tools);
    (caps.tools || caps.tools_via_mcp) && s.settings.tools && conv_tools
}

/// The newest artifact of `kind` created after `since` in the current chat, if a tool made one.
fn made_by_tool(c: &Ctx, kind: ArtifactKind, since: i64) -> Option<String> {
    c.state().artifacts.iter().find(|a| a.kind == kind && a.created_at >= since).map(|a| a.id.clone())
}

fn last_answer(c: &Ctx) -> String {
    c.state().messages.iter().rev().find(|m| m.role == Role::Assistant).map(|m| m.content.clone()).unwrap_or_default()
}

// ---- commands -------------------------------------------------------------------------------

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "learning_app.create",
        label: "Als Lern-App öffnen",
        group: "Lern-Apps",
        shortcut: None,
        params: "html?: string (eine HTML-Datei oder eine Antwort mit ```html-Block), from_message?: string, title?: string",
        palette: false,
        run: |c, p| Box::pin(app_create_cmd(c, p)),
    },
    CommandSpec {
        id: "learning_app.request",
        label: "Lern-App erstellen lassen",
        group: "Lern-Apps",
        shortcut: None,
        params: "topic: string, subject?: string, grade?: string",
        palette: false,
        run: |c, p| Box::pin(app_request(c, p)),
    },
    CommandSpec {
        id: "learning_app.open",
        label: "Lern-App öffnen",
        group: "Lern-Apps",
        shortcut: None,
        params: "id: string",
        palette: false,
        run: |c, p| Box::pin(app_open(c, p)),
    },
    CommandSpec {
        id: "learning_app.save",
        label: "Lern-App speichern",
        group: "Lern-Apps",
        shortcut: None,
        params: "id: string, path: string",
        palette: false,
        run: |c, p| Box::pin(app_save(c, p)),
    },
    CommandSpec {
        id: "worksheet.create",
        label: "Als Arbeitsblatt öffnen",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "content?: string (aiscraft-worksheet-JSON oder eine Antwort mit dem Block), worksheet?: object, from_message?: string, title?: string, repair?: bool (einmal vom Modell reparieren lassen)",
        palette: false,
        run: |c, p| Box::pin(ws_create_cmd(c, p)),
    },
    CommandSpec {
        id: "worksheet.request",
        label: "Arbeitsblatt erstellen lassen",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "topic: string, subject?: string, grade?: string, count?: number",
        palette: false,
        run: |c, p| Box::pin(ws_request(c, p)),
    },
    CommandSpec {
        id: "worksheet.from_document",
        label: "Dokument als Arbeitsblatt",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string (ein Dokument)",
        palette: false,
        run: |c, p| Box::pin(ws_from_document(c, p)),
    },
    CommandSpec {
        id: "worksheet.validate",
        label: "Arbeitsblatt prüfen",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "content: string",
        palette: false,
        run: |_c, p| {
            Box::pin(async move {
                let content = req_str(&p, "content")?;
                Ok(match worksheet::parse_lenient(&content) {
                    Ok((ws, problems)) => json!({"ok": problems.is_empty(), "tasks": ws.tasks.len(), "problems": problems, "worksheet": ws}),
                    Err(e) => json!({"ok": false, "tasks": 0, "problems": [e]}),
                })
            })
        },
    },
    CommandSpec {
        id: "worksheet.update_task",
        label: "Aufgabe ändern",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string, index?: number (ohne: neue Aufgabe am Ende), task?: object, remove?: bool, move_to?: number",
        palette: false,
        run: |c, p| Box::pin(ws_update_task(c, p)),
    },
    CommandSpec {
        id: "worksheet.set_points",
        label: "Punkte vergeben",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string, points: bool (aus: zählt richtige Aufgaben; die Punkte der Aufgaben bleiben erhalten)",
        palette: false,
        run: |c, p| Box::pin(ws_set_points(c, p)),
    },
    CommandSpec {
        id: "worksheet.check",
        label: "Antworten prüfen",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string, answers: {task_id: answer} (choice: [index], true_false: bool, cloze: [text], matching: [right index per left], ordering: [item index], sonst Text)",
        palette: false,
        run: |c, p| Box::pin(ws_check(c, p)),
    },
    CommandSpec {
        id: "worksheet.export",
        label: "Arbeitsblatt exportieren",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string, format: pdf|docx|json, variant?: arbeitsblatt|loesung, path?: string",
        palette: false,
        run: |c, p| Box::pin(ws_export(c, p)),
    },
    CommandSpec {
        id: "worksheet.print",
        label: "Arbeitsblatt drucken",
        group: "Arbeitsblätter",
        shortcut: None,
        params: "id: string, variant?: arbeitsblatt|loesung",
        palette: false,
        run: |c, p| Box::pin(ws_print(c, p)),
    },
];

pub(crate) async fn app_create_cmd(c: Ctx, p: Value) -> Result<Value> {
    let src = match opt_str(&p, "from_message") {
        Some(mid) => message_text(&c, &mid)?,
        None => req_str(&p, "html")?,
    };
    let id = create_app(&c, &src, opt_str(&p, "title"), None)?;
    Ok(json!({"id": id}))
}

async fn app_request(c: Ctx, p: Value) -> Result<Value> {
    let topic = topic_line(&p)?;
    let tools = tools_on(&c);
    let since = now_ms();
    crate::chat::send_with(c.clone(), app_prompt(&topic, tools), vec![], vec![], None, false).await?;
    if let Some(id) = made_by_tool(&c, ArtifactKind::App, since) {
        return Ok(json!({"id": id, "created": true}));
    }
    let answer = last_answer(&c);
    match create_app(&c, &answer, None, None) {
        Ok(id) => Ok(json!({"id": id, "created": true})),
        Err(_) => {
            c.notice(NoticeLevel::Warning, "Die Antwort enthält keine vollständige HTML-Datei. Bitte das Modell, die App als ```html-Block auszugeben.");
            Ok(json!({"created": false}))
        }
    }
}

fn app_of(c: &Ctx, id: &str) -> Result<(Artifact, String)> {
    let a = get_artifact(c, id)?;
    if a.kind != ArtifactKind::App {
        return Err(Error::invalid("Das ist keine Lern-App"));
    }
    let html = doc_text(&state_of(c, id)?)?;
    if html.len() > MAX_APP_BYTES {
        return Err(Error::invalid("Die Lern-App ist größer als 2 MB"));
    }
    Ok((a, html))
}

async fn app_open(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (a, html) = app_of(&c, &id)?;
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keine Fenster und keinen Browser (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    let path = open_copy(&c, &a, "html", with_csp(&html).as_bytes())?;
    let how = show_html(&c, &path, &a.title)?;
    Ok(json!({"path": path.display().to_string(), "shown_in": how}))
}

async fn app_save(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let path = req_str(&p, "path")?;
    let (a, html) = app_of(&c, &id)?;
    crate::disk::write(Path::new(&path), labeled_app(&c, &a, &with_csp(&html)).as_bytes())?;
    c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
    Ok(json!({"path": path}))
}

/// Writes a file to open into `<data>/open` (never somewhere the person did not choose).
fn open_copy(c: &Ctx, a: &Artifact, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = c.0.open_dir.clone();
    crate::disk::create_dir(&dir)?;
    let short: String = a.id.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    let name = crate::office::file_name(&format!("{} {short}", a.title), ext);
    let path = dir.join(name);
    crate::disk::write(&path, bytes)?;
    Ok(path)
}

/// Shows a local HTML file: in the preview window when it is installed, else in the browser.
fn show_html(c: &Ctx, path: &Path, title: &str) -> Result<&'static str> {
    if spawn_preview(c, &[std::ffi::OsStr::new("--file"), path.as_os_str()], title) {
        return Ok("preview");
    }
    system_open(path)?;
    Ok("browser")
}

/// The preview program, when it is installed (feature `preview` of the app).
pub(crate) fn preview_program(c: &Ctx) -> Option<PathBuf> {
    c.0.preview.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().filter(|b| b.is_file())
}

/// Starts the preview program with `args` and a window title; false when it is not installed
/// or did not start.
pub(crate) fn spawn_preview(c: &Ctx, args: &[&std::ffi::OsStr], title: &str) -> bool {
    if let Some(bin) = preview_program(c) {
        let child = std::process::Command::new(&bin)
            .args(args)
            .arg("--title")
            .arg(title.chars().take(80).collect::<String>())
            .arg("--watch-stdin")
            // The preview exits when this pipe closes, i.e. when AIS.ChatCraft exits.
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(child) => {
                let pid = child.id();
                {
                    let mut kids = c.0.previews.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    kids.retain_mut(|k| matches!(k.try_wait(), Ok(None)));
                    kids.push(child);
                }
                // A preview that dies right away (missing WebKitGTK, a crash) would leave the
                // person with nothing: say so.
                let ctx = c.clone();
                c.spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    if let Some(code) = preview_failed(&ctx, pid) {
                        ctx.notice(NoticeLevel::Warning, preview_gone_message(code));
                    }
                });
                return true;
            }
            Err(e) => log::warn!("aiscraft-preview: {e}"),
        }
    }
    false
}

/// Whether the preview process `pid` has ended with an error; then it is forgotten, and the
/// result is its exit code (none when a signal ended it).
pub(crate) fn preview_failed(c: &Ctx, pid: u32) -> Option<Option<i32>> {
    let mut kids = c.0.previews.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut ended = None;
    kids.retain_mut(|k| {
        if k.id() != pid {
            return true;
        }
        match k.try_wait() {
            Ok(Some(status)) => {
                ended = Some(status);
                false
            }
            _ => true,
        }
    });
    ended.filter(|s| !s.success()).map(|s| s.code())
}

/// What a person reads when the preview stopped: which program, and what to do.
pub(crate) fn preview_gone_message(code: Option<i32>) -> String {
    let code = code.map(|c| format!(" (Exit-Code {c})")).unwrap_or_default();
    format!(
        "Die Vorschau (aiscraft-preview) wurde unerwartet beendet{code}. Bitte die Lern-App noch einmal öffnen; wenn das wieder passiert, AIS.ChatCraft neu starten."
    )
}

/// Opens a file with the system's default program (browser, PDF viewer).
fn system_open(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    let mut child = cmd
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::unsupported(format!("Kein Programm zum Öffnen gefunden: {e}")))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub(crate) async fn ws_create_cmd(c: Ctx, p: Value) -> Result<Value> {
    let repair = p.get("repair").and_then(Value::as_bool).unwrap_or(false);
    let (ws, problems) = if let Some(obj) = p.get("worksheet").filter(|v| v.is_object()) {
        worksheet::from_value(obj).map_err(Error::invalid)?
    } else {
        let src = match opt_str(&p, "from_message") {
            Some(mid) => message_text(&c, &mid)?,
            None => req_str(&p, "content")?,
        };
        let block = worksheet_blocks(&src).into_iter().next().map(str::to_string).unwrap_or(src);
        parse_or_repair(&c, &block, repair).await?
    };
    let id = create_worksheet(&c, &ws, opt_str(&p, "title"), None)?;
    if !problems.is_empty() {
        c.notice(NoticeLevel::Warning, format!("Arbeitsblatt geöffnet, bitte prüfen: {}", problems.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
    }
    Ok(json!({"id": id, "tasks": ws.tasks.len(), "problems": problems}))
}

async fn ws_request(c: Ctx, p: Value) -> Result<Value> {
    let topic = topic_line(&p)?;
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(8).clamp(1, 30) as usize;
    let tools = tools_on(&c);
    let since = now_ms();
    crate::chat::send_with(c.clone(), worksheet_prompt(&topic, count, tools, points_default(&c)), vec![], vec![], None, false).await?;
    if let Some(id) = made_by_tool(&c, ArtifactKind::Worksheet, since) {
        return Ok(json!({"id": id, "created": true}));
    }
    let answer = last_answer(&c);
    let Some(block) = worksheet_blocks(&answer).into_iter().next().map(str::to_string) else {
        c.notice(NoticeLevel::Warning, "Die Antwort enthält kein Arbeitsblatt. Bitte das Modell, es als ```aiscraft-worksheet-Block auszugeben.");
        return Ok(json!({"created": false}));
    };
    let (ws, problems) = parse_or_repair(&c, &block, true).await?;
    let id = create_worksheet(&c, &ws, None, None)?;
    Ok(json!({"id": id, "created": true, "tasks": ws.tasks.len(), "problems": problems}))
}

async fn ws_from_document(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let a = get_artifact(&c, &id)?;
    if a.kind != ArtifactKind::Document {
        return Err(Error::invalid("Nur Dokumente lassen sich in ein Arbeitsblatt umwandeln"));
    }
    let text: String = doc_text(&state_of(&c, &id)?)?.chars().take(200_000).collect();
    let system = format!(
        "Du wandelst ein Dokument in ein interaktives Arbeitsblatt um: Aufgaben aus dem Dokument werden passende Aufgabentypen mit Lösungen{}, Lesetexte werden info-Blöcke. Deutsch, Du-Form. Antworte nur mit einem ```aiscraft-worksheet-Codeblock (gültiges JSON).\n\n{}",
        if points_default(&c) { " und Punkten" } else { "" },
        worksheet::prompt_schema_for(points_default(&c))
    );
    let out = crate::chat::complete(&c, Some(&system), &format!("<dokument titel=\"{}\">\n{text}\n</dokument>", a.title.replace('"', "'"))).await?;
    let block = worksheet_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
    let (ws, problems) = parse_or_repair(&c, &block, true).await?;
    let new_id = create_worksheet(&c, &ws, Some(ws.title.clone()).filter(|t| t != "Arbeitsblatt").or(Some(a.title.clone())), a.conversation_id.clone())?;
    Ok(json!({"id": new_id, "tasks": ws.tasks.len(), "problems": problems}))
}

/// The worksheet stored in an artifact.
pub(crate) fn worksheet_of(c: &Ctx, id: &str) -> Result<(Artifact, Worksheet, Vec<u8>)> {
    let a = get_artifact(c, id)?;
    if a.kind != ArtifactKind::Worksheet {
        return Err(Error::invalid("Das ist kein Arbeitsblatt"));
    }
    let state = state_of(c, id)?;
    let (ws, _) = worksheet::parse_lenient(&doc_text(&state)?).map_err(|e| Error::invalid(format!("Arbeitsblatt beschädigt: {e}")))?;
    Ok((a, ws, state))
}

pub(crate) fn save_worksheet(c: &Ctx, id: &str, base: &[u8], ws: &Worksheet, note: &str) -> Result<()> {
    let new_state = doc_replace(base, &worksheet::to_json(ws))?;
    save_state(c, id, &new_state, Some(note))
}

async fn ws_update_task(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (_, mut ws, state) = worksheet_of(&c, &id)?;
    let note = h5p::apply_task_edit(&mut ws.tasks, &p)?;
    worksheet::normalize_ids(&mut ws);
    save_worksheet(&c, &id, &state, &ws, &note)?;
    Ok(json!({"tasks": ws.tasks.len(), "problems": worksheet::validate(&ws)}))
}

async fn ws_set_points(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let points = p.get("points").and_then(Value::as_bool).ok_or_else(|| Error::invalid("points fehlt (true oder false)"))?;
    let (_, mut ws, state) = worksheet_of(&c, &id)?;
    if ws.points != points {
        ws.points = points;
        save_worksheet(&c, &id, &state, &ws, if points { "Mit Punkten" } else { "Ohne Punkte" })?;
    }
    Ok(json!({"points": points}))
}

async fn ws_check(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (_, ws, _) = worksheet_of(&c, &id)?;
    let empty = serde_json::Map::new();
    let given = p.get("answers").and_then(Value::as_object).unwrap_or(&empty);
    let answers: std::collections::HashMap<String, Response> =
        ws.tasks.iter().filter_map(|t| given.get(&t.id).map(|v| (t.id.clone(), Response::from_json(&t.body, v)))).collect();
    Ok(serde_json::to_value(worksheet::score(&ws, &answers))?)
}

fn variant(p: &Value) -> print::Variant {
    match opt_str(p, "variant").as_deref() {
        Some("loesung" | "lösung" | "solution" | "loesungsblatt" | "lösungsblatt") => print::Variant::Solution,
        _ => print::Variant::Sheet,
    }
}

/// Bytes of a worksheet export: (bytes, extension).
/// A worksheet as a file; `note`: Markdown put under the sheet („Erstellt mit KI“, #57), not
/// into the JSON.
pub(crate) fn worksheet_bytes(ws: &Worksheet, format: &str, v: print::Variant, note: &str) -> Result<(Vec<u8>, &'static str)> {
    let md = || format!("{}{note}", print::worksheet_markdown(ws, v));
    let title = format!("{} ({})", ws.title, v.label());
    Ok(match format {
        "pdf" => (crate::office::pdf::markdown_pdf(&title, &md())?, "pdf"),
        "docx" | "word" => (crate::office::markdown_docx(&title, &md())?, "docx"),
        "json" => (worksheet::to_json(ws).into_bytes(), "json"),
        "markdown" | "md" => (md().into_bytes(), "md"),
        other => return Err(Error::invalid(format!("Format {other} gibt es für Arbeitsblätter nicht (pdf, docx, json, markdown)"))),
    })
}

async fn ws_export(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (a, ws, _) = worksheet_of(&c, &id)?;
    let format = opt_str(&p, "format").unwrap_or_else(|| "pdf".into());
    let (bytes, _) = worksheet_bytes(&ws, &format, variant(&p), &note_md(&c, &a))?;
    if let Some(path) = opt_str(&p, "path") {
        crate::disk::write(Path::new(&path), &bytes)?;
        c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
        return Ok(json!({"path": path}));
    }
    use base64::Engine as _;
    Ok(json!({"base64": base64::engine::general_purpose::STANDARD.encode(&bytes), "bytes": bytes.len()}))
}

async fn ws_print(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let (a, ws, _) = worksheet_of(&c, &id)?;
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung öffnet AIS.ChatCraft keine anderen Programme (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    let v = variant(&p);
    let (bytes, _) = worksheet_bytes(&ws, "pdf", v, &note_md(&c, &a))?;
    let named = Artifact { title: format!("{} {}", a.title, v.label()), ..a };
    let path = open_copy(&c, &named, "pdf", &bytes)?;
    system_open(&path)?;
    Ok(json!({"path": path.display().to_string()}))
}

/// The AI note (#57) of an artifact as Markdown to put under it; empty when the setting says no.
fn note_md(c: &Ctx, a: &Artifact) -> String {
    crate::ai_label::note_for_artifact(c, a).map(|n| n.markdown()).unwrap_or_default()
}

/// A learning app with the AI note (#57) as a footer before `</body>` (or at the end).
fn labeled_app(c: &Ctx, a: &Artifact, html: &str) -> String {
    let Some(note) = crate::ai_label::note_for_artifact(c, a) else { return html.to_string() };
    let at = html.to_ascii_lowercase().rfind("</body>").unwrap_or(html.len());
    let (head, tail) = html.split_at_checked(at).unwrap_or((html, ""));
    format!("{head}{}\n{tail}", note.html())
}

/// `artifact.export` for learning apps and worksheets; `None` for other kinds.
pub(crate) fn export_for(c: &Ctx, a: &Artifact, p: &Value) -> Option<Result<Value>> {
    let format = opt_str(p, "format");
    let bytes: Result<Vec<u8>> = match a.kind {
        ArtifactKind::App => match format.as_deref().unwrap_or("html") {
            "html" => app_of(c, &a.id).map(|(_, h)| labeled_app(c, a, &with_csp(&h)).into_bytes()),
            other => Err(Error::invalid(format!("Format {other} gibt es für Lern-Apps nicht (html)"))),
        },
        ArtifactKind::Worksheet => {
            worksheet_of(c, &a.id).and_then(|(_, ws, _)| worksheet_bytes(&ws, format.as_deref().unwrap_or("pdf"), variant(p), &note_md(c, a)).map(|(b, _)| b))
        }
        ArtifactKind::H5p => h5p::export_bytes(c, a, format.as_deref()),
        _ => return None,
    };
    Some(bytes.and_then(|bytes| {
        if let Some(path) = opt_str(p, "path") {
            crate::disk::write(Path::new(&path), &bytes)?;
            c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
            return Ok(json!({"path": path}));
        }
        match String::from_utf8(bytes) {
            Ok(text) => Ok(json!({"text": text})),
            Err(e) => {
                use base64::Engine as _;
                let b = e.into_bytes();
                Ok(json!({"base64": base64::engine::general_purpose::STANDARD.encode(&b), "bytes": b.len()}))
            }
        }
    }))
}

/// `artifact.revise` for learning apps and worksheets: a new version from the model.
pub(crate) async fn revise(c: &Ctx, a: &Artifact, instruction: &str) -> Result<Value> {
    let state = state_of(c, &a.id)?;
    let current = doc_text(&state)?;
    // The points switch belongs to the teacher: a revision keeps it.
    let sheet_points = match a.kind {
        ArtifactKind::Worksheet => worksheet::parse_lenient(&current).map(|(ws, _)| ws.points).unwrap_or(true),
        _ => true,
    };
    let (system, prompt) = match a.kind {
        ArtifactKind::App => (
            "Du überarbeitest eine Lern-App, eine eigenständige HTML-Datei, nach der Anweisung der Person. Weiterhin keine externen Skripte, Schriften, Bilder oder Netzwerkzugriffe. Antworte nur mit der vollständigen, überarbeiteten Datei in einem ```html-Codeblock.".to_string(),
            format!("<lern-app>\n{current}\n</lern-app>\n\nAnweisung: {instruction}"),
        ),
        ArtifactKind::Worksheet => (
            format!(
                "Du überarbeitest ein Arbeitsblatt im JSON-Format aiscraft-worksheet nach der Anweisung der Person. Antworte nur mit dem vollständigen, überarbeiteten JSON in einem ```aiscraft-worksheet-Codeblock.\n\n{}",
                worksheet::prompt_schema_for(sheet_points)
            ),
            format!("<arbeitsblatt>\n{current}\n</arbeitsblatt>\n\nAnweisung: {instruction}"),
        ),
        ArtifactKind::H5p => (h5p::revise_system(), format!("<h5p>\n{current}\n</h5p>\n\nAnweisung: {instruction}")),
        _ => return Err(Error::unsupported("Überarbeiten geht hier nicht")),
    };
    let out = crate::chat::complete(c, Some(&system), &prompt).await?;
    let revised = match a.kind {
        ArtifactKind::App => app_html(&out).map_err(|_| Error::protocol("Das Modell hat keine vollständige HTML-Datei zurückgegeben"))?,
        ArtifactKind::H5p => h5p::revised(c, &a.id, out).await?,
        _ => {
            let block = worksheet_blocks(&out).into_iter().next().map(str::to_string).unwrap_or(out);
            let (mut ws, _) = parse_or_repair(c, &block, true).await?;
            ws.points = sheet_points;
            worksheet::to_json(&ws)
        }
    };
    c.store(|st| st.add_artifact_version(&a.id, "Vor KI-Überarbeitung", &state, now_ms()))?;
    let new_state = doc_replace(&state, &revised)?;
    save_state(c, &a.id, &new_state, Some(&format!("KI: {}", instruction.chars().take(60).collect::<String>())))?;
    c.notice(NoticeLevel::Success, format!("{} überarbeitet. Die vorige Fassung liegt unter Versionen.", a.kind.label()));
    Ok(json!({"text": revised}))
}

/// What a model without tools is told when the artifact goes along as context.
pub(crate) fn change_note(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::App => {
            "\n\n(Wenn du diese Lern-App ändern sollst, gib die vollständige neue Fassung als eigenständige HTML-Datei in einem ```html-Codeblock aus.)"
        }
        ArtifactKind::Worksheet => {
            "\n\n(Wenn du dieses Arbeitsblatt ändern sollst, gib die vollständige neue Fassung im selben JSON-Format in einem ```aiscraft-worksheet-Codeblock aus.)"
        }
        ArtifactKind::H5p => h5p::CHANGE_NOTE,
        _ => "",
    }
}

// ---- model tools ----------------------------------------------------------------------------

/// The tools; `worksheet_points`: the setting „Arbeitsblätter mit Punkten“, so new sheets from
/// `worksheet_create` are described without points when it is off.
pub(crate) fn tool_specs(worksheet_points: bool) -> Vec<ToolSpec> {
    let mut specs = vec![
        ToolSpec {
            name: "learning_app_create".into(),
            description: "Legt eine Lern-App an: eine eigenständige HTML-Datei mit eingebettetem CSS und JavaScript, die neben dem Chat geöffnet wird. Keine externen Skripte, CDNs, Schriften oder Bilder, kein Netzwerk (wird blockiert), Deutsch, barrierefrei, offline lauffähig.".into(),
            input_schema: json!({"type": "object", "properties": {
                "title": {"type": "string"},
                "html": {"type": "string", "description": "die vollständige Datei von <!doctype html> bis </html>, höchstens 2 MB"}
            }, "required": ["title", "html"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "learning_app_update".into(),
            description: "Ersetzt eine Lern-App durch eine neue Fassung (die alte bleibt als Version erhalten).".into(),
            input_schema: json!({"type": "object", "properties": {"id": {"type": "string"}, "html": {"type": "string"}}, "required": ["id", "html"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "worksheet_create".into(),
            description: format!(
                "Legt ein interaktives Arbeitsblatt an, das Lehrkräfte bearbeiten, Schülerinnen und Schüler am Rechner lösen und das als PDF (Arbeitsblatt und Lösungsblatt) gedruckt werden kann. Für Aufgaben zum Ausfüllen besser als document_create.\n{}",
                worksheet::prompt_schema_for(worksheet_points)
            ),
            input_schema: json!({"type": "object", "properties": {
                "worksheet": {"type": "object", "description": "das Arbeitsblatt im Format aiscraft-worksheet (title, subject, grade, instructions, tasks)"}
            }, "required": ["worksheet"], "additionalProperties": false}),
        },
        ToolSpec {
            name: "worksheet_update".into(),
            description: "Ersetzt ein Arbeitsblatt durch eine neue, vollständige Fassung im selben Format (die alte bleibt als Version erhalten).".into(),
            input_schema: json!({"type": "object", "properties": {"id": {"type": "string"}, "worksheet": {"type": "object"}}, "required": ["id", "worksheet"], "additionalProperties": false}),
        },
    ];
    specs.extend(h5p::tool_specs());
    specs.extend(board::tool_specs());
    specs.extend(calendar::tool_specs());
    specs
}

/// Runs one of the tools above; `None` when `name` is none of them.
pub(crate) fn run_tool(c: &Ctx, name: &str, a: &Value) -> Option<Result<String>> {
    if let Some(r) = h5p::run_tool(c, name, a).or_else(|| board::run_tool(c, name, a)) {
        return Some(r);
    }
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(match name {
        "learning_app_create" => create_app(c, &s("html"), Some(s("title")), None).map(|id| format!("Lern-App angelegt und geöffnet (ID {id}).")),
        "learning_app_update" => (|| {
            let id = s("id");
            let (_, _) = app_of(c, &id)?;
            let html = app_html(&s("html"))?;
            let state = state_of(c, &id)?;
            save_state(c, &id, &doc_replace(&state, &html)?, Some("Vom Modell geändert"))?;
            Ok("Lern-App aktualisiert.".to_string())
        })(),
        "worksheet_create" => {
            let v = a.get("worksheet").cloned().unwrap_or_else(|| a.clone());
            // Some models send the JSON as a string.
            let parsed = match &v {
                Value::String(src) => worksheet::parse_lenient(src),
                _ => worksheet::from_value(&v),
            };
            parsed.map_err(Error::invalid).and_then(|(ws, problems)| {
                let id = create_worksheet(c, &ws, None, None)?;
                let mut out = format!("Arbeitsblatt „{}“ mit {} Aufgaben angelegt und geöffnet (ID {id}).", ws.title, ws.tasks.len());
                if !problems.is_empty() {
                    out.push_str(&format!(" Probleme, bitte mit worksheet_update beheben: {}", problems.join("; ")));
                }
                Ok(out)
            })
        }
        "worksheet_update" => (|| {
            let id = s("id");
            let (_, old, state) = worksheet_of(c, &id)?;
            let v = a.get("worksheet").cloned().unwrap_or(Value::Null);
            let (ws, problems) = match &v {
                Value::String(src) => worksheet::parse_lenient(src),
                _ => worksheet::from_value(&v),
            }
            .map_err(Error::invalid)?;
            // The points switch belongs to the teacher: the model's new version keeps it.
            let mut ws = ws;
            ws.points = old.points;
            save_worksheet(c, &id, &state, &ws, "Vom Modell geändert")?;
            Ok(if problems.is_empty() {
                "Arbeitsblatt aktualisiert.".to_string()
            } else {
                format!("Arbeitsblatt aktualisiert. Probleme: {}", problems.join("; "))
            })
        })(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;

    const APP: &str = "<!DOCTYPE html>\n<html lang=\"de\"><head><title>Brüche &amp; mehr</title><meta http-equiv=\"refresh\" content=\"0;url=https://evil.example\"></head><body><script>fetch('https://evil.example')</script></body></html>";

    #[test]
    fn html_documents_only_whole_files() {
        let answer = format!("Hier:\n```html\n{APP}\n```\nund ein Schnipsel:\n```html\n<b>fett</b>\n```\n```html\n<html>offen");
        let docs = html_documents(&answer);
        assert_eq!(docs, vec![APP]);
        assert!(html_documents("```html\n<div></div>\n```").is_empty());
        assert!(html_documents("").is_empty());
        assert_eq!(html_title(APP).as_deref(), Some("Brüche & mehr"));
        assert_eq!(html_title("<html></html>"), None);
        let blocks = worksheet_blocks("```aiscraft-worksheet\n{}\n```\n~~~python\nx\n~~~");
        assert_eq!(blocks, vec!["{}\n"]);
    }

    #[test]
    fn csp_comes_first_and_refresh_is_disarmed() {
        let out = with_csp(APP);
        let csp_at = out.find("Content-Security-Policy").unwrap();
        assert!(csp_at < out.find("<head>").unwrap() && csp_at < out.find("<script>").unwrap());
        assert!(out.contains(CSP));
        assert!(!out.to_lowercase().contains("\"refresh\""), "{out}");
        assert!(out.contains("default-src 'none'"));
        // Without <html>, after the doctype; without both, at the start.
        assert!(with_csp("<!doctype html><p>x").starts_with("<!doctype html><meta http-equiv"));
        assert!(with_csp("<p>x").starts_with("<meta http-equiv"));
        for s in ["", "<html", "<!doctype", "ä<html ä>ö", "http-equiv", "http-equiv=refresh"] {
            let _ = with_csp(s);
        }
    }

    #[test]
    fn prompts_name_the_constraints() {
        let p = app_prompt("Brüche", false);
        assert!(p.contains("```html") && p.contains("keine externen Skripte") && p.contains("offline") && p.contains("Du-Form"));
        assert!(app_prompt("x", true).contains("learning_app_create"));
        let w = worksheet_prompt("Brüche", 6, false, true);
        assert!(w.contains("```aiscraft-worksheet") && w.contains("\"type\":\"cloze\""));
        assert!(w.contains("Lösung und Punkten") && !w.contains(worksheet::NO_POINTS_RULE));
        assert!(worksheet_prompt("x", 6, true, true).contains("worksheet_create"));
        // Without points: the model is told so, and the spec it gets says the same.
        let w = worksheet_prompt("Brüche", 6, false, false);
        assert!(w.contains(worksheet::NO_POINTS_RULE) && w.contains("\"points\":false") && !w.contains("und Punkten"), "{w}");
        let spec = |points: bool| tool_specs(points).into_iter().find(|t| t.name == "worksheet_create").unwrap().description;
        assert!(spec(false).contains(worksheet::NO_POINTS_RULE) && !spec(true).contains(worksheet::NO_POINTS_RULE));
    }
}
