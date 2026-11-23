//! The command registry. Every user-visible action is a command with an id, a label, a group
//! (the menu it sits in), an optional shortcut and a params description. The egui shell, the
//! CLI and the control channel dispatch commands by id; `app.commands` lists them for agents.

use aiscraft_core::{BackendConfig, BackendKind, Conversation, ConversationKind, Error, ErrorKind, MemoryArea, MemoryNote, Message, Project, Result, Role};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::{BackendStatus, Ctx, NoticeAction, NoticeLevel, SearchState, Selection, Settings, chat, new_id, now_ms};

pub type RunFn = fn(Ctx, Value) -> BoxFuture<'static, Result<Value>>;

pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub shortcut: Option<&'static str>,
    /// Params as `name: type` pairs, `?` for optional.
    pub params: &'static str,
    /// Shown in the command palette.
    pub palette: bool,
    pub run: RunFn,
}

macro_rules! cmd {
    ($id:literal, $label:literal, $group:literal, $shortcut:expr, $params:literal, $palette:expr, $f:expr) => {
        CommandSpec { id: $id, label: $label, group: $group, shortcut: $shortcut, params: $params, palette: $palette, run: $f }
    };
}

pub fn commands() -> &'static [CommandSpec] {
    COMMANDS
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.id == id)
}

