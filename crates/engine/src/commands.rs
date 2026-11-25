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

/// Every command: the core ones and those of built-in plug-ins.
pub fn commands() -> &'static [CommandSpec] {
    static ALL: std::sync::OnceLock<Vec<CommandSpec>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        #[cfg_attr(not(feature = "collab"), allow(unused_mut))]
        let mut all: Vec<CommandSpec> = COMMANDS.iter().map(CommandSpec::copy).collect();
        #[cfg(feature = "collab")]
        all.extend(crate::collab::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::office::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::office::message::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::learn::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::learn::h5p::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::learn::board::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::learn::calendar::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::ai_label::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::readability::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::attach::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::templates::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::research::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::voice_mode::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::local_files::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::ext::COMMANDS.iter().map(CommandSpec::copy));
        all.extend(crate::apps::COMMANDS.iter().map(CommandSpec::copy));
        all
    })
}

impl CommandSpec {
    fn copy(&self) -> CommandSpec {
        CommandSpec { id: self.id, label: self.label, group: self.group, shortcut: self.shortcut, params: self.params, palette: self.palette, run: self.run }
    }
}

pub fn find(id: &str) -> Option<&'static CommandSpec> {
    commands().iter().find(|c| c.id == id)
}

static COMMANDS: &[CommandSpec] = &[
    cmd!("chat.new", "Neuer Chat", "Chat", Some("Ctrl+N"), "backend?: string, model?: string, project?: string, kind?: chat|image", true, |c, p| Box::pin(
        chat_new(c, p)
    )),
    cmd!("chat.open", "Chat öffnen", "Chat", None, "id: string", false, |c, p| Box::pin(chat_open(c, p))),
    cmd!(
        "chat.quick_ask",
        "Schnelleingabe",
        "Chat",
        None,
        "text?: string (wird gleich gesendet), screenshot?: bool (Bildschirmfoto mitschicken). Holt das Fenster mit einem neuen Chat nach vorn; für ein Tastenkürzel des Desktops: aiscraft --quick",
        false,
        |c, p| Box::pin(crate::quick::run(c, p))
    ),
    cmd!(
        "message.citation",
        "Beleg zeigen",
        "Chat",
        None,
        "message_id: string, index: number (der Beleg der Antwort, ab 0) | close: true. Zeigt die belegte Stelle im Dokument",
        false,
        |c, p| Box::pin(async move { crate::citations::open(&c, &p) })
    ),
    cmd!(
        "chat.find",
        "Im Chat suchen",
        "Chat",
        Some("Ctrl+F"),
        "query?: string, step?: next|prev, close?: bool. Antwort: total, current (ab 1), message_id",
        false,
        |c, p| Box::pin(async move { crate::find::run(&c, &p) })
    ),
    cmd!(
        "chat.temporary",
        "Temporärer Chat",
        "Chat",
        None,
        "on?: bool (Standard: an). An: ein neuer Chat, der nicht gespeichert wird; aus: zurück zu einem normalen neuen Chat",
        true,
        |c, p| Box::pin(chat_temporary(c, p))
    ),
    cmd!("chat.close", "Chat schließen", "Chat", Some("Escape"), "", true, |c, _| Box::pin(chat_close(c))),
    cmd!(
        "chat.send",
        "Senden",
        "Chat",
        Some("Enter"),
        "text: string, files?: [path | {path, range}], artifact?: string (as context), force?: bool (past the monthly limit), aspect_ratio?: quadratic|landscape|portrait (image chats), links?: [url] (web pages, at most 5), privacy?: {off?: [{text, n}], on?: [{text, n}], add?: [string]} (Namen schützen: hits switched in the preview, terms added by hand)",
        false,
        |c, p| Box::pin(async move {
            let text = opt_str(&p, "text").unwrap_or_default();
            if let Some(r) = opt_str(&p, "aspect_ratio") {
                set_aspect(&c, &r)?;
            }
            // A file is a path, or {path, range} for only some pages, a sheet or some slides.
            let files: Vec<String> =
                p.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| crate::attach::file_param(&c, v)).collect()).unwrap_or_default();
            let links: Vec<String> =
                p.get("links").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let decisions = crate::privacy::Decisions::from_value(p.get("privacy"));
            chat::send_full(c, text, files, links, opt_str(&p, "artifact"), p.get("force").and_then(Value::as_bool).unwrap_or(false), decisions).await
        })
    ),
    cmd!("chat.stop", "Antwort anhalten", "Chat", Some("Escape"), "conversation?: string", true, |c, p| Box::pin(async move {
        let conv = opt_str(&p, "conversation").or_else(|| c.state().current.clone()).ok_or_else(|| Error::invalid("Kein Chat offen"))?;
        Ok(json!({"stopped": chat::stop(&c, &conv)}))
    })),
    cmd!("chat.regenerate", "Antwort neu erzeugen", "Chat", Some("Ctrl+R"), "", true, |c, _| Box::pin(chat::regenerate(c))),
    cmd!("chat.edit", "Nachricht bearbeiten und neu senden", "Chat", None, "message_id: string, text: string", false, |c, p| Box::pin(async move {
        chat::edit(c, req_str(&p, "message_id")?, req_str(&p, "text")?).await
    })),
    cmd!(
        "chat.variant",
        "Fassung zeigen",
        "Chat",
        None,
        "message_id: string, index: number (1 = erste Fassung; nach Bearbeiten oder Neu erzeugen)",
        false,
        |c, p| Box::pin(async move {
            let index = p.get("index").and_then(Value::as_u64).ok_or_else(|| Error::invalid("index fehlt (eine Zahl ab 1)"))?;
            chat::select_variant(&c, &req_str(&p, "message_id")?, usize::try_from(index).unwrap_or(usize::MAX))
        })
    ),
    cmd!("chat.select_model", "Modell wählen", "Chat", None, "backend: string, model: string", false, |c, p| Box::pin(select_model(c, p))),
    cmd!(
        "chat.set_sources",
        "Quellen des Chats",
        "Chat",
        None,
        "tools?: bool, memory?: bool, project?: bool, history?: bool, web?: bool (web search or the page reader), inline?: bool (memory and project at the top of a message, for backends without a system prompt), other_services?: bool (recall may send excerpts from chats with other services to this one), own_style?: bool (dein Schreibstil bei Schreibaufträgen), connectors?: [id] | null (alle), protect?: bool | null (Namen schützen; null = automatisch, an sobald es eine Klassenliste gibt), effort?: low|medium|high|max | null (wie gründlich das Modell nachdenkt; null = Standard des Modells)",
        false,
        |c, p| Box::pin(set_sources(c, p))
    ),
    cmd!(
        "chat.set_style",
        "Antwortstil wählen",
        "Chat",
        None,
        "style: normal|concise|explanatory|formal|custom, sample?: string (Beispieltext für custom, bis 4000 Zeichen)",
        false,
        |c, p| Box::pin(set_style(c, p))
    ),
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
    cmd!("chat.export", "Chat exportieren", "Chat", Some("Ctrl+Shift+E"), "id?: string, format?: markdown|json, path?: string", true, |c, p| Box::pin(
        chat_export(c, p)
    )),
    cmd!(
        "chat.import",
        "Chats aus claude.ai oder ChatGPT übernehmen",
        "Chat",
        None,
        "path: string (Datenexport als ZIP oder conversations.json), project?: string",
        false,
        |c, p| Box::pin(crate::chat_import::import(c, p))
    ),
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
    cmd!(
        "message.check_links",
        "Links in einer Antwort prüfen",
        "Chat",
        None,
        "message_id: string, timeout_ms?: number (pro Link, Standard 8000)",
        false,
        |c, p| Box::pin(crate::links::check_command(c, p))
    ),
    cmd!("chat.save_document", "Erzeugtes Dokument speichern", "Chat", None, "message_id: string, step: number, path: string", false, |c, p| Box::pin(
        save_document(c, p)
    )),
    cmd!("search", "Alle Chats durchsuchen", "Suche", Some("Ctrl+Shift+F"), "query: string", true, |c, p| Box::pin(async move {
        let query = opt_str(&p, "query").unwrap_or_default();
        let hits = search_chats(&c, &query)?;
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
    cmd!(
        "project.file_add",
        "Datei ins Projektwissen",
        "Projekte",
        None,
        "project: string, path: string (PDF, Office, Text …; der Text wird auf dem Gerät gelesen)",
        false,
        |c, p| Box::pin(crate::knowledge::add(c, p))
    ),
    cmd!("project.file_remove", "Datei aus dem Projektwissen entfernen", "Projekte", None, "id: string", false, |c, p| Box::pin(async move {
        crate::knowledge::remove(&c, &p)
    })),
    cmd!(
        "project.files",
        "Projektwissen anzeigen",
        "Projekte",
        None,
        "project: string. Antwort: files, usage {tokens, window, percent, whole}",
        false,
        |c, p| Box::pin(async move { crate::knowledge::list(&c, &p) })
    ),
    cmd!("project.delete", "Projekt löschen", "Projekte", None, "id: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        c.store(|st| st.delete_project(&id))?;
        c.update(|s| {
            s.projects.retain(|x| x.id != id);
            s.project_files.retain(|f| f.project_id != id);
            for conv in s.conversations.iter_mut().filter(|x| x.project_id.as_deref() == Some(&id)) {
                conv.project_id = None;
            }
            if s.selection.project_id.as_deref() == Some(&id) {
                s.selection.project_id = None;
            }
        });
        Ok(json!({}))
    })),
    cmd!("memory.add", "Notiz merken", "Gedächtnis", None, "text: string, area?: about|preferences|people|routines|lessons|style", false, |c, p| Box::pin(
        async move {
            let text = req_str(&p, "text")?.trim().to_string();
            if text.is_empty() {
                return Err(Error::invalid("Notiz darf nicht leer sein"));
            }
            let area = opt_str(&p, "area").and_then(|a| MemoryArea::parse(&a)).unwrap_or(MemoryArea::About);
            // Typed by the owner into the memory panel: the note is its own source.
            let note = crate::memory::add_from_owner(&c, area, text.clone(), None, text)?;
            Ok(json!({"id": note.id}))
        }
    )),
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
        edit_memory(&c, &req_str(&p, "id")?, |n| {
            n.archived = false;
            n.replaced_by = None;
        })
    })),
    cmd!(
        "style.learn",
        "Schreibstil aus eigenen Texten lernen",
        "Gedächtnis",
        None,
        "Liest deine eigenen Nachrichten in Chats mit dem gewählten Dienst (ohne Namen und Zahlen) und legt eine Stilnotiz an, die bei Schreibaufträgen mitgeht",
        true,
        |c, p| Box::pin(crate::style::learn(c, p))
    ),
    cmd!(
        "memory.replace",
        "Notiz durch neuere ersetzen",
        "Gedächtnis",
        None,
        "id: string (die ältere Notiz), by: string (die neuere). Die ältere wird vergessen und zeigt „ersetzt durch“",
        false,
        |c, p| Box::pin(async move { crate::memory::replace(&c, &req_str(&p, "id")?, &req_str(&p, "by")?) })
    ),
    cmd!(
        "backend.add",
        "Backend hinzufügen",
        "Backends",
        None,
        "kind: ais_chat|openai|anthropic|claude_cli|vertex, name?: string, base_url?: string (Vertex: Region, Standard global), secret?: string, key_file?: string (Vertex: Pfad zur JSON-Schlüsseldatei des Dienstkontos)",
        false,
        |c, p| Box::pin(backend_add(c, p))
    ),
    cmd!(
        "backend.update",
        "Backend bearbeiten",
        "Backends",
        None,
        "id: string, name?: string, base_url?: string, default_model?: string, send_context?: bool",
        false,
        |c, p| Box::pin(backend_update(c, p))
    ),
    cmd!("backend.remove", "Backend entfernen", "Backends", None, "id: string", false, |c, p| Box::pin(backend_remove(c, p))),
    cmd!("backend.login", "Anmelden", "Backends", None, "id: string, secret?: string, key_file?: string (Vertex: JSON-Schlüsseldatei)", false, |c, p| {
        Box::pin(backend_login(c, p))
    }),
    cmd!("backend.models_check", "Modelle neu prüfen", "Backends", None, "id?: string", true, |c, p| Box::pin(models_check(c, p))),
    cmd!("backend.logout", "Abmelden", "Backends", None, "id: string", false, |c, p| Box::pin(async move {
        let id = req_str(&p, "id")?;
        c.0.secrets.set(&id, None)?;
        let config = c.backend(&id)?.config().clone();
        c.install_backend(config)?;
        c.update(|s| set_status(s, &id, BackendStatus::SignedOut { message: "Abgemeldet".into() }));
        Ok(json!({}))
    })),
    cmd!(
        "backend.browser_login",
        "Im Browser anmelden",
        "Backends",
        None,
        "id?: string (AIS.chat), silent?: bool (erst ohne Fenster versuchen), window?: bool (false: nie ein Fenster)",
        true,
        |c, p| Box::pin(browser_login(c, p))
    ),
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
    cmd!("persona.refresh", "Assistenten, Dialogpartner und Lernszenarien laden", "Backends", None, "backend?: string", true, |c, p| Box::pin(
        persona_refresh(c, p)
    )),
    cmd!(
        "persona.start",
        "Chat mit Assistent, Dialogpartner oder Lernszenario",
        "Chat",
        None,
        "kind: assistant|character|learning_scenario, id: string, backend?: string, text?: string (gleich senden)",
        false,
        |c, p| Box::pin(persona_start(c, p))
    ),
    cmd!(
        "persona.picture",
        "Bild eines Assistenten, Dialogpartners oder Lernszenarios laden",
        "Backends",
        None,
        "kind: assistant|character|learning_scenario, id: string, backend?: string, fresh?: bool (nicht aus dem Zwischenspeicher)",
        false,
        |c, p| Box::pin(crate::pictures::picture(c, p))
    ),
    cmd!(
        "persona.favorite",
        "Assistent, Dialogpartner oder Lernszenario als Favorit markieren",
        "Backends",
        None,
        "kind: assistant|character|learning_scenario, id: string, favorite?: bool (Standard: umschalten), backend?: string",
        false,
        |c, p| Box::pin(persona_favorite(c, p))
    ),
    cmd!("remote.import", "Chat vom Server übernehmen", "Backends", None, "backend: string, id: string, title?: string", false, |c, p| Box::pin(
        remote_import(c, p)
    )),
    cmd!(
        "settings.set",
        "Einstellung ändern",
        "Einstellungen",
        None,
        "key: theme|transparency|look_chosen|memory|zoom|enter_sends|tools|trust_check|auto_allow_covered|monthly_token_limit|learn_skills|recall|check_links|proactive|quiet_start|quiet_end|update_check|worksheet_points|ai_label|ai_label_prompt, value: any",
        false,
        |c, p| Box::pin(async move {
            let key = req_str(&p, "key")?;
            if key == "trust_rules" {
                return Err(Error::invalid("Regeln ändern trust.rule_add und trust.rule_remove"));
            }
            if key == "fallback_models" || key == "fallback_other_services" {
                return Err(Error::invalid("Ersatzmodelle ändert fallback.set"));
            }
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            let mut settings = serde_json::to_value(&c.state().settings)?;
            let Some(obj) = settings.as_object_mut().filter(|o| o.contains_key(&key)) else {
                return Err(Error::invalid(format!("Unbekannte Einstellung {key}")));
            };
            obj.insert(key.clone(), value);
            let mut parsed: Settings = serde_json::from_value(settings).map_err(|e| Error::invalid(format!("{key}: {e}")))?;
            // "Modern" was called "Mutig" in early builds: the old values still work.
            if let Some(rest) = parsed.theme.strip_prefix("mutig") {
                parsed.theme = format!("modern{rest}");
            }
            if key == "theme" && !crate::state::THEMES.contains(&parsed.theme.as_str()) {
                return Err(Error::invalid(format!("Unbekanntes Erscheinungsbild {}; möglich: {}", parsed.theme, crate::state::THEMES.join(", "))));
            }
            if key == "ai_label" && !matches!(parsed.ai_label.as_str(), "ais-chat" | "all" | "off") {
                return Err(Error::invalid("ai_label: ais-chat, all oder off"));
            }
            if key == "transparency" && !matches!(parsed.transparency.as_str(), "gradient" | "off") {
                return Err(Error::invalid("transparency: gradient oder off"));
            }
            if !parsed.zoom.is_finite() {
                parsed.zoom = 1.0;
            }
            parsed.zoom = parsed.zoom.clamp(0.5, 3.0);
            c.store(|st| st.set_setting("settings", &serde_json::to_value(&parsed)?))?;
            c.update(|s| s.settings = parsed);
            Ok(json!({}))
        })
    ),
    cmd!(
        "fallback.set",
        "Ersatzmodelle festlegen",
        "Einstellungen",
        None,
        "models?: [{backend, model}] (in dieser Reihenfolge, höchstens 5), other_services?: bool (Ersatzmodelle bei anderen Diensten dürfen den Chat bekommen)",
        false,
        |c, p| Box::pin(crate::fallback::set(c, p))
    ),
    cmd!("notice.dismiss", "Hinweis schließen", "App", None, "id: number", false, |c, p| Box::pin(async move {
        let id = p.get("id").and_then(Value::as_u64).ok_or_else(|| Error::invalid("id fehlt"))?;
        c.update(|s| s.notices.retain(|n| n.id != id));
        Ok(json!({}))
    })),
    cmd!("notice.read", "Benachrichtigungen als gelesen markieren", "App", None, "id?: string (ohne: alle)", true, |c, p| Box::pin(crate::inbox::read(c, p))),
    cmd!("notice.clear", "Benachrichtigungen leeren", "App", None, "id?: string (ohne: alle)", false, |c, p| Box::pin(crate::inbox::clear(c, p))),
    cmd!(
        "artifact.create",
        "Neues Dokument",
        "Artefakte",
        None,
        "kind?: document|board, title?: string, content?: string, from_message?: string",
        true,
        |c, p| Box::pin(crate::artifacts::create(c, p))
    ),
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
    cmd!("artifact.save", "Artefakt speichern", "Artefakte", None, "id: string, text?: string, state_b64?: string, version?: string", false, |c, p| Box::pin(
        crate::artifacts::save(c, p)
    )),
    cmd!("artifact.revise", "Mit KI überarbeiten", "Artefakte", None, "id: string, instruction: string", false, |c, p| Box::pin(crate::artifacts::revise(
        c, p
    ))),
    cmd!("artifact.export", "Artefakt exportieren", "Artefakte", None, "id: string, format?: markdown|html|docx|pdf|json, path?: string", false, |c, p| {
        Box::pin(crate::artifacts::export(c, p))
    }),
    cmd!("artifact.export_paths", "Dateien zum Bereitstellen", "Artefakte", None, "id: string (je Eintrag: label, hint, format, variant, ext, suffix für artifact.export)", false, |c, p| {
        Box::pin(crate::artifacts::export_paths_cmd(c, p))
    }),
    cmd!("artifact.versions", "Versionen","Artefakte", None, "id: string", false, |c, p| Box::pin(crate::artifacts::versions(c, p))),
    cmd!("artifact.restore", "Version wiederherstellen", "Artefakte", None, "id: string, version: number", false, |c, p| Box::pin(crate::artifacts::restore(
        c, p
    ))),
    cmd!("artifact.rename", "Artefakt umbenennen", "Artefakte", None, "id: string, title: string", false, |c, p| Box::pin(crate::artifacts::edit_meta(
        c,
        p,
        |a, p| {
            let t = req_str(p, "title")?.trim().chars().take(200).collect::<String>();
            if t.is_empty() {
                return Err(Error::invalid("Titel darf nicht leer sein"));
            }
            a.title = t;
            Ok(())
        }
    ))),
    cmd!("artifact.board_add", "Whiteboard ergänzen", "Artefakte", None, "id: string, outline: string (eingerückte Liste)", false, |c, p| Box::pin(
        crate::artifacts::board_add(c, p)
    )),
    cmd!("artifact.delete", "Artefakt löschen", "Artefakte", None, "id: string", false, |c, p| Box::pin(crate::artifacts::delete(c, p))),
    cmd!(
        "connector.add",
        "Konnektor hinzufügen",
        "Konnektoren",
        None,
        "name: string, command?: string (Befehlszeile), args?: [string], env?: {KEY: value} | \"KEY=value\\n…\", url?: string, headers?: {Name: value}",
        false,
        |c, p| Box::pin(crate::tools::add(c, p))
    ),
    cmd!("connector.remove", "Konnektor entfernen", "Konnektoren", None, "id: string", false, |c, p| Box::pin(crate::tools::remove(c, p))),
    cmd!("connector.toggle", "Konnektor ein/aus", "Konnektoren", None, "id: string, enabled: bool", false, |c, p| Box::pin(crate::tools::toggle(c, p))),
    cmd!("connector.refresh", "Konnektor neu verbinden", "Konnektoren", None, "id: string", false, |c, p| Box::pin(crate::tools::refresh(c, p))),
    cmd!(
        "connector.insert",
        "Konnektor-Ergebnis einfügen",
        "Konnektoren",
        None,
        "tool: string (Name aus tools.list), arguments?: {…} (von der Person geschrieben). Nur lesende Werkzeuge; das Ergebnis kommt zur Prüfung ins Eingabefeld und geht erst beim Senden mit",
        false,
        |c, p| Box::pin(crate::tools::insert(c, p))
    ),
    cmd!("tool.answer", "Werkzeugaufruf erlauben oder ablehnen", "Konnektoren", None, "id: string, allow: bool", false, |c, p| Box::pin(crate::tools::answer(
        c, p
    ))),
    cmd!(
        "trust.rule_add",
        "Eigene Regel für die Vertrauensprüfung",
        "Konnektoren",
        None,
        "kind: allow|forbid, text: string (z. B. „nie Mails an Eltern senden“), connector?: string (Konnektor-ID, leer: alle)",
        false,
        |c, p| Box::pin(crate::tools::rule_add(c, p))
    ),
    cmd!("trust.rule_remove", "Eigene Regel löschen", "Konnektoren", None, "id: string", false, |c, p| Box::pin(crate::tools::rule_remove(c, p))),
    cmd!("tools.list", "Werkzeuge auflisten", "Konnektoren", None, "", false, |c, _| Box::pin(
        async move { Ok(serde_json::to_value(crate::tools::specs(&c))?) }
    )),
    cmd!(
        "schedule.create",
        "Zeitplan anlegen",
        "Zeitpläne",
        None,
        "title: string, prompt: string, cron?: \"0 7 * * 1-5\" | at?: \"morgen 7:30\", watch?: bool, backend?: string, model?: string, conversation?: string",
        false,
        |c, p| Box::pin(crate::schedules::create(c, p))
    ),
    cmd!(
        "schedule.update",
        "Zeitplan ändern",
        "Zeitpläne",
        None,
        "id: string, enabled?: bool, title?: string, prompt?: string, cron?: string, backend?: string, model?: string (das Modell, mit dem der Zeitplan läuft)",
        false,
        |c, p| Box::pin(crate::schedules::edit(c, p))
    ),
    cmd!(
        "schedule.templates",
        "Vorlagen für Zeitpläne",
        "Zeitpläne",
        None,
        "Antwort: id, title, prompt, cron, rhythm, watch, needs, missing (was fehlt und was zu tun ist), needs_url",
        false,
        |c, p| Box::pin(crate::schedule_templates::list(c, p))
    ),
    cmd!(
        "schedule.from_template",
        "Zeitplan aus Vorlage anlegen",
        "Zeitpläne",
        None,
        "template: morgen|woche|mails|vertretung, url?: string (Seite, für vertretung), title?: string, prompt?: string, cron?: string. Fehlt ein Konnektor oder ein Modell, das Webseiten liest, sagt der Befehl, was zu tun ist",
        false,
        |c, p| Box::pin(crate::schedule_templates::create(c, p))
    ),
    cmd!(
        "schedule.ideas",
        "Zeitplan-Ideen suchen",
        "Zeitpläne",
        None,
        "Sucht in deinen eigenen Nachrichten nach Bitten, die du immer wieder stellst (an mindestens drei Tagen, gleicher Wochentag oder gleiche Uhrzeit). Antwort: id, title, prompt, cron, rhythm, backend_id, model_id, days",
        true,
        |c, p| Box::pin(crate::schedule_ideas::list(c, p))
    ),
    cmd!(
        "schedule.idea_accept",
        "Zeitplan-Idee anlegen",
        "Zeitpläne",
        None,
        "id: string, title?: string, prompt?: string, cron?: string, backend?: string, model?: string (Änderungen vor dem Anlegen)",
        false,
        |c, p| Box::pin(crate::schedule_ideas::accept(c, p))
    ),
    cmd!("schedule.idea_dismiss", "Zeitplan-Idee ablehnen", "Zeitpläne", None, "id: string. Die Idee kommt nicht wieder", false, |c, p| Box::pin(
        crate::schedule_ideas::dismiss(c, p)
    )),
    cmd!("schedule.delete", "Zeitplan löschen", "Zeitpläne", None, "id: string", false, |c, p| Box::pin(crate::schedules::delete(c, p))),
    cmd!("schedule.run", "Zeitplan jetzt ausführen", "Zeitpläne", None, "id: string", false, |c, p| Box::pin(crate::schedules::run_now(c, p))),
    cmd!("log.list", "Aktionsprotokoll", "App", None, "limit?: number (Standard 100), conversation?: string", false, |c, p| Box::pin(crate::action_log::list(
        c, p
    ))),
    cmd!("log.export", "Aktionsprotokoll als CSV exportieren", "App", None, "path: string", false, |c, p| Box::pin(crate::action_log::export(c, p))),
    cmd!("usage.report", "Nutzung", "App", None, "months?: number (default 6)", true, |c, p| Box::pin(crate::usage::report(c, p))),
    cmd!(
        "task.start",
        "Aufgabe im Hintergrund starten",
        "Aufgaben",
        None,
        "title: string, instructions: string, conversation?: string (wohin das Ergebnis geht)",
        false,
        |c, p| Box::pin(crate::tasks::start(c, p))
    ),
    cmd!("skill.create", "Vorgehen anlegen", "Fähigkeiten", None, "title: string, when?: string, steps: string", false, |c, p| Box::pin(
        crate::skills::create(c, p)
    )),
    cmd!("skill.update", "Vorgehen ändern", "Fähigkeiten", None, "id: string, title?: string, when?: string, steps?: string", false, |c, p| Box::pin(
        crate::skills::update(c, p)
    )),
    cmd!("skill.delete", "Vorgehen löschen", "Fähigkeiten", None, "id: string", false, |c, p| Box::pin(crate::skills::delete(c, p))),
    cmd!("memory.tidy", "Notizen aufräumen", "Gedächtnis", None, "", false, |c, _| Box::pin(async move {
        Ok(json!({"archived": crate::skills::lint_memory(&c)?}))
    })),
    cmd!("voice.speak", "Vorlesen", "Sprache", None, "message_id?: string, text?: string", false, |c, p| Box::pin(crate::voice::speak(c, p))),
    cmd!("voice.stop", "Vorlesen anhalten", "Sprache", None, "", false, |c, p| Box::pin(crate::voice::stop_speaking(c, p))),
    cmd!("voice.record_start", "Diktat starten", "Sprache", Some("Ctrl+D"), "", false, |c, p| Box::pin(crate::voice::record_start(c, p))),
    cmd!("voice.record_stop", "Diktat beenden", "Sprache", None, "cancel?: bool", false, |c, p| Box::pin(crate::voice::record_stop(c, p))),
    cmd!(
        "vault.add",
        "Tresor-Eintrag anlegen",
        "Tresor",
        None,
        "name: string, username?: string, password?: string, totp?: base32, url?: string",
        false,
        |c, p| Box::pin(crate::vault::add(c, p))
    ),
    cmd!(
        "privacy.list_import",
        "Klassenliste einlesen",
        "Namen schützen",
        None,
        "path: string (csv, xlsx, xls, ods, vcf), name?: string, project?: string (nur für diesen Projekt; ohne: alle Chats)",
        true,
        |c, p| Box::pin(crate::privacy::list_import(c, p))
    ),
    cmd!(
        "privacy.list_add",
        "Klassenliste aus Text",
        "Namen schützen",
        None,
        "text: string (eine Person pro Zeile: „Lea Müller“ oder „Müller, Lea“), name?: string, project?: string",
        false,
        |c, p| Box::pin(crate::privacy::list_add(c, p))
    ),
    cmd!("privacy.list_remove", "Klassenliste löschen", "Namen schützen", None, "id: string", false, |c, p| Box::pin(crate::privacy::list_remove(c, p))),
    cmd!("privacy.lists", "Klassenlisten anzeigen", "Namen schützen", None, "", false, |c, p| Box::pin(crate::privacy::lists(c, p))),
    cmd!(
        "privacy.preview",
        "Vorschau: was rausgeht",
        "Namen schützen",
        None,
        "text: string, files?: [path], off?: [{text, n}], on?: [{text, n}], add?: [string] (or the same inside privacy: {...})",
        false,
        |c, p| Box::pin(crate::privacy::preview(c, p))
    ),
    cmd!("privacy.mapping", "Platzhalter des Chats anzeigen", "Namen schützen", None, "conversation?: string", true, |c, p| Box::pin(
        crate::privacy::show_mapping(c, p)
    )),
    cmd!("privacy.reveal", "Platzhalter zurückübersetzen", "Namen schützen", None, "text: string, conversation?: string", false, |c, p| Box::pin(
        crate::privacy::reveal_cmd(c, p)
    )),
    cmd!("vault.remove", "Tresor-Eintrag löschen", "Tresor", None, "id: string", false, |c, p| Box::pin(crate::vault::remove(c, p))),
    cmd!("vault.code", "Einmalcode anzeigen", "Tresor", None, "id: string", false, |c, p| Box::pin(crate::vault::code(c, p))),
    cmd!("vault.import_bitwarden", "Aus Bitwarden übernehmen", "Tresor", None, "path: string (CSV-Export)", false, |c, p| Box::pin(
        crate::vault::import_bitwarden(c, p)
    )),
    cmd!("hints.check", "Jetzt auf offene Punkte prüfen", "Hinweise", None, "", true, |c, _| Box::pin(async move {
        Ok(json!({"hint": crate::proactive::check(&c, true).await?}))
    })),
    cmd!(
        "app.check_update",
        "Nach Updates suchen",
        "App",
        None,
        "background?: bool (nur wenn eingeschaltet und höchstens einmal am Tag, Fehler still)",
        true,
        |c, p| Box::pin(async move { crate::update::check(&c, p.get("background").and_then(Value::as_bool).unwrap_or(false)).await })
    ),
    cmd!("app.update_open", "Neue Version im Browser öffnen", "App", None, "what?: notes|download", false, |c, p| Box::pin(async move {
        crate::update::open(&c, &p)
    })),
    cmd!("app.update_skip", "Diese Version überspringen", "App", None, "version?: string (sonst die angebotene)", false, |c, p| Box::pin(async move {
        crate::update::skip(&c, &p)
    })),
    cmd!("app.update_dismiss", "Update-Hinweis ausblenden", "App", None, "", false, |c, _| Box::pin(async move { crate::update::dismiss(&c) })),
    cmd!("app.state", "Zustand lesen", "App", None, "", false, |c, _| Box::pin(async move { Ok(serde_json::to_value(&*c.state())?) })),
    cmd!("app.commands", "Befehle auflisten", "App", None, "", false, |_, _| Box::pin(async move {
        Ok(Value::Array(
            commands().iter().map(|c| json!({"id": c.id, "label": c.label, "group": c.group, "shortcut": c.shortcut, "params": c.params})).collect(),
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
            let status = if e.kind == ErrorKind::Auth {
                BackendStatus::SignedOut { message: e.message.clone() }
            } else {
                BackendStatus::Failed { message: e.message.clone() }
            };
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
    let checks = backend.model_checks();
    c.update(|s| {
        if let Some(b) = s.backends.iter_mut().find(|b| b.config.id == id) {
            b.models = models.clone();
            b.image_models = image_models.clone();
            b.model_checks = checks.clone();
        }
        if s.selection.backend_id == id && s.selection.kind == ConversationKind::Chat && !models.iter().any(|m| m.id == s.selection.model_id) {
            s.selection.model_id = default.filter(|d| models.iter().any(|m| &m.id == d)).or_else(|| models.first().map(|m| m.id.clone())).unwrap_or_default();
        }
    });
    if !checks.is_empty() {
        c.save_backend_cache(id);
    }
    Ok(())
}

/// Try every model of a backend again (Vertex AI): usable ones go into the menu, the others are
/// listed in the settings with the reason.
async fn models_check(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let backend = c.backend(&id)?;
    backend.discover_models().await?;
    refresh_backend(&c, &id).await?;
    let checks = backend.model_checks();
    let usable = checks.iter().filter(|m| m.usable).count();
    c.notice(NoticeLevel::Info, format!("{}: {usable} von {} Modellen nutzbar", backend.config().name, checks.len()));
    Ok(json!({"usable": usable, "models": checks}))
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
        s.find = None;
        s.citation = None;
        s.messages.clear();
        s.variants.clear();
        s.selection = Selection { backend_id, model_id, project_id, sources: Default::default(), persona: None, kind, aspect_ratio, temporary: false };
    });
    crate::temporary::discard_closed(&c);
    persist_selection(&c);
    Ok(json!({}))
}

/// Start a temporary chat (#34), or go back to a normal new chat.
async fn chat_temporary(c: Ctx, p: Value) -> Result<Value> {
    let on = match p.get("on") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(Error::invalid("on muss true oder false sein")),
    };
    if on && !c.state().temporary_possible(&c.state().selection.backend_id) {
        return Err(crate::temporary::not_with_ais_chat());
    }
    chat_new(c.clone(), json!({})).await?;
    if on {
        c.update(|s| {
            s.selection.temporary = true;
            // Nothing from the notes goes along either; it can be switched on under „Quellen“.
            s.selection.sources.memory = false;
        });
    }
    Ok(json!({"temporary": on}))
}

/// Close the open chat: back to the start page. A running answer keeps going in the background.
async fn chat_close(c: Ctx) -> Result<Value> {
    let closed = c.state().current.clone();
    // Like a new chat with the same backend and model, without persona or image mode.
    if chat_new(c.clone(), json!({})).await.is_err() {
        c.update(|s| {
            s.current = None;
            s.messages.clear();
            s.variants.clear();
            s.selection.persona = None;
            s.selection.kind = ConversationKind::Chat;
        });
        crate::temporary::discard_closed(&c);
    }
    Ok(json!({"closed": closed}))
}

async fn chat_open(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let conv = c.state().conversation(&id).cloned().or(c.store(|st| st.conversation(&id))?).ok_or_else(|| Error::not_found("Chat nicht gefunden"))?;
    let messages = c.store(|st| st.messages(&id))?;
    let variants = c.store(|st| st.variants(&id))?;
    let n = messages.len();
    c.update(|s| {
        s.current = Some(id.clone());
        s.find = None;
        s.citation = None;
        s.messages = messages;
        s.variants = variants;
        s.selection = Selection {
            backend_id: conv.backend_id.clone(),
            model_id: conv.model_id.clone(),
            project_id: conv.project_id.clone(),
            sources: conv.sources.clone(),
            persona: conv.persona.clone(),
            kind: conv.kind,
            aspect_ratio: s.selection.aspect_ratio.clone(),
            temporary: conv.temporary,
        };
    });
    crate::temporary::discard_closed(&c);
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
    // A temporary chat stays away from AIS.chat, which would keep it in its history.
    let temporary = match &current {
        Some(id) => state.conversation(id).is_some_and(|x| x.temporary),
        None => state.selection.temporary,
    };
    if temporary && !state.temporary_possible(&backend) {
        return Err(crate::temporary::not_with_ais_chat());
    }
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
    src.web = flag("web", src.web);
    src.inline = flag("inline", src.inline);
    src.other_services = flag("other_services", src.other_services);
    src.own_style = flag("own_style", src.own_style);
    src.knowledge = flag("knowledge", src.knowledge);
    match p.get("protect") {
        Some(Value::Null) => src.protect = None,
        Some(Value::Bool(b)) => src.protect = Some(*b),
        Some(_) => return Err(Error::invalid("protect muss true, false oder null sein")),
        None => {}
    }
    match p.get("effort") {
        Some(Value::Null) => src.effort = None,
        Some(Value::String(e)) if aiscraft_core::EFFORTS.iter().any(|(id, _)| id == e) => src.effort = Some(e.clone()),
        Some(Value::String(e)) if e == "default" || e.is_empty() => src.effort = None,
        Some(_) => return Err(Error::invalid("effort muss low, medium, high, max oder null sein")),
        None => {}
    }
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

/// The answer style of the open chat (or of the next new one), #31.
async fn set_style(c: Ctx, p: Value) -> Result<Value> {
    let style = req_str(&p, "style")?;
    let mut src = c.state().selection.sources.clone();
    if let Some(sample) = p.get("sample") {
        let sample = sample.as_str().ok_or_else(|| Error::invalid("sample muss ein Text sein"))?.trim();
        src.style_sample = (!sample.is_empty()).then(|| sample.chars().take(aiscraft_core::MAX_STYLE_SAMPLE).collect());
    }
    match style.as_str() {
        "normal" | "" => src.style = None,
        "custom" if src.style_sample.is_none() => return Err(Error::invalid("Für einen eigenen Stil fehlt ein Beispieltext")),
        s if aiscraft_core::STYLES.iter().any(|(id, _)| *id == s) => src.style = Some(s.to_string()),
        other => return Err(Error::invalid(format!("Unbekannter Stil {other} (normal, concise, explanatory, formal, custom)"))),
    }
    let current = c.state().current.clone();
    c.update(|s| s.selection.sources = src.clone());
    if let Some(id) = current {
        let s2 = src.clone();
        edit_conversation(&c, &id, |conv| conv.sources = s2)?;
    }
    Ok(json!({"style": src.style, "sample": src.style_sample}))
}

async fn chat_delete(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    chat::stop(&c, &id);
    if c.state().conversation(&id).is_some_and(|x| x.temporary) {
        crate::temporary::discard(&c, &id);
    }
    c.store(|st| st.delete_conversation(&id))?;
    crate::privacy::forget(&c, &id);
    c.update(|s| {
        s.conversations.retain(|x| x.id != id);
        if s.current.as_deref() == Some(&id) {
            s.current = None;
            s.messages.clear();
            s.variants.clear();
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
        // A format request: the teacher's words and one line instead of the whole spec.
        match crate::learn::formats::split_request(&m.content).filter(|_| m.role == Role::User) {
            Some(r) => out.push_str(&format!("## {who}\n\n{}\n\n_Mit Formatvorgabe: {}_\n\n", r.topic, r.format.label)),
            None => out.push_str(&format!("## {who}\n\n{}\n\n", m.content.trim())),
        }
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
        // Placeholders of „Namen schützen“ back to the names; the JSON keeps what was sent.
        "markdown" | "md" => {
            let mut md = markdown_export(&conv.title, &messages);
            // „Erstellt mit KI“ (#57) under the chat.
            if let Some(note) = crate::ai_label::note_for_chat(&c, &conv, &messages) {
                md.push_str(&note.markdown());
            }
            crate::privacy::reveal(&c, &id, &md)
        }
        other => return Err(Error::invalid(format!("Unbekanntes Format {other}"))),
    };
    if let Some(path) = opt_str(&p, "path") {
        crate::disk::write(std::path::Path::new(&path), text.as_bytes())?;
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
    let bytes = if binary {
        base64::engine::general_purpose::STANDARD.decode(doc.content.trim()).unwrap_or_else(|_| doc.content.into_bytes())
    } else {
        // Text documents: placeholders back to the names (#55).
        crate::privacy::reveal(&c, c.state().current.as_deref().unwrap_or(""), &doc.content).into_bytes()
    };
    crate::disk::write(std::path::Path::new(&path), &bytes)?;
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
    let size = std::fs::metadata(&src).map_err(|e| Error::storage(format!("{src}: {e}")))?.len();
    if size > crate::files::MAX_FILE_BYTES {
        return Err(Error::invalid("Das Bild ist größer als 25 MB"));
    }
    let bytes = std::fs::read(&src).map_err(|e| Error::storage(format!("{src}: {e}")))?;
    // „Erstellt mit KI“ (#57) as the picture's comment.
    let bytes = match crate::ai_label::note_for_message(&c, Some(&mid)) {
        Some(note) => crate::ai_label::picture_with_note(&bytes, &note.plain()),
        None => bytes,
    };
    crate::disk::write(std::path::Path::new(&path), &bytes)?;
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
    let conv = Conversation {
        id: id.clone(),
        title,
        backend_id,
        model_id,
        project_id: None,
        created_at: now,
        updated_at: now,
        pinned: false,
        sources: Default::default(),
        persona,
        kind,
        remote_id: None,
        temporary: false,
    };
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
                usage: None,
                tool_calls: vec![],
                tool_result: None,
                thinking: String::new(),
                citations: Vec::new(),
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

/// Chats whose title contains every word of the query (as people see the title, names shown
/// again where „Namen schützen“ stored placeholders), newest first; then hits in the messages, one
/// list. A chat found by its title is not listed again for its messages.
fn search_chats(c: &Ctx, query: &str) -> Result<Vec<aiscraft_store::SearchHit>> {
    const LIMIT: usize = 50;
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).filter(|w| !w.is_empty()).take(12).collect();
    let mut hits = Vec::new();
    if !words.is_empty() {
        // A temporary chat (#34) is never found.
        let mut convs: Vec<Conversation> = c.state().conversations.iter().filter(|x| !x.temporary).cloned().collect();
        convs.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
        for conv in convs {
            let title = crate::privacy::reveal(c, &conv.id, &conv.title);
            let lower = title.to_lowercase();
            if words.iter().all(|w| lower.contains(w.as_str())) {
                hits.push(aiscraft_store::SearchHit {
                    conversation_id: conv.id.clone(),
                    message_id: String::new(),
                    title,
                    snippet: "Treffer im Titel".into(),
                    backend_id: conv.backend_id.clone(),
                });
                if hits.len() >= LIMIT {
                    return Ok(hits);
                }
            }
        }
    }
    let by_title: std::collections::HashSet<String> = hits.iter().map(|h| h.conversation_id.clone()).collect();
    for mut h in c.store(|st| st.search(query, LIMIT))? {
        if by_title.contains(&h.conversation_id) {
            continue;
        }
        h.title = crate::privacy::reveal(c, &h.conversation_id, &h.title);
        h.snippet = crate::privacy::reveal(c, &h.conversation_id, &h.snippet);
        hits.push(h);
        if hits.len() >= LIMIT {
            break;
        }
    }
    Ok(hits)
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
    if result.is_ok() {
        c.save_backend_cache(&id);
    }
    Ok(serde_json::to_value(result?)?)
}

/// Mark or unmark a favourite on the server. The list shows the change at once and goes back
/// when the server refuses.
async fn persona_favorite(c: Ctx, p: Value) -> Result<Value> {
    let backend_id = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let kind =
        aiscraft_core::PersonaKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss assistant, character oder learning_scenario sein"))?;
    let id = req_str(&p, "id")?;
    let backend = c.backend(&backend_id)?;
    let current =
        c.state().backend(&backend_id).and_then(|b| b.personas.as_ref().and_then(|l| l.iter().find(|x| x.kind == kind && x.id == id).map(|x| x.favorite)));
    let favorite = match (p.get("favorite").and_then(Value::as_bool), current) {
        (Some(f), _) => f,
        (None, Some(f)) => !f,
        (None, None) => return Err(Error::not_found(format!("{} nicht gefunden. Erst die Liste laden.", kind.label()))),
    };
    let mark = |c: &Ctx, value: bool| {
        c.update(|s| {
            if let Some(x) = s
                .backends
                .iter_mut()
                .find(|b| b.config.id == backend_id)
                .and_then(|b| b.personas.as_mut())
                .and_then(|l| l.iter_mut().find(|x| x.kind == kind && x.id == id))
            {
                x.favorite = value;
            }
        })
    };
    mark(&c, favorite);
    if let Err(e) = backend.set_favorite(kind, id.clone(), favorite).await {
        if let Some(before) = current {
            mark(&c, before);
        }
        return Err(Error { message: format!("Der Favorit konnte nicht gespeichert werden: {}", e.message), ..e });
    }
    c.save_backend_cache(&backend_id);
    Ok(json!({"kind": kind.as_str(), "id": id, "favorite": favorite}))
}

/// Start a new chat with an assistant, character or learning scenario. The chat is created on
/// the first message, like any new chat; a character's greeting comes first.
async fn persona_start(c: Ctx, p: Value) -> Result<Value> {
    let backend_id = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let kind =
        aiscraft_core::PersonaKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss assistant, character oder learning_scenario sein"))?;
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
        s.variants.clear();
        s.selection = Selection {
            backend_id,
            model_id,
            project_id,
            sources: Default::default(),
            persona: Some(info.persona()),
            kind: ConversationKind::Chat,
            aspect_ratio: s.selection.aspect_ratio.clone(),
            temporary: false,
        };
    });
    crate::temporary::discard_closed(&c);
    persist_selection(&c);
    let out = serde_json::to_value(&info)?;
    if let Some(text) = opt_str(&p, "text").filter(|t| !t.trim().is_empty()) {
        chat::send_with(c, text, Vec::new(), Vec::new(), None, false).await?;
    }
    Ok(out)
}

// ---- backends ---------------------------------------------------------------------------

async fn backend_add(c: Ctx, p: Value) -> Result<Value> {
    let kind =
        BackendKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss ais_chat, openai, anthropic , claude_cli oder vertex sein"))?;
    let base_url = opt_str(&p, "base_url").filter(|u| !u.trim().is_empty()).unwrap_or_else(|| kind.default_base_url().to_string());
    let name = opt_str(&p, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| match kind {
        BackendKind::AisChat => "AIS.chat".into(),
        BackendKind::Anthropic => "Claude (API)".into(),
        BackendKind::ClaudeCli => "Claude (Abo)".into(),
        BackendKind::Vertex => "Google Vertex AI".into(),
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
    let config = BackendConfig {
        id: format!("{}-{}", kind_slug(kind), &new_id()[..8]),
        name,
        kind,
        base_url,
        default_model,
        send_context: kind != BackendKind::AisChat,
    };
    // Build first so a bad address is reported before anything is saved.
    aiscraft_backends::build(config.clone(), None, c.secret_sink())?;
    if let Some(secret) = secret_param(&p)? {
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
        BackendKind::Vertex => "vertex",
    }
}

/// The secret of `backend.add` and `backend.login`: `secret` as given, or the content of
/// `key_file` (the service account key of Vertex AI, read here so it never passes the UI).
fn secret_param(p: &Value) -> Result<Option<String>> {
    if let Some(secret) = opt_str(p, "secret").filter(|s| !s.trim().is_empty()) {
        return Ok(Some(secret));
    }
    let Some(path) = opt_str(p, "key_file").filter(|s| !s.trim().is_empty()) else { return Ok(None) };
    let path = std::path::PathBuf::from(path.trim());
    let meta = std::fs::metadata(&path).map_err(|e| Error::not_found(format!("Schlüsseldatei nicht lesbar: {} ({e})", path.display())))?;
    if !meta.is_file() || meta.len() > aiscraft_backends::vertex::MAX_KEY_BYTES as u64 {
        return Err(Error::invalid("Das ist keine Schlüsseldatei; gesucht ist die JSON-Datei eines Dienstkontos (etwa 2 KB)"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| Error::invalid(format!("Schlüsseldatei nicht lesbar: {} ({e})", path.display())))?;
    Ok(Some(text))
}

fn normalize_secret(kind: BackendKind, secret: &str) -> Result<String> {
    match kind {
        BackendKind::Vertex => aiscraft_backends::vertex::normalize_key(secret),
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
    let raw = secret_param(&p)?.ok_or_else(|| Error::invalid("secret oder key_file fehlt"))?;
    let secret = normalize_secret(config.kind, &raw)?;
    c.0.secrets.set(&id, Some(&secret))?;
    c.install_backend(config.clone())?;
    // A new key may belong to another project: its models are tried afresh.
    c.backend(&id)?.seed_model_checks(Vec::new());
    refresh_backend(&c, &id).await?;
    let account = match c.state().backend(&id).map(|b| b.status.clone()) {
        Some(BackendStatus::Ready { account }) => account,
        _ => Default::default(),
    };
    let who = account.display_name.clone().unwrap_or_else(|| config.name.clone());
    c.notice_with(
        NoticeLevel::Success,
        format!("Angemeldet bei {}: {who}", config.name),
        (config.kind == BackendKind::AisChat).then(|| NoticeAction {
            label: "Verlauf laden".into(),
            command: "remote.refresh".into(),
            params: json!({"backend": id}),
        }),
    );
    Ok(serde_json::to_value(account)?)
}
