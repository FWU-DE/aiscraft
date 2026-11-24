use std::path::PathBuf;
use std::sync::Arc;

use aiscraft_engine::{Engine, NoticeLevel, State};
use egui::{Align2, CornerRadius, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, RichText, Stroke};
use serde_json::{Value, json};

use crate::dialogs::Dialog;
use crate::palette::Palette;
use crate::theme::{self, ThemeWatcher, Tokens};

/// A request from the control channel that needs the UI thread.
pub enum UiRequest {
    /// Save a PNG of the window.
    Screenshot { path: PathBuf, reply: std::sync::mpsc::Sender<Result<Value, String>> },
    /// Open a dialog by name: settings, memory, login, add_backend, palette, welcome.
    Show { what: String, reply: std::sync::mpsc::Sender<Result<Value, String>> },
}

/// View state only; the engine owns the data.
pub struct AisCraftApp {
    control: Option<std::sync::mpsc::Receiver<UiRequest>>,
    pub(crate) engine: Engine,
    pub(crate) state: Arc<State>,
    pub(crate) tokens: Tokens,
    theme_key: (String, bool),
    watcher: ThemeWatcher,
    pub(crate) md: egui_commonmark::CommonMarkCache,
    pub(crate) composer: String,
    pub(crate) pending_files: Vec<PathBuf>,
    /// Web links for the next message, and the address being typed (`Some` while the field is open).
    pub(crate) pending_links: Vec<String>,
    pub(crate) link_input: Option<String>,
    pub(crate) sidebar_open: bool,
    pub(crate) search_text: String,
    pub(crate) project_filter: Option<String>,
    pub(crate) editing: Option<(String, String)>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) palette: Option<Palette>,
    pub(crate) focus_composer: bool,
    pub(crate) focus_search: bool,
    pub(crate) remote_open: bool,
    /// Backends whose assistants, characters and scenarios were asked for already.
    pub(crate) personas_requested: std::collections::HashSet<String>,
    /// The "Entdecken" page, when open (replaces the chat column).
    pub(crate) explore: Option<crate::explore::ExploreView>,
    /// Persona pictures asked for already, by picture key.
    pub(crate) pictures_requested: std::collections::HashSet<String>,
    /// Small persona textures by file (`None`: could not be read).
    pub(crate) persona_textures: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    /// Pictures still to decode this frame, so a full grid never stalls one frame.
    pub(crate) decode_budget: usize,
    last_current: Option<String>,
    pub(crate) artifact: Option<crate::artifact_panel::ArtifactView>,
    /// Send the open artifact as context with the next message.
    pub(crate) attach_artifact: bool,
    /// Teilen: code or link to join, relay address and key being typed.
    pub(crate) collab_join: String,
    pub(crate) collab_url: String,
    pub(crate) collab_key: String,
    /// The handbook window, when open.
    pub(crate) manual: Option<crate::manual::ManualView>,
    /// Text from a finished dictation, picked up by the composer.
    pub(crate) dictated: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    /// Generated images by file, decoded once (`None`: could not be read).
    pub(crate) images: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    /// Numbers the start page reads from the engine, refreshed every few seconds.
    pub(crate) home: crate::home::HomeCache,
}

pub(crate) const CONTENT_WIDTH: f32 = 760.0;

