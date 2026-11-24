//! The panel next to the chat that shows the open document, whiteboard, learning app or
//! worksheet (the last two in `learn_panel.rs` and `worksheet.rs`).

use std::time::{Duration, Instant};

use aiscraft_board::{Board, BoardStyle, BoardView, ViewState};
use aiscraft_core::ArtifactKind;
use base64::Engine as _;
use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{Glyph, filter_chip, glyph_button, muted, segmented};

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

    /// Keep the view in step with the engine: open, switch, reload on outside changes.
    pub(crate) fn sync_artifact(&mut self) {
        let want = self.state.open_artifact.clone();
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
            ArtifactKind::Document | ArtifactKind::App | ArtifactKind::Worksheet => match aiscraft_engine::doc_replace(&v.base, &v.doc) {
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

    pub(crate) fn artifact_panel(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(meta) = self.state.open_artifact.as_ref().and_then(|id| self.state.artifacts.iter().find(|a| &a.id == id)).cloned() else { return };
        // Header, two rows (design review 2026-11): the title with the close button, then the
        // actions. One row let the right-to-left buttons run over the title in a narrow panel.
        Frame::new().inner_margin(Margin { left: 16, right: 8, top: 6, bottom: 4 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::learn_panel::kind_icon(meta.kind)).color(t.accent).size(18.0)).on_hover_text(meta.kind.label());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if glyph_button(ui, &t, Glyph::Close, "Schließen").clicked() {
                        self.save_if_dirty();
                        self.run("artifact.close", json!({}));
                    }
                    if self.state.collab.available {
                        self.share_menu(ui, &meta);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let r = ui
                            .add(egui::Label::new(crate::theme::semibold(&meta.title).size(18.0)).truncate().sense(egui::Sense::click()))
                            .on_hover_text("Doppelklick zum Umbenennen");
                        if r.double_clicked() {
                            self.dialog = Some(crate::dialogs::Dialog::RenameArtifact { id: meta.id.clone(), title: meta.title.clone() });
                        }
                    });
                });
            });
            ui.horizontal(|ui| {
                if meta.kind == ArtifactKind::Document
                    && let Some(v) = self.artifact.as_mut()
                {
                    segmented(ui, &t, &mut v.preview, [(false, "Bearbeiten"), (true, "Vorschau")]);
                    ui.add_space(4.0);
                }
                let mut attach = self.attach_artifact;
                if filter_chip(ui, &t, &mut attach, "Im Chat").on_hover_text("Inhalt bei der nächsten Nachricht als Kontext mitschicken").clicked() {
                    self.attach_artifact = attach;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Rare actions go into the overflow menu, so the row fits a narrow panel.
                    if meta.kind == ArtifactKind::Document {
                        let more = glyph_button(ui, &t, Glyph::More, "Weitere Aktionen");
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(220.0);
                            if ui
                                .button("Als Arbeitsblatt")
                                .on_hover_text("Daraus ein interaktives Arbeitsblatt machen lassen (eine Anfrage ans Modell)")
                                .clicked()
                            {
                                self.save_if_dirty();
                                self.run("worksheet.from_document", json!({"id": meta.id}));
                                ui.close();
                            }
                        });
                    }
                    let export = glyph_button(ui, &t, Glyph::Download, "Exportieren");
                    egui::Popup::menu(&export).show(|ui| {
                        let formats: &[(&str, &str)] = match meta.kind {
                            ArtifactKind::Board => &[("markdown", "md"), ("json", "json")],
                            ArtifactKind::App => &[("html", "html")],
                            ArtifactKind::Worksheet => &[("pdf", "pdf"), ("docx", "docx"), ("json", "json")],
                            ArtifactKind::Document => &[("markdown", "md"), ("html", "html"), ("docx", "docx"), ("pdf", "pdf")],
                        };
                        for (f, ext) in formats {
                            let label = if *ext == "docx" { "Als Word (DOCX)".to_string() } else { format!("Als {}", ext.to_uppercase()) };
                            if ui.button(label).clicked() {
                                let name: String = meta.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
                                if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.{ext}", name.trim())).save_file() {
                                    self.save_if_dirty();
                                    self.run("artifact.export", json!({"id": meta.id, "format": f, "path": path.display().to_string()}));
                                }
                                ui.close();
                            }
                        }
                    });
                    let versions = glyph_button(ui, &t, Glyph::History, "Versionen");
                    egui::Popup::menu(&versions).show(|ui| {
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
                    });
                });
            });
        });
        ui.separator();
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
        }
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
        ScrollArea::vertical().id_salt(("doc", v.id.clone())).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
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

    fn board_body(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let style = BoardStyle {
            bg: t.bg,
            grid: t.border,
            text: t.text,
            muted: t.muted,
            accent: t.accent,
            selection: t.accent,
            surface: t.surface,
            border: t.border,
            ..if t.dark { BoardStyle::dark() } else { BoardStyle::light() }
        };
        let Some(v) = self.artifact.as_mut() else { return };
        let Some((board, view, saved)) = v.board.as_mut() else {
            ui.label(RichText::new("Whiteboard lässt sich nicht lesen.").color(t.danger));
            return;
        };
        BoardView::new(board, view, &style).show(ui);
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
