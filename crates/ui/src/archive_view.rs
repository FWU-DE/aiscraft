//! Archive and trash (#77): the page with the archived chats (search, „Öffnen“,
//! „Wiederherstellen“) or the trash („Wiederherstellen“, „Endgültig löschen“, „Papierkorb
//! leeren“), the dialogs for deleting and purging, and the multi-select in the sidebar.

use aiscraft_core::{BackendKind, Conversation, TRASH_DAYS};
use egui::{Align, Frame, Key, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{muted, small_muted};

/// The archive page, when open (replaces the chat column).
#[derive(Debug, Default)]
pub struct ArchiveView {
    /// The trash instead of the archive.
    pub trash: bool,
    pub query: String,
    pub focus_search: bool,
}

/// Chats about to go into the trash.
#[derive(Debug, Clone)]
pub struct DeleteChats {
    pub ids: Vec<String>,
    /// „„Titel““ or „3 Chats“.
    pub what: String,
    /// One chat with AIS.chat: the dialog says it stays in AIS.chat's history and offers to
    /// delete it there too.
    pub ais_chat: bool,
    /// The owner's explicit yes to delete it in AIS.chat as well.
    pub also_ais_chat: bool,
}

/// Chats about to be deleted for good; `ids` empty: the whole trash.
#[derive(Debug, Clone)]
pub struct PurgeChats {
    pub ids: Vec<String>,
    pub what: String,
    pub forget_notes: bool,
}

/// Width of the page's column.
const PAGE_WIDTH: f32 = 760.0;

impl AisCraftApp {
    pub(crate) fn open_archive(&mut self, trash: bool) {
        self.explore = None;
        self.archive = Some(ArchiveView { trash, query: String::new(), focus_search: true });
    }

    /// Ask before moving these chats into the trash.
    pub(crate) fn ask_delete(&mut self, ids: Vec<String>) {
        let convs: Vec<&Conversation> = ids.iter().filter_map(|id| self.state.conversation(id)).collect();
        let Some(first) = convs.first() else { return };
        let what = if convs.len() == 1 { format!("„{}“", self.engine.reveal(&first.id, &first.title)) } else { format!("{} Chats", convs.len()) };
        let ais = |c: &Conversation| self.state.backend(&c.backend_id).is_some_and(|b| b.config.kind == BackendKind::AisChat);
        let ais_chat = convs.len() == 1 && ais(first);
        let any_ais = convs.iter().any(|c| ais(c));
        let ids = convs.iter().map(|c| c.id.clone()).collect();
        self.dialog = Some(Dialog::DeleteChats(DeleteChats { ids, what, ais_chat: ais_chat || any_ais, also_ais_chat: false }));
    }

    pub(crate) fn delete_chats_dialog(&mut self, ui: &mut Ui, d: &mut DeleteChats) -> bool {
        let t = self.tokens.clone();
        ui.label(RichText::new(format!("{} löschen?", d.what)).strong());
        ui.add_space(4.0);
        ui.label(format!("Gelöscht wird in AIS.ChatCraft. Im Papierkorb lässt es sich {TRASH_DAYS} Tage lang wiederherstellen, danach ist es endgültig weg."));
        if d.ais_chat {
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Im Verlauf von AIS.chat bleibt der Chat stehen: AIS.chat speichert jeden Chat selbst. Löschen in AIS.ChatCraft ändert daran nichts.",
                )
                .color(t.warning),
            );
            if d.ids.len() == 1 {
                ui.checkbox(&mut d.also_ais_chat, "Auch in AIS.chat löschen (lässt sich dort nicht rückgängig machen)");
            } else {
                ui.label(small_muted(&t, "Einzeln über das Kontextmenü lässt sich ein Chat auch in AIS.chat löschen."));
            }
        }
        ui.add_space(8.0);
        let mut keep = true;
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Löschen").color(t.on_accent)).fill(t.danger)).clicked() {
                let p = if d.ids.len() == 1 && d.also_ais_chat { json!({"ids": d.ids, "ais_chat": true}) } else { json!({"ids": d.ids}) };
                self.run("chat.delete", p);
                self.selected_chats.retain(|id| !d.ids.contains(id));
                keep = false;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        keep
    }

    pub(crate) fn purge_chats_dialog(&mut self, ui: &mut Ui, d: &mut PurgeChats) -> bool {
        let t = self.tokens.clone();
        ui.label(RichText::new(format!("{} endgültig löschen?", d.what)).strong());
        ui.add_space(4.0);
        ui.label(
            "Das lässt sich nicht rückgängig machen. Nachrichten, Suchtreffer, die Kopien der Anhänge und die Zuordnung von „Namen schützen“ werden entfernt.",
        );
        ui.add_space(4.0);
        ui.label(small_muted(
            &t,
            "Notizen im Gedächtnis, die aus dem Chat stammen, bleiben. Dokumente, Whiteboards und Arbeitsblätter aus dem Chat bleiben ebenfalls.",
        ));
        ui.checkbox(&mut d.forget_notes, "Auch Notizen aus diesem Chat vergessen");
        ui.add_space(8.0);
        let mut keep = true;
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Endgültig löschen").color(t.on_accent)).fill(t.danger)).clicked() {
                if d.ids.is_empty() {
                    self.run("chat.empty_trash", json!({"forget_notes": d.forget_notes}));
                } else {
                    self.run("chat.purge", json!({"ids": d.ids, "forget_notes": d.forget_notes}));
                }
                keep = false;
            }
            if ui.button("Abbrechen").clicked() {
                keep = false;
            }
        });
        keep
    }

    /// „Archiv“ and „Papierkorb“ at the bottom of the chat list, when they hold something.
    pub(crate) fn archive_links(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let archived = self.state.conversations.iter().filter(|c| c.archived).count();
        let trashed = self.state.trash.len();
        if archived == 0 && trashed == 0 {
            return;
        }
        ui.add_space(10.0);
        let open = self.archive.as_ref().map(|v| v.trash);
        if archived > 0 {
            let r = crate::kit::side_item(ui, &t, crate::icons::Icon::Download, &format!("Archiv ({archived})"), open == Some(false));
            if r.on_hover_text("Archivierte Chats").clicked() {
                if open == Some(false) {
                    self.archive = None;
                } else {
                    self.open_archive(false);
                }
            }
        }
        if trashed > 0 {
            let r = crate::kit::side_item(ui, &t, crate::icons::Icon::Trash, &format!("Papierkorb ({trashed})"), open == Some(true));
            if r.on_hover_text(format!("Gelöschte Chats, {TRASH_DAYS} Tage lang wiederherstellbar")).clicked() {
                if open == Some(true) {
                    self.archive = None;
                } else {
                    self.open_archive(true);
                }
            }
        }
    }

    /// Esc closes the page. Returns whether it took the key.
    pub(crate) fn archive_keys(&mut self, ctx: &egui::Context) -> bool {
        if self.archive.is_none() {
            return false;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.archive = None;
            self.focus_composer = true;
            return true;
        }
        false
    }

    pub(crate) fn archive_page(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(mut view) = self.archive.take() else { return };
        let archived: Vec<Conversation> = self.state.archived_conversations().into_iter().cloned().collect();
        let trash: Vec<Conversation> = self.state.trash.clone();
        let mut close = false;
        egui::Panel::top("archive-head").frame(Frame::new().fill(t.bg).inner_margin(Margin { left: 20, right: 20, top: 12, bottom: 10 })).show(ui, |ui| {
            column(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(muted(&t, "‹  Zurück zum Chat")).frame(false)).on_hover_text("Esc").clicked() {
                        close = true;
                    }
                });
                ui.add_space(2.0);
                ui.label(RichText::new(if view.trash { "Papierkorb" } else { "Archiv" }).size(24.0).strong());
                ui.label(muted(
                    &t,
                    if view.trash {
                        format!("Gelöschte Chats bleiben {TRASH_DAYS} Tage hier und lassen sich wiederherstellen; danach werden sie endgültig gelöscht.")
                    } else {
                        "Archivierte Chats bleiben ganz erhalten, stehen aber nicht in der Seitenleiste. Die Suche über alle Chats findet sie.".to_string()
                    },
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::selectable(!view.trash, format!("Archiv ({})", archived.len()))).clicked() {
                        view.trash = false;
                    }
                    if ui.add(egui::Button::selectable(view.trash, format!("Papierkorb ({})", trash.len()))).clicked() {
                        view.trash = true;
                    }
                });
                ui.add_space(6.0);
                crate::widgets::field_then_buttons(
                    ui,
                    |ui| {
                        if view.trash && !trash.is_empty() && ui.add(egui::Button::new(RichText::new("Papierkorb leeren").color(t.danger))).clicked() {
                            self.dialog = Some(Dialog::PurgeChats(PurgeChats {
                                ids: Vec::new(),
                                what: format!("Alle {} Chats im Papierkorb", trash.len()),
                                forget_notes: false,
                            }));
                        }
                    },
                    |ui, w| {
                        let r = ui.add(egui::TextEdit::singleline(&mut view.query).hint_text("Nach Titel suchen").desired_width(w));
                        if view.focus_search {
                            r.request_focus();
                            view.focus_search = false;
                        }
                    },
                );
            });
        });
        let words: Vec<String> = view.query.to_lowercase().split_whitespace().map(str::to_string).collect();
        let list: Vec<&Conversation> = if view.trash { trash.iter().collect() } else { archived.iter().collect() };
        let shown: Vec<(&Conversation, String)> = list
            .into_iter()
            .map(|c| (c, self.engine.reveal(&c.id, &c.title)))
            .filter(|(_, title)| {
                let lower = title.to_lowercase();
                words.iter().all(|w| lower.contains(w.as_str()))
            })
            .collect();
        ScrollArea::vertical().id_salt("archive-list").auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(8.0);
            column(ui, |ui| {
                if shown.is_empty() {
                    ui.label(muted(
                        &t,
                        match (view.trash, words.is_empty()) {
                            (_, false) => "Nichts gefunden.",
                            (true, true) => "Der Papierkorb ist leer.",
                            (false, true) => "Noch nichts archiviert. Über das Kontextmenü eines Chats in der Seitenleiste legst du ihn ins Archiv.",
                        },
                    ));
                }
                for (c, title) in shown {
                    close |= self.archive_row(ui, c, &title, view.trash);
                }
            });
        });
        if close {
            self.focus_composer = true;
        } else if self.archive.is_none() {
            self.archive = Some(view);
        }
    }

    /// One chat of the list; returns whether it was opened (the page closes).
    fn archive_row(&mut self, ui: &mut Ui, c: &Conversation, title: &str, trash: bool) -> bool {
        let t = self.tokens.clone();
        let mut opened = false;
        let title = if title.trim().is_empty() { "Ohne Titel" } else { title };
        let project = c.project_id.as_ref().and_then(|p| self.state.projects.iter().find(|x| &x.id == p)).map(|p| p.name.clone());
        let detail = if trash {
            let left = c.deleted_at.map(|at| TRASH_DAYS - (aiscraft_engine::now_ms() - at) / 86_400_000).unwrap_or(TRASH_DAYS).max(0);
            if left == 1 { "noch 1 Tag im Papierkorb".to_string() } else { format!("noch {left} Tage im Papierkorb") }
        } else {
            format!("zuletzt {}", crate::widgets::age_group(c.updated_at))
        };
        let detail = match project {
            Some(p) => format!("{p} · {detail}"),
            None => detail,
        };
        Frame::new().fill(t.surface).corner_radius(egui::CornerRadius::same(t.radius)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            crate::widgets::field_then_buttons(
                ui,
                |ui| {
                    if trash {
                        if ui
                            .add(egui::Button::new(RichText::new("Endgültig löschen").color(t.danger)))
                            .on_hover_text("Lässt sich nicht rückgängig machen")
                            .clicked()
                        {
                            self.dialog = Some(Dialog::PurgeChats(PurgeChats { ids: vec![c.id.clone()], what: format!("„{title}“"), forget_notes: false }));
                        }
                        if ui.button("Wiederherstellen").clicked() {
                            self.run("chat.restore", json!({"id": c.id}));
                        }
                    } else {
                        if ui.button("Löschen").clicked() {
                            self.ask_delete(vec![c.id.clone()]);
                        }
                        if ui.button("Wiederherstellen").on_hover_text("Wieder in die Seitenleiste").clicked() {
                            self.run("chat.unarchive", json!({"id": c.id}));
                        }
                        if ui.button("Öffnen").clicked() {
                            self.run("chat.open", json!({"id": c.id}));
                            opened = true;
                        }
                    }
                },
                |ui, w| {
                    ui.allocate_ui_with_layout(egui::vec2(w, 36.0), Layout::top_down(Align::Min), |ui| {
                        ui.set_width(w);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(egui::Label::new(RichText::new(title).strong()).truncate());
                        ui.add(egui::Label::new(small_muted(&t, detail)).truncate());
                    });
                },
            );
        });
        ui.add_space(6.0);
        opened
    }

    // ---- multi-select in the sidebar ------------------------------------------------------

    /// A click on a chat in the sidebar with Strg (toggle) or Umschalt (a range from the last
    /// one picked). `order` is the list as shown. Returns whether the click was a selection.
    pub(crate) fn select_click(&mut self, id: &str, mods: egui::Modifiers, order: &[String]) -> bool {
        if mods.shift {
            let anchor = self.select_anchor.clone().or_else(|| self.state.current.clone()).filter(|a| order.contains(a));
            let to = order.iter().position(|x| x == id);
            let from = anchor.as_ref().and_then(|a| order.iter().position(|x| x == a)).or(to);
            if let (Some(from), Some(to)) = (from, to) {
                let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
                self.selected_chats = order.get(lo..=hi).map(<[String]>::to_vec).unwrap_or_default();
            }
            return true;
        }
        if mods.command || mods.ctrl {
            // The open chat counts as picked when the selection starts.
            if self.selected_chats.is_empty()
                && let Some(cur) = self.state.current.clone().filter(|c| c != id && order.contains(c))
            {
                self.selected_chats.push(cur);
            }
            if let Some(pos) = self.selected_chats.iter().position(|x| x == id) {
                self.selected_chats.remove(pos);
            } else {
                self.selected_chats.push(id.to_string());
            }
            self.select_anchor = Some(id.to_string());
            return true;
        }
        false
    }

    /// Above the list while chats are picked: how many, and what to do with all of them.
    pub(crate) fn selection_bar(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        // Picked chats that went away (deleted, archived elsewhere) drop out.
        let present: Vec<String> = self.state.listed_conversations().map(|c| c.id.clone()).collect();
        self.selected_chats.retain(|id| present.contains(id));
        if self.selected_chats.is_empty() {
            return;
        }
        let ids = self.selected_chats.clone();
        ui.add_space(6.0);
        Frame::new().fill(t.side_active.gamma_multiply(0.5)).corner_radius(egui::CornerRadius::same(10)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} ausgewählt", ids.len())).strong().color(t.side_text));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("Auswahl aufheben").clicked() {
                        self.selected_chats.clear();
                        self.select_anchor = None;
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Archivieren").clicked() {
                    self.archive_chats(ids.clone());
                }
                if ui.small_button(RichText::new("Löschen").color(t.danger)).clicked() {
                    self.ask_delete(ids.clone());
                }
                // Pin or unpin all of them (#123).
                let all_pinned = ids.iter().all(|id| self.state.conversation(id).is_some_and(|c| c.pinned));
                if ui.small_button(if all_pinned { "Lösen" } else { "Anheften" }).clicked() {
                    for id in &ids {
                        self.run("chat.pin", json!({"id": id, "pinned": !all_pinned}));
                    }
                }
                ui.menu_button(RichText::new("In Projekt").small(), |ui| self.project_menu(ui, &ids, None));
                if ui.small_button("Exportieren").clicked() {
                    self.export_many(&ids);
                }
            });
        });
        ui.add_space(4.0);
    }

    /// Export several chats as Markdown files into a folder the owner picks.
    fn export_many(&self, ids: &[String]) {
        let Some(dir) = rfd::FileDialog::new().set_title("Ordner für die Chats wählen").pick_folder() else { return };
        let mut used = std::collections::HashSet::new();
        for id in ids {
            let title = self.state.conversation(id).map(|c| self.engine.reveal(&c.id, &c.title)).unwrap_or_else(|| "chat".into());
            let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
            let safe = safe.trim().chars().take(80).collect::<String>();
            let base = if safe.is_empty() { "chat".to_string() } else { safe };
            let mut name = format!("{base}.md");
            let mut n = 2;
            while !used.insert(name.clone()) || dir.join(&name).exists() {
                name = format!("{base} ({n}).md");
                n += 1;
            }
            self.run("chat.export", json!({"id": id, "format": "markdown", "path": dir.join(name).display().to_string()}));
        }
    }
}

/// A centred column at most [`PAGE_WIDTH`] wide.
fn column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width().min(PAGE_WIDTH);
    let pad = ((ui.available_width() - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui);
        });
    });
}
