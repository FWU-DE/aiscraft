//! Choosing the look: small previews painted with each look's own tokens (no screenshots), the
//! scheme (hell, dunkel, wie das System) and Glas' transparency. Used by the welcome dialog and
//! by Einstellungen → Darstellung; a choice applies at once through `settings.set`.

use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::icons::Icon;
use crate::theme::{self, Look, Mode, Tokens};

/// The looks offered, with their names and one line each.
pub(crate) const LOOKS: [(Look, &str, &str); 4] = [
    (Look::Modern, "Modern", "Nachtblaue Seitenleiste, kräftige Schrift"),
    (Look::Glas, "Glas", "Leichte Glasflächen auf weichem Verlauf"),
    (Look::Classic, "Klassisch", "Das bisherige, schlichte Aussehen"),
    (Look::Omarchy, "Omarchy", "Farben deines Omarchy-Desktops"),
];

impl AisCraftApp {
    fn current_look(&self) -> (Look, Mode) {
        theme::parse(&self.state.settings.theme)
    }

    fn set_look(&self, look: Look, mode: Mode) {
        // Omarchy has no scheme of its own; coming back from it keeps "like the system".
        let mode = if look == Look::Omarchy { Mode::System } else { mode };
        self.run("settings.set", json!({"key": "theme", "value": theme::setting(look, mode)}));
    }

    /// One card per look; `with_omarchy` adds the desktop theme (settings only).
    pub(crate) fn look_cards(&mut self, ui: &mut Ui, with_omarchy: bool) {
        let t = self.tokens.clone();
        let (look, mode) = self.current_look();
        // Omarchy only where there is an Omarchy desktop (or it is chosen already).
        let with_omarchy = with_omarchy && (theme::omarchy_available() || look == Look::Omarchy);
        let looks: Vec<_> = LOOKS.iter().filter(|(l, ..)| with_omarchy || *l != Look::Omarchy).collect();
        let n = looks.len() as f32;
        let gap = 12.0;
        let w = ((ui.available_width() - gap * (n - 1.0)) / n).max(110.0);
        let dark = t.dark;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (l, name, line) in looks {
                let preview = match l {
                    Look::Omarchy => Some(t.clone()).filter(|t| t.look == Look::Omarchy).unwrap_or_else(|| theme::resolve("omarchy", dark, false)),
                    l => theme::tokens(*l, dark),
                };
                if look_card(ui, &t, &preview, *l == look, name, line, w).clicked() {
                    self.set_look(*l, mode);
                }
            }
        });
    }

    /// Hell | Dunkel | Wie das System, for the current look.
    pub(crate) fn mode_picker(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let (look, mode) = self.current_look();
        if look == Look::Omarchy {
            ui.label(crate::widgets::small_muted(&t, "Hell oder dunkel bestimmt das Omarchy-Theme."));
            return;
        }
        let mut m = mode;
        crate::widgets::segmented(ui, &t, &mut m, [(Mode::Light, "Hell"), (Mode::Dark, "Dunkel"), (Mode::System, "Wie das System")]);
        if m != mode {
            self.set_look(look, m);
        }
    }

    /// Glas: translucent panels over the gradient, or opaque ones.
    pub(crate) fn transparency_picker(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let mut v = self.state.settings.transparency == "off";
        crate::widgets::segmented(ui, &t, &mut v, [(true, "Aus"), (false, "Verlauf")]);
        if v != (self.state.settings.transparency == "off") {
            self.run("settings.set", json!({"key": "transparency", "value": if v { "off" } else { "gradient" }}));
        }
    }
}

/// A selectable card: the preview, the name and a line; framed in the accent when chosen.
fn look_card(ui: &mut Ui, t: &Tokens, preview: &Tokens, on: bool, name: &str, line: &str, w: f32) -> egui::Response {
    let ph = (w * 0.56).round();
    let h = ph + 74.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, name));
    let p = ui.painter();
    let hovered = resp.hovered() || resp.has_focus();
    let stroke = if on {
        Stroke::new(2.0, t.accent)
    } else if hovered {
        Stroke::new(1.0, t.text2)
    } else {
        Stroke::new(1.0, t.opaque(t.border))
    };
    p.rect(rect, CornerRadius::same(16), t.opaque(t.surface), stroke, StrokeKind::Inside);
    let pv = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(w - 16.0, ph));
    paint_preview(p, pv, preview);
    let ty = pv.bottom() + 10.0;
    p.text(pos2(rect.left() + 12.0, ty), Align2::LEFT_TOP, name, FontId::new(15.0, egui::FontFamily::Name(theme::SEMIBOLD.into())), t.text);
    let galley = p.layout(line.to_string(), FontId::proportional(12.5), t.muted, w - 24.0);
    p.galley(pos2(rect.left() + 12.0, ty + 21.0), galley, t.muted);
    if on {
        let c = pos2(rect.right() - 20.0, ty + 9.0);
        p.circle_filled(c, 10.0, t.accent);
        Icon::Check.paint(p, c, 13.0, t.on_accent);
    }
    resp.on_hover_text(line)
}

