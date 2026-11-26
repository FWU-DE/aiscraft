//! Building blocks of the Modern and Glas anatomy (design/build/app.py): pills, round icon
//! buttons, sidebar entries, section labels, tiles. Each paints itself from the tokens and
//! carries its words as accessible label, so tests and screen readers find it by name.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Galley, Rect, Response, Sense, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::icons::Icon;
use crate::theme::{self, Tokens};

pub fn font(size: f32, family: &str) -> FontId {
    match family {
        "" => FontId::proportional(size),
        f => FontId::new(size, FontFamily::Name(f.into())),
    }
}

/// One line of text, cut with "…" at `max_w`.
pub fn line(ui: &Ui, text: &str, font: FontId, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap.max_width = max_w.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Upper-case text with extra letter spacing (section labels, "FORMEL").
pub fn caps(ui: &Ui, text: &str, size: f32, color: Color32, spacing: f32) -> Arc<Galley> {
    let job = LayoutJob::single_section(
        text.to_uppercase(),
        TextFormat { font_id: font(size, theme::SEMIBOLD), color, extra_letter_spacing: spacing, ..Default::default() },
    );
    ui.fonts_mut(|f| f.layout_job(job))
}

/// A section label as text: upper case, small, muted (for `ui.label`).
pub fn caps_text(t: &Tokens, text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase()).size(11.5).color(t.muted).family(FontFamily::Name(theme::SEMIBOLD.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pill {
    /// The accent fill (a gradient in Glas).
    Primary,
    /// Tinted accent.
    Soft,
    /// Outlined on the surface (actions under answers).
    Ghost,
    /// Filled neutral (model picker, "Quellen").
    Chip,
    /// The second accent, tinted ("Neu: …").
    Accent2,
    /// Text only, in the link colour ("Alle anzeigen", "Handbuch").
    Link,
}

/// Fill a rect with the accent: flat, or Glas' diagonal gradient.
pub fn accent_fill(p: &egui::Painter, rect: Rect, radius: CornerRadius, t: &Tokens) {
    if t.accent_end == t.accent {
        p.rect_filled(rect, radius, t.accent);
    } else {
        crate::backdrop::vertical_gradient(p, rect, radius, t.accent, t.accent_end);
    }
}

/// A pill button with an optional leading icon and trailing chevron. Size is the text size.
pub fn pill(ui: &mut Ui, t: &Tokens, kind: Pill, icon: Option<Icon>, label: &str, size: f32, trailing: Option<Icon>) -> Response {
    let (fill, fg, stroke) = match kind {
        Pill::Primary => (t.accent, t.on_accent, Color32::TRANSPARENT),
        Pill::Soft => (t.accent_soft, t.accent_ink, Color32::TRANSPARENT),
        Pill::Ghost => (t.surface, t.text2, if t.glass.is_some() { t.card_stroke } else { t.border }),
        Pill::Chip => (t.chip, t.chip_text, Color32::TRANSPARENT),
        Pill::Accent2 => (t.accent2_soft, t.accent2_ink, Color32::TRANSPARENT),
        Pill::Link => (Color32::TRANSPARENT, t.link, Color32::TRANSPARENT),
    };
    let text = (!label.is_empty()).then(|| line(ui, label, font(size, theme::SEMIBOLD), fg, 600.0));
    let isz = size + 3.0;
    let pad_l = if icon.is_some() { 10.0 } else { 13.0 };
    let pad_r = if trailing.is_some() { 9.0 } else { 13.0 };
    let gap = 7.0;
    let mut w = pad_l + pad_r;
    if icon.is_some() {
        w += isz;
    }
    if let Some(g) = &text {
        w += g.size().x + if icon.is_some() { gap } else { 0.0 };
    }
    if trailing.is_some() {
        w += size + 1.0 + gap;
    }
    let h = (size * 2.3).round().max(28.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let name = if label.is_empty() { "" } else { label };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    let p = ui.painter();
    let r = CornerRadius::same((h / 2.0) as u8);
    if kind == Pill::Primary {
        accent_fill(p, rect, r, t);
    } else {
        p.rect(rect, r, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    }
    if resp.hovered() || resp.has_focus() {
        let layer = if kind == Pill::Primary { Color32::from_white_alpha(28) } else { t.text.gamma_multiply(0.06) };
        p.rect_filled(rect, r, layer);
    }
    let mut x = rect.left() + pad_l;
    if let Some(i) = icon {
        i.paint(p, pos2(x + isz / 2.0, rect.center().y), isz, fg);
        x += isz + gap;
    }
    if let Some(g) = text {
        let gw = g.size().x;
        p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, fg);
        x += gw + gap;
    }
    if let Some(i) = trailing {
        i.paint(p, pos2(x + (size + 1.0) / 2.0, rect.center().y), size + 1.0, fg);
    }
    resp
}

/// A round icon button (36 px target); its name is the tooltip and the accessible label.
pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: Icon, name: &str, color: Color32, size: f32, box_: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(box_, box_), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    let p = ui.painter();
    if resp.is_pointer_button_down_on() {
        p.circle_filled(rect.center(), box_ / 2.0, color.gamma_multiply(0.16));
    } else if resp.hovered() || resp.has_focus() {
        p.circle_filled(rect.center(), box_ / 2.0, color.gamma_multiply(0.10));
    }
    let _ = t;
    icon.paint(p, rect.center(), size, color);
    resp.on_hover_text(name)
}

/// The white tile with the AIS.chat logo.
pub fn logo_tile(ui: &mut Ui, d: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(d, d), Sense::click());
    paint_logo_tile(ui.painter(), rect);
    resp
}

pub fn paint_logo_tile(p: &egui::Painter, rect: Rect) {
    let d = rect.width();
    let r = CornerRadius::same((d * 0.28) as u8);
    p.rect_filled(rect.translate(vec2(0.0, 1.0)), r, Color32::from_black_alpha(30));
    p.rect_filled(rect, r, Color32::WHITE);
    let h = d * 0.5 * 230.1 / 269.8;
    let lw = d * 0.5;
    let logo = Rect::from_center_size(rect.center(), vec2(lw, h));
    paint_logo(p, logo, Color32::from_rgb(0x10, 0x7f, 0xad));
}

/// The AIS logo (ring, bar and dot) in `rect` (aspect 269.8 × 230.1).
pub fn paint_logo(p: &egui::Painter, rect: Rect, color: Color32) {
    let scale = rect.height() / 230.1;
    let at = |x: f32, y: f32| rect.min + vec2(x * scale, y * scale);
    let ring = Rect::from_min_max(at(22.25, 22.85), at(241.95, 102.35));
    p.rect_stroke(ring, CornerRadius::same((39.75 * scale).round().clamp(0.0, 255.0) as u8), Stroke::new(40.5 * scale, color), StrokeKind::Middle);
    let bar = Rect::from_min_max(at(161.1, 138.6), at(257.1, 179.4));
    p.rect_filled(bar, CornerRadius::same((20.4 * scale).round().clamp(0.0, 255.0) as u8), color);
    p.circle_filled(at(249.7, 207.9), 17.4 * scale, color);
}

/// A section heading in the sidebar: upper case, spaced, with an optional "+" on the right.
/// Returns the plus button's response.
pub fn section(ui: &mut Ui, t: &Tokens, label: &str, plus: Option<&str>) -> Option<Response> {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
    let g = caps(ui, label, 11.5, t.side_muted, if w < 236.0 { 0.8 } else { 1.1 });
    ui.painter().galley(pos2(rect.left() + 12.0, rect.center().y - g.size().y / 2.0), g, t.side_muted);
    let tip = plus?;
    let b = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(24.0, 24.0));
    let resp = ui.interact(b, ui.scope_id().with(("section-plus", label)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    if resp.hovered() {
        ui.painter().circle_filled(b.center(), 12.0, t.side_text.gamma_multiply(0.12));
    }
    Icon::Plus.paint(ui.painter(), b.center(), 15.0, t.side_muted);
    Some(resp.on_hover_text(tip))
}

/// An entry of the sidebar: icon and label; the active one filled (Modern: with the coral bar).
pub fn side_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, active: bool) -> Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    paint_side_item(ui, t, icon, label, active, rect, &resp, false);
    resp
}

/// Where a sidebar row's „…“ button sits (#123): 24 px, 4 px inside the row's right edge.
pub fn side_more_rect(row: Rect) -> Rect {
    Rect::from_center_size(pos2(row.right() - 4.0 - 12.0, row.center().y), vec2(24.0, 24.0))
}

/// How far right a sidebar row's label may reach: with the „…“ button it ends 6 px before it.
pub fn side_label_right(row: Rect, more: bool) -> f32 {
    if more { side_more_rect(row).left() - 6.0 } else { row.right() - 11.0 }
}

/// A chat's row in the sidebar: like [`side_item`], with a „…“ button (named `name`) on the
/// right while the row is hovered, focused or active, or its menu is open (#123). The title is
/// shortened before the button, never under it. Returns the row and, when shown, the button.
pub fn side_chat_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, active: bool, name: &str) -> (Response, Option<Response>) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    let more_id = resp.id.with("more");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
    let show = active || menu_open || resp.has_focus() || ui.rect_contains_pointer(rect);
    paint_side_item(ui, t, icon, label, active, rect, &resp, show);
    if !show {
        return (resp, None);
    }
    let r = side_more_rect(rect);
    let more = ui.interact(r, more_id, Sense::click());
    more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let fg = if active { t.side_active_text } else { t.side_text2 };
    if more.hovered() || more.has_focus() || menu_open {
        ui.painter().circle_filled(r.center(), 12.0, fg.gamma_multiply(0.14));
    }
    Icon::More.paint(ui.painter(), r.center(), 16.0, fg);
    let more = more.on_hover_text(name);
    (resp, Some(more))
}

