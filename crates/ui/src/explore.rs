//! "Entdecken": assistants, characters and learning scenarios as a page with search, tabs,
//! filters and a card grid, and a detail view per entry. Replaces the chat column while open.

use aiscraft_core::{BackendKind, PersonaInfo, PersonaKind};
use aiscraft_engine::{BackendStatus, picture_key};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, Rect, RichText, ScrollArea, Sense, Stroke, Ui, vec2};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::Dialog;
use crate::theme::Tokens;
use crate::widgets::{accent_button, card, muted, small_muted};

const PAGE_WIDTH: f32 = 1180.0;
const MIN_CARD: f32 = 210.0;
const GAP: f32 = 14.0;
/// Cards this close below or above the visible area ask for their picture already, so it is
/// there when they scroll in (a little more than one row of cards).
const PICTURE_LOOKAHEAD: f32 = 320.0;
/// At most this many pictures asked for and not yet arrived; cards past it ask in a later frame,
/// so a fast scroll does not queue the pictures of every card it passed.
const PICTURES_IN_FLIGHT: usize = 6;
/// Cards ahead of the visible area use at most this many of them; visible cards keep the rest.
const PICTURES_AHEAD: usize = 2;
/// A request that never answers frees its place after this many seconds.
const PICTURE_PATIENCE: f64 = 30.0;

/// Entries of one kind (or all), as the grid lists them.
type Section<'a> = (Option<PersonaKind>, Vec<&'a (String, PersonaInfo)>);

/// What the page shows; pure view state.
#[derive(Debug, Default)]
pub(crate) struct ExploreView {
    pub query: String,
    /// `None` = all kinds.
    pub tab: Option<PersonaKind>,
    pub favorites: bool,
    pub own: bool,
    pub detail: Option<Pick>,
    pub focus_search: bool,
}

/// One entry, by backend, kind and id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pick {
    pub backend: String,
    pub kind: PersonaKind,
    pub id: String,
}

/// Lower case without umlauts and accents, so "ubung", "uebung" and "Übung" find each other.
pub(crate) fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' | 'à' | 'á' | 'â' | 'ã' | 'å' => out.push('a'),
            'ö' | 'ò' | 'ó' | 'ô' | 'õ' | 'ø' => out.push('o'),
            'ü' | 'ù' | 'ú' | 'û' => out.push('u'),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'ç' => out.push('c'),
            'ñ' => out.push('n'),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out.replace("ae", "a").replace("oe", "o").replace("ue", "u")
}

/// Does every word of the query occur in name or description?
pub(crate) fn matches_query(p: &PersonaInfo, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let hay = fold(&format!("{} {}", p.name, p.description.as_deref().unwrap_or("")));
    terms.iter().all(|t| hay.contains(t.as_str()))
}

pub(crate) fn query_terms(q: &str) -> Vec<String> {
    fold(q).split_whitespace().map(str::to_string).collect()
}

/// "Johann Wolfgang von Goethe" → "JG"; "Mathe-Coach" → "MC".
pub(crate) fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| c.is_whitespace() || c == '-').filter(|w| w.chars().next().is_some_and(char::is_alphanumeric)).collect();
    let pick: Vec<&str> = match words.as_slice() {
        [] => return "?".into(),
        [one] => vec![one],
        [first, .., last] => {
            // Skip small connecting words for the last letter ("von", "de").
            let last = words.get(1..).unwrap_or_default().iter().rev().find(|w| w.chars().next().is_some_and(char::is_uppercase)).unwrap_or(last);
            vec![first, last]
        }
    };
    pick.iter().filter_map(|w| w.chars().next()).flat_map(char::to_uppercase).collect()
}

fn avatar_colors(id: &str, dark: bool) -> (Color32, Color32) {
    let h = id.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (h % 360) as f32 / 360.0;
    let bg = egui::ecolor::Hsva::new(hue, if dark { 0.45 } else { 0.32 }, if dark { 0.42 } else { 0.93 }, 1.0);
    let fg = egui::ecolor::Hsva::new(hue, if dark { 0.15 } else { 0.7 }, if dark { 0.97 } else { 0.42 }, 1.0);
    (Color32::from(bg), Color32::from(fg))
}

