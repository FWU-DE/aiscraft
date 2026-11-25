//! „Browser übernehmen“ (#107): when the browser in the private computer needs the teacher
//! (sign-in, captcha, a confirmation), the task waits and this panel shows under the messages:
//! the live picture of the browser's virtual display, the teacher's clicks, scrolling and typing
//! going back to that browser, and „Zurück an AIS.ChatCraft“. Without a take-over nothing shows.

use aiscraft_engine::sandbox::browser::Takeover;
use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::{accent_button, small_muted};

/// The browser's viewport (Nod: 1440×900).
const ASPECT: f32 = 900.0 / 1440.0;
const MAX_HEIGHT: f32 = 520.0;

/// Nod's name for a key the panel passes on.
fn key_name(k: egui::Key) -> Option<&'static str> {
    use egui::Key;
    Some(match k {
        Key::Enter => "Enter",
        Key::Backspace => "Backspace",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Delete => "Delete",
        Key::ArrowUp => "ArrowUp",
        Key::ArrowDown => "ArrowDown",
        Key::ArrowLeft => "ArrowLeft",
        Key::ArrowRight => "ArrowRight",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        _ => return None,
    })
}

impl AisCraftApp {
    /// The panel of a take-over in the open chat.
    pub(crate) fn browser_takeover_panel(&mut self, ui: &mut Ui) {
        let Some(t) = self.state.sandbox.takeover.clone() else { return };
        if self.state.current.as_deref() != Some(t.conversation_id.as_str()) {
            return;
        }
        let tokens = self.tokens.clone();
        let mut inputs: Vec<Value> = Vec::new();
        let mut give_back = false;
        let shown = Frame::new()
            .fill(tokens.surface)
            .stroke(Stroke::new(1.5, tokens.accent))
            .corner_radius(CornerRadius::same(tokens.radius))
            .inner_margin(Margin::same(14))
            .show(ui, |ui| {
                ui.label(RichText::new("Browser: Du hast die Steuerung").strong());
                if !t.reason.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&t.reason).color(tokens.text)).wrap());
                }
                if !t.url.is_empty() {
                    ui.add(egui::Label::new(small_muted(&tokens, &t.url)).truncate());
                }
                ui.add_space(6.0);
                self.browser_picture(ui, &t, &mut inputs);
                if let Some(p) = &t.problem {
                    ui.label(RichText::new(p).small().color(tokens.warning));
                }
                ui.add(
                    egui::Label::new(small_muted(
                        &tokens,
                        "Klicke ins Bild, dann tippst du direkt im Browser. Solange du steuerst, wartet AIS.ChatCraft und tut nichts.",
                    ))
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if accent_button(ui, &tokens, "Zurück an AIS.ChatCraft").clicked() {
                        give_back = true;
                    }
                });
            });
        // The chat scrolls to the panel once per take-over.
        let scrolled = egui::Id::unique("browser-takeover-scrolled");
        let marker = format!("{}\n{}", t.conversation_id, t.reason);
        if ui.data(|d| d.get_temp::<String>(scrolled)).as_deref() != Some(marker.as_str()) {
            shown.response.scroll_to_me(Some(egui::Align::Center));
            ui.data_mut(|d| d.insert_temp(scrolled, marker));
        }
        for i in inputs {
            self.run("browser.input", i);
        }
        if give_back {
            self.run("browser.return", json!({}));
        }
    }

    /// The live picture, scaled to the panel, and what the teacher does on it.
    fn browser_picture(&mut self, ui: &mut Ui, t: &Takeover, inputs: &mut Vec<Value>) {
        let w = ui.available_width().max(1.0);
        let h = (w * ASPECT).min(MAX_HEIGHT);
        let size = egui::vec2(h / ASPECT, h);
        let tex_id = egui::Id::unique("browser-takeover-frame");
        let mut tex: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(tex_id));
        if tex.as_ref().is_none_or(|(n, _)| *n != t.frame)
            && let Some((n, bytes)) = self.engine.browser_frame()
            && let Some(img) = image::load_from_memory(&bytes).ok().filter(|i| i.width() <= 4096 && i.height() <= 4096)
        {
            let rgba = img.to_rgba8();
            let px = [rgba.width() as usize, rgba.height() as usize];
            let handle = ui.ctx().load_texture("browser-takeover", egui::ColorImage::from_rgba_unmultiplied(px, rgba.as_raw()), egui::TextureOptions::LINEAR);
            tex = Some((n, handle.clone()));
            ui.data_mut(|d| d.insert_temp(tex_id, (n, handle)));
        }
        let resp = match &tex {
            Some((_, handle)) => ui.add(egui::Image::new((handle.id(), size)).fit_to_exact_size(size).sense(Sense::click_and_drag())),
            None => {
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                ui.painter().rect_filled(rect, CornerRadius::same(6), self.tokens.code_bg);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Bild wird geladen …", egui::FontId::proportional(14.0), self.tokens.muted);
                resp
            }
        }
        .on_hover_cursor(egui::CursorIcon::PointingHand);
        let rect = resp.rect;
        if resp.clicked() || resp.drag_started() {
            resp.request_focus();
        }
        if resp.has_focus() {
            ui.memory_mut(|m| m.set_focus_lock_filter(resp.id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true }));
        }
        let frac = |p: egui::Pos2| ((p.x - rect.left()) / rect.width().max(1.0), (p.y - rect.top()) / rect.height().max(1.0));
        let focused = resp.has_focus();
        let events = ui.input(|i| i.events.clone());
        for e in events {
            match e {
                egui::Event::PointerButton { pos, button, pressed, .. } if rect.contains(pos) => {
                    let (x, y) = frac(pos);
                    let button = match button {
                        egui::PointerButton::Secondary => "right",
                        egui::PointerButton::Middle => "middle",
                        _ => "left",
                    };
                    inputs.push(json!({"type": "mouse", "event": if pressed { "down" } else { "up" }, "x": x.clamp(0.0, 1.0), "y": y.clamp(0.0, 1.0), "button": button}));
                }
                egui::Event::MouseWheel { delta, .. } if resp.hovered() => {
                    inputs.push(json!({"type": "wheel", "dx": (-delta.x * 40.0) as i64, "dy": (-delta.y * 40.0) as i64}));
                }
                egui::Event::Text(text) if focused && !text.is_empty() => inputs.push(json!({"type": "text", "text": text})),
                egui::Event::Key { key, pressed: true, .. } if focused => {
                    if let Some(k) = key_name(key) {
                        inputs.push(json!({"type": "key", "key": k}));
                    }
                }
                _ => {}
            }
        }
    }
}
