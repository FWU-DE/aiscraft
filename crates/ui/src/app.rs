use std::path::PathBuf;
use std::sync::Arc;

use aiscraft_engine::{Engine, NoticeLevel, State};
use egui::{Align2, CornerRadius, Frame, Id, Key, KeyboardShortcut, Margin, Modifiers, RichText, Stroke};
use serde_json::{Value, json};

use crate::dialogs::Dialog;
use crate::palette::Palette;
use crate::theme::{self, OmarchyTheme, ThemeWatcher, Tokens};

/// A request from the control channel that needs the UI thread.
pub enum UiRequest {
    /// Save a PNG of the window.
    Screenshot { path: PathBuf, reply: std::sync::mpsc::Sender<Result<Value, String>> },
    /// Open a dialog by name: settings, memory, login, add_backend, palette, welcome, look, share
    /// (the open artifact's), join.
    Show { what: String, reply: std::sync::mpsc::Sender<Result<Value, String>> },
}

/// Where the panels sat in the last frame and the widths they start with, in points. Read by the
/// layout quality gate (`tests/layout_gate.rs`): a panel that grows from frame to frame or ends up
/// wider than its default means a row inside it overflows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLayout {
    pub window: egui::Rect,
    /// None while the sidebar is collapsed.
    pub sidebar: Option<egui::Rect>,
    pub sidebar_default: f32,
    /// None while no artifact is open.
    pub artifact: Option<egui::Rect>,
    pub artifact_default: f32,
    pub main: egui::Rect,
    /// A dialog, the handbook or the palette lies over the panels.
    pub overlay: bool,
}

impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            window: egui::Rect::NOTHING,
            sidebar: None,
            sidebar_default: 0.0,
            artifact: None,
            artifact_default: 0.0,
            main: egui::Rect::NOTHING,
            overlay: false,
        }
    }
}

/// View state only; the engine owns the data.
pub struct AisCraftApp {
    control: Option<std::sync::mpsc::Receiver<UiRequest>>,
    pub(crate) engine: Engine,
    pub(crate) state: Arc<State>,
    pub(crate) tokens: Tokens,
    theme_key: (String, bool, String),
    watcher: ThemeWatcher,
    /// Where the Omarchy desktop theme comes from (the desktop's; tests set their own).
    pub(crate) omarchy: OmarchyTheme,
    /// Whether that theme is there: read at start and when the watcher sees `colors.toml`
    /// change, not every frame.
    pub(crate) omarchy_available: bool,
    pub(crate) md: egui_commonmark::CommonMarkCache,
    pub(crate) composer: String,
    /// The next composer drawn is the start page's large one (Modern, Glas).
    pub(crate) composer_big: bool,
    /// How wide the composer must be for all its buttons to fit in one row (measured in the last
    /// frame). Its column grows up to this when the window has room.
    pub(crate) composer_need: f32,
    panel_layout: PanelLayout,
    /// Draw the next "Erstellen" menu as an icon (narrow composer).
    pub(crate) create_as_icon: bool,
    pub(crate) pending_files: Vec<PathBuf>,
    /// Web links for the next message, and the address being typed (`Some` while the field is open).
    pub(crate) pending_links: Vec<String>,
    pub(crate) link_input: Option<String>,
    /// The quick entry last brought to the front (#35).
    pub(crate) quick_seen: u64,
    /// The search field of Strg+F in the open chat (#41), while it is open.
    pub(crate) find: Option<String>,
    pub(crate) focus_find: bool,
    /// The hit last scrolled to (query, index), so the view jumps once per step.
    pub(crate) find_scrolled: Option<(String, usize)>,
    /// The example text being written for a custom answer style (#31).
    pub(crate) style_draft: Option<String>,
    /// „Namen schützen“: the person's choices in the preview for the next message (#55).
    pub(crate) privacy: crate::privacy_panel::Choices,
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
    /// The archive or trash page, when open (replaces the chat column, #77).
    pub(crate) archive: Option<crate::archive_view::ArchiveView>,
    /// Chats picked in the sidebar with Strg or Umschalt (#77), in the order picked.
    pub(crate) selected_chats: Vec<String>,
    /// Where a range picked with Umschalt starts.
    pub(crate) select_anchor: Option<String>,
    /// Persona pictures asked for already, by picture key.
    pub(crate) pictures_requested: std::collections::HashSet<String>,
    /// Persona pictures asked for that have not arrived yet, with egui's input time of the ask.
    pub(crate) pictures_pending: std::collections::HashMap<String, f64>,
    /// Small persona textures by file (`None`: could not be read).
    pub(crate) persona_textures: std::collections::HashMap<String, Option<egui::TextureHandle>>,
    /// Pictures still to decode this frame, so a full grid never stalls one frame.
    pub(crate) decode_budget: usize,
    last_current: Option<String>,
    pub(crate) artifact: Option<crate::artifact_panel::ArtifactView>,
    /// Contents in their own window (#75): their views and what the windows did.
    pub(crate) windows: crate::windows::Windows,
    /// The window being drawn right now (`None`: the panel next to the chat).
    pub(crate) in_window: Option<String>,
    /// The artifact („Im Chat“, in the panel or its own window) sent as context with the next message.
    pub(crate) attach_artifact: Option<String>,
    /// Settings › Dienste › Zusammenarbeit: relay address and key being typed.
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
    /// When each toast was first shown (egui's input time, seconds); plain information goes away
    /// on its own after a while.
    toast_seen: std::collections::HashMap<u64, f64>,
    /// The window width at which the sidebar was last folded or unfolded automatically.
    narrow: Option<bool>,
}