impl AisCraftApp {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine, control: Option<std::sync::mpsc::Receiver<UiRequest>>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        engine.on_change(move || ctx.request_repaint());
        let state = engine.state();
        let system_dark = cc.egui_ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark);
        let tokens = theme::resolve(&state.settings.theme, system_dark);
        tokens.apply(&cc.egui_ctx, state.settings.zoom);
        let first_run = state.backends.iter().all(|b| !b.has_secret) && state.backends.len() == 1;
        Self {
            control,
            engine,
            theme_key: (state.settings.theme.clone(), system_dark),
            state,
            tokens,
            watcher: ThemeWatcher::new(),
            md: Default::default(),
            composer: String::new(),
            pending_files: Vec::new(),
            pending_links: Vec::new(),
            link_input: None,
            sidebar_open: true,
            search_text: String::new(),
            project_filter: None,
            editing: None,
            dialog: first_run.then_some(Dialog::Welcome),
            palette: None,
            focus_composer: true,
            focus_search: false,
            remote_open: false,
            personas_requested: Default::default(),
            explore: None,
            pictures_requested: Default::default(),
            persona_textures: Default::default(),
            decode_budget: 0,
            last_current: None,
            artifact: None,
            attach_artifact: false,
            collab_join: String::new(),
            collab_url: String::new(),
            collab_key: String::new(),
            dictated: Default::default(),
            manual: None,
            images: Default::default(),
            home: Default::default(),
        }
    }

    /// The texture of an image file (at most 25 MB, 8192 px a side), loaded on first use.
    pub(crate) fn image_texture(&mut self, ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
        if let Some(t) = self.images.get(path) {
            return t.clone();
        }
        let tex = std::fs::metadata(path)
            .ok()
            .filter(|m| m.len() <= 25 * 1024 * 1024)
            .and_then(|_| std::fs::read(path).ok())
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .filter(|img| img.width() <= 8192 && img.height() <= 8192)
            .map(|img| {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                ctx.load_texture(path, egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
            });
        // Keep the cache small: generated images are few, but never let it grow without bound.
        if self.images.len() > 64 {
            self.images.clear();
        }
        self.images.insert(path.to_string(), tex.clone());
        tex
    }

    pub(crate) fn run(&self, cmd: &str, params: Value) {
        self.engine.dispatch(cmd, params);
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let system_dark = ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark);
        let key = (self.state.settings.theme.clone(), system_dark);
        let follow = key.0 == "omarchy" && self.watcher.changed();
        if key != self.theme_key || follow {
            self.tokens = theme::resolve(&key.0, system_dark);
            self.theme_key = key;
        }
        self.tokens.apply(ctx, self.state.settings.zoom);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let sc = |m: Modifiers, k: Key| KeyboardShortcut::new(m, k);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        // F1: the handbook, except over an open whiteboard, which has its own shortcut overview.
        let over_board = self
            .state
            .open_artifact
            .as_ref()
            .and_then(|id| self.state.artifacts.iter().find(|a| &a.id == id))
            .is_some_and(|a| a.kind == aiscraft_core::ArtifactKind::Board)
            && ctx.pointer_hover_pos().is_some_and(|p| p.x > ctx.content_rect().center().x);
        if !over_board && pressed(sc(Modifiers::NONE, Key::F1)) {
            self.manual = if self.manual.is_some() { None } else { Some(Default::default()) };
        }
        let modal_open = self.dialog.is_some() || self.palette.is_some();
        if pressed(sc(Modifiers::COMMAND, Key::K)) || pressed(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::P)) {
            self.palette = if self.palette.is_some() { None } else { Some(Palette::default()) };
        }
        if modal_open {
            return;
        }
        if self.explore_keys(ctx) {
            return;
        }
        if pressed(sc(Modifiers::COMMAND, Key::N)) {
            self.new_chat();
        }
        if pressed(sc(Modifiers::COMMAND, Key::B)) {
            self.sidebar_open = !self.sidebar_open;
        }
        if pressed(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
            self.sidebar_open = true;
            self.focus_search = true;
        }
        if pressed(sc(Modifiers::COMMAND, Key::Comma)) {
            self.dialog = Some(Dialog::settings());
        }
        if pressed(sc(Modifiers::COMMAND, Key::R)) {
            self.run("chat.regenerate", json!({}));
        }
        if pressed(sc(Modifiers::COMMAND | Modifiers::SHIFT, Key::E)) {
            self.export_dialog();
        }
        let generating = self.state.current.as_ref().is_some_and(|c| self.state.is_generating(c));
        if generating && pressed(sc(Modifiers::NONE, Key::Escape)) {
            self.run("chat.stop", json!({}));
        }
        // Esc with an empty composer and nothing else open closes the chat: back to the start page.
        let composer_or_nothing = ctx.memory(|m| m.focused()).is_none_or(|f| f == Id::unique("composer"));
        if !generating
            && self.state.current.is_some()
            && self.composer.trim().is_empty()
            && self.pending_files.is_empty()
            && self.link_input.is_none()
            && self.editing.is_none()
            && self.state.open_artifact.is_none()
            && composer_or_nothing
            && !egui::Popup::is_any_open(ctx)
            && pressed(sc(Modifiers::NONE, Key::Escape))
        {
            self.close_chat();
        }
    }

    pub(crate) fn new_chat(&mut self) {
        self.explore = None;
        self.run("chat.new", json!({}));
        self.composer.clear();
        self.pending_files.clear();
        self.pending_links.clear();
        self.link_input = None;
        self.editing = None;
        self.focus_composer = true;
    }

    /// Back to the start page.
    pub(crate) fn close_chat(&mut self) {
        self.explore = None;
        self.run("chat.close", json!({}));
        self.composer.clear();
        self.editing = None;
        self.focus_composer = true;
    }

    pub(crate) fn export_dialog(&self) {
        let Some(id) = self.state.current.clone() else { return };
        let title = self.state.conversation(&id).map(|c| c.title.clone()).unwrap_or_else(|| "chat".into());
        let safe: String = title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        if let Some(path) =
            rfd::FileDialog::new().set_file_name(format!("{}.md", safe.trim())).add_filter("Markdown", &["md"]).add_filter("JSON", &["json"]).save_file()
        {
            let format = if path.extension().is_some_and(|e| e == "json") { "json" } else { "markdown" };
            self.run("chat.export", json!({"id": id, "format": format, "path": path.display().to_string()}));
        }
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let t = self.tokens.clone();
        let notices = self.state.notices.clone();
        if notices.is_empty() {
            return;
        }
        egui::Area::new(Id::unique("toasts")).anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0)).order(egui::Order::Foreground).show(ctx, |ui| {
            ui.set_max_width(380.0);
            for n in notices.iter().rev() {
                let color = match n.level {
                    NoticeLevel::Info => t.muted,
                    NoticeLevel::Success => t.success,
                    NoticeLevel::Warning => t.warning,
                    NoticeLevel::Error => t.danger,
                };
                Frame::new()
                    .fill(t.surface)
                    .stroke(Stroke::new(1.0, t.border))
                    .corner_radius(CornerRadius::same(t.radius))
                    .inner_margin(Margin::symmetric(12, 10))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.set_width(340.0);
                        ui.horizontal_top(|ui| {
                            crate::widgets::status_dot(ui, color);
                            ui.add(egui::Label::new(RichText::new(&n.text)).wrap());
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(14.0);
                            if let Some(a) = &n.action
                                && ui.small_button(&a.label).clicked()
                            {
                                self.run(&a.command, a.params.clone());
                                self.run("notice.dismiss", json!({"id": n.id}));
                            }
                            if ui.small_button("Schließen").clicked() {
                                self.run("notice.dismiss", json!({"id": n.id}));
                            }
                        });
                    });
                ui.add_space(6.0);
            }
        });
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.pending_files.extend(dropped);
            self.focus_composer = true;
        }
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let t = &self.tokens;
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::unique("drop")));
            painter.rect_filled(screen, 0.0, t.bg.gamma_multiply(0.85));
            painter.text(screen.center(), Align2::CENTER_CENTER, "Dateien hier ablegen", egui::FontId::proportional(22.0), t.text);
        }
    }
}