#[allow(clippy::too_many_arguments)]
fn paint_side_item(ui: &Ui, t: &Tokens, icon: Icon, label: &str, active: bool, rect: Rect, resp: &Response, more: bool) {
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    let p = ui.painter();
    let r = CornerRadius::same(12);
    if active {
        if t.glass.is_some() && !t.dark {
            p.add(
                egui::epaint::Shadow { offset: [0, 2], blur: 6, spread: 0, color: Color32::from_rgba_unmultiplied(0x0b, 0x2a, 0x45, 0x1a) }.as_shape(rect, r),
            );
        }
        p.rect_filled(rect, r, t.side_active);
        if t.flush_modern() {
            p.rect_filled(Rect::from_min_size(pos2(rect.left(), rect.center().y - 9.0), vec2(3.0, 18.0)), CornerRadius::same(2), t.accent2);
        }
    } else if resp.hovered() || resp.has_focus() || (more && ui.rect_contains_pointer(rect)) {
        p.rect_filled(rect, r, t.side_active.gamma_multiply(0.5));
    }
    let fg = if active { t.side_active_text } else { t.side_text2 };
    let ic = if active {
        t.side_active_text
    } else if icon == Icon::Folder {
        t.side_muted
    } else {
        t.side_text2
    };
    icon.paint(p, pos2(rect.left() + 12.0 + 8.5, rect.center().y), 17.0, ic);
    let family = if active { theme::SEMIBOLD } else { "" };
    let g = line(ui, label, font(14.5, family), fg, side_label_right(rect, more) - (rect.left() + 41.0));
    p.galley(pos2(rect.left() + 41.0, rect.center().y - g.size().y / 2.0), g, fg);
}

