//! The panel next to the chat that shows the open document or whiteboard.

use std::time::{Duration, Instant};

use aiscraft_board::{Board, BoardStyle, BoardView, ViewState};
use aiscraft_core::ArtifactKind;
use base64::Engine as _;
use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, icon_button, muted, small_muted};

const SAVE_AFTER: Duration = Duration::from_millis(700);

pub struct ArtifactView {
    pub id: String,
    pub kind: ArtifactKind,
    rev_loaded: u64,
    /// Our own save is on its way; the next revision bump is ours and needs no reload.
    own_saves: u32,
    doc: String,
    /// The document state `doc` was loaded from or last saved as: saves are edits on top of
    /// it, so changes that arrived meanwhile (model, people in a shared room) are kept.
    base: Vec<u8>,
    dirty_since: Option<Instant>,
    pub preview: bool,
    board: Option<(Board, ViewState, u64)>,
    pub revise: String,
    pub revising: bool,
    /// Set by the background revision when it ends, success or not.
    revise_done: std::sync::Arc<std::sync::atomic::AtomicBool>,
    error: Option<String>,
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
            base: if kind == ArtifactKind::Document { bytes } else { Vec::new() },
            dirty_since: None,
            board,
            revise: String::new(),
            revising: false,
            revise_done: Default::default(),
            error: None,
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
                        self.artifact = self.load_artifact(id);
                        if let Some(v) = self.artifact.as_mut() {
                            v.preview = preview || revising;
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

    fn save_artifact_now(&mut self) {
        let Some(v) = self.artifact.as_mut() else { return };
        match v.kind {
            ArtifactKind::Document => match aiscraft_engine::doc_replace(&v.base, &v.doc) {
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
        // Header
        Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(if meta.kind == ArtifactKind::Board { "▣" } else { "📄" }).color(t.accent));
                let r = ui
                    .add(egui::Label::new(RichText::new(&meta.title).strong()).truncate().sense(egui::Sense::click()))
                    .on_hover_text("Doppelklick zum Umbenennen");
                if r.double_clicked() {
                    self.dialog = Some(crate::dialogs::Dialog::RenameArtifact { id: meta.id.clone(), title: meta.title.clone() });
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if icon_button(ui, &t, "×", "Schließen").clicked() {
                        self.save_if_dirty();
                        self.run("artifact.close", json!({}));
                    }
                    ui.menu_button(RichText::new("⬇").color(t.muted), |ui| {
                        let formats: &[(&str, &str)] =
                            if meta.kind == ArtifactKind::Board { &[("markdown", "md"), ("json", "json")] } else { &[("markdown", "md"), ("html", "html")] };
                        for (f, ext) in formats {
                            if ui.button(format!("Als {}", ext.to_uppercase())).clicked() {
                                let name: String = meta.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
                                if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{}.{ext}", name.trim())).save_file() {
                                    self.save_if_dirty();
                                    self.run("artifact.export", json!({"id": meta.id, "format": f, "path": path.display().to_string()}));
                                }
                                ui.close();
                            }
                        }
                        if meta.kind == ArtifactKind::Document {
                            ui.label(small_muted(&t, "HTML lässt sich im Browser als PDF drucken."));
                        }
                    })
                    .response
                    .on_hover_text("Exportieren");
                    ui.menu_button(RichText::new("⟳").color(t.muted), |ui| {
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
                    })
                    .response
                    .on_hover_text("Versionen");
                    let mut attach = self.attach_artifact;
                    if ui.toggle_value(&mut attach, "Im Chat").on_hover_text("Inhalt bei der nächsten Nachricht als Kontext mitschicken").changed() {
                        self.attach_artifact = attach;
                    }
                    if self.state.collab.available {
                        self.share_menu(ui, &meta);
                    }
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
        // Revise with AI.
        let mut do_revise = None;
        Frame::new().inner_margin(Margin::symmetric(12, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut v.preview, false, "Bearbeiten");
                ui.selectable_value(&mut v.preview, true, "Vorschau");
            });
            ui.horizontal(|ui| {
                let r = ui.add_enabled(
                    !v.revising,
                    egui::TextEdit::singleline(&mut v.revise)
                        .hint_text("Mit KI überarbeiten, z. B. „Kürzer und für Klasse 5“")
                        .desired_width(ui.available_width() - 90.0),
                );
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if v.revising {
                    ui.add(egui::Spinner::new().color(t.accent));
                } else if (accent_button(ui, &t, "Los").clicked() || enter) && !v.revise.trim().is_empty() {
                    do_revise = Some(v.revise.trim().to_string());
                }
            });
        });
        if let Some(instruction) = do_revise {
            self.save_if_dirty();
            if let Some(v) = self.artifact.as_mut() {
                v.revising = true;
                v.preview = true;
                let id = v.id.clone();
                let engine = self.engine.clone();
                let done = v.revise_done.clone();
                done.store(false, std::sync::atomic::Ordering::Relaxed);
                // Runs in the background; the panel reloads when the engine saves the new version.
                std::thread::spawn(move || {
                    if let Err(e) = engine.call("artifact.revise", json!({"id": id, "instruction": instruction})) {
                        engine.ctx().notice(aiscraft_engine::NoticeLevel::Error, e.message);
                    }
                    done.store(true, std::sync::atomic::Ordering::Relaxed);
                });
                v.revise.clear();
            }
        }
        let Some(v) = self.artifact.as_mut() else { return };
        if v.revising && v.revise_done.load(std::sync::atomic::Ordering::Relaxed) {
            v.revising = false;
        }
        if v.revising {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        ScrollArea::vertical().id_salt(("doc", v.id.clone())).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(16, 8)).show(ui, |ui| {
                if v.preview {
                    if v.doc.trim().is_empty() {
                        ui.label(muted(&t, "Noch leer. Unter „Bearbeiten“ schreiben oder eine Antwort als Dokument öffnen."));
                    }
                    egui_commonmark::CommonMarkViewer::new().show(ui, &mut self.md, &v.doc);
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