static COMMANDS: &[CommandSpec] = &[
    cmd!("chat.new", "Neuer Chat", "Chat", Some("Ctrl+N"), "backend?: string, model?: string, project?: string, kind?: chat|image", true, |c, p| Box::pin(chat_new(c, p))),
    cmd!("chat.open", "Chat öffnen", "Chat", None, "id: string", false, |c, p| Box::pin(chat_open(c, p))),
    cmd!("chat.send", "Senden", "Chat", Some("Enter"), "text: string, files?: [path], artifact?: string (as context), force?: bool (past the monthly limit), aspect_ratio?: quadratic|landscape|portrait (image chats), links?: [url] (web pages, at most 5)", false, |c, p| Box::pin(async move {
        let text = opt_str(&p, "text").unwrap_or_default();
        if let Some(r) = opt_str(&p, "aspect_ratio") {
            set_aspect(&c, &r)?;
        }
        let files: Vec<String> = p.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let links: Vec<String> = p.get("links").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        chat::send_with(c, text, files, links, opt_str(&p, "artifact"), p.get("force").and_then(Value::as_bool).unwrap_or(false)).await
    })),
    cmd!("chat.stop", "Antwort anhalten", "Chat", Some("Escape"), "conversation?: string", true, |c, p| Box::pin(async move {
        let conv = opt_str(&p, "conversation").or_else(|| c.state().current.clone()).ok_or_else(|| Error::invalid("Kein Chat offen"))?;
        Ok(json!({"stopped": chat::stop(&c, &conv)}))
    })),
    cmd!("chat.regenerate", "Antwort neu erzeugen", "Chat", Some("Ctrl+R"), "", true, |c, _| Box::pin(chat::regenerate(c))),
    cmd!("chat.edit", "Nachricht bearbeiten und neu senden", "Chat", None, "message_id: string, text: string", false, |c, p| Box::pin(async move {
        chat::edit(c, req_str(&p, "message_id")?, req_str(&p, "text")?).await
    })),
    cmd!("chat.select_model", "Modell wählen", "Chat", None, "backend: string, model: string", false, |c, p| Box::pin(select_model(c, p))),
    cmd!("chat.set_sources", "Quellen des Chats", "Chat", None, "tools?: bool, memory?: bool, project?: bool, history?: bool, connectors?: [id] | null (alle)", false, |c, p| Box::pin(set_sources(c, p))),
    cmd!("chat.rename", "Chat umbenennen", "Chat", Some("F2"), "id: string, title: string", false, |c, p| Box::pin(async move {
        let title = req_str(&p, "title")?.trim().chars().take(200).collect::<String>();
        if title.is_empty() {
            return Err(Error::invalid("Titel darf nicht leer sein"));
        }
        edit_conversation(&c, &req_str(&p, "id")?, |conv| conv.title = title)?;
        Ok(json!({}))
    })),
    cmd!("chat.pin", "Chat anheften", "Chat", None, "id: string, pinned: bool", false, |c, p| Box::pin(async move {
        let pinned = p.get("pinned").and_then(Value::as_bool).unwrap_or(true);
        edit_conversation(&c, &req_str(&p, "id")?, |conv| conv.pinned = pinned)?;
        Ok(json!({}))
    })),
    cmd!("chat.move", "In Projekt verschieben", "Chat", None, "id: string, project?: string", false, |c, p| Box::pin(async move {
        let project = opt_str(&p, "project");
        if let Some(pid) = &project
            && !c.state().projects.iter().any(|x| &x.id == pid)
        {
            return Err(Error::not_found("Projekt nicht gefunden"));
        }
        edit_conversation(&c, &req_str(&p, "id")?, |conv| conv.project_id = project)?;
        Ok(json!({}))
    })),
    cmd!("chat.delete", "Chat löschen", "Chat", None, "id: string", false, |c, p| Box::pin(chat_delete(c, p))),
    cmd!("chat.export", "Chat exportieren", "Chat", Some("Ctrl+Shift+E"), "id?: string, format?: markdown|json, path?: string", true, |c, p| Box::pin(chat_export(c, p))),
    cmd!("chat.new_image", "Neuer Bild-Chat", "Chat", None, "backend?: string, model?: string", true, |c, p| Box::pin(async move {
        let mut p = if p.is_object() { p } else { json!({}) };
        if let Some(o) = p.as_object_mut() {
            o.insert("kind".into(), json!("image"));
        }
        chat_new(c, p).await
    })),
    cmd!("chat.set_aspect", "Bildformat wählen", "Chat", None, "ratio: quadratic|landscape|portrait", false, |c, p| Box::pin(async move {
        set_aspect(&c, &req_str(&p, "ratio")?)?;
        Ok(json!({}))
    })),
    cmd!("chat.save_image", "Bild speichern", "Chat", None, "message_id: string, index?: number, path: string", false, |c, p| Box::pin(save_image(c, p))),
    cmd!("chat.save_document", "Erzeugtes Dokument speichern", "Chat", None, "message_id: string, step: number, path: string", false, |c, p| Box::pin(save_document(c, p))),
    cmd!("search", "Alle Chats durchsuchen", "Suche", Some("Ctrl+Shift+F"), "query: string", true, |c, p| Box::pin(async move {
        let query = opt_str(&p, "query").unwrap_or_default();
        let hits = c.store(|st| st.search(&query, 50))?;
        let out = serde_json::to_value(&hits)?;
        c.update(|s| s.search = (!query.trim().is_empty()).then_some(SearchState { query, hits }));
        Ok(out)
    })),
    cmd!("search.clear", "Suche schließen", "Suche", None, "", false, |c, _| Box::pin(async move {
        c.update(|s| s.search = None);
        Ok(json!({}))
    })),
    cmd!("project.create", "Neues Projekt", "Projekte", None, "name: string, instructions?: string", true, |c, p| Box::pin(async move {
        let name = req_str(&p, "name")?.trim().to_string();
        if name.is_empty() {
            return Err(Error::invalid("Name darf nicht leer sein"));
        }
        let project = Project { id: new_id(), name, instructions: opt_str(&p, "instructions").unwrap_or_default(), created_at: now_ms() };
        c.store(|st| st.save_project(&project))?;
        let id = project.id.clone();
        c.update(|s| {
            s.projects.push(project);
            s.projects.sort_by_key(|p| p.name.to_lowercase());
        });
        Ok(json!({"id": id}))
    })),
    cmd!("project.update", "Projekt bearbeiten", "Projekte", None, "id: string, name?: string, instructions?: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        let mut project = c.state().projects.iter().find(|x| x.id == id).cloned().ok_or_else(|| Error::not_found("Projekt nicht gefunden"))?;
        if let Some(n) = opt_str(&p, "name").map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
            project.name = n;
        }
        if let Some(i) = opt_str(&p, "instructions") {
            project.instructions = i;
        }
        c.store(|st| st.save_project(&project))?;
        c.update(|s| {
            if let Some(x) = s.projects.iter_mut().find(|x| x.id == id) {
                *x = project;
            }
        });
        Ok(json!({}))
    })),
    cmd!("project.delete", "Projekt löschen", "Projekte", None, "id: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        c.store(|st| st.delete_project(&id))?;
        c.update(|s| {
            s.projects.retain(|x| x.id != id);
            for conv in s.conversations.iter_mut().filter(|x| x.project_id.as_deref() == Some(&id)) {
                conv.project_id = None;
            }
            if s.selection.project_id.as_deref() == Some(&id) {
                s.selection.project_id = None;
            }
        });
        Ok(json!({}))
    })),
    cmd!("memory.add", "Notiz merken", "Gedächtnis", None, "text: string, area?: about|preferences|people|routines|lessons", false, |c, p| Box::pin(async move {
        let text = req_str(&p, "text")?.trim().to_string();
        if text.is_empty() {
            return Err(Error::invalid("Notiz darf nicht leer sein"));
        }
        let area = opt_str(&p, "area").and_then(|a| MemoryArea::parse(&a)).unwrap_or(MemoryArea::About);
        // Typed by the owner into the memory panel: the note is its own source.
        let note = MemoryNote { id: new_id(), area, source_quote: text.clone(), text, source_message_id: None, created_at: now_ms(), archived: false };
        c.store(|st| st.save_memory(&note))?;
        let id = note.id.clone();
        c.update(|s| s.memory.push(note));
        Ok(json!({"id": id}))
    })),
    cmd!("memory.update", "Notiz ändern", "Gedächtnis", None, "id: string, text?: string, area?: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        edit_memory(&c, &id, |n| {
            if let Some(t) = opt_str(&p, "text").map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
                n.text = t;
            }
            if let Some(a) = opt_str(&p, "area").and_then(|a| MemoryArea::parse(&a)) {
                n.area = a;
            }
        })
    })),
    cmd!("memory.archive", "Notiz vergessen", "Gedächtnis", None, "id: string", false, |c, p| Box::pin(async move {
        edit_memory(&c, &req_str(&p, "id")?, |n| n.archived = true)
    })),
    cmd!("memory.restore", "Notiz wiederherstellen", "Gedächtnis", None, "id: string", false, |c, p| Box::pin(async move {
        edit_memory(&c, &req_str(&p, "id")?, |n| n.archived = false)
    })),
    cmd!("backend.add", "Backend hinzufügen", "Backends", None, "kind: ais_chat|openai|anthropic|claude_cli, name?: string, base_url?: string, secret?: string", false, |c, p| Box::pin(backend_add(c, p))),
    cmd!("backend.update", "Backend bearbeiten", "Backends", None, "id: string, name?: string, base_url?: string, default_model?: string, send_context?: bool", false, |c, p| Box::pin(backend_update(c, p))),
    cmd!("backend.remove", "Backend entfernen", "Backends", None, "id: string", false, |c, p| Box::pin(backend_remove(c, p))),
    cmd!("backend.login", "Anmelden", "Backends", None, "id: string, secret: string", false, |c, p| Box::pin(backend_login(c, p))),
    cmd!("backend.logout", "Abmelden", "Backends", None, "id: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        c.0.secrets.set(&id, None)?;
        let config = c.backend(&id)?.config().clone();
        c.install_backend(config)?;
        c.update(|s| set_status(s, &id, BackendStatus::SignedOut { message: "Abgemeldet".into() }));
        Ok(json!({}))
    })),
    cmd!("backend.browser_login", "Im Browser anmelden", "Backends", None, "id?: string (AIS.chat), silent?: bool (erst ohne Fenster versuchen), window?: bool (false: nie ein Fenster)", true, |c, p| Box::pin(browser_login(c, p))),
    cmd!("backend.check", "Verbindung prüfen", "Backends", None, "id?: string", true, |c, p| Box::pin(async move {
        let id = opt_str(&p, "id").unwrap_or_else(|| c.state().selection.backend_id.clone());
        refresh_backend(&c, &id).await?;
        let s = c.state();
        Ok(serde_json::to_value(s.backend(&id))?)
    })),
    cmd!("remote.refresh", "Verlauf vom Server laden", "Backends", None, "backend?: string", true, |c, p| Box::pin(async move {
        let id = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
        let list = c.backend(&id)?.remote_conversations().await?;
        let out = serde_json::to_value(&list)?;
        c.update(|s| {
            s.remote.insert(id, list);
        });
        Ok(out)
    })),
    cmd!("persona.refresh", "Assistenten, Figuren und Lernszenarien laden", "Backends", None, "backend?: string", true, |c, p| Box::pin(persona_refresh(c, p))),
    cmd!("persona.start", "Chat mit Assistent, Figur oder Lernszenario", "Chat", None, "kind: assistant|character|learning_scenario, id: string, backend?: string", false, |c, p| Box::pin(persona_start(c, p))),
    cmd!("remote.import", "Chat vom Server übernehmen", "Backends", None, "backend: string, id: string, title?: string", false, |c, p| Box::pin(remote_import(c, p))),
    cmd!("settings.set", "Einstellung ändern", "Einstellungen", None, "key: theme|memory|zoom|enter_sends|tools|trust_check|auto_allow_covered|monthly_token_limit|learn_skills|recall|proactive|quiet_start|quiet_end, value: any", false, |c, p| Box::pin(async move {
        let key = req_str(&p, "key")?;
        let value = p.get("value").cloned().unwrap_or(Value::Null);
        let mut settings = serde_json::to_value(&c.state().settings)?;
        let Some(obj) = settings.as_object_mut().filter(|o| o.contains_key(&key)) else {
            return Err(Error::invalid(format!("Unbekannte Einstellung {key}")));
        };
        obj.insert(key.clone(), value);
        let mut parsed: Settings = serde_json::from_value(settings).map_err(|e| Error::invalid(format!("{key}: {e}")))?;
        if !parsed.zoom.is_finite() {
            parsed.zoom = 1.0;
        }
        parsed.zoom = parsed.zoom.clamp(0.5, 3.0);
        c.store(|st| st.set_setting("settings", &serde_json::to_value(&parsed)?))?;
        c.update(|s| s.settings = parsed);
        Ok(json!({}))
    })),
    cmd!("notice.dismiss", "Hinweis schließen", "App", None, "id: number", false, |c, p| Box::pin(async move {
        let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| Error::invalid("id fehlt"))?;
        c.update(|s| s.notices.retain(|n| n.id != id));
        Ok(json!({}))
    })),
    cmd!("artifact.create", "Neues Dokument", "Artefakte", None, "kind?: document|board, title?: string, content?: string, from_message?: string", true, |c, p| Box::pin(crate::artifacts::create(c, p))),
    cmd!("artifact.open", "Artefakt öffnen", "Artefakte", None, "id: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        if !c.state().artifacts.iter().any(|a| a.id == id) {
            return Err(Error::not_found("Artefakt nicht gefunden"));
        }
        c.update(|s| s.open_artifact = Some(id));
        Ok(json!({}))
    })),
    cmd!("artifact.close", "Artefakt schließen", "Artefakte", None, "", false, |c, _| Box::pin(async move {
        c.update(|s| s.open_artifact = None);
        Ok(json!({}))
    })),
    cmd!("artifact.get", "Artefakt lesen", "Artefakte", None, "id: string", false, |c, p| Box::pin(crate::artifacts::get(c, p))),
    cmd!("artifact.save", "Artefakt speichern", "Artefakte", None, "id: string, text?: string, state_b64?: string, version?: string", false, |c, p| Box::pin(crate::artifacts::save(c, p))),
    cmd!("artifact.revise", "Mit KI überarbeiten", "Artefakte", None, "id: string, instruction: string", false, |c, p| Box::pin(crate::artifacts::revise(c, p))),
    cmd!("artifact.export", "Artefakt exportieren", "Artefakte", None, "id: string, format?: markdown|html|json, path?: string", false, |c, p| Box::pin(crate::artifacts::export(c, p))),
    cmd!("artifact.versions", "Versionen", "Artefakte", None, "id: string", false, |c, p| Box::pin(crate::artifacts::versions(c, p))),
    cmd!("artifact.restore", "Version wiederherstellen", "Artefakte", None, "id: string, version: number", false, |c, p| Box::pin(crate::artifacts::restore(c, p))),
    cmd!("artifact.rename", "Artefakt umbenennen", "Artefakte", None, "id: string, title: string", false, |c, p| Box::pin(crate::artifacts::edit_meta(c, p, |a, p| {
        let t = req_str(p, "title")?.trim().chars().take(200).collect::<String>();
        if t.is_empty() {
            return Err(Error::invalid("Titel darf nicht leer sein"));
        }
        a.title = t;
        Ok(())
    }))),
    cmd!("artifact.board_add", "Whiteboard ergänzen", "Artefakte", None, "id: string, outline: string (eingerückte Liste)", false, |c, p| Box::pin(crate::artifacts::board_add(c, p))),
    cmd!("artifact.delete", "Artefakt löschen", "Artefakte", None, "id: string", false, |c, p| Box::pin(crate::artifacts::delete(c, p))),
    cmd!("connector.add", "Konnektor hinzufügen", "Konnektoren", None, "name: string, command?: string (Befehlszeile), args?: [string], env?: {KEY: value} | \"KEY=value\\n…\", url?: string, headers?: {Name: value}", false, |c, p| Box::pin(crate::tools::add(c, p))),
    cmd!("connector.remove", "Konnektor entfernen", "Konnektoren", None, "id: string", false, |c, p| Box::pin(crate::tools::remove(c, p))),
    cmd!("connector.toggle", "Konnektor ein/aus", "Konnektoren", None, "id: string, enabled: bool", false, |c, p| Box::pin(crate::tools::toggle(c, p))),
    cmd!("connector.refresh", "Konnektor neu verbinden", "Konnektoren", None, "id: string", false, |c, p| Box::pin(crate::tools::refresh(c, p))),
    cmd!("tool.answer", "Werkzeugaufruf erlauben oder ablehnen", "Konnektoren", None, "id: string, allow: bool", false, |c, p| Box::pin(crate::tools::answer(c, p))),
    cmd!("tools.list", "Werkzeuge auflisten", "Konnektoren", None, "", false, |c, _| Box::pin(async move { Ok(serde_json::to_value(crate::tools::specs(&c))?) })),
    cmd!("schedule.create", "Zeitplan anlegen", "Zeitpläne", None, "title: string, prompt: string, cron?: \"0 7 * * 1-5\" | at?: \"morgen 7:30\", watch?: bool, backend?: string, model?: string, conversation?: string", false, |c, p| Box::pin(crate::schedules::create(c, p))),
    cmd!("schedule.update", "Zeitplan ändern", "Zeitpläne", None, "id: string, enabled?: bool, title?: string, prompt?: string, cron?: string", false, |c, p| Box::pin(crate::schedules::edit(c, p))),
    cmd!("schedule.delete", "Zeitplan löschen", "Zeitpläne", None, "id: string", false, |c, p| Box::pin(crate::schedules::delete(c, p))),
    cmd!("schedule.run", "Zeitplan jetzt ausführen", "Zeitpläne", None, "id: string", false, |c, p| Box::pin(crate::schedules::run_now(c, p))),
    cmd!("usage.report", "Nutzung", "App", None, "months?: number (default 6)", true, |c, p| Box::pin(crate::usage::report(c, p))),
    cmd!("task.start", "Aufgabe im Hintergrund starten", "Aufgaben", None, "title: string, instructions: string, conversation?: string (wohin das Ergebnis geht)", false, |c, p| Box::pin(crate::tasks::start(c, p))),
    cmd!("skill.create", "Vorgehen anlegen", "Fähigkeiten", None, "title: string, when?: string, steps: string", false, |c, p| Box::pin(crate::skills::create(c, p))),
    cmd!("skill.update", "Vorgehen ändern", "Fähigkeiten", None, "id: string, title?: string, when?: string, steps?: string", false, |c, p| Box::pin(crate::skills::update(c, p))),
    cmd!("skill.delete", "Vorgehen löschen", "Fähigkeiten", None, "id: string", false, |c, p| Box::pin(crate::skills::delete(c, p))),
    cmd!("memory.tidy", "Notizen aufräumen", "Gedächtnis", None, "", false, |c, _| Box::pin(async move { Ok(json!({"archived": crate::skills::lint_memory(&c)?})) })),
    cmd!("voice.speak", "Vorlesen", "Sprache", None, "message_id?: string, text?: string", false, |c, p| Box::pin(crate::voice::speak(c, p))),
    cmd!("voice.stop", "Vorlesen anhalten", "Sprache", None, "", false, |c, p| Box::pin(crate::voice::stop_speaking(c, p))),
    cmd!("voice.record_start", "Diktat starten", "Sprache", Some("Ctrl+D"), "", false, |c, p| Box::pin(crate::voice::record_start(c, p))),
    cmd!("voice.record_stop", "Diktat beenden", "Sprache", None, "cancel?: bool", false, |c, p| Box::pin(crate::voice::record_stop(c, p))),
    cmd!("vault.add", "Tresor-Eintrag anlegen", "Tresor", None, "name: string, username?: string, password?: string, totp?: base32, url?: string", false, |c, p| Box::pin(crate::vault::add(c, p))),
    cmd!("vault.remove", "Tresor-Eintrag löschen", "Tresor", None, "id: string", false, |c, p| Box::pin(crate::vault::remove(c, p))),
    cmd!("vault.code", "Einmalcode anzeigen", "Tresor", None, "id: string", false, |c, p| Box::pin(crate::vault::code(c, p))),
    cmd!("vault.import_bitwarden", "Aus Bitwarden übernehmen", "Tresor", None, "path: string (CSV-Export)", false, |c, p| Box::pin(crate::vault::import_bitwarden(c, p))),
    cmd!("hints.check", "Jetzt auf offene Punkte prüfen", "Hinweise", None, "", true, |c, _| Box::pin(async move { Ok(json!({"hint": crate::proactive::check(&c, true).await?})) })),
    cmd!("app.state", "Zustand lesen", "App", None, "", false, |c, _| Box::pin(async move { Ok(serde_json::to_value(&*c.state())?) })),
    cmd!("app.commands", "Befehle auflisten", "App", None, "", false, |_, _| Box::pin(async move {
        Ok(Value::Array(
            COMMANDS
                .iter()
                .map(|c| json!({"id": c.id, "label": c.label, "group": c.group, "shortcut": c.shortcut, "params": c.params}))
                .collect(),
        ))
    })),
];

