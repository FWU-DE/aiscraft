//! Contents in their own window (#75): a document, whiteboard, worksheet or H5P content lifted
//! out of the panel into a native window of the same process (an egui viewport; no web engine, no
//! second process). Which contents are out, and where their windows stood, lives in the engine
//! (`artifact.detach`, `artifact.attach`, `artifact.window`); here are the views and the windows.
//!
//! Each window shows the same content with the same toolbar as the panel ([`AisCraftApp::
//! artifact_view`]); its view state (text being typed, zoom of the board) is kept per window and
//! swapped into `self.artifact` while it draws. Tests can't host a second native window: there
//! (`embed_viewports`) no window opens, and [`AisCraftApp::show_as_window`] draws a window's
//! content as the whole screen instead.

use std::collections::HashMap;

use aiscraft_core::ArtifactKind;
use egui::{Frame, Key, Modifiers, Rect, Ui, ViewportBuilder, ViewportCommand, ViewportId};
use serde_json::json;

use crate::app::AisCraftApp;

/// A window's size the first time it opens, in points.
const FIRST_SIZE: [f32; 2] = [960.0, 760.0];
/// How long a window must stand still before its place is stored (seconds, egui's clock).
const PLACE_AFTER: f64 = 1.0;

/// What the UI knows about one open window.
#[derive(Default)]
pub(crate) struct WindowSeen {
    /// The engine's `focus` last acted on.
    focus: u64,
    /// The place the window opened at (kept, so the builder does not change with every move).
    opened_at: Option<(Option<[f32; 2]>, [f32; 2])>,
    /// The window's place in the last frame, and since when it stands there.
    place: Option<(Rect, f64)>,
    /// The place last sent to the engine.
    stored: Option<Rect>,
    /// Closed by the person; not drawn again while the engine takes it back.
    closing: bool,
}

/// The windows' view state, kept by the app.
#[derive(Default)]
pub(crate) struct Windows {
    pub(crate) views: HashMap<String, crate::artifact_panel::ArtifactView>,
    pub(crate) seen: HashMap<String, WindowSeen>,
    /// Draw this window's content as the whole screen (tests, layout gate).
    pub(crate) as_window: Option<String>,
}

impl AisCraftApp {
    /// Draw the following frames as the own window of `id` instead of the main window, or the
    /// main window again with `None`. kittest hosts one viewport only, so the UI tests and the
    /// layout gate look at a detached content this way; the real app never calls it.
    pub fn show_as_window(&mut self, id: Option<String>) {
        self.windows.as_window = id;
    }

    /// „In eigenem Fenster“: the panel's view moves along (no reload, nothing typed is lost).
    pub(crate) fn detach_artifact(&mut self, id: &str) {
        self.save_if_dirty();
        if self.artifact.as_ref().is_some_and(|v| v.id == id)
            && let Some(v) = self.artifact.take()
        {
            self.windows.views.insert(id.to_string(), v);
        }
        self.run("artifact.detach", json!({"id": id}));
    }

    /// Save what was typed in the window of `id` right now (before it goes along as context).
    pub(crate) fn save_window_if_dirty(&mut self, id: &str) {
        if self.windows.views.contains_key(id) {
            self.with_window_view(id, |app| app.save_if_dirty());
        }
    }

    /// The whiteboard in the window being drawn is presented (full screen, no toolbar).
    pub(crate) fn presenting(&self) -> bool {
        self.in_window.as_ref().is_some_and(|id| self.state.detached.iter().any(|d| &d.id == id && d.presenting))
    }

    /// Run `f` with the window's view of `id` in `self.artifact` (the panel's view waits aside).
    fn with_window_view<R>(&mut self, id: &str, f: impl FnOnce(&mut Self) -> R) -> R {
        let mut view = self.windows.views.remove(id);
        std::mem::swap(&mut self.artifact, &mut view);
        let before = self.in_window.replace(id.to_string());
        let r = f(self);
        self.in_window = before;
        std::mem::swap(&mut self.artifact, &mut view);
        if let Some(v) = view {
            self.windows.views.insert(id.to_string(), v);
        }
        r
    }

    /// Keep the windows' views in step with the engine; save and drop those taken back.
    pub(crate) fn sync_windows(&mut self) {
        let out: Vec<String> = self.state.detached.iter().map(|d| d.id.clone()).collect();
        let gone: Vec<String> = self.windows.views.keys().filter(|k| !out.contains(k)).cloned().collect();
        for id in gone {
            self.with_window_view(&id, |app| app.save_if_dirty());
            self.windows.views.remove(&id);
        }
        self.windows.seen.retain(|k, _| out.contains(k));
        for id in out {
            self.with_window_view(&id, |app| app.sync_view(Some(id.clone())));
        }
        // „Im Chat“ only for a content that is on screen.
        if let Some(a) = &self.attach_artifact
            && self.panel_artifact().as_ref() != Some(a)
            && !self.state.detached.iter().any(|d| &d.id == a)
        {
            self.attach_artifact = None;
        }
    }

