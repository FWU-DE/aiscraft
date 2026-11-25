//! Command palette (Ctrl+K): commands, chats, projects and models in one fuzzy list.

use aiscraft_engine::{BackendStatus, commands};
use egui::{Id, Key, RichText};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::widgets::{fuzzy_score, small_muted};

#[derive(Debug, Default)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
}

#[derive(Debug, Clone)]
enum Item {
    Command { id: &'static str, label: &'static str, group: &'static str, shortcut: Option<&'static str> },
    Chat { id: String, title: String },
    Project { id: String, name: String },
    Model { backend: String, model: String, label: String },
    Ui { label: &'static str, group: &'static str, action: UiAction },
    Template { id: String, title: String },
}

#[derive(Debug, Clone, Copy)]
enum UiAction {
    Settings,
    Memory,
    AddBackend,
    NewProject,
    ToggleSidebar,
    Manual,
    Export,
    Search,
    Explore,
    ConnectorInsert,
    NewTemplate,
}

impl Item {
    fn text(&self) -> String {
        match self {
            Item::Command { label, .. } | Item::Ui { label, .. } => label.to_string(),
            Item::Chat { title, .. } => title.clone(),
            Item::Project { name, .. } => format!("Projekt: {name}"),
            Item::Model { label, .. } => format!("Modell: {label}"),
            Item::Template { title, .. } => format!("Vorlage: {title}"),
        }
    }
    /// What the row is about (Chat, Dokumente, Einstellungen …), shown on its right unless it has a shortcut.
    fn kind(&self) -> &'static str {
        match self {
            Item::Command { group, .. } => group_label(group),
            Item::Ui { group, .. } => group,
            Item::Chat { .. } => "Chat",
            Item::Project { .. } => "Projekt",
            Item::Model { .. } => "Modell",
            Item::Template { .. } => "Vorlage",
        }
    }
}