// ---- params -----------------------------------------------------------------------------

pub(crate) fn opt_str(p: &Value, key: &str) -> Option<String> {
    p.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn req_str(p: &Value, key: &str) -> Result<String> {
    opt_str(p, key).ok_or_else(|| Error::invalid(format!("Parameter {key} (Text) fehlt")))
}

// ---- helpers ----------------------------------------------------------------------------

fn set_status(s: &mut crate::State, id: &str, status: BackendStatus) {
    if let Some(b) = s.backends.iter_mut().find(|b| b.config.id == id) {
        b.status = status;
    }
}

/// Check the backend, load its models and fix up the selection if it points at nothing.
pub(crate) async fn refresh_backend(c: &Ctx, id: &str) -> Result<()> {
    let backend = c.backend(id)?;
    c.update(|s| set_status(s, id, BackendStatus::Checking));
    match backend.check().await {
        Ok(account) => {
            c.update(|s| set_status(s, id, BackendStatus::Ready { account }));
        }
        Err(e) => {
            let status =
                if e.kind == ErrorKind::Auth { BackendStatus::SignedOut { message: e.message.clone() } } else { BackendStatus::Failed { message: e.message.clone() } };
            c.update(|s| set_status(s, id, status));
            return Err(e);
        }
    }
    let models = backend.models().await?;
    if let Ok(quota) = backend.quota().await {
        c.update(|s| {
            if let Some(b) = s.backends.iter_mut().find(|b| b.config.id == id) {
                b.quota = quota;
            }
        });
    }
    // No image models is fine (most backends, or switched off by the state).
    let image_models = backend.image_models().await.unwrap_or_default();
    let default = backend.config().default_model.clone();
    c.update(|s| {
        if let Some(b) = s.backends.iter_mut().find(|b| b.config.id == id) {
            b.models = models.clone();
            b.image_models = image_models.clone();
        }
        if s.selection.backend_id == id && s.selection.kind == ConversationKind::Chat && !models.iter().any(|m| m.id == s.selection.model_id) {
            s.selection.model_id = default.filter(|d| models.iter().any(|m| &m.id == d)).or_else(|| models.first().map(|m| m.id.clone())).unwrap_or_default();
        }
    });
    Ok(())
}

pub(crate) fn edit_conversation(c: &Ctx, id: &str, f: impl FnOnce(&mut Conversation)) -> Result<()> {
    let mut conv = c.state().conversation(id).cloned().ok_or_else(|| Error::not_found("Chat nicht gefunden"))?;
    f(&mut conv);
    c.store(|st| st.save_conversation(&conv))?;
    c.update(|s| {
        if let Some(x) = s.conversations.iter_mut().find(|x| x.id == conv.id) {
            *x = conv;
        }
        s.conversations.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.updated_at.cmp(&a.updated_at)));
    });
    Ok(())
}