/// Information and success toasts disappear after this long (two and a half times as long with a
/// button; not while the pointer rests on them); warnings and errors stay until closed.
const TOAST_SECONDS: f32 = 6.0;
/// Below this window width the sidebar folds away by itself (Strg+B brings it back).
const NARROW_WINDOW: f32 = 1000.0;

pub(crate) const CONTENT_WIDTH: f32 = 760.0;

/// The frames of sidebar, artifact panel and chat area for a look: flush panels (Modern,
/// Omarchy) or floating glass surfaces 12 px apart with radius 24 (Glas).
struct PanelFrames {
    sidebar: Frame,
    sidebar_width: f32,
    panel: Frame,
    main: Frame,
}

/// Gap between Glas' floating surfaces and to the window edge.
pub(crate) const GLASS_GAP: i8 = 12;

impl PanelFrames {
    fn new(t: &Tokens) -> Self {
        let g = GLASS_GAP;
        let side_margin = Margin { left: 14, right: 14, top: 18, bottom: 14 };
        match t.look {
            theme::Look::Glas => {
                let r = CornerRadius::same(t.r_panel);
                let shade = |a: u8| egui::epaint::Shadow {
                    offset: [0, 14],
                    blur: 36,
                    spread: 0,
                    color: if t.dark {
                        egui::Color32::from_black_alpha(a.saturating_mul(2))
                    } else {
                        egui::Color32::from_rgba_unmultiplied(0x1b, 0x3a, 0x5a, a)
                    },
                };
                Self {
                    sidebar: Frame::new()
                        .fill(t.side_top)
                        .stroke(Stroke::new(1.0, t.side_stroke))
                        .corner_radius(r)
                        .outer_margin(Margin { left: g, right: 0, top: g, bottom: g })
                        .inner_margin(side_margin)
                        .shadow(shade(0x14)),
                    sidebar_width: 252.0 + f32::from(g),
                    panel: Frame::new()
                        .fill(t.panel)
                        .stroke(Stroke::new(1.0, t.panel_stroke))
                        .corner_radius(r)
                        .outer_margin(Margin { left: 0, right: g, top: g, bottom: g })
                        .shadow(shade(0x1f)),
                    main: Frame::new().fill(t.main).stroke(Stroke::new(1.0, t.panel_stroke)).corner_radius(r).outer_margin(Margin::same(g)).shadow(shade(0x1a)),
                }
            }
            theme::Look::Modern | theme::Look::Omarchy => Self {
                // The gradient is painted by the sidebar itself (egui frames fill flat).
                sidebar: Frame::new().inner_margin(side_margin),
                sidebar_width: 268.0,
                panel: Frame::new().fill(t.panel).inner_margin(Margin::ZERO),
                main: Frame::new().fill(t.main),
            },
        }
    }
}