impl AisCraftApp {
    fn control_requests(&mut self, ctx: &egui::Context) {
        let mut reqs = Vec::new();
        if let Some(rx) = &self.control {
            while let Ok(r) = rx.try_recv() {
                reqs.push(r);
            }
        }
        for r in reqs {
            match r {
                UiRequest::Screenshot { path, reply } => {
                    let cb = egui::ScreenshotCallback::new(move |img| {
                        let _ = reply
                            .send(save_png(&path, &img).map(|()| json!({"path": path.display().to_string(), "width": img.size[0], "height": img.size[1]})));
                    });
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(cb));
                }
                UiRequest::Show { what, reply } => {
                    let ok = match what.as_str() {
                        "settings" => {
                            self.dialog = Some(Dialog::settings());
                            true
                        }
                        "memory" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Memory, new_note: String::new() });
                            true
                        }
                        "about" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::About, new_note: String::new() });
                            true
                        }
                        "appearance" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Appearance, new_note: String::new() });
                            true
                        }
                        "login" => {
                            self.dialog = Some(Dialog::login(&self.state.selection.backend_id));
                            true
                        }
                        "add_backend" => {
                            self.dialog = Some(Dialog::add_backend());
                            true
                        }
                        "welcome" => {
                            self.dialog = Some(Dialog::Welcome);
                            true
                        }
                        "manual" => {
                            self.manual = Some(Default::default());
                            true
                        }
                        "palette" => {
                            self.palette = Some(Palette::default());
                            true
                        }
                        "personas" => {
                            self.open_explore();
                            true
                        }
                        // "persona:<kind>:<id>": the page with one entry's details.
                        w if w.starts_with("persona:") => {
                            let mut parts = w.splitn(3, ':').skip(1);
                            let kind = parts.next().and_then(aiscraft_core::PersonaKind::parse);
                            let id = parts.next().unwrap_or("").to_string();
                            let backend = self
                                .state
                                .backends
                                .iter()
                                .find(|b| b.personas.as_ref().is_some_and(|l| l.iter().any(|p| Some(p.kind) == kind && p.id == id)))
                                .map(|b| b.config.id.clone());
                            match (kind, backend) {
                                (Some(kind), Some(backend)) => {
                                    self.open_explore();
                                    if let Some(v) = self.explore.as_mut() {
                                        v.focus_search = false;
                                        v.detail = Some(crate::explore::Pick { backend, kind, id });
                                    }
                                    true
                                }
                                _ => false,
                            }
                        }
                        "none" => {
                            self.dialog = None;
                            self.palette = None;
                            true
                        }
                        _ => false,
                    };
                    let _ = reply.send(if ok { Ok(json!({})) } else { Err(format!("Unbekannt: {what}")) });
                }
            }
            ctx.request_repaint();
        }
    }
}