fn edit_memory(c: &Ctx, id: &str, f: impl FnOnce(&mut MemoryNote)) -> Result<Value> {
    let mut note = c.state().memory.iter().find(|n| n.id == id).cloned().ok_or_else(|| Error::not_found("Notiz nicht gefunden"))?;
    f(&mut note);
    c.store(|st| st.save_memory(&note))?;
    c.update(|s| {
        if let Some(n) = s.memory.iter_mut().find(|n| n.id == id) {
            *n = note;
        }
    });
    Ok(json!({}))
}

fn persist_selection(c: &Ctx) {
    let sel = c.state().selection.clone();
    if let Ok(v) = serde_json::to_value(&sel) {
        let _ = c.store(|st| st.set_setting("selection", &v));
    }
}

// ---- chat -------------------------------------------------------------------------------

async fn chat_new(c: Ctx, p: Value) -> Result<Value> {
    let state = c.state();
    let backend_id = opt_str(&p, "backend").unwrap_or_else(|| state.selection.backend_id.clone());
    let view = state.backend(&backend_id).ok_or_else(|| Error::not_found(format!("Kein Backend {backend_id}")))?;
    let model_id = opt_str(&p, "model")
        .or_else(|| (state.selection.backend_id == backend_id).then(|| state.selection.model_id.clone()).filter(|m| !m.is_empty()))
        .or_else(|| view.config.default_model.clone())
        .or_else(|| view.models.first().map(|m| m.id.clone()))
        .unwrap_or_default();
    let project_id = match p.get("project") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) => None,
        _ => state.selection.project_id.clone(),
    };
    let kind = match opt_str(&p, "kind") {
        Some(k) => ConversationKind::parse(&k).ok_or_else(|| Error::invalid("kind muss chat oder image sein"))?,
        None => ConversationKind::Chat,
    };
    let model_id = if kind == ConversationKind::Image {
        let wanted = opt_str(&p, "model");
        if view.image_models.is_empty() {
            return Err(Error::unsupported(format!("{} bietet keine Bildmodelle an (oder die Bilderzeugung ist für dich ausgeschaltet).", view.config.name)));
        }
        match wanted {
            Some(m) if view.image_models.iter().any(|x| x.id == m) => m,
            Some(m) => return Err(Error::not_found(format!("Kein Bildmodell {m}"))),
            None => view.image_models.first().map(|m| m.id.clone()).unwrap_or_default(),
        }
    } else if view.image_models.iter().any(|m| m.id == model_id) {
        view.config.default_model.clone().or_else(|| view.models.first().map(|m| m.id.clone())).unwrap_or_default()
    } else {
        model_id
    };
    let aspect_ratio = state.selection.aspect_ratio.clone();
    c.update(|s| {
        s.current = None;
        s.messages.clear();
        s.selection = Selection { backend_id, model_id, project_id, sources: Default::default(), persona: None, kind, aspect_ratio };
    });
    persist_selection(&c);
    Ok(json!({}))
}