/// The part of a texture that fills `target` (like CSS `object-fit: cover`), biased to the top,
/// where faces usually are.
fn cover_uv(tex: egui::Vec2, target: egui::Vec2) -> Rect {
    if tex.x <= 0.0 || tex.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let scale = (target.x / tex.x).max(target.y / tex.y);
    let (uw, vh) = ((target.x / scale / tex.x).min(1.0), (target.y / scale / tex.y).min(1.0));
    let u0 = (1.0 - uw) / 2.0;
    let v0 = (1.0 - vh) * 0.3;
    Rect::from_min_max(egui::pos2(u0, v0), egui::pos2(u0 + uw, v0 + vh))
}

fn clipped(text: &str, size: f32, color: Color32, width: f32, rows: usize) -> LayoutJob {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: FontId::proportional(size), color, ..Default::default() });
    job.wrap = TextWrapping { max_width: width.max(10.0), max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    job
}

fn pill(ui: &Ui, t: &Tokens, pos: egui::Pos2, text: &str, fill: Color32, color: Color32) -> f32 {
    let galley = ui.painter().layout_no_wrap(text.to_string(), FontId::proportional(11.5), color);
    let rect = Rect::from_min_size(pos, galley.size() + vec2(14.0, 4.0));
    ui.painter().rect_filled(rect, CornerRadius::same(t.radius), fill);
    ui.painter().galley(pos + vec2(7.0, 2.0), galley, color);
    rect.width()
}

/// A tab or filter chip: a rounded, selectable button; the count after the label is dimmed.
fn chip(ui: &mut Ui, t: &Tokens, selected: bool, text: impl Into<String>, count: Option<usize>) -> egui::Response {
    let fg = if selected { t.on_accent } else { t.text };
    let mut job = LayoutJob::default();
    let font = FontId::proportional(14.0);
    job.append(&text.into(), 0.0, TextFormat { font_id: font.clone(), color: fg, ..Default::default() });
    if let Some(n) = count {
        let dim = if selected { t.on_accent.gamma_multiply(0.75) } else { t.muted };
        job.append(&n.to_string(), 6.0, TextFormat { font_id: FontId::proportional(12.5), color: dim, valign: Align::Center, ..Default::default() });
    }
    ui.add(
        egui::Button::new(job)
            .fill(if selected { t.accent } else { t.surface })
            .stroke(Stroke::new(1.0, if selected { t.accent } else { t.border }))
            .corner_radius(CornerRadius::same(14))
            .min_size(vec2(0.0, 28.0)),
    )
}

impl AisCraftApp {
    pub(crate) fn open_explore(&mut self) {
        self.archive = None;
        if self.explore.is_none() {
            self.explore = Some(ExploreView { focus_search: true, ..Default::default() });
        }
        self.prefetch_personas(true);
    }

    /// AIS.chat backends that are signed in.
    fn persona_backends(&self) -> Vec<aiscraft_engine::BackendView> {
        self.state.backends.iter().filter(|b| b.config.kind == BackendKind::AisChat && matches!(b.status, BackendStatus::Ready { .. })).cloned().collect()
    }

    /// Load the lists once per backend and run (quietly: the page shows errors itself). The list
    /// remembered from the last run shows meanwhile.
    pub(crate) fn prefetch_personas(&mut self, _visible: bool) {
        for b in self.persona_backends() {
            if b.personas_error.is_none() && self.personas_requested.insert(b.config.id.clone()) {
                self.engine.dispatch_quiet("persona.refresh", json!({"backend": b.config.id}));
            }
        }
    }

    /// Ask the engine for a persona's picture: once per run, and only while fewer than
    /// [`PICTURES_IN_FLIGHT`] asks are open ([`PICTURES_AHEAD`] for a card not yet visible). A
    /// card that finds no place asks again when it is drawn next.
    fn request_persona_picture(&mut self, ctx: &egui::Context, backend: &str, p: &PersonaInfo, ahead: bool) {
        if p.picture_url.is_none() {
            return;
        }
        let key = picture_key(backend, p.kind, &p.id);
        if self.pictures_requested.contains(&key) || self.state.persona_pictures.contains_key(&key) {
            return;
        }
        let now = ctx.input(|i| i.time);
        let arrived = &self.state.persona_pictures;
        self.pictures_pending.retain(|k, since| !arrived.contains_key(k) && now - *since < PICTURE_PATIENCE);
        let cap = if ahead { PICTURES_AHEAD } else { PICTURES_IN_FLIGHT };
        if self.pictures_pending.len() >= cap {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
            return;
        }
        self.pictures_pending.insert(key.clone(), now);
        self.pictures_requested.insert(key);
        self.engine.dispatch_quiet("persona.picture", json!({"backend": backend, "kind": p.kind.as_str(), "id": p.id}));
    }