fn save_png(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let [w, h] = img.size;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    let buf = image::RgbaImage::from_raw(w as u32, h as u32, bytes).ok_or("Bildgröße passt nicht")?;
    buf.save(path).map_err(|e| e.to_string())
}

impl eframe::App for AisCraftApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.state = self.engine.state();
        if self.state.current != self.last_current {
            self.last_current = self.state.current.clone();
            self.editing = None;
            self.focus_composer = true;
            self.explore = None;
        }
        self.control_requests(&ctx);
        self.sync_theme(&ctx);
        self.shortcuts(&ctx);
        self.dropped_files(&ctx);

        let t = self.tokens.clone();
        // The panel needs its own copy to animate; a click inside the sidebar (its collapse
        // button) changes `self.sidebar_open` meanwhile and must win over that copy.
        let before = self.sidebar_open;
        let mut sidebar_open = before;
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(272.0)
            .size_range(200.0..=420.0)
            .frame(Frame::new().fill(t.sidebar).inner_margin(Margin::symmetric(10, 12)))
            .show_collapsible(ui, &mut sidebar_open, |ui| self.sidebar(ui));
        if self.sidebar_open == before {
            self.sidebar_open = sidebar_open;
        }

        self.sync_artifact();
        if self.state.open_artifact.is_some() {
            let w = ui.available_width();
            egui::Panel::right("artifact")
                .resizable(true)
                .default_size((w * 0.45).clamp(360.0, 760.0))
                .size_range(320.0..=(w - 360.0).max(340.0))
                .frame(Frame::new().fill(t.surface).inner_margin(Margin::ZERO))
                .show(ui, |ui| self.artifact_panel(ui));
        } else {
            self.attach_artifact = false;
        }

        egui::CentralPanel::default().frame(Frame::new().fill(t.bg)).show(ui, |ui| {
            if self.explore.is_some() {
                self.explore_page(ui);
            } else {
                self.conversation(ui);
            }
        });

        self.toasts(&ctx);
        self.manual_ui(&ctx);
        self.dialogs(&ctx);
        self.palette_ui(&ctx);

        // While an answer streams, the engine requests repaints; spinners need a steady tick too.
        if !self.state.generating.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.state.settings.theme == "omarchy" {
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
    }
}
