//! One menu for a chat, in two places (#123): the sidebar's row (right click and its „…“) and
//! the chat header's „…“ (Weitere Aktionen). Both draw [`AisCraftApp::chat_menu`], so the two
//! cannot drift apart again; the header adds „Chat schließen (Esc)“.

use aiscraft_core::{BackendKind, Conversation};
use egui::{RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;

/// Where the menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    Sidebar,
    Header,
}

/// One entry of the menu, in the order the menu shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Rename,
    Pin,
    Project,
    Duplicate,
    Export,
    OpenInAisChat,
    Separator,
    Archive,
    Delete,
    Close,
}

/// The accessible name of the „…“ button on a chat's row in the sidebar.
pub(crate) const ROW_BUTTON: &str = "Chat-Aktionen";

impl AisCraftApp {
    /// The link to a chat in AIS.chat, for chats there.
    pub(crate) fn ais_link_for(&self, c: &Conversation) -> Option<String> {
        let b = self.state.backend(&c.backend_id).filter(|b| b.config.kind == BackendKind::AisChat)?;
        let path = if c.kind == aiscraft_core::ConversationKind::Image {
            format!("/image-generation/d/{}", c.remote_id.as_deref().unwrap_or(&c.id))
        } else {
            aiscraft_core::ais_conversation_path(c.persona.as_ref(), &c.id)
        };
        Some(format!("{}{path}", b.config.base_url.trim_end_matches('/')))
    }

    /// What the menu holds for this chat, in order.
    pub(crate) fn chat_menu_entries(&self, c: &Conversation, place: Place) -> Vec<Entry> {
        let mut out = vec![Entry::Rename];
        // A temporary chat (#34) is gone once closed: nothing to pin, move, copy or archive.
        if !c.temporary {
            out.extend([Entry::Pin, Entry::Project, Entry::Duplicate]);
        }
        out.push(Entry::Export);
        if self.ais_link_for(c).is_some() {
            out.push(Entry::OpenInAisChat);
        }
        out.push(Entry::Separator);
        if !c.temporary && !c.archived {
            out.push(Entry::Archive);
        }
        out.push(Entry::Delete);
        if place == Place::Header {
            out.push(Entry::Close);
        }
        out
    }

    /// The chat's menu (sidebar and header).
    pub(crate) fn chat_menu(&mut self, ui: &mut Ui, c: &Conversation, place: Place) {
        let t = self.tokens.clone();
        ui.set_min_width(220.0);
        for entry in self.chat_menu_entries(c, place) {
            match entry {
                Entry::Rename => {
                    if ui.button("Umbenennen …").on_hover_text("F2").clicked() {
                        self.dialog = Some(Dialog::Rename { id: c.id.clone(), title: c.title.clone() });
                        ui.close();
                    }
                }
                Entry::Pin => {
                    if ui.button(if c.pinned { "Lösen" } else { "Anheften" }).clicked() {
                        self.run("chat.pin", json!({"id": c.id, "pinned": !c.pinned}));
                        ui.close();
                    }
                }
                Entry::Project => {
                    let ids = vec![c.id.clone()];
                    ui.menu_button("In Projekt", |ui| self.project_menu(ui, &ids, c.project_id.as_deref()));
                }
                Entry::Duplicate => {
                    if ui.button("Duplizieren").on_hover_text("Eine Kopie mit allen Nachrichten, um eine andere Richtung auszuprobieren").clicked() {
                        self.run("chat.duplicate", json!({"id": c.id}));
                        ui.close();
                    }
                }
                Entry::Export => {
                    ui.menu_button("Als Datei speichern", |ui| {
                        if ui.button("Markdown (.md)").clicked() {
                            self.export_chat(&c.id, "markdown");
                            ui.close();
                        }
                        if ui.button("JSON (.json)").clicked() {
                            self.export_chat(&c.id, "json");
                            ui.close();
                        }
                    });
                }
                Entry::OpenInAisChat => {
                    if let Some(url) = self.ais_link_for(c)
                        && ui.button("In AIS.chat öffnen").clicked()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        ui.close();
                    }
                }
                Entry::Separator => {
                    ui.separator();
                }
                Entry::Archive => {
                    if ui.button("Archivieren").on_hover_text("Aus der Seitenleiste ins Archiv; bleibt ganz erhalten").clicked() {
                        self.archive_chats(vec![c.id.clone()]);
                        ui.close();
                    }
                }
                Entry::Delete => {
                    if ui.button(RichText::new("Löschen").color(t.danger)).on_hover_text("In den Papierkorb (Entf)").clicked() {
                        self.ask_delete(vec![c.id.clone()]);
                        ui.close();
                    }
                }
                Entry::Close => {
                    if ui.button("Chat schließen (Esc)").clicked() {
                        self.close_chat();
                        ui.close();
                    }
                }
            }
        }
    }

    /// „In Projekt ▸“ for these chats: no project, or one of them; the current one is ticked.
    pub(crate) fn project_menu(&mut self, ui: &mut Ui, ids: &[String], current: Option<&str>) {
        if ui.radio(current.is_none() && ids.len() == 1, "Kein Projekt").clicked() {
            for id in ids {
                self.run("chat.move", json!({"id": id, "project": null}));
            }
            ui.close();
        }
        for p in self.state.projects.clone() {
            if ui.radio(ids.len() == 1 && current == Some(p.id.as_str()), &p.name).clicked() {
                for id in ids {
                    self.run("chat.move", json!({"id": id, "project": p.id}));
                }
                ui.close();
            }
        }
    }

    /// Archive these chats; an open one among them closes, back to the start page, where the
    /// toast offers „Rückgängig“.
    pub(crate) fn archive_chats(&mut self, ids: Vec<String>) {
        let open = self.state.current.as_ref().is_some_and(|cur| ids.contains(cur));
        self.run("chat.archive", json!({"ids": ids}));
        self.selected_chats.clear();
        if open {
            self.close_chat();
        }
    }

    /// The open chat, if any.
    pub(crate) fn current_chat(&self) -> Option<Conversation> {
        self.state.current.as_ref().and_then(|id| self.state.conversation(id)).cloned()
    }
}