    /// A persona's picture as a small texture (decoded once, a few per frame), fetched on first use.
    fn persona_thumb(&mut self, ctx: &egui::Context, backend: &str, p: &PersonaInfo) -> Option<egui::TextureHandle> {
        p.picture_url.as_ref()?;
        let key = picture_key(backend, p.kind, &p.id);
        let path = match self.state.persona_pictures.get(&key) {
            None => {
                self.request_persona_picture(ctx, backend, p, false);
                return None;
            }
            Some(None) => return None,
            Some(Some(path)) => path.clone(),
        };
        if let Some(t) = self.persona_textures.get(&path) {
            return t.clone();
        }
        if self.decode_budget == 0 {
            ctx.request_repaint();
            return None;
        }
        self.decode_budget -= 1;
        let tex = std::fs::metadata(&path)
            .ok()
            .filter(|m| m.len() <= 4 * 1024 * 1024)
            .and_then(|_| std::fs::read(&path).ok())
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .filter(|img| img.width() <= 8192 && img.height() <= 8192)
            .map(|img| {
                let rgba = img.thumbnail(360, 360).to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                ctx.load_texture(format!("persona:{path}"), egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()), egui::TextureOptions::LINEAR)
            });
        if self.persona_textures.len() > 240 {
            self.persona_textures.clear();
        }
        self.persona_textures.insert(path, tex.clone());
        tex
    }

    /// The picture, or a coloured tile with initials.
    pub(crate) fn paint_persona_picture(&mut self, ui: &mut Ui, rect: Rect, backend: &str, p: &PersonaInfo, radius: u8, large: bool) {
        let tex = if large {
            let key = picture_key(backend, p.kind, &p.id);
            match self.state.persona_pictures.get(&key).cloned().flatten() {
                Some(path) => self.image_texture(ui.ctx(), &path),
                None => self.persona_thumb(ui.ctx(), backend, p),
            }
        } else {
            self.persona_thumb(ui.ctx(), backend, p)
        };
        match tex {
            Some(tex) => {
                egui::Image::new(&tex).uv(cover_uv(tex.size_vec2(), rect.size())).corner_radius(CornerRadius::same(radius)).paint_at(ui, rect);
            }
            None => {
                let (bg, fg) = avatar_colors(&p.id, self.tokens.dark);
                ui.painter().rect_filled(rect, CornerRadius::same(radius), bg);
                let size = (rect.width().min(rect.height()) * 0.36).clamp(12.0, 64.0);
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, initials(&p.name), FontId::proportional(size), fg);
            }
        }
    }

    /// A card; the second value is true when its star was clicked (toggle the favourite).
    fn persona_card(&mut self, ui: &mut Ui, backend: &str, p: &PersonaInfo, w: f32, compact: bool) -> (egui::Response, bool) {
        let t = self.tokens.clone();
        let pic_h = if compact { 72.0 } else { (w * 0.52).clamp(96.0, 150.0) };
        let h = if compact { 100.0 } else { pic_h + 126.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &p.name));
        if !ui.is_rect_visible(rect) {
            // Not drawn; just ahead of the visible area it asks for its picture already.
            if ui.clip_rect().expand2(vec2(0.0, PICTURE_LOOKAHEAD)).intersects(rect) {
                self.request_persona_picture(ui.ctx(), backend, p, true);
            }
            return (resp, false);
        }
        // The star sits on the picture's top right corner and answers before the card does.
        let star_center = if compact { rect.min + vec2(12.0 + pic_h - 12.0, 24.0) } else { rect.min + vec2(w - 8.0 - 14.0, 8.0 + 14.0) };
        let star = ui.interact(Rect::from_center_size(star_center, vec2(24.0, 24.0)), resp.id.with("fav"), Sense::click());
        let star_label = if p.favorite { "Favorit entfernen" } else { "Als Favorit markieren" };
        star.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, star_label));
        let hovered = resp.hovered() || resp.has_focus() || star.hovered();
        ui.painter().rect(
            rect,
            CornerRadius::same(t.radius),
            if hovered { t.surface_hi } else { t.surface },
            Stroke::new(1.0, if hovered { t.accent } else { t.border }),
            egui::StrokeKind::Inside,
        );
        let inner = t.radius.saturating_sub(3);
        let (pic, text_left, text_top, text_w) = if compact {
            let pic = Rect::from_min_size(rect.min + vec2(12.0, 12.0), vec2(pic_h, pic_h));
            (pic, pic.right() + 12.0, rect.top() + 12.0, rect.right() - pic.right() - 24.0)
        } else {
            let pic = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(w - 16.0, pic_h));
            (pic, rect.left() + 14.0, pic.bottom() + 10.0, w - 28.0)
        };
        self.paint_persona_picture(ui, pic, backend, p, inner, false);
        let gold = Color32::from_rgb(0xff, 0xc8, 0x3d);
        if p.favorite || hovered {
            let c = star_center;
            ui.painter().circle_filled(c, 11.0, Color32::from_black_alpha(if star.hovered() { 200 } else { 140 }));
            let (glyph, color) = if p.favorite { ("★", gold) } else { ("☆", if star.hovered() { gold } else { Color32::WHITE }) };
            ui.painter().text(c + vec2(0.0, 0.5), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color);
        }
        let mut y = text_top;
        let name = ui.painter().layout_job(clipped(&p.name, 15.0, t.text, text_w, 1));
        let name_h = name.size().y;
        ui.painter().galley(egui::pos2(text_left, y), name, t.text);
        y += name_h + 5.0;
        let mut x = text_left;
        x += pill(ui, &t, egui::pos2(x, y), p.kind.label(), t.selection, t.text) + 6.0;
        if p.own {
            pill(ui, &t, egui::pos2(x, y), "von mir", t.surface_hi, t.muted);
        }
        y += 23.0;
        if let Some(d) = p.description.as_deref().or(p.exercise.as_deref()) {
            let rows = if compact { 2 } else { 3 };
            let galley = ui.painter().layout_job(clipped(d, 13.0, t.muted, text_w, rows));
            ui.painter().with_clip_rect(rect.shrink(1.0)).galley(egui::pos2(text_left, y), galley, t.muted);
        }
        let toggled = star.on_hover_text(star_label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        (resp.on_hover_cursor(egui::CursorIcon::PointingHand), toggled)
    }

    /// Mark or unmark a favourite in AIS.chat.
    pub(crate) fn toggle_favorite(&self, backend: &str, kind: PersonaKind, id: &str, favorite: bool) {
        self.run("persona.favorite", json!({"backend": backend, "kind": kind.as_str(), "id": id, "favorite": favorite}));
    }

    /// The entries the page lists, with their backend: favourites first, then own, then by name.
    fn explore_entries(&self) -> Vec<(String, PersonaInfo)> {
        let mut out: Vec<(String, PersonaInfo)> =
            self.persona_backends().iter().flat_map(|b| b.personas.iter().flatten().map(|p| (b.config.id.clone(), p.clone()))).collect();
        out.sort_by(|a, b| b.1.favorite.cmp(&a.1.favorite).then(b.1.own.cmp(&a.1.own)).then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())));
        out
    }

    fn explore_find(&self, pick: &Pick) -> Option<PersonaInfo> {
        self.state.backend(&pick.backend)?.personas.as_ref()?.iter().find(|p| p.kind == pick.kind && p.id == pick.id).cloned()
    }

    pub(crate) fn start_persona(&mut self, backend: &str, kind: PersonaKind, id: &str, text: Option<String>) {
        let mut params = json!({"backend": backend, "kind": kind.as_str(), "id": id});
        if let (Some(text), Some(o)) = (text, params.as_object_mut()) {
            o.insert("text".into(), json!(text));
        }
        self.run("persona.start", params);
        self.explore = None;
        self.composer.clear();
        self.focus_composer = true;
    }

    /// Esc, Strg+F and Enter while the page is open. Returns true when Esc was used.
    pub(crate) fn explore_keys(&mut self, ctx: &egui::Context) -> bool {
        let Some(view) = self.explore.as_mut() else { return false };
        if ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::F))) {
            view.detail = None;
            view.focus_search = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            if view.detail.is_some() {
                view.detail = None;
            } else {
                self.explore = None;
                self.focus_composer = true;
            }
            return true;
        }
        let nothing_focused = ctx.memory(|m| m.focused().is_none());
        if nothing_focused
            && let Some(pick) = view.detail.clone()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter))
        {
            self.start_persona(&pick.backend, pick.kind, &pick.id, None);
        }
        false
    }

    pub(crate) fn explore_page(&mut self, ui: &mut Ui) {
        self.decode_budget = 6;
        self.prefetch_personas(true);
        let detail = self.explore.as_ref().and_then(|v| v.detail.clone());
        match detail {
            Some(pick) => self.explore_detail(ui, pick),
            None => self.explore_grid(ui),
        }
    }

    fn explore_grid(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let Some(mut view) = self.explore.take() else { return };
        let backends = self.persona_backends();
        let entries = self.explore_entries();
        let terms = query_terms(&view.query);
        let filtered: Vec<&(String, PersonaInfo)> =
            entries.iter().filter(|(_, p)| matches_query(p, &terms) && (!view.favorites || p.favorite) && (!view.own || p.own)).collect();
        let count = |k: Option<PersonaKind>| filtered.iter().filter(|(_, p)| k.is_none_or(|k| p.kind == k)).count();
        let shown: Vec<&(String, PersonaInfo)> = filtered.iter().copied().filter(|(_, p)| view.tab.is_none_or(|k| p.kind == k)).collect();
        let mut close = false;
        let mut refresh = false;
        let mut open: Option<Pick> = None;

        egui::Panel::top("explore-head").frame(Frame::new().fill(t.bg).inner_margin(Margin { left: 20, right: 20, top: 12, bottom: 10 })).show(ui, |ui| {
            page_column(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(muted(&t, "‹  Zurück zum Chat")).frame(false)).on_hover_text("Esc").clicked() {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !backends.is_empty() && ui.add(egui::Button::new(small_muted(&t, "Neu laden")).frame(false)).clicked() {
                            refresh = true;
                        }
                    });
                });
                ui.add_space(2.0);
                ui.label(RichText::new("Assistenten, Dialogpartner & Lernszenarien").size(24.0).strong());
                ui.label(muted(&t, "Aus AIS.chat: eigene, an deiner Schule geteilte und offizielle."));
                ui.add_space(10.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut view.query)
                        .hint_text("🔍  Name oder Beschreibung suchen")
                        .desired_width(f32::INFINITY)
                        .margin(Margin::symmetric(12, 8)),
                );
                let search = search.on_hover_text("Strg+F");
                if std::mem::take(&mut view.focus_search) {
                    search.request_focus();
                }
                if search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && shown.len() == 1 {
                    open = shown.first().map(|(b, p)| Pick { backend: b.clone(), kind: p.kind, id: p.id.clone() });
                }
                ui.add_space(10.0);
                // One row that scrolls sideways in a narrow window instead of leaving a chip alone below.
                egui::ScrollArea::horizontal().id_salt("explore-chips").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(
                    ui,
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                            if chip(ui, &t, view.tab.is_none(), "Alle", Some(count(None))).clicked() {
                                view.tab = None;
                            }
                            for k in PersonaKind::ALL {
                                if chip(ui, &t, view.tab == Some(k), k.plural(), Some(count(Some(k)))).clicked() {
                                    view.tab = if view.tab == Some(k) { None } else { Some(k) };
                                }
                            }
                            ui.add_space(4.0);
                            ui.separator();
                            ui.add_space(4.0);
                            if chip(ui, &t, view.favorites, "★ Favoriten", None).on_hover_text("Nur deine Favoriten (der Stern auf einer Karte)").clicked() {
                                view.favorites = !view.favorites;
                            }
                            if chip(ui, &t, view.own, "Meine", None).on_hover_text("Nur die, die du selbst angelegt hast").clicked() {
                                view.own = !view.own;
                            }
                        });
                    },
                );
            });
        });

        ScrollArea::vertical().id_salt("explore-grid").auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 20, right: 20, top: 6, bottom: 24 }).show(ui, |ui| {
                page_column(ui, |ui| {
                    if backends.is_empty() {
                        empty_card(ui, &t, "Nicht bei AIS.chat angemeldet", "Assistenten, Dialogpartner und Lernszenarien kommen aus AIS.chat.");
                        if let Some(ais) = self.state.backends.iter().find(|b| b.config.kind == BackendKind::AisChat)
                            && accent_button(ui, &t, "Bei AIS.chat anmelden").clicked()
                        {
                            self.dialog = Some(Dialog::login(&ais.config.id));
                        }
                        return;
                    }
                    let loading = backends.iter().any(|b| b.personas.is_none() && b.personas_error.is_none());
                    for b in backends.iter().filter(|b| b.personas.is_none()) {
                        if let Some(e) = &b.personas_error {
                            empty_card(ui, &t, "Konnte die Liste nicht laden", e);
                            if ui.button("Erneut versuchen").clicked() {
                                refresh = true;
                            }
                            ui.add_space(12.0);
                        }
                    }
                    if entries.is_empty() {
                        if loading {
                            ui.add_space(30.0);
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Spinner::new().size(28.0));
                                ui.label(muted(&t, "Lade Assistenten, Dialogpartner und Lernszenarien …"));
                            });
                        } else if backends.iter().all(|b| b.personas.is_some()) {
                            empty_card(ui, &t, "Noch keine", "In AIS.chat angelegte und mit dir geteilte erscheinen hier.");
                        }
                        return;
                    }
                    let no_favorites = !entries.iter().any(|(_, p)| p.favorite);
                    if shown.is_empty() && view.favorites && no_favorites {
                        empty_card(
                            ui,
                            &t,
                            "Noch keine Favoriten",
                            "Favoriten sind Assistenten, Dialogpartner und Lernszenarien, die du oft brauchst. Zeige auf eine Karte und klicke auf den Stern ☆ \
                             oben rechts: Sie erscheint dann hier und in der Seitenleiste, auch in AIS.chat im Browser.",
                        );
                        if accent_button(ui, &t, "Alle zeigen").clicked() {
                            view.favorites = false;
                        }
                        return;
                    }
                    if shown.is_empty() {
                        let what = match view.tab {
                            Some(k) => k.plural().to_string(),
                            None => "Treffer".into(),
                        };
                        let detail = if view.query.trim().is_empty() {
                            "Für diese Filter gibt es nichts.".to_string()
                        } else {
                            format!("Keine {what} für „{}“.", view.query.trim())
                        };
                        empty_card(ui, &t, "Nichts gefunden", &detail);
                        if ui.button("Filter zurücksetzen").clicked() {
                            view.query.clear();
                            view.tab = None;
                            view.favorites = false;
                            view.own = false;
                        }
                        return;
                    }
                    // All kinds: one section each, so the page reads like a catalogue.
                    let sections: Vec<Section> = match view.tab {
                        Some(k) => vec![(Some(k), shown.clone())],
                        None => PersonaKind::ALL
                            .iter()
                            .map(|k| (Some(*k), shown.iter().copied().filter(|(_, p)| p.kind == *k).collect::<Vec<_>>()))
                            .filter(|(_, v)| !v.is_empty())
                            .collect(),
                    };
                    let many = sections.len() > 1;
                    for (kind, items) in sections {
                        if many && let Some(k) = kind {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(k.plural()).size(17.0).strong());
                                ui.label(muted(&t, items.len().to_string()));
                            });
                            ui.add_space(6.0);
                        }
                        if let Some(p) = self.card_grid(ui, &items) {
                            open = Some(p);
                        }
                        ui.add_space(10.0);
                    }
                });
            });
        });

        if refresh {
            for b in &backends {
                self.engine.dispatch_quiet("persona.refresh", json!({"backend": b.config.id}));
            }
        }
        if let Some(p) = open {
            view.detail = Some(p);
        }
        if close {
            self.focus_composer = true;
        } else {
            self.explore = Some(view);
        }
    }

    fn card_grid(&mut self, ui: &mut Ui, items: &[&(String, PersonaInfo)]) -> Option<Pick> {
        let avail = ui.available_width();
        let cols = ((avail + GAP) / (MIN_CARD + GAP)).floor().clamp(1.0, 6.0) as usize;
        let compact = cols == 1 && avail < 460.0;
        let w = ((avail - GAP * (cols as f32 - 1.0)) / cols as f32).floor();
        let mut picked = None;
        for row in items.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP;
                for (backend, p) in row.iter().map(|x| (&x.0, &x.1)) {
                    let (r, star) = self.persona_card(ui, backend, p, w, compact);
                    if star {
                        self.toggle_favorite(backend, p.kind, &p.id, !p.favorite);
                    }
                    let r = match &p.description {
                        Some(d) if d.chars().count() > 120 => r.on_hover_text(d.chars().take(600).collect::<String>()),
                        _ => r,
                    };
                    if r.clicked() {
                        picked = Some(Pick { backend: backend.clone(), kind: p.kind, id: p.id.clone() });
                    }
                }
            });
            ui.add_space(GAP);
        }
        picked
    }

    fn explore_detail(&mut self, ui: &mut Ui, pick: Pick) {
        let t = self.tokens.clone();
        let info = self.explore_find(&pick);
        let mut back = false;
        let mut start: Option<Option<String>> = None;
        // Starting a chat stays in reach however long the page is.
        if let Some(p) = &info {
            egui::Panel::bottom("explore-detail-start")
                .frame(Frame::new().fill(t.bg).stroke(Stroke::new(1.0, t.border)).inner_margin(Margin::symmetric(20, 10)))
                .show(ui, |ui| {
                    page_column(ui, |ui| {
                        ui.horizontal(|ui| {
                            if accent_button(ui, &t, &format!("Chat mit {} starten", p.name)).on_hover_text("Enter").clicked() {
                                start = Some(None);
                            }
                            ui.label(small_muted(&t, p.kind.label()));
                        });
                    });
                });
        }
        ScrollArea::vertical().id_salt(("explore-detail", &pick.id)).auto_shrink([false, false]).show(ui, |ui| {
            Frame::new().inner_margin(Margin { left: 20, right: 20, top: 12, bottom: 28 }).show(ui, |ui| {
                page_column(ui, |ui| {
                    if ui.add(egui::Button::new(muted(&t, "‹  Alle Assistenten, Dialogpartner & Lernszenarien")).frame(false)).on_hover_text("Esc").clicked()
                    {
                        back = true;
                    }
                    ui.add_space(10.0);
                    let Some(p) = info else {
                        empty_card(ui, &t, "Nicht mehr da", "Vielleicht gelöscht oder nicht mehr geteilt.");
                        return;
                    };
                    let wide = ui.available_width() >= 620.0;
                    let side = if wide { 232.0 } else { 148.0 };
                    let header = |ui: &mut Ui, this: &mut Self| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            badge(ui, &t, p.kind.label(), t.selection, t.text);
                            if p.own {
                                badge(ui, &t, "von mir", t.surface_hi, t.muted);
                            }
                            let (label, hint) = if p.favorite {
                                ("★ Favorit", "Favorit entfernen")
                            } else {
                                ("☆ Als Favorit markieren", "Erscheint dann in der Seitenleiste")
                            };
                            if ui
                                .add(egui::Button::new(RichText::new(label).size(12.0)).corner_radius(CornerRadius::same(t.radius)))
                                .on_hover_text(hint)
                                .clicked()
                            {
                                this.toggle_favorite(&pick.backend, p.kind, &p.id, !p.favorite);
                            }
                        });
                        ui.add_space(4.0);
                        ui.add(egui::Label::new(RichText::new(&p.name).size(28.0).strong()).wrap());
                        if let Some(d) = &p.description {
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(d).size(15.0).color(t.muted)).wrap());
                        }
                        if let Some(m) = p.model_id.as_ref().and_then(|m| this.state.backend(&pick.backend)?.models.iter().find(|x| &x.id == m)) {
                            ui.add_space(4.0);
                            ui.label(small_muted(&t, format!("Modell: {}", m.name)));
                        }
                        if let Some(on) = p.web_search {
                            ui.add_space(2.0);
                            ui.label(small_muted(&t, format!("Websuche: {} (in AIS.chat eingestellt)", if on { "an" } else { "aus" })));
                        }
                    };
                    if wide {
                        ui.horizontal_top(|ui| {
                            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                            self.paint_persona_picture(ui, rect, &pick.backend, &p, t.radius.saturating_add(4), true);
                            ui.add_space(14.0);
                            ui.vertical(|ui| header(ui, self));
                        });
                    } else {
                        let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                        self.paint_persona_picture(ui, rect, &pick.backend, &p, t.radius.saturating_add(4), true);
                        ui.add_space(10.0);
                        header(ui, self);
                    }
                    ui.add_space(18.0);
                    if let Some(g) = &p.initial_message {
                        card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(small_muted(&t, "BEGRÜSSUNG"));
                            ui.add(egui::Label::new(g).wrap());
                        });
                        ui.add_space(10.0);
                    }
                    if let Some(ex) = &p.exercise {
                        card(&t).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(small_muted(&t, "AUFGABE"));
                            ui.add(egui::Label::new(ex).wrap());
                        });
                        ui.add_space(10.0);
                    }
                    if !p.prompt_suggestions.is_empty() {
                        ui.add_space(4.0);
                        ui.label(small_muted(&t, "SO KANNST DU ANFANGEN"));
                        ui.label(small_muted(&t, format!("Vorschläge von „{}“ aus AIS.chat", p.name)));
                        ui.add_space(4.0);
                        for s in &p.prompt_suggestions {
                            let r = ui.add(
                                egui::Button::new(RichText::new(s).color(t.text))
                                    .wrap()
                                    .fill(t.surface)
                                    .stroke(Stroke::new(1.0, t.border))
                                    .corner_radius(CornerRadius::same(t.radius))
                                    .min_size(vec2(ui.available_width(), 34.0)),
                            );
                            if r.on_hover_text("Chat starten und so beginnen").clicked() {
                                start = Some(Some(s.clone()));
                            }
                            ui.add_space(2.0);
                        }
                    }
                });
            });
        });
        if let Some(text) = start {
            self.start_persona(&pick.backend, pick.kind, &pick.id, text);
        } else if back && let Some(v) = self.explore.as_mut() {
            v.detail = None;
        }
    }
}