impl AisCraftApp {
    /// The panels of the last frame (see [`PanelLayout`]).
    pub fn panel_layout(&self) -> PanelLayout {
        self.panel_layout
    }

    /// A content's own window drawn as the whole screen ([`Self::show_as_window`]): the window
    /// is the panel, and the panel must stay as wide as the window.
    pub(crate) fn set_window_layout(&mut self, window: egui::Rect, content: egui::Rect) {
        self.panel_layout = PanelLayout {
            window,
            sidebar: None,
            sidebar_default: 0.0,
            artifact: Some(content),
            artifact_default: window.width(),
            main: content,
            overlay: self.dialog.is_some() || self.palette.is_some(),
        };
    }

    pub fn new(cc: &eframe::CreationContext<'_>, engine: Engine, control: Option<std::sync::mpsc::Receiver<UiRequest>>) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        engine.on_change(move || ctx.request_repaint());
        let state = engine.state();
        let system_dark = cc.egui_ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark);
        let omarchy = OmarchyTheme::desktop();
        let tokens = theme::resolve(&state.settings.theme, system_dark, state.settings.transparency == "off", &omarchy);
        tokens.apply(&cc.egui_ctx, state.settings.zoom);
        let first_run = state.backends.iter().all(|b| !b.has_secret) && state.backends.len() == 1;
        // A fresh profile gets the whole welcome (look, then sign-in); people who are set up
        // already see only the look question, once.
        let welcome = if first_run {
            Some(Dialog::Welcome { look_only: false })
        } else if !state.settings.look_chosen {
            // The question starts on Modern like the system's scheme; the old Klassisch values
            // ("system", "light", "dark") are moved there. Omarchy stays, a deliberate choice.
            if matches!(state.settings.theme.as_str(), "system" | "light" | "dark") || state.settings.theme.starts_with("classic") {
                engine.dispatch("settings.set", json!({"key": "theme", "value": theme::setting(theme::Look::Modern, theme::Mode::System)}));
            }
            Some(Dialog::Welcome { look_only: true })
        } else {
            None
        };
        let mut app = Self {
            control,
            engine,
            theme_key: (state.settings.theme.clone(), system_dark, state.settings.transparency.clone()),
            state,
            tokens,
            watcher: ThemeWatcher::new(omarchy.clone()),
            omarchy_available: omarchy.available(),
            omarchy,
            md: Default::default(),
            composer: String::new(),
            composer_big: false,
            composer_need: 0.0,
            panel_layout: PanelLayout::default(),
            create_as_icon: false,
            pending_files: Vec::new(),
            pending_links: Vec::new(),
            link_input: None,
            quick_seen: 0,
            find: None,
            focus_find: false,
            find_scrolled: None,
            style_draft: None,
            privacy: Default::default(),
            sidebar_open: true,
            search_text: String::new(),
            project_filter: None,
            editing: None,
            dialog: welcome,
            palette: None,
            focus_composer: true,
            focus_search: false,
            remote_open: false,
            personas_requested: Default::default(),
            explore: None,
            archive: None,
            selected_chats: Vec::new(),
            select_anchor: None,
            pictures_requested: Default::default(),
            pictures_pending: Default::default(),
            persona_textures: Default::default(),
            decode_budget: 0,
            last_current: None,
            artifact: None,
            windows: Default::default(),
            in_window: None,
            attach_artifact: None,
            collab_url: String::new(),
            collab_key: String::new(),
            dictated: Default::default(),
            manual: None,
            images: Default::default(),
            home: Default::default(),
            toast_seen: Default::default(),
            narrow: None,
        };
        app.register_code_theme();
        app
    }

    /// Read the Omarchy theme from `omarchy` instead of the desktop's (tests, offscreen pictures).
    pub fn with_omarchy_theme(mut self, omarchy: OmarchyTheme) -> Self {
        self.watcher = ThemeWatcher::new(omarchy.clone());
        self.omarchy_available = omarchy.available();
        self.tokens = theme::resolve(&self.theme_key.0, self.theme_key.1, self.theme_key.2 == "off", &omarchy);
        self.omarchy = omarchy;
        self.register_code_theme();
        self
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

    /// Modern and Glas draw code on their own background: load the matching syntax theme.
    fn register_code_theme(&mut self) {
        if let Some(xml) = self.tokens.code_theme_xml() {
            let name = self.tokens.code_theme();
            if let Err(e) = self.md.add_syntax_theme_from_bytes(name, xml.as_bytes()) {
                log::warn!("Farbschema für Code: {e}");
            }
        }
    }

    fn sync_theme(&mut self, ctx: &egui::Context) {
        let system_dark = ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark);
        let key = (self.state.settings.theme.clone(), system_dark, self.state.settings.transparency.clone());
        let changed = self.watcher.changed(ctx.input(|i| i.time));
        if changed {
            self.omarchy_available = self.omarchy.available();
        }
        let follow = key.0 == "omarchy" && changed;
        if key != self.theme_key || follow {
            self.tokens = theme::resolve(&key.0, system_dark, key.2 == "off", &self.omarchy);
            self.register_code_theme();
            self.theme_key = key;
            // Open connector views (#74) follow the look.
            let (theme, styles) = crate::views::look(&self.tokens);
            self.run("view.context", json!({"theme": theme, "styles": styles}));
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
        if self.archive_keys(ctx) || self.explore_keys(ctx) {
            return;
        }
        if pressed(sc(Modifiers::COMMAND, Key::N)) {
            self.new_chat();
        }
        // Strg+F: find in the open chat (#41); Esc closes the search first.
        if self.state.current.is_some() && pressed(sc(Modifiers::COMMAND, Key::F)) {
            if self.find.is_none() {
                self.find = Some(self.state.find.as_ref().map(|f| f.query.clone()).unwrap_or_default());
            }
            self.focus_find = true;
        }
        if self.find.is_some() && pressed(sc(Modifiers::NONE, Key::Escape)) {
            self.close_find();
            return;
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

    pub(crate) fn close_find(&mut self) {
        self.find = None;
        self.find_scrolled = None;
        self.run("chat.find", json!({"close": true}));
        self.focus_composer = true;
    }

    /// A quick entry (#35, `aiscraft --quick`): the window to the front, a new chat, the composer
    /// focused, a screenshot waiting in it, a draft typed before kept.
    fn quick_entry(&mut self, ctx: &egui::Context) {
        let Some(q) = self.state.quick.clone() else { return };
        if q.seq == self.quick_seen {
            return;
        }
        self.quick_seen = q.seq;
        self.explore = None;
        self.manual = None;
        // A draft typed before stays: the quick entry only adds the screenshot.
        self.take_files(q.files.iter().map(PathBuf::from).collect());
        self.focus_composer = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(crate) fn new_chat(&mut self) {
        self.explore = None;
        self.archive = None;
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
        self.archive = None;
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
        self.toast_seen.retain(|id, _| notices.iter().any(|n| n.id == *id));
        if notices.is_empty() {
            return;
        }
        // egui's clock, not the wall clock: the test harness advances it per frame, so a toast
        // stays for the same frames however busy the machine is (#71).
        let now = ctx.input(|i| i.time);
        let area =
            egui::Area::new(Id::unique("toasts")).anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                ui.set_max_width(360.0);
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
                            ui.set_width(320.0);
                            ui.horizontal_top(|ui| {
                                ui.vertical(|ui| {
                                    ui.add_space(5.0);
                                    crate::widgets::status_dot(ui, color);
                                });
                                ui.add(egui::Label::new(RichText::new(crate::widgets::plain(&n.text))).wrap());
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
        // Plain information fades out by itself, unless the pointer rests on the toasts.
        let resting = area.response.contains_pointer();
        for n in &notices {
            let seen = self.toast_seen.entry(n.id).or_insert(now);
            if resting {
                *seen = now;
            }
            // With a button ("Verlauf laden") it stays longer, so there is time to use it.
            let after = if n.action.is_some() { TOAST_SECONDS * 2.5 } else { TOAST_SECONDS };
            let transient = matches!(n.level, NoticeLevel::Info | NoticeLevel::Success);
            if transient && now - *seen >= f64::from(after) {
                // Asked once; the engine removes it with its next state.
                *seen = now + 3600.0;
                self.run("notice.dismiss", json!({"id": n.id}));
            }
        }
        if notices.iter().any(|n| matches!(n.level, NoticeLevel::Info | NoticeLevel::Success)) {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Files dropped or picked for the composer: `.h5p` packages open as H5P artifacts,
    /// everything else is attached to the next message.
    pub(crate) fn take_files(&mut self, files: Vec<PathBuf>) {
        for f in files {
            if f.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("h5p")) {
                self.run("h5p.open", json!({"path": f.display().to_string()}));
            } else {
                self.pending_files.push(f);
                self.focus_composer = true;
            }
        }
    }

    fn dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            self.take_files(dropped);
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
                        "usage" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Usage, new_note: String::new() });
                            true
                        }
                        "connectors" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Connectors, new_note: String::new() });
                            true
                        }
                        "privacy" => {
                            self.dialog = Some(Dialog::Settings { tab: crate::dialogs::SettingsTab::Privacy, new_note: String::new() });
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
                            self.dialog = Some(Dialog::Welcome { look_only: false });
                            true
                        }
                        "look" => {
                            self.dialog = Some(Dialog::Welcome { look_only: true });
                            true
                        }
                        "manual" => {
                            self.manual = Some(Default::default());
                            true
                        }
                        "import_chats" => {
                            self.dialog = Some(Dialog::ImportChats);
                            true
                        }
                        "palette" => {
                            self.palette = Some(Palette::default());
                            true
                        }
                        // The share dialog of the open artifact (#60).
                        "share" => match self.state.open_artifact.clone() {
                            Some(id) => {
                                self.dialog = Some(Dialog::Share { id, way: None });
                                true
                            }
                            None => false,
                        },
                        "join" => {
                            self.dialog = Some(Dialog::Join { code: String::new() });
                            true
                        }
                        "personas" => {
                            self.open_explore();
                            true
                        }
                        "archive" | "trash" => {
                            self.open_archive(what == "trash");
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
                        // "research", "connector_insert", "template:<id>", "template_new", "project:<id>" (#32, #37, #53, #54).
                        w if self.show_know(w) => true,
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
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.state = self.engine.state();
        if let Some(id) = self.windows.as_window.clone() {
            // A content's own window as the whole screen (tests; see `show_as_window`).
            self.sync_theme(&ctx);
            self.sync_windows();
            self.window_frame(ui, &id);
            self.dialogs(&ctx);
            return;
        }
        if self.state.current != self.last_current {
            self.last_current = self.state.current.clone();
            self.editing = None;
            self.focus_composer = true;
            self.explore = None;
            self.archive = None;
        }
        self.control_requests(&ctx);
        self.quick_entry(&ctx);
        self.know_frame(&ctx);
        self.sync_theme(&ctx);
        self.shortcuts(&ctx);
        self.dropped_files(&ctx);

        let t = self.tokens.clone();
        // The panel needs its own copy to animate; a click inside the sidebar (its collapse
        // button) changes `self.sidebar_open` meanwhile and must win over that copy.
        // A narrow window folds the sidebar away, a wide one brings it back; only when the width
        // crosses the line, so Strg+B still wins in between.
        let narrow = ctx.content_rect().width() < NARROW_WINDOW;
        if self.narrow != Some(narrow) {
            if self.narrow.is_some() || narrow {
                self.sidebar_open = !narrow;
            }
            self.narrow = Some(narrow);
        }
        // Glas: the soft backdrop behind everything; the panels float on it.
        if let Some(g) = &t.glass {
            let painter = ctx.layer_painter(egui::LayerId::background());
            crate::backdrop::paint(&painter, ctx.content_rect(), g, 1.0);
        }
        let frames = PanelFrames::new(&t);
        let mut layout = PanelLayout { window: ctx.content_rect(), sidebar_default: frames.sidebar_width, ..Default::default() };
        let before = self.sidebar_open;
        let mut sidebar_open = before;
        layout.sidebar = egui::Panel::left("sidebar")
            .resizable(true)
            .show_separator_line(false)
            .default_size(frames.sidebar_width)
            .size_range(200.0..=420.0)
            .frame(frames.sidebar)
            .show_collapsible(ui, &mut sidebar_open, |ui| self.sidebar(ui))
            .map(|r| r.response.rect);
        if self.sidebar_open == before {
            self.sidebar_open = sidebar_open;
        }

        self.sync_artifact();
        self.sync_windows();
        if let Some(open) = self.panel_artifact() {
            let w = ui.available_width();
            layout.artifact_default = (w * 0.46).clamp(380.0, 760.0);
            let panel = egui::Panel::right("artifact")
                .resizable(true)
                .show_separator_line(false)
                .default_size(layout.artifact_default)
                .size_range(320.0..=(w - 360.0).max(340.0))
                .frame(frames.panel)
                .show(ui, |ui| {
                    if t.flush_modern() {
                        // A soft shadow cast to the left onto the chat, and the hairline.
                        let r = ui.max_rect();
                        let shade =
                            egui::epaint::Shadow { offset: [-8, 0], blur: 24, spread: 0, color: egui::Color32::from_rgba_unmultiplied(0x0b, 0x2a, 0x45, 0x14) };
                        ui.ctx()
                            .layer_painter(egui::LayerId::new(egui::Order::Background, Id::unique("panel-shadow")))
                            .add(shade.as_shape(r, CornerRadius::ZERO));
                        ui.painter().line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.0, t.panel_stroke));
                    }
                    if let Some(g) = &t.glass {
                        crate::backdrop::glass_edge(ui.painter(), ui.max_rect(), f32::from(t.r_panel), g.edge);
                    }
                    self.artifact_view(ui, &open)
                });
            layout.artifact = Some(panel.response.rect);
        }

        layout.main = egui::CentralPanel::default()
            .frame(frames.main)
            .show(ui, |ui| {
                if let Some(g) = &t.glass {
                    crate::backdrop::glass_edge(ui.painter(), ui.max_rect(), f32::from(t.r_panel), g.edge);
                }
                if self.archive.is_some() {
                    self.archive_page(ui);
                } else if self.explore.is_some() {
                    self.explore_page(ui);
                } else {
                    self.conversation(ui);
                }
            })
            .response
            .rect;

        self.toasts(&ctx);
        self.manual_ui(&ctx);
        self.citation_ui(&ctx);
        self.dialogs(&ctx);
        self.palette_ui(&ctx);
        layout.overlay = self.dialog.is_some() || self.manual.is_some() || self.palette.is_some() || self.state.citation.is_some();
        self.panel_layout = layout;
        self.artifact_windows(&ctx, frame);

        // While an answer streams, the engine requests repaints; spinners need a steady tick too.
        if !self.state.generating.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.state.settings.theme == "omarchy" {
            ctx.request_repaint_after(std::time::Duration::from_secs(2));
        }
    }
}