async fn chat_open(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let conv = c.state().conversation(&id).cloned().or(c.store(|st| st.conversation(&id))?).ok_or_else(|| Error::not_found("Chat nicht gefunden"))?;
    let messages = c.store(|st| st.messages(&id))?;
    let n = messages.len();
    c.update(|s| {
        s.current = Some(id.clone());
        s.messages = messages;
        s.selection = Selection {
            backend_id: conv.backend_id.clone(),
            model_id: conv.model_id.clone(),
            project_id: conv.project_id.clone(),
            sources: conv.sources.clone(),
            persona: conv.persona.clone(),
            kind: conv.kind,
            aspect_ratio: s.selection.aspect_ratio.clone(),
        };
    });
    Ok(json!({"messages": n}))
}

async fn select_model(c: Ctx, p: Value) -> Result<Value> {
    let backend = req_str(&p, "backend")?;
    let model = req_str(&p, "model")?;
    let state = c.state();
    let Some(view) = state.backend(&backend) else {
        return Err(Error::not_found(format!("Kein Backend {backend}")));
    };
    // Image models draw, text models write: each stays in its own kind of chat.
    let image_model = view.image_models.iter().any(|m| m.id == model);
    let kind = state.current.as_ref().and_then(|id| state.conversation(id)).map(|c| c.kind).unwrap_or(state.selection.kind);
    if image_model && kind == ConversationKind::Chat {
        if state.current.is_some() {
            return Err(Error::invalid("Bildmodelle gibt es in Bild-Chats. Starte einen neuen Bild-Chat."));
        }
        return chat_new(c, json!({"backend": backend, "model": model, "kind": "image"})).await;
    }
    if !image_model && kind == ConversationKind::Image {
        if state.current.is_some() {
            return Err(Error::invalid("In einem Bild-Chat zeichnet ein Bildmodell. Für Text starte einen neuen Chat."));
        }
        return chat_new(c, json!({"backend": backend, "model": model})).await;
    }
    let current = state.current.clone();
    c.update(|s| {
        s.selection.backend_id = backend.clone();
        s.selection.model_id = model.clone();
    });
    if let Some(id) = current {
        // Switching models mid-chat is allowed; the next answer comes from the new model.
        edit_conversation(&c, &id, |conv| {
            conv.backend_id = backend.clone();
            conv.model_id = model.clone();
        })?;
    }
    persist_selection(&c);
    Ok(json!({}))
}