/// A small key cap ("Strg K").
pub fn kbd(ui: &Ui, p: &egui::Painter, right_center: egui::Pos2, text: &str, fg: Color32, bg: Color32) -> Rect {
    let g = line(ui, text, font(11.5, theme::SEMIBOLD), fg, 200.0);
    let size = g.size() + vec2(14.0, 4.0);
    let rect = Rect::from_min_size(pos2(right_center.x - size.x, right_center.y - size.y / 2.0), size);
    p.rect_filled(rect, CornerRadius::same(6), bg);
    p.galley(rect.min + vec2(7.0, 2.0), g, fg);
    rect
}

/// A round avatar with initials on the coral gradient.
pub fn avatar(p: &egui::Painter, center: egui::Pos2, d: f32, initials: &str) {
    let rect = Rect::from_center_size(center, vec2(d, d));
    crate::backdrop::vertical_gradient(p, rect, CornerRadius::same((d / 2.0) as u8), Color32::from_rgb(0xf0, 0x66, 0x3f), Color32::from_rgb(0xc2, 0x41, 0x7a));
    p.text(center, Align2::CENTER_CENTER, initials, font((d * 0.38).round(), theme::BOLD), Color32::WHITE);
}

/// A tile with a centred icon (suggestion cards, the artifact panel's symbol).
pub fn icon_tile(p: &egui::Painter, rect: Rect, radius: u8, fill: Color32, icon: Icon, size: f32, fg: Color32) {
    p.rect_filled(rect, CornerRadius::same(radius), fill);
    icon.paint(p, rect.center(), size, fg);
}

/// The initials of a name ("Julia Berger" → "JB").
pub fn initials(name: &str) -> String {
    let mut out: String = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(2).collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

/// A card frame of the designed looks: surface, hairline, radius and the soft shadow.
pub fn card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.card_stroke))
        .corner_radius(CornerRadius::same(t.r_card))
        .shadow(t.card_shadow)
        .inner_margin(egui::Margin::same(14))
}

#[cfg(test)]
mod tests {
    #[test]
    fn initials_of_names() {
        assert_eq!(super::initials("Julia Berger"), "JB");
        assert_eq!(super::initials("demo"), "D");
        assert_eq!(super::initials(""), "?");
        assert_eq!(super::initials("anne-marie von x"), "AV");
    }

    /// #123: a chat's title is shortened before the „…“ button, which stays inside the row.
    #[test]
    fn the_more_button_never_covers_the_title() {
        for w in [120.0, 236.0, 300.0] {
            let row = egui::Rect::from_min_size(egui::pos2(14.0, 100.0), egui::vec2(w, 32.0));
            let more = super::side_more_rect(row);
            assert!(row.contains_rect(more) && row.right() - more.right() >= 4.0, "{w}: inside the row");
            assert!(super::side_label_right(row, true) <= more.left() - 4.0, "{w}: the title ends before the button");
            assert!(super::side_label_right(row, false) > super::side_label_right(row, true), "{w}: without it the title has the room");
        }
    }
}