    /// Open, update and close the native windows. Without native windows (tests, platforms that
    /// embed viewports) none opens; the engine still knows which contents are out.
    pub(crate) fn artifact_windows(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if ctx.embed_viewports() || self.windows.as_window.is_some() {
            return;
        }
        // The screens there are now, in points, to tell whether a stored place is still visible.
        let ppp = ctx.pixels_per_point().max(0.1);
        let screens: Option<Vec<Rect>> = frame.winit_window().map(|w| {
            w.available_monitors()
                .map(|m| {
                    let (p, s) = (m.position(), m.size());
                    Rect::from_min_size(egui::pos2(p.x as f32 / ppp, p.y as f32 / ppp), egui::vec2(s.width as f32 / ppp, s.height as f32 / ppp))
                })
                .collect()
        });
        for d in self.state.detached.clone() {
            let Some(meta) = self.state.artifacts.iter().find(|a| a.id == d.id).cloned() else { continue };
            let seen = self.windows.seen.entry(d.id.clone()).or_default();
            if seen.closing {
                continue;
            }
            let vid = ViewportId::from_hash_of(("artifact-window", &d.id));
            // Where it opens: the stored place if a screen is still there, else the main screen.
            let (pos, size) = *seen.opened_at.get_or_insert_with(|| {
                let place = d.place.map(|p| (egui::pos2(p.x, p.y), [p.width, p.height]));
                let visible = |pos: egui::Pos2| match &screens {
                    Some(all) => all.iter().any(|s| s.contains(pos + egui::vec2(60.0, 16.0))),
                    None => false,
                };
                match place {
                    Some((pos, size)) if visible(pos) => (Some([pos.x, pos.y]), size),
                    Some((_, size)) => (None, size),
                    None => (None, FIRST_SIZE),
                }
            });
            if d.focus != seen.focus {
                seen.focus = d.focus;
                ctx.send_viewport_cmd_to(vid, ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(vid, ViewportCommand::Focus);
            }
            let mut builder = ViewportBuilder::default()
                .with_title(format!("{} – AIS.ChatCraft", meta.title))
                .with_app_id("ai.uds.aischatcraft")
                .with_inner_size(size)
                .with_min_inner_size([360.0, 280.0])
                .with_fullscreen(d.presenting);
            if let Some(pos) = pos {
                builder = builder.with_position(pos);
            }
            let id = d.id.clone();
            ctx.show_viewport_immediate(vid, builder, |ui, _| self.window_frame(ui, &id));
        }
    }

    /// One window's content: the same view and toolbar as in the panel; presenting, the
    /// whiteboard alone. Closing the window takes the content back.
    pub(crate) fn window_frame(&mut self, ui: &mut Ui, id: &str) {
        let ctx = ui.ctx().clone();
        let t = self.tokens.clone();
        let presenting = self.state.detached.iter().any(|d| d.id == id && d.presenting);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.with_window_view(id, |app| app.save_if_dirty());
            self.windows.seen.entry(id.to_string()).or_default().closing = true;
            self.run("artifact.attach", json!({"id": id}));
            return;
        }
        if presenting && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.run("artifact.detach", json!({"id": id, "present": false}));
        }
        self.remember_place(&ctx, id, presenting);
        if let Some(g) = &t.glass {
            crate::backdrop::paint(&ctx.layer_painter(egui::LayerId::background()), ctx.content_rect(), g, 1.0);
        }
        let kind = self.state.artifacts.iter().find(|a| a.id == id).map(|a| a.kind);
        let rect = egui::CentralPanel::default()
            .frame(Frame::new().fill(t.panel))
            .show(ui, |ui| {
                self.with_window_view(id, |app| {
                    if presenting && kind == Some(ArtifactKind::Board) {
                        app.board_body(ui);
                    } else {
                        app.artifact_view(ui, id);
                    }
                });
            })
            .response
            .rect;
        if self.windows.as_window.as_deref() == Some(id) {
            // The layout gate reads the window like the panel: as wide as the window, never wider.
            self.set_window_layout(ctx.content_rect(), rect);
        }
    }

    /// Send the window's place to the engine once it has stood still for a moment.
    fn remember_place(&mut self, ctx: &egui::Context, id: &str, presenting: bool) {
        let (outer, inner, now) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect, i.time));
        let (Some(outer), Some(inner)) = (outer, inner) else { return };
        if presenting || ctx.input(|i| i.viewport().fullscreen == Some(true) || i.viewport().minimized == Some(true)) {
            return;
        }
        let place = Rect::from_min_size(outer.min, inner.size());
        let seen = self.windows.seen.entry(id.to_string()).or_default();
        match seen.place {
            Some((p, _)) if p != place => seen.place = Some((place, now)),
            None => seen.place = Some((place, now)),
            Some((p, since)) if now - since >= PLACE_AFTER && seen.stored != Some(p) => {
                seen.stored = Some(p);
                self.engine.dispatch("artifact.window", json!({"id": id, "x": p.min.x, "y": p.min.y, "width": p.width(), "height": p.height()}));
            }
            Some(_) => {}
        }
        if seen.stored != seen.place.map(|(p, _)| p) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PLACE_AFTER));
        }
    }
}
