//! The panel next to the chat that shows the open document, whiteboard, learning app or
//! worksheet (the last two in `learn_panel.rs` and `worksheet.rs`).

use std::time::{Duration, Instant};

use aiscraft_board::{Board, BoardStyle, BoardView, ViewState};
use aiscraft_core::ArtifactKind;
use base64::Engine as _;
use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::muted;

pub(crate) const SAVE_AFTER: Duration = Duration::from_millis(700);

pub struct ArtifactView {
    pub id: String,
    pub kind: ArtifactKind,
    rev_loaded: u64,
    /// Our own save is on its way; the next revision bump is ours and needs no reload.
    own_saves: u32,
    pub(crate) doc: String,
    /// The document state `doc` was loaded from or last saved as: saves are edits on top of
    /// it, so changes that arrived meanwhile (model, people in a shared room) are kept.
    base: Vec<u8>,
    pub(crate) dirty_since: Option<Instant>,
    pub preview: bool,
    board: Option<(Board, ViewState, u64)>,
    pub revise: String,
    pub revising: bool,
    /// Set by the background revision when it ends, success or not.
    pub(crate) revise_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    error: Option<String>,
    /// An open worksheet: parsed tasks, mode, a pupil's answers.
    pub(crate) sheet: crate::worksheet::SheetState,
}

