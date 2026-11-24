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
    Frame::popup(&ctx.global_style()).fill(t.surface).corner_radius(CornerRadius::same(t.radius.saturating_add(4))).inner_margin(Margin::same(DIALOG_MARGIN))
}

pub fn accent_button(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(t.on_accent).strong()).fill(t.accent).corner_radius(CornerRadius::same(t.control_radius())))
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

/// The notification bell, drawn (the emoji bell in the bundled font is small and tilted), with
/// the unread count as a badge. Its accessible name is "Benachrichtigungen" plus the count.
pub fn bell_button(ui: &mut Ui, t: &Tokens, unread: usize) -> Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(30.0, 28.0), egui::Sense::click());
    let name = if unread > 0 { format!("Benachrichtigungen: {unread} neu") } else { "Benachrichtigungen".to_string() };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    let p = ui.painter();
    if resp.hovered() || resp.has_focus() {
        p.rect_filled(rect, CornerRadius::same(t.control_radius()), ui.visuals().widgets.hovered.bg_fill);
    }
    let color = if unread > 0 || resp.hovered() { t.text } else { t.muted };
    let c = rect.center() - egui::vec2(0.0, 1.0);
    // Dome: a half circle on top of straight sides that flare out to the rim.
    let r = 5.0;
    let mut pts = Vec::with_capacity(20);
    pts.push(c + egui::vec2(-7.0, 5.0));
    pts.push(c + egui::vec2(-r, 2.5));
    for i in 0..=12 {
        let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
        pts.push(c + egui::vec2(r * a.cos(), -1.5 + r * a.sin()));
    }
    pts.push(c + egui::vec2(r, 2.5));
    pts.push(c + egui::vec2(7.0, 5.0));
    p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    p.circle_filled(c + egui::vec2(0.0, 7.2), 1.8, color);
    if unread > 0 {
        let text = if unread > 9 { "9+".to_string() } else { unread.to_string() };
        let galley = p.layout_no_wrap(text, egui::FontId::proportional(10.0), t.on_accent);
        let w = (galley.size().x + 6.0).max(14.0);
        let badge = egui::Rect::from_center_size(egui::pos2(rect.right() - w / 2.0 - 1.0, rect.top() + 7.0), egui::vec2(w, 14.0));
        p.rect(badge, CornerRadius::same(7), t.accent, Stroke::new(1.5, ui.visuals().panel_fill), egui::StrokeKind::Outside);
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