/// The app in miniature, in the look's own colours: backdrop, sidebar, lines of text, a user
/// bubble and the composer with its send button.
pub(crate) fn paint_preview(p: &egui::Painter, r: Rect, t: &Tokens) {
    let p = p.with_clip_rect(r.intersect(p.clip_rect()));
    let round = CornerRadius::same(10);
    let w = r.width();
    let h = r.height();
    let at = |x: f32, y: f32| pos2(r.left() + x * w, r.top() + y * h);
    if let Some(g) = &t.glass {
        p.rect_filled(r, round, g.backdrop[0]);
        crate::backdrop::paint(&p, r, g, 1.0);
    } else {
        p.rect_filled(r, round, t.opaque(t.main));
    }
    let gap = if t.glass.is_some() { 0.03 * w } else { 0.0 };
    let side = Rect::from_min_max(pos2(r.left() + gap, r.top() + gap), pos2(r.left() + 0.24 * w, r.bottom() - gap));
    let side_round = if t.glass.is_some() { CornerRadius::same(7) } else { CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 } };
    crate::backdrop::vertical_gradient(&p, side, side_round, t.side_top, t.side_bottom);
    // Sidebar: the "Neuer Chat" pill and three entries, the second active.
    p.rect_filled(Rect::from_min_size(side.min + vec2(0.12 * side.width(), 0.1 * h), vec2(0.76 * side.width(), 0.09 * h)), CornerRadius::same(8), t.side_new);
    for i in 0..3 {
        let y = side.top() + (0.3 + i as f32 * 0.14) * h;
        let line = Rect::from_min_size(pos2(side.left() + 0.12 * side.width(), y), vec2(0.6 * side.width(), 0.045 * h));
        if i == 1 {
            p.rect_filled(line.expand2(vec2(0.05 * side.width(), 0.03 * h)), CornerRadius::same(4), t.opaque(t.side_active));
        }
        p.rect_filled(line, CornerRadius::same(2), t.side_text2.gamma_multiply(0.7));
    }
    let main = if t.glass.is_some() {
        let m = Rect::from_min_max(pos2(side.right() + gap, r.top() + gap), pos2(r.right() - gap, r.bottom() - gap));
        p.rect(m, CornerRadius::same(7), t.main, Stroke::new(1.0, t.panel_stroke), StrokeKind::Inside);
        m
    } else {
        Rect::from_min_max(pos2(side.right(), r.top()), r.max)
    };
    let mx = |f: f32| main.left() + f * main.width();
    // Title, user bubble, answer lines.
    p.rect_filled(Rect::from_min_max(pos2(mx(0.1), at(0.0, 0.12).y), pos2(mx(0.5), at(0.0, 0.19).y)), CornerRadius::same(3), t.text);
    p.rect_filled(Rect::from_min_max(pos2(mx(0.55), at(0.0, 0.25).y), pos2(mx(0.9), at(0.0, 0.37).y)), CornerRadius::same(6), t.user_bubble);
    for (i, wv) in [0.7, 0.55, 0.62].iter().enumerate() {
        let y = at(0.0, 0.44 + i as f32 * 0.09).y;
        p.rect_filled(Rect::from_min_size(pos2(mx(0.1), y), vec2(wv * main.width(), 0.04 * h)), CornerRadius::same(2), t.text2.gamma_multiply(0.45));
    }
    let comp = Rect::from_min_max(pos2(mx(0.08), at(0.0, 0.74).y), pos2(mx(0.92), at(0.0, 0.9).y));
    p.rect(comp, CornerRadius::same(8), t.composer, Stroke::new(1.0, t.composer_stroke), StrokeKind::Inside);
    p.circle_filled(pos2(comp.right() - comp.height() * 0.5, comp.center().y), comp.height() * 0.32, t.accent);
    if t.flush_modern() {
        // The coral bar on the active entry.
        let y = side.top() + 0.44 * h;
        p.rect_filled(Rect::from_min_size(pos2(side.left(), y - 0.01 * h), vec2(2.0, 0.065 * h)), CornerRadius::same(1), t.accent2);
    }
}