/// The engine's command groups in the words the rest of the app uses.
fn group_label(group: &'static str) -> &'static str {
    match group {
        "Backends" => "Dienste",
        "Artefakte" => "Dokumente",
        "App" => "Einstellungen",
        g => g,
    }
}

/// "Ctrl+Shift+E" as German keyboards label it: "Strg+Umschalt+E".
fn shortcut_label(s: &str) -> String {
    s.replace("Ctrl", "Strg").replace("Shift", "Umschalt").replace("Escape", "Esc")
}

impl AisCraftApp {
    fn palette_items(&self) -> Vec<Item> {
        // Commands that need params get a UI action instead of a bare dispatch.
        let ui_handled = ["chat.export", "project.create", "search"];
        let mut items: Vec<Item> = commands()
            .iter()
            .filter(|c| c.palette && !ui_handled.contains(&c.id))
            .map(|c| Item::Command { id: c.id, label: c.label, group: c.group, shortcut: c.shortcut })
            .collect();
        items.extend([
            Item::Ui { label: "Einstellungen", group: "Einstellungen", action: UiAction::Settings },
            Item::Ui { label: "Handbuch", group: "Hilfe", action: UiAction::Manual },
            Item::Ui { label: "Gedächtnis öffnen", group: "Gedächtnis", action: UiAction::Memory },
            Item::Ui { label: "Dienst hinzufügen", group: "Dienste", action: UiAction::AddBackend },
            Item::Ui { label: "Neues Projekt", group: "Projekte", action: UiAction::NewProject },
            Item::Ui { label: "Seitenleiste ein/aus", group: "Ansicht", action: UiAction::ToggleSidebar },
            Item::Ui { label: "Chat exportieren", group: "Chat", action: UiAction::Export },
            Item::Ui { label: "Alle Chats durchsuchen", group: "Suche", action: UiAction::Search },
            Item::Ui { label: "Assistenten, Dialogpartner & Lernszenarien entdecken", group: "AIS.chat", action: UiAction::Explore },
            Item::Ui { label: "Konnektor-Ergebnis einfügen", group: "Konnektoren", action: UiAction::ConnectorInsert },
            Item::Ui { label: "Neue Vorlage", group: "Vorlage", action: UiAction::NewTemplate },
        ]);
        items.extend(self.state.templates.iter().map(|t| Item::Template { id: t.id.clone(), title: t.title.clone() }));
        for b in &self.state.backends {
            if matches!(b.status, BackendStatus::Ready { .. }) {
                for m in &b.models {
                    items.push(Item::Model { backend: b.config.id.clone(), model: m.id.clone(), label: format!("{} · {}", b.config.name, m.name) });
                }
            }
        }
        items.extend(self.state.projects.iter().map(|p| Item::Project { id: p.id.clone(), name: p.name.clone() }));
        items.extend(self.state.conversations.iter().map(|c| Item::Chat { id: c.id.clone(), title: c.title.clone() }));
        items
    }

    pub(crate) fn palette_ui(&mut self, ctx: &egui::Context) {
        let Some(mut pal) = self.palette.take() else { return };
        let t = self.tokens.clone();
        let mut scored: Vec<(i32, Item)> = self.palette_items().into_iter().filter_map(|it| fuzzy_score(&pal.query, &it.text()).map(|s| (s, it))).collect();
        if !pal.query.is_empty() {
            scored.sort_by_key(|x| std::cmp::Reverse(x.0));
        }
        scored.truncate(40);
        let n = scored.len();
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown)) {
            pal.selected = (pal.selected + 1).min(n.saturating_sub(1));
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp)) {
            pal.selected = pal.selected.saturating_sub(1);
        }
        let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let mut chosen = None;
        let mut close = false;
        let modal = egui::Modal::new(Id::unique("palette")).frame(crate::widgets::dialog_frame(ctx, &t).inner_margin(egui::Margin::same(14))).show(ctx, |ui| {
            ui.set_width(560.0);
            let r = ui.add(egui::TextEdit::singleline(&mut pal.query).hint_text("Befehl, Chat, Projekt oder Modell …").desired_width(f32::INFINITY));
            r.request_focus();
            if r.changed() {
                pal.selected = 0;
            }
            ui.add_space(6.0);
            // A whole number of rows (28 px and the gap), so the last one is never cut in half.
            let row = 28.0 + ui.spacing().item_spacing.y;
            egui::ScrollArea::vertical().max_height(11.0 * row - ui.spacing().item_spacing.y).show(ui, |ui| {
                for (i, (_, it)) in scored.iter().enumerate() {
                    let sel = i == pal.selected;
                    let resp = ui.add(egui::Button::selectable(sel, RichText::new(it.text())).truncate().min_size(egui::vec2(ui.available_width(), 28.0)));
                    let hint = match it {
                        Item::Command { shortcut: Some(s), .. } => shortcut_label(s),
                        _ => it.kind().to_string(),
                    };
                    ui.painter().text(
                        resp.rect.right_center() - egui::vec2(10.0, 0.0),
                        egui::Align2::RIGHT_CENTER,
                        hint,
                        egui::FontId::proportional(12.0),
                        t.muted,
                    );
                    if sel {
                        resp.scroll_to_me(None);
                    }
                    if resp.clicked() {
                        chosen = Some(it.clone());
                    }
                }
                if scored.is_empty() {
                    ui.label(small_muted(&t, "Nichts gefunden."));
                }
            });
        });
        if modal.should_close() {
            close = true;
        }
        if enter {
            chosen = scored.get(pal.selected).map(|x| x.1.clone());
        }
        if let Some(it) = chosen {
            self.palette_run(it);
            return;
        }
        if !close {
            self.palette = Some(pal);
        }
    }

    fn palette_run(&mut self, it: Item) {
        match it {
            Item::Command { id: "chat.new", .. } => self.new_chat(),
            Item::Command { id, .. } => self.run(id, json!({})),
            Item::Chat { id, .. } => self.run("chat.open", json!({"id": id})),
            Item::Project { id, .. } => {
                self.project_filter = Some(id.clone());
                self.run("chat.new", json!({"project": id}));
            }
            Item::Model { backend, model, .. } => self.run("chat.select_model", json!({"backend": backend, "model": model})),
            Item::Template { id, .. } => {
                if let Some(t) = self.state.templates.iter().find(|t| t.id == id) {
                    self.dialog = Some(Dialog::Know(crate::know::KnowDialog::use_template(t)));
                }
            }
            Item::Ui { action, .. } => match action {
                UiAction::Settings => self.dialog = Some(Dialog::settings()),
                UiAction::Memory => self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Memory, new_note: String::new() }),
                UiAction::AddBackend => self.dialog = Some(Dialog::add_backend()),
                UiAction::NewProject => self.dialog = Some(Dialog::project(None)),
                UiAction::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
                UiAction::Manual => self.manual = Some(Default::default()),
                UiAction::Export => self.export_dialog(),
                UiAction::Explore => self.open_explore(),
                UiAction::ConnectorInsert => self.dialog = Some(Dialog::Know(crate::know::KnowDialog::Connector { tool: None, args: Vec::new() })),
                UiAction::NewTemplate => self.dialog = Some(Dialog::Know(crate::know::KnowDialog::edit_template(None))),
                UiAction::Search => {
                    self.sidebar_open = true;
                    self.focus_search = true;
                }
            },
        }
    }
}