/// Switch the sources of the open chat (or of the next new one).
async fn set_sources(c: Ctx, p: Value) -> Result<Value> {
    let mut src = c.state().selection.sources.clone();
    let flag = |k: &str, cur: bool| p.get(k).and_then(Value::as_bool).unwrap_or(cur);
    src.tools = flag("tools", src.tools);
    src.memory = flag("memory", src.memory);
    src.project = flag("project", src.project);
    src.history = flag("history", src.history);
    match p.get("connectors") {
        Some(Value::Null) => src.connectors = None,
        Some(Value::Array(a)) => src.connectors = Some(a.iter().filter_map(|v| v.as_str().map(str::to_string)).take(200).collect()),
        Some(_) => return Err(Error::invalid("connectors muss eine Liste von IDs oder null sein")),
        None => {}
    }
    let current = c.state().current.clone();
    c.update(|s| s.selection.sources = src.clone());
    if let Some(id) = current {
        let s2 = src.clone();
        edit_conversation(&c, &id, |conv| conv.sources = s2)?;
    }
    Ok(serde_json::to_value(src)?)
}

async fn chat_delete(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    chat::stop(&c, &id);
    c.store(|st| st.delete_conversation(&id))?;
    c.update(|s| {
        s.conversations.retain(|x| x.id != id);
        if s.current.as_deref() == Some(&id) {
            s.current = None;
            s.messages.clear();
        }
    });
    Ok(json!({}))
}

pub(crate) fn markdown_export(title: &str, messages: &[Message]) -> String {
    let mut out = format!("# {title}\n\n");
    for m in messages {
        let who = match m.role {
            Role::User => "Du",
            Role::Assistant => "Antwort",
            Role::System => "System",
            Role::Tool => continue,
        };
        out.push_str(&format!("## {who}\n\n{}\n\n", m.content.trim()));
        for a in &m.attachments {
            out.push_str(&format!("_Anhang: {}_\n\n", a.name));
        }
    }
    out
}

async fn chat_export(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id").or_else(|| c.state().current.clone()).ok_or_else(|| Error::invalid("Kein Chat gewählt"))?;
    let conv = c.state().conversation(&id).cloned().ok_or_else(|| Error::not_found("Chat nicht gefunden"))?;
    let messages = c.store(|st| st.messages(&id))?;
    let text = match opt_str(&p, "format").as_deref().unwrap_or("markdown") {
        "json" => serde_json::to_string_pretty(&json!({"conversation": conv, "messages": messages}))?,
        "markdown" | "md" => markdown_export(&conv.title, &messages),
        other => return Err(Error::invalid(format!("Unbekanntes Format {other}"))),
    };
    if let Some(path) = opt_str(&p, "path") {
        std::fs::write(&path, &text).map_err(|e| Error::storage(format!("{path}: {e}")))?;
        c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
        return Ok(json!({"path": path}));
    }
    Ok(json!({"text": text}))
}

async fn save_document(c: Ctx, p: Value) -> Result<Value> {
    use base64::Engine as _;
    let mid = req_str(&p, "message_id")?;
    let step = p.get("step").and_then(Value::as_u64).ok_or_else(|| Error::invalid("step fehlt"))? as usize;
    let path = req_str(&p, "path")?;
    let doc = c
        .state()
        .messages
        .iter()
        .find(|m| m.id == mid)
        .and_then(|m| m.activity.get(step))
        .and_then(|s| s.document.clone())
        .ok_or_else(|| Error::not_found("Dokument nicht gefunden"))?;
    let binary = ["docx", "pptx", "xlsx", "pdf", "odt"].contains(&doc.format.as_str());
    let bytes = if binary { base64::engine::general_purpose::STANDARD.decode(doc.content.trim()).unwrap_or_else(|_| doc.content.into_bytes()) } else { doc.content.into_bytes() };
    std::fs::write(&path, bytes).map_err(|e| Error::storage(format!("{path}: {e}")))?;
    c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
    Ok(json!({"path": path}))
}

fn set_aspect(c: &Ctx, ratio: &str) -> Result<()> {
    if !aiscraft_core::ASPECT_RATIOS.iter().any(|(k, _)| *k == ratio) {
        return Err(Error::invalid("Bildformat muss quadratic, landscape oder portrait sein"));
    }
    let r = ratio.to_string();
    c.update(|s| s.selection.aspect_ratio = r);
    persist_selection(c);
    Ok(())
}

