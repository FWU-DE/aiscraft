//! Small shared widgets.

use egui::{Color32, CornerRadius, Frame, Margin, Response, RichText, Stroke, Ui};

use crate::theme::Tokens;

pub fn card(t: &Tokens) -> Frame {
    Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.border)).corner_radius(CornerRadius::same(t.radius)).inner_margin(Margin::same(14))
}

pub fn accent_button(ui: &mut Ui, t: &Tokens, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(t.on_accent).strong()).fill(t.accent).corner_radius(CornerRadius::same(t.radius.saturating_sub(2))))
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

pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: &str, tip: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(icon).color(t.muted).size(15.0)).frame(false)).on_hover_text(tip)
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
}
