//! Small shared widgets.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Stroke, Ui};

use crate::theme::Tokens;

pub fn card(t: &Tokens) -> Frame {
    Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::same(16))
}

/// Inner margin of every dialog.
pub const DIALOG_MARGIN: i8 = 22;

/// The frame of every dialog: the same inner margin on all sides, so titles and buttons never sit
/// on the edge.
pub fn dialog_frame(ctx: &egui::Context, t: &Tokens) -> Frame {
    let edge = if t.glass.is_some() { Stroke::new(1.0, t.panel_stroke) } else { Stroke::NONE };
    Frame::popup(&ctx.global_style())
        .fill(t.modal)
        .stroke(edge)
        .shadow(t.modal_shadow)
        .corner_radius(CornerRadius::same(24))
        .inner_margin(Margin::same(DIALOG_MARGIN + 4))
}

pub fn accent_button(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    crate::kit::pill(ui, t, crate::kit::Pill::Primary, None, text, 14.0, None)
}

/// A field followed by buttons in one row: `trailing` lays out the buttons first, from the right
/// (so the first one it adds sits rightmost), and `field` gets the width they leave. A fixed
/// reserve such as `available_width() - 90.0` overflowed once spacing or text grew (150 %, wider
/// looks), and an overflowing row widened its panel a little every frame.
pub fn field_then_buttons<F, R>(ui: &mut Ui, trailing: impl FnOnce(&mut Ui) -> R, field: impl FnOnce(&mut Ui, f32) -> F) -> (F, R) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let r = trailing(ui);
        let w = ui.available_width().max(40.0);
        (field(ui, w), r)
    })
    .inner
}

/// A secondary action that reads as a link ("Später"), so the main button stands out.
pub fn link_button(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(t.accent)).frame(false))
}

/// The AIS logo (ring, bar and dot), drawn from its SVG geometry (viewBox 269.8 × 230.1).
pub fn ais_logo(ui: &mut Ui, height: f32, color: Color32) -> Response {
    let scale = height / 230.1;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(269.8 * scale, height), egui::Sense::hover());
    let p = |x: f32, y: f32| rect.min + egui::vec2(x * scale, y * scale);
    let painter = ui.painter();
    // Ring: outer 2..262.2 × 2.6..122.6 (radius 60), inner 42.5..221.7 × 43.1..82.1, so a stroke
    // of 40.5 along the midline.
    let ring = egui::Rect::from_min_max(p(22.25, 22.85), p(241.95, 102.35));
    painter.rect_stroke(ring, CornerRadius::same((39.75 * scale).round().clamp(0.0, 255.0) as u8), Stroke::new(40.5 * scale, color), egui::StrokeKind::Middle);
    let bar = egui::Rect::from_min_max(p(161.1, 138.6), p(257.1, 179.4));
    painter.rect_filled(bar, CornerRadius::same((20.4 * scale).round().clamp(0.0, 255.0) as u8), color);
    painter.circle_filled(p(249.7, 207.9), 17.4 * scale, color);
    resp
}

/// A symbol button with a 28 px target that lights up on hover, and its name as tooltip.
pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str) -> Response {
    let r = ui.add(egui::Button::new(RichText::new(icon).color(t.muted).size(15.0)).frame_when_inactive(false).min_size(egui::vec2(28.0, 28.0)));
    r.on_hover_text(tip)
}

/// A Material 3 segmented button: one outlined pill, the chosen segment filled with the
/// selection colour (secondary container). Each segment keeps its text as accessible label.
pub fn segmented<T: PartialEq + Copy, const N: usize>(ui: &mut Ui, t: &Tokens, value: &mut T, options: [(T, &str); N]) {
    segmented_icons(ui, t, value, options.map(|(v, l)| (v, None, l)));
}

/// A segmented control with optional icons (Modern, Glas): a recessed pill, the chosen segment
/// raised on the surface.
pub fn segmented_icons<T: PartialEq + Copy, const N: usize>(ui: &mut Ui, t: &Tokens, value: &mut T, options: [(T, Option<crate::icons::Icon>, &str); N]) {
    let fill = if t.glass.is_some() { t.chip } else { t.sunken };
    // In a right-to-left row the segments would come out mirrored.
    let mut options = options;
    if ui.layout().prefer_right_to_left() {
        options.reverse();
    }
    Frame::new().fill(fill).corner_radius(CornerRadius::same(18)).inner_margin(Margin::same(3)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.horizontal(|ui| {
            for (option, icon, label) in options {
                let on = *value == option;
                let fg = if on { t.text } else { t.muted };
                let g = crate::kit::line(ui, label, crate::kit::font(13.5, crate::theme::SEMIBOLD), fg, 300.0);
                let w = 22.0 + g.size().x + if icon.is_some() { 22.0 } else { 0.0 };
                let (rect, r) = ui.allocate_exact_size(egui::vec2(w, 30.0), egui::Sense::click());
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, label));
                let p = ui.painter();
                if on {
                    let face = if t.glass.is_some() { if t.dark { Color32::from_white_alpha(36) } else { Color32::WHITE } } else { t.surface };
                    p.add(
                        egui::epaint::Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_rgba_unmultiplied(0x0b, 0x2a, 0x45, 0x1f) }
                            .as_shape(rect, CornerRadius::same(15)),
                    );
                    p.rect_filled(rect, CornerRadius::same(15), face);
                } else if r.hovered() {
                    p.rect_filled(rect, CornerRadius::same(15), t.text.gamma_multiply(0.05));
                }
                let mut x = rect.left() + 11.0;
                if let Some(i) = icon {
                    i.paint(p, egui::pos2(x + 8.0, rect.center().y), 16.0, fg);
                    x += 22.0;
                }
                p.galley(egui::pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
                if r.clicked() {
                    *value = option;
                }
            }
        });
    });
}