async fn save_image(c: Ctx, p: Value) -> Result<Value> {
    let mid = req_str(&p, "message_id")?;
    let index = p.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    let path = req_str(&p, "path")?;
    let src = c
        .state()
        .messages
        .iter()
        .find(|m| m.id == mid)
        .and_then(|m| m.attachments.iter().filter(|a| a.mime.starts_with("image/")).nth(index))
        .and_then(|a| a.local_path.clone())
        .ok_or_else(|| Error::not_found("Bild nicht gefunden"))?;
    std::fs::copy(&src, &path).map_err(|e| Error::storage(format!("{path}: {e}")))?;
    c.notice(NoticeLevel::Success, format!("Gespeichert: {path}"));
    Ok(json!({"path": path}))
}

async fn remote_import(c: Ctx, p: Value) -> Result<Value> {
    let backend_id = req_str(&p, "backend")?;
    let id = req_str(&p, "id")?;
    let backend = c.backend(&backend_id)?;
    let msgs = backend.remote_messages(id.clone()).await?;
    let title = opt_str(&p, "title")
        .or_else(|| c.state().remote.get(&backend_id).and_then(|l| l.iter().find(|r| r.id == id)).map(|r| r.title.clone()))
        .or_else(|| msgs.iter().find(|m| m.1 == Role::User).map(|m| aiscraft_core::title_from(&m.2)))
        .unwrap_or_else(|| "Aus AIS.chat".into());
    let now = now_ms();
    let state = c.state();
    let model_id = if state.selection.backend_id == backend_id { state.selection.model_id.clone() } else { String::new() };
    // The local id is the server's id, so continuing the chat continues it on the server too.
    // Held with an assistant, character or learning scenario: keep sending through it.
    let persona = state.remote.get(&backend_id).and_then(|l| l.iter().find(|r| r.id == id)).and_then(|r| r.persona.clone()).map(|mut p| {
        if p.name.is_empty() {
            p.name = state.persona_info(&backend_id, &p).map(|i| i.name.clone()).unwrap_or_else(|| p.kind.label().to_string());
        }
        p
    });
    let kind = state.remote.get(&backend_id).and_then(|l| l.iter().find(|r| r.id == id)).map(|r| r.kind).unwrap_or_default();
    let conv = Conversation { id: id.clone(), title, backend_id, model_id, project_id: None, created_at: now, updated_at: now, pinned: false, sources: Default::default(), persona, kind, remote_id: None };
    c.store(|st| {
        st.save_conversation(&conv)?;
        for (i, (mid, role, content)) in msgs.iter().enumerate() {
            st.save_message(&Message {
                id: mid.clone(),
                conversation_id: id.clone(),
                role: *role,
                content: content.clone(),
                created_at: now + i as i64,
                model: None,
                attachments: vec![],
                activity: vec![],
                error: None,
                usage: None, tool_calls: vec![], tool_result: None,
            })?;
        }
        Ok(())
    })?;
    c.update(|s| {
        s.conversations.retain(|x| x.id != conv.id);
        s.conversations.insert(0, conv);
    });
    chat_open(c, json!({"id": id})).await
}

// ---- assistants, characters, learning scenarios -----------------------------------------

async fn persona_refresh(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let result = c.backend(&id)?.personas().await;
    c.update(|s| {
        if let Some(b) = s.backends.iter_mut().find(|b| b.config.id == id) {
            match &result {
                Ok(list) => {
                    b.personas = Some(list.clone());
                    b.personas_error = None;
                }
                Err(e) => b.personas_error = Some(e.message.clone()),
            }
        }
    });
    Ok(serde_json::to_value(result?)?)
}

/// Start a new chat with an assistant, character or learning scenario. The chat is created on
/// the first message, like any new chat; a character's greeting comes first.
async fn persona_start(c: Ctx, p: Value) -> Result<Value> {
    let backend_id = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let kind = aiscraft_core::PersonaKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss assistant, character oder learning_scenario sein"))?;
    let id = req_str(&p, "id")?;
    if c.state().backend(&backend_id).is_some_and(|b| b.personas.is_none()) {
        persona_refresh(c.clone(), json!({"backend": backend_id})).await?;
    }
    let state = c.state();
    let view = state.backend(&backend_id).ok_or_else(|| Error::not_found(format!("Kein Backend {backend_id}")))?;
    let info = view
        .personas
        .as_ref()
        .and_then(|l| l.iter().find(|x| x.kind == kind && x.id == id))
        .cloned()
        .ok_or_else(|| Error::not_found(format!("{} nicht gefunden. Vielleicht gelöscht oder nicht mehr geteilt.", kind.label())))?;
    // Its own model where the backend offers it, else the chosen one.
    let model_id = info
        .model_id
        .clone()
        .filter(|m| view.models.iter().any(|x| &x.id == m))
        .or_else(|| (state.selection.backend_id == backend_id).then(|| state.selection.model_id.clone()).filter(|m| !m.is_empty()))
        .or_else(|| view.config.default_model.clone())
        .or_else(|| view.models.first().map(|m| m.id.clone()))
        .unwrap_or_default();
    let project_id = state.selection.project_id.clone();
    c.update(|s| {
        s.current = None;
        s.messages.clear();
        s.selection = Selection { backend_id, model_id, project_id, sources: Default::default(), persona: Some(info.persona()), kind: ConversationKind::Chat, aspect_ratio: s.selection.aspect_ratio.clone() };
    });
    persist_selection(&c);
    Ok(serde_json::to_value(&info)?)
}

// ---- backends ---------------------------------------------------------------------------