impl AisCraftApp {
    fn load_artifact(&mut self, id: &str) -> Option<ArtifactView> {
        let got = self.engine.call("artifact.get", json!({"id": id}));
        let v = match got {
            Ok(v) => v,
            Err(e) => {
                return Some(ArtifactView {
                    id: id.to_string(),
                    kind: ArtifactKind::Document,
                    rev_loaded: self.state.artifact_rev,
                    own_saves: 0,
                    doc: String::new(),
                    base: Vec::new(),
                    dirty_since: None,
                    preview: false,
                    board: None,
                    revise: String::new(),
                    revising: false,
                    revise_done: Default::default(),
                    error: Some(e.message),
                    sheet: Default::default(),
                });
            }
        };
        let kind = v.pointer("/artifact/kind").and_then(|k| k.as_str()).and_then(ArtifactKind::parse).unwrap_or(ArtifactKind::Document);
        let text = v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let bytes = v.get("state_b64").and_then(|s| s.as_str()).and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()).unwrap_or_default();
        let board = if kind == ArtifactKind::Board {
            Board::from_state(&bytes).ok().map(|b| {
                let rev = b.revision();
                let mut vs = ViewState::default();
                vs.zoom_to_fit();
                (b, vs, rev)
            })
        } else {
            None
        };
        Some(ArtifactView {
            id: id.to_string(),
            kind,
            rev_loaded: self.state.artifact_rev,
            own_saves: 0,
            preview: kind == ArtifactKind::Document && !text.is_empty(),
            doc: text,
            base: if kind.is_text() { bytes } else { Vec::new() },
            dirty_since: None,
            board,
            revise: String::new(),
            revising: false,
            revise_done: Default::default(),
            error: None,
            sheet: Default::default(),
        })
    }

    /// The artifact shown in the panel: the open one, unless it is in its own window (#75).
    pub(crate) fn panel_artifact(&self) -> Option<String> {
        self.state.open_artifact.clone().filter(|id| !self.state.detached.iter().any(|d| &d.id == id))
    }

    /// Keep the panel's view in step with the engine: open, switch, reload on outside changes.
    pub(crate) fn sync_artifact(&mut self) {
        let want = self.panel_artifact();
        self.sync_view(want);
    }

    /// Keep `self.artifact` in step with the engine for `want` (the panel's artifact, or a
    /// window's while [`crate::windows`] has swapped its view in).
    pub(crate) fn sync_view(&mut self, want: Option<String>) {
        match (&want, &self.artifact) {
            (None, _) => self.artifact = None,
            (Some(id), Some(v)) if &v.id == id => {
                let rev = self.state.artifact_rev;
                if rev != v.rev_loaded {
                    // Shared: any change may carry someone else's edits, so take them in.
                    let shared = self.state.share(id).is_some();
                    let ours = v.own_saves > 0 && !shared;
                    let dirty = v.dirty_since.is_some();
                    if shared && !dirty && v.kind == ArtifactKind::Board {
                        let id = id.clone();
                        self.merge_board(&id, rev);
                    } else if ours {
                        if let Some(v) = self.artifact.as_mut() {
                            v.own_saves -= 1;
                            v.rev_loaded = rev;
                        }
                    } else if !dirty {
                        let revising = v.revising;
                        let preview = v.preview;
                        let mode = v.sheet.mode;
                        self.artifact = self.load_artifact(id);
                        if let Some(v) = self.artifact.as_mut() {
                            v.preview = preview || revising;
                            v.sheet.mode = mode;
                        }
                    }
                }
            }
            (Some(id), _) => self.artifact = self.load_artifact(id),
        }
    }

    /// Merge the stored whiteboard into the open one, keeping zoom and selection.
    fn merge_board(&mut self, id: &str, rev: u64) {
        let bytes = self
            .engine
            .call("artifact.get", json!({"id": id}))
            .ok()
            .and_then(|v| v.get("state_b64").and_then(|s| s.as_str()).and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()));
        let Some(v) = self.artifact.as_mut() else { return };
        v.rev_loaded = rev;
        v.own_saves = v.own_saves.saturating_sub(1);
        if let (Some(bytes), Some((board, _, saved))) = (bytes, v.board.as_mut())
            && board.apply_update(&bytes).is_ok()
        {
            *saved = board.revision();
        }
    }

    pub(crate) fn save_artifact_now(&mut self) {
        let Some(v) = self.artifact.as_mut() else { return };
        match v.kind {
            ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet | ArtifactKind::H5p => match aiscraft_engine::doc_replace(&v.base, &v.doc) {
                Ok(state) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&state);
                    v.base = state;
                    self.engine.dispatch("artifact.save", json!({"id": v.id, "state_b64": b64}));
                }
                Err(_) => self.engine.dispatch("artifact.save", json!({"id": v.id, "text": v.doc})),
            },
            ArtifactKind::Board => {
                if let Some((b, _, saved)) = v.board.as_mut() {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(b.encode_state());
                    *saved = b.revision();
                    self.engine.dispatch("artifact.save", json!({"id": v.id, "state_b64": b64}));
                }
            }
        }
        v.own_saves += 1;
        v.dirty_since = None;
    }

    /// The content `id` with its toolbar, drawn into any `Ui`: the panel next to the chat or its
    /// own window (#75). `self.artifact` must hold this content's view.
    pub(crate) fn artifact_view(&mut self, ui: &mut Ui, id: &str) {
        let t = self.tokens.clone();
        let Some(meta) = self.state.artifacts.iter().find(|a| a.id == id).cloned() else { return };
        self.artifact_header(ui, &meta);
        self.share_strip(ui, &meta.id);
        if let Some(err) = self.artifact.as_ref().and_then(|v| v.error.clone()) {
            ui.label(RichText::new(err).color(t.danger));
            return;
        }
        match meta.kind {
            ArtifactKind::Document => self.document_body(ui),
            ArtifactKind::Board => self.board_body(ui),
            ArtifactKind::App => self.app_body(ui),
            ArtifactKind::Worksheet => self.worksheet_body(ui),
            ArtifactKind::H5p => self.h5p_body(ui),
        }
    }

    /// The export menu's entries for an artifact.
    fn export_entries(&mut self, ui: &mut Ui, meta: &aiscraft_core::Artifact) {
        let formats: &[(&str, &str)] = match meta.kind {
            ArtifactKind::Board => &[("png", "png"), ("markdown", "md"), ("json", "json")],
            ArtifactKind::App => &[("html", "html")],
            ArtifactKind::Worksheet => &[("pdf", "pdf"), ("docx", "docx"), ("json", "json")],
            ArtifactKind::H5p => &[("h5p", "h5p"), ("json", "json")],
            ArtifactKind::Document => &[("markdown", "md"), ("html", "html"), ("docx", "docx"), ("pdf", "pdf")],
        };
        for (f, ext) in formats {
            let label = match *ext {
                "docx" => "Als Word (DOCX)".to_string(),
                "h5p" => "Als H5P (.h5p)".to_string(),
                "png" => "Als Bild (PNG)".to_string(),
                _ => format!("Als {}", ext.to_uppercase()),
            };
            if ui.button(label).clicked() {
                let name: String = meta.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
                if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.{ext}", name.trim())).save_file() {
                    self.save_if_dirty();
                    let path = path.display().to_string();
                    match (meta.kind, *ext) {
                        (ArtifactKind::H5p, "h5p") => self.run("h5p.export", json!({"id": meta.id, "path": path})),
                        _ => self.run("artifact.export", json!({"id": meta.id, "format": f, "path": path})),
                    }
                }
                ui.close();
            }
        }
    }

    /// The versions menu: each saved version, a click restores it. Returns how many there are.
    fn version_entries(&mut self, ui: &mut Ui, meta: &aiscraft_core::Artifact) {
        ui.set_min_width(240.0);
        match self.engine.call("artifact.versions", json!({"id": meta.id})) {
            Ok(v) => {
                for ver in v.as_array().cloned().unwrap_or_default().into_iter().take(30) {
                    let n = ver.get("version").and_then(|x| x.as_i64()).unwrap_or(0);
                    let note = ver.get("note").and_then(|x| x.as_str()).unwrap_or("");
                    if ui.button(format!("{n}. {note}")).on_hover_text("Diese Fassung wiederherstellen").clicked() {
                        self.save_if_dirty();
                        self.run("artifact.restore", json!({"id": meta.id, "version": n}));
                        ui.close();
                    }
                }
            }
            Err(e) => {
                ui.label(e.message);
            }
        }
    }

    /// The rarer actions: turn a document into a worksheet.
    fn more_entries(&mut self, ui: &mut Ui, meta: &aiscraft_core::Artifact) {
        ui.set_min_width(220.0);
        if meta.kind == ArtifactKind::Document
            && ui.button("Als Arbeitsblatt").on_hover_text("Daraus ein interaktives Arbeitsblatt machen lassen (eine Anfrage ans Modell)").clicked()
        {
            self.save_if_dirty();
            self.run("worksheet.from_document", json!({"id": meta.id}));
            ui.close();
        }
    }

    /// The kind as an accent tile, the title in Bold, "Teilen" and close; below
    /// "Bearbeiten | Vorschau", "Im Chat", the version chip, export and more.
    fn artifact_header(&mut self, ui: &mut Ui, meta: &aiscraft_core::Artifact) {
        let t = self.tokens.clone();
        use crate::icons::Icon;
        Frame::new().inner_margin(Margin { left: 20, right: 14, top: 16, bottom: 8 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let (tile, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                crate::kit::accent_fill(ui.painter(), tile, egui::CornerRadius::same(10), &t);
                crate::learn_panel::kind_lucide(meta.kind).paint(ui.painter(), tile.center(), 17.0, t.on_accent);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if self.in_window.is_some() {
                        // In its own window (#75): back into the panel; closing the window does the same.
                        if crate::kit::icon_button(ui, &t, Icon::Panel, "Zurück ins Panel", t.text2, 19.0, 34.0).clicked() {
                            self.save_if_dirty();
                            self.run("artifact.attach", json!({"id": meta.id}));
                        }
                        if meta.kind == ArtifactKind::Board
                            && crate::kit::icon_button(ui, &t, Icon::Monitor, "Präsentieren (Vollbild, Esc beendet)", t.text2, 18.0, 34.0).clicked()
                        {
                            self.save_if_dirty();
                            self.run("artifact.detach", json!({"id": meta.id, "present": true}));
                        }
                    } else {
                        if crate::kit::icon_button(ui, &t, Icon::X, "Schließen", t.text2, 19.0, 34.0).clicked() {
                            self.save_if_dirty();
                            self.run("artifact.close", json!({}));
                        }
                        if meta.kind != ArtifactKind::App
                            && crate::kit::icon_button(ui, &t, Icon::External, "In eigenem Fenster", t.text2, 18.0, 34.0).clicked()
                        {
                            self.detach_artifact(&meta.id);
                        }
                    }
                    self.share_button(ui, meta);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let r = ui
                            .add(egui::Label::new(crate::theme::bold(&meta.title).size(18.0).color(t.text)).truncate().sense(egui::Sense::click()))
                            .on_hover_text("Doppelklick zum Umbenennen");
                        if r.double_clicked() {
                            self.dialog = Some(crate::dialogs::Dialog::RenameArtifact { id: meta.id.clone(), title: meta.title.clone() });
                        }
                    });
                });
            });
        });
        Frame::new().inner_margin(Margin { left: 20, right: 14, top: 4, bottom: 12 }).show(ui, |ui| {
            // „Bearbeiten | Vorschau“ and „Im Chat“ on the left, the actions on the right. The
            // actions are placed first; where the left group does not fit next to them (a narrow
            // panel), it moves to a row of its own instead of lying on top of them.
            let left = |app: &mut Self, ui: &mut Ui| {
                if meta.kind == ArtifactKind::Document
                    && let Some(v) = app.artifact.as_mut()
                {
                    crate::widgets::segmented_icons(ui, &t, &mut v.preview, [(false, Some(Icon::Pen), "Bearbeiten"), (true, Some(Icon::Eye), "Vorschau")]);
                }
                let on = app.attach_artifact.as_deref() == Some(meta.id.as_str());
                let kind = if on { crate::kit::Pill::Soft } else { crate::kit::Pill::Ghost };
                let icon = if meta.kind == ArtifactKind::Document { None } else { Some(Icon::Chat) };
                if crate::kit::pill(ui, &t, kind, icon, "Im Chat", 13.0, None)
                    .on_hover_text("Inhalt bei der nächsten Nachricht als Kontext mitschicken")
                    .clicked()
                {
                    app.attach_artifact = if on { None } else { Some(meta.id.clone()) };
                }
            };
            let need_id = ui.scope_id().with("toolbar-left-width");
            let need = ui.ctx().data(|d| d.get_temp::<f32>(need_id)).unwrap_or(0.0);
            let mut fits = true;
            // One row as tall as the segmented control, so both groups centre on the same line.
            let row = egui::vec2(ui.available_width(), 36.0);
            ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if matches!(meta.kind, ArtifactKind::Document | ArtifactKind::Board) {
                    let more = crate::kit::icon_button(ui, &t, Icon::More, "Weitere Aktionen", t.text2, 18.0, 32.0);
                    if meta.kind != ArtifactKind::Board {
                        egui::Popup::menu(&more).show(|ui| self.more_entries(ui, meta));
                    } else {
                        egui::Popup::menu(&more).show(|ui| {
                            if ui.button("Umbenennen …").clicked() {
                                self.dialog = Some(crate::dialogs::Dialog::RenameArtifact { id: meta.id.clone(), title: meta.title.clone() });
                                ui.close();
                            }
                        });
                    }
                }
                let export = crate::kit::icon_button(ui, &t, Icon::Download, "Exportieren", t.text2, 18.0, 32.0);
                egui::Popup::menu(&export).show(|ui| self.export_entries(ui, meta));
                let count = self.engine.call("artifact.versions", json!({"id": meta.id})).ok().and_then(|v| v.as_array().map(|a| a.len())).unwrap_or(0);
                let versions = if count > 0 {
                    crate::kit::pill(ui, &t, crate::kit::Pill::Chip, Some(Icon::History), &format!("V{count}"), 13.0, None).on_hover_text("Versionen")
                } else {
                    crate::kit::icon_button(ui, &t, Icon::History, "Versionen", t.text2, 18.0, 32.0)
                };
                egui::Popup::menu(&versions).show(|ui| self.version_entries(ui, meta));
                ui.add_space(8.0);
                fits = ui.available_width() >= need;
                if fits {
                    let w = ui
                        .with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            left(self, ui);
                        })
                        .response
                        .rect
                        .width();
                    ui.ctx().data_mut(|d| d.insert_temp(need_id, w));
                }
            });
            if !fits {
                ui.add_space(6.0);
                let w = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        left(self, ui);
                    })
                    .response
                    .rect
                    .width();
                ui.ctx().data_mut(|d| d.insert_temp(need_id, w));
            }
        });
        let r = ui.cursor();
        ui.painter().line_segment([egui::pos2(ui.max_rect().left(), r.top()), egui::pos2(ui.max_rect().right(), r.top())], egui::Stroke::new(1.0, t.border));
    }

    pub(crate) fn save_if_dirty(&mut self) {
        if self.artifact.as_ref().is_some_and(|v| v.dirty_since.is_some()) {
            self.save_artifact_now();
        }
    }

    fn document_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(v) = self.artifact.as_mut() else { return };
        // Revise with AI: the bar sits under the document, like the chat composer.
        let was_revising = v.revising;
        self.revise_bar(ui, "Mit KI überarbeiten, z. B. „Kürzer und für Klasse 5“");
        let Some(v) = self.artifact.as_mut() else { return };
        if v.revising && !was_revising {
            v.preview = true;
        }
        let mut board = None;
        // The document as a sheet on a recessed area.
        let sheet = if t.dark { t.opaque(t.surface) } else { egui::Color32::WHITE };
        let (area, page) = (
            Frame::new().fill(if t.glass.is_some() { egui::Color32::TRANSPARENT } else { t.sunken }).inner_margin(Margin {
                left: 18,
                right: 18,
                top: 18,
                bottom: 0,
            }),
            Frame::new()
                .fill(sheet)
                .corner_radius(egui::CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 })
                .inner_margin(Margin { left: 28, right: 28, top: 26, bottom: 24 })
                .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: egui::Color32::from_rgba_unmultiplied(0x0b, 0x2a, 0x45, 0x14) }),
        );
        area.show(ui, |ui| {
            ScrollArea::vertical().id_salt(("doc", v.id.clone())).auto_shrink([false, false]).show(ui, |ui| {
                page.show(ui, |ui| {
                    ui.set_min_height(ui.available_height());
                    ui.set_width(ui.available_width());
                    if v.preview {
                        if v.doc.trim().is_empty() {
                            ui.label(muted(&t, "Noch leer. Unter „Bearbeiten“ schreiben oder eine Antwort als Dokument öffnen."));
                        }
                        for (k, seg) in crate::markup::segments(&v.doc).into_iter().enumerate() {
                            if let Some(crate::markup::Action::Board(outline)) = crate::markup::show(ui, &mut self.md, &t, &seg, ("doc", &v.id, k)) {
                                board = Some(outline);
                            }
                        }
                    } else {
                        let r = ui.add(
                            egui::TextEdit::multiline(&mut v.doc)
                                .desired_width(f32::INFINITY)
                                .desired_rows(30)
                                .frame(Frame::NONE)
                                .hint_text("# Titel\n\nText in Markdown …"),
                        );
                        if r.changed() {
                            v.dirty_since = Some(Instant::now());
                        }
                    }
                });
            })
        });
        if let Some(outline) = board {
            self.run("artifact.create", json!({"kind": "board", "content": outline}));
        }
        if self.artifact.as_ref().and_then(|v| v.dirty_since).is_some_and(|d| d.elapsed() >= SAVE_AFTER) {
            self.save_artifact_now();
        } else if self.artifact.as_ref().is_some_and(|v| v.dirty_since.is_some()) {
            ui.ctx().request_repaint_after(SAVE_AFTER);
        }
    }

    pub(crate) fn board_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let style = BoardStyle {
            // Glas: the panel's own glass shows through the canvas.
            bg: if t.glass.is_some() { egui::Color32::TRANSPARENT } else { t.opaque(t.panel) },
            grid: t.opaque(t.border),
            text: t.text,
            muted: t.muted,
            accent: t.accent,
            selection: t.accent,
            surface: t.opaque(t.composer),
            border: t.opaque(t.border),
            on_accent: t.on_accent,
            accent_soft: t.opaque(t.accent_soft),
            rounded: true,
            title_family: Some(egui::FontFamily::Name(crate::theme::BOLD.into())),
            ..if t.dark { BoardStyle::dark() } else { BoardStyle::light() }
        };
        let presenting = self.presenting();
        let Some(v) = self.artifact.as_mut() else { return };
        let Some((board, view, saved)) = v.board.as_mut() else {
            ui.label(RichText::new("Whiteboard lässt sich nicht lesen.").color(t.danger));
            return;
        };
        // Presenting (#75): the board alone, without its tool bars.
        view.toolbar = !presenting;
        BoardView::new(board, view, &style).show(ui);
        if std::mem::take(&mut view.wants_mindmap) {
            // The last answer of the open chat becomes the mind map.
            let answer =
                self.state.messages.iter().rev().find(|m| m.role == aiscraft_core::Role::Assistant && !m.content.trim().is_empty()).map(|m| m.content.clone());
            match answer {
                Some(outline) => self.engine.dispatch("artifact.board_add", json!({"id": v.id, "outline": outline})),
                None => {
                    self.engine.ctx().notice(aiscraft_engine::NoticeLevel::Info, "Öffne zuerst einen Chat mit einer Antwort, dann wird daraus eine Mindmap.");
                }
            }
        }
        if board.revision() != *saved && v.dirty_since.is_none() {
            v.dirty_since = Some(Instant::now());
        }
        if v.dirty_since.is_some_and(|d| d.elapsed() >= SAVE_AFTER) {
            self.save_artifact_now();
        } else if v.dirty_since.is_some() {
            ui.ctx().request_repaint_after(SAVE_AFTER);
        }
    }
}