/// An on/off switch (46 × 26), the accent when on.
pub fn toggle(ui: &mut Ui, t: &Tokens, on: &mut bool, name: &str) -> Response {
    let (rect, mut r) = ui.allocate_exact_size(egui::vec2(46.0, 26.0), egui::Sense::click());
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *on, name));
    if r.clicked() {
        *on = !*on;
        r.mark_changed();
    }
    let p = ui.painter();
    let k = ui.ctx().animate_bool(r.id, *on);
    if *on {
        crate::kit::accent_fill(p, rect, CornerRadius::same(13), t);
    } else {
        p.rect_filled(rect, CornerRadius::same(13), t.opaque(t.border));
    }
    let x = egui::lerp((rect.left() + 13.0)..=(rect.right() - 13.0), k);
    p.circle_filled(egui::pos2(x, rect.center().y + 1.0), 10.0, Color32::from_black_alpha(40));
    p.circle_filled(egui::pos2(x, rect.center().y), 10.0, Color32::WHITE);
    r
}

/// The notification bell, drawn (the emoji bell in the bundled font is small and tilted), with
/// the unread count as a badge. Its accessible name is "Benachrichtigungen" plus the count.
pub fn bell_button(ui: &mut Ui, t: &Tokens, unread: usize) -> Response {
    let size = egui::vec2(36.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let name = if unread > 0 { format!("Benachrichtigungen: {unread} neu") } else { "Benachrichtigungen".to_string() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let p = ui.painter();
    if resp.hovered() || resp.has_focus() {
        p.circle_filled(rect.center(), 18.0, t.text2.gamma_multiply(0.10));
    }
    crate::icons::Icon::Bell.paint(p, rect.center(), 19.0, t.text2);
    if unread > 0 {
        let text = if unread > 9 { "9+".to_string() } else { unread.to_string() };
        let galley = p.layout_no_wrap(text, egui::FontId::proportional(10.0), t.on_accent);
        let w = (galley.size().x + 6.0).max(14.0);
        let badge = egui::Rect::from_center_size(egui::pos2(rect.right() - w / 2.0 - 3.0, rect.top() + 9.0), egui::vec2(w, 14.0));
        p.rect(badge, CornerRadius::same(7), t.accent, Stroke::new(1.5, t.opaque(t.main)), egui::StrokeKind::Outside);
        p.galley(badge.center() - galley.size() / 2.0, galley, t.on_accent);
    }
    resp.on_hover_text(name)
}

pub fn status_dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

pub fn muted(t: &Tokens, s: impl Into<String>) -> RichText {
    RichText::new(s).color(t.muted)
}

pub fn small_muted(t: &Tokens, s: impl Into<String>) -> RichText {
    RichText::new(s).color(t.muted).small()
}

/// Text without Markdown marks, for one-line places (toasts, notifications) that do not render it.
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let line = line.trim_start().trim_start_matches('#').trim_start();
        let line = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).unwrap_or(line);
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(line);
    }
    out.replace("**", "").replace("__", "").replace('`', "").trim().to_string()
}

/// Bucket for the chat list.
pub fn age_group(ms: i64) -> &'static str {
    let days = aiscraft_engine::now_ms().saturating_sub(ms) / 86_400_000;
    match days {
        i64::MIN..=0 => "Heute",
        1 => "Gestern",
        2..=6 => "Letzte 7 Tage",
        7..=29 => "Letzte 30 Tage",
        _ => "Älter",
    }
}

/// Subsequence match score for the command palette; `None` = no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0i32;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, c) in t.iter().enumerate() {
        if q.get(qi) == Some(c) {
            score += match last {
                Some(l) if l + 1 == i => 5,
                _ => 1,
            };
            if i == 0 || t.get(i.wrapping_sub(1)).is_some_and(|p| !p.is_alphanumeric()) {
                score += 3;
            }
            last = Some(i);
            qi += 1;
            if qi == q.len() {
                return Some(score - (t.len() as i32 / 20));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert!(fuzzy_score("nch", "Neuer Chat").is_some());
        assert!(fuzzy_score("xyz", "Neuer Chat").is_none());
        assert!(fuzzy_score("neu", "Neuer Chat") > fuzzy_score("neu", "Antwort neu erzeugen"));
        assert_eq!(fuzzy_score("", "x"), Some(0));
    }

    #[test]
    fn plain_strips_markdown() {
        assert_eq!(plain("## Die **Photosynthese** ist `x`"), "Die Photosynthese ist x");
        assert_eq!(plain("**Morgenplan** fertig\n- erster Punkt"), "Morgenplan fertig erster Punkt");
        assert_eq!(plain(""), "");
    }
}