async fn backend_add(c: Ctx, p: Value) -> Result<Value> {
    let kind = BackendKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss ais_chat, openai, anthropic oder claude_cli sein"))?;
    let base_url = opt_str(&p, "base_url").filter(|u| !u.trim().is_empty()).unwrap_or_else(|| kind.default_base_url().to_string());
    let name = opt_str(&p, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| match kind {
        BackendKind::AisChat => "AIS.chat".into(),
        BackendKind::Anthropic => "Claude (API)".into(),
        BackendKind::ClaudeCli => "Claude (Abo)".into(),
        BackendKind::OpenAi if base_url.contains("11434") => "Ollama".into(),
        BackendKind::OpenAi if base_url.contains("1234") => "LM Studio".into(),
        BackendKind::OpenAi if base_url.contains("openrouter") => "OpenRouter".into(),
        BackendKind::OpenAi if base_url.contains("ais-chat") => "AIS.chat API".into(),
        BackendKind::OpenAi => "OpenAI".into(),
    });
    let default_model = match kind {
        BackendKind::Anthropic | BackendKind::ClaudeCli => Some(aiscraft_backends::ANTHROPIC_DEFAULT_MODEL.to_string()),
        _ => None,
    };
    let config = BackendConfig { id: format!("{}-{}", kind_slug(kind), &new_id()[..8]), name, kind, base_url, default_model, send_context: kind != BackendKind::AisChat };
    // Build first so a bad address is reported before anything is saved.
    aiscraft_backends::build(config.clone(), None, c.secret_sink())?;
    if let Some(secret) = opt_str(&p, "secret").filter(|s| !s.trim().is_empty()) {
        c.0.secrets.set(&config.id, Some(&normalize_secret(kind, &secret)?))?;
    }
    c.store(|st| st.save_backend(&config))?;
    c.install_backend(config.clone())?;
    let ctx = c.clone();
    let id = config.id.clone();
    c.spawn(async move {
        let _ = refresh_backend(&ctx, &id).await;
    });
    Ok(json!({"id": config.id}))
}

fn kind_slug(k: BackendKind) -> &'static str {
    match k {
        BackendKind::AisChat => "ais",
        BackendKind::OpenAi => "openai",
        BackendKind::Anthropic => "anthropic",
        BackendKind::ClaudeCli => "claude",
    }
}

fn normalize_secret(kind: BackendKind, secret: &str) -> Result<String> {
    match kind {
        BackendKind::AisChat => aiscraft_backends::normalize_session_cookie(secret)
            .ok_or_else(|| Error::invalid(format!("Das ist kein AIS.chat-Sitzungs-Cookie. Gesucht ist der Wert von {}.", aiscraft_backends::SESSION_COOKIE))),
        _ => Ok(secret.trim().to_string()),
    }
}

async fn backend_update(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let mut config = c.backend(&id)?.config().clone();
    if let Some(n) = opt_str(&p, "name").filter(|n| !n.trim().is_empty()) {
        config.name = n;
    }
    if let Some(u) = opt_str(&p, "base_url").filter(|u| !u.trim().is_empty()) {
        config.base_url = u.trim().to_string();
    }
    if let Some(m) = p.get("default_model") {
        config.default_model = m.as_str().map(str::to_string).filter(|m| !m.is_empty());
    }
    if let Some(b) = p.get("send_context").and_then(Value::as_bool) {
        config.send_context = b;
    }
    aiscraft_backends::build(config.clone(), None, c.secret_sink())?;
    c.store(|st| st.save_backend(&config))?;
    c.install_backend(config)?;
    let _ = refresh_backend(&c, &id).await;
    Ok(json!({}))
}

async fn backend_remove(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    c.store(|st| st.delete_backend(&id))?;
    c.0.secrets.set(&id, None)?;
    c.0.backends.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&id);
    c.update(|s| {
        s.backends.retain(|b| b.config.id != id);
        if s.selection.backend_id == id {
            s.selection = Selection { backend_id: s.backends.first().map(|b| b.config.id.clone()).unwrap_or_default(), ..Default::default() };
        }
    });
    persist_selection(&c);
    Ok(json!({}))
}

/// Sign in to AIS.chat in a browser window and take the session cookie from it. With
/// `silent`, try without a window first (works while the identity provider still knows the
/// person), then fall back to the window.
pub(crate) async fn browser_login(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id").unwrap_or_else(|| "ais-chat".into());
    let config = c.backend(&id)?.config().clone();
    if config.kind != BackendKind::AisChat {
        return Err(Error::unsupported("Anmelden im Browser gibt es nur für AIS.chat"));
    }
    if !c.0.browser_allowed {
        return Err(Error::unsupported("In dieser Umgebung startet AIS.ChatCraft keinen Browser (Tests, Skripte, AISCRAFT_NO_BROWSER)"));
    }
    let opts = |headless: bool, secs: u64| aiscraft_backends::browser_login::LoginOptions {
        base_url: config.base_url.clone(),
        profile_dir: c.0.browser_profile.clone(),
        headless,
        timeout: std::time::Duration::from_secs(secs),
        browser: None,
    };
    let silent = p.get("silent").and_then(Value::as_bool).unwrap_or(false);
    // `window: false` never opens a visible window (tests, agents, the automatic re-login).
    let window = p.get("window").and_then(Value::as_bool).unwrap_or(true);
    let cookie = match (silent, window) {
        (true, true) => match aiscraft_backends::browser_login::login(opts(true, 25)).await {
            Ok(c) => c,
            Err(_) => aiscraft_backends::browser_login::login(opts(false, 600)).await?,
        },
        (true, false) => aiscraft_backends::browser_login::login(opts(true, 25)).await?,
        (false, _) => aiscraft_backends::browser_login::login(opts(false, 600)).await?,
    };
    backend_login(c, json!({"id": id, "secret": cookie})).await
}

async fn backend_login(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let config = c.backend(&id)?.config().clone();
    let secret = normalize_secret(config.kind, &req_str(&p, "secret")?)?;
    c.0.secrets.set(&id, Some(&secret))?;
    c.install_backend(config.clone())?;
    refresh_backend(&c, &id).await?;
    let account = match c.state().backend(&id).map(|b| b.status.clone()) {
        Some(BackendStatus::Ready { account }) => account,
        _ => Default::default(),
    };
    let who = account.display_name.clone().unwrap_or_else(|| config.name.clone());
    c.notice_with(
        NoticeLevel::Success,
        format!("Angemeldet bei {}: {who}", config.name),
        (config.kind == BackendKind::AisChat)
            .then(|| NoticeAction { label: "Verlauf laden".into(), command: "remote.refresh".into(), params: json!({"backend": id}) }),
    );
    Ok(serde_json::to_value(account)?)
}