fn badge(ui: &mut Ui, t: &Tokens, text: &str, fill: Color32, color: Color32) {
    Frame::new().fill(fill).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(12.0).color(color));
    });
}

fn empty_card(ui: &mut Ui, t: &Tokens, title: &str, text: &str) {
    card(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).strong());
        ui.add(egui::Label::new(muted(t, text)).wrap());
    });
    ui.add_space(8.0);
}

/// A centred column at most [`PAGE_WIDTH`] wide.
fn page_column(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    page_column_max(ui, PAGE_WIDTH, add);
}

fn page_column_max(ui: &mut Ui, max: f32, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width().min(max);
    let pad = ((ui.available_width() - w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, desc: Option<&str>) -> PersonaInfo {
        PersonaInfo {
            kind: PersonaKind::Character,
            id: "x".into(),
            name: name.into(),
            description: desc.map(str::to_string),
            own: false,
            initial_message: None,
            exercise: None,
            prompt_suggestions: Vec::new(),
            model_id: None,
            picture_url: None,
            favorite: false,
            web_search: None,
        }
    }

    #[test]
    fn search_ignores_case_and_umlauts() {
        let goethe = p("Johann Wolfgang von Goethe", Some("Dichter und Naturforscher aus Weimar"));
        let uebung = p("Übungsheft Brüche", None);
        for q in ["goethe", "GOETHE", "göthe", "weimar dichter", "natur"] {
            assert!(matches_query(&goethe, &query_terms(q)), "{q}");
        }
        for q in ["ubung", "uebung", "Übung", "bruche", "brueche"] {
            assert!(matches_query(&uebung, &query_terms(q)), "{q}");
        }
        assert!(!matches_query(&goethe, &query_terms("schiller")));
        assert!(!matches_query(&goethe, &query_terms("goethe schiller")), "every word must match");
        assert!(matches_query(&goethe, &query_terms("   ")));
        assert_eq!(fold("Straße"), "strasse");
    }

    #[test]
    fn initials_and_cover() {
        assert_eq!(initials("Johann Wolfgang von Goethe"), "JG");
        assert_eq!(initials("Mathe-Coach"), "MC");
        assert_eq!(initials("marie"), "M");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("Ägypten erkunden"), "ÄE");
        // A tall picture in a wide box: full width, a top-biased slice of the height.
        let uv = cover_uv(vec2(100.0, 200.0), vec2(200.0, 100.0));
        assert!((uv.width() - 1.0).abs() < 1e-4 && (uv.height() - 0.25).abs() < 1e-4 && uv.min.y > 0.0 && uv.min.y < 0.375);
        let full = cover_uv(vec2(0.0, 0.0), vec2(10.0, 10.0));
        assert_eq!(full.max, egui::pos2(1.0, 1.0));
    }
}
