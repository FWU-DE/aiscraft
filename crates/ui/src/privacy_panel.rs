//! „Namen schützen“ in the composer (#55): the switch and the preview of exactly the text that
//! leaves the device, with each replacement marked. A hit is switched off or on with a click,
//! terms are added by hand. The engine finds and replaces; this only shows and asks.

use egui::text::{LayoutJob, TextFormat};
use egui::{CornerRadius, Frame, Margin, RichText, Stroke, Ui};
use serde_json::{Value, json};

use crate::app::AisCraftApp;
use crate::widgets::small_muted;

/// The person's choices for the next message.
#[derive(Debug, Default, Clone)]
pub(crate) struct Choices {
    /// Hits switched off or on, by lower-case text and occurrence.
    pub off: Vec<(String, usize)>,
    pub on: Vec<(String, usize)>,
    pub add: Vec<String>,
    /// The term being typed.
    pub term: String,
    pub term_open: bool,
    /// What the last preview was asked for.
    asked: u64,
}

impl Choices {
    pub fn params(&self) -> Value {
        let pairs = |v: &[(String, usize)]| v.iter().map(|(t, n)| json!({"text": t, "n": n})).collect::<Vec<_>>();
        json!({"off": pairs(&self.off), "on": pairs(&self.on), "add": self.add})
    }

    pub fn clear(&mut self) {
        *self = Choices::default();
    }

    fn toggle(&mut self, text: &str, n: usize, now_on: bool, ambiguous: bool) {
        let key = (text.to_lowercase(), n);
        self.off.retain(|k| *k != key);
        self.on.retain(|k| *k != key);
        // Back to the default (sure hits on, unsure ones off) or away from it.
        match (now_on, ambiguous) {
            (true, false) => self.off.push(key),
            (false, true) => self.on.push(key),
            _ => {}
        }
    }
}

fn hash(parts: &[&str]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

impl AisCraftApp {
    /// The switch next to „Quellen“.
    pub(crate) fn privacy_switch(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let on = self.state.protection_on();
        let label = if on { "Namen geschützt" } else { "Namen schützen" };
        let hint = if on {
            "Namen aus deinen Klassenlisten, E-Mail-Adressen, Telefonnummern, IBAN, Geburtsdaten und Adressen gehen als Platzhalter wie [S1] raus. Klicken schaltet es für diesen Chat aus."
        } else if self.state.privacy.lists.is_empty() {
            "Ersetzt E-Mail-Adressen, Telefonnummern, IBAN, Geburtsdaten und Adressen vor dem Senden durch Platzhalter. Namen erkennt es mit einer Klassenliste (in den Einstellungen unter „Namen schützen“)."
        } else {
            "Für diesen Chat ausgeschaltet. Klicken schaltet es wieder ein."
        };
        let r = if t.designed() {
            crate::kit::pill(ui, &t, if on { crate::kit::Pill::Soft } else { crate::kit::Pill::Chip }, Some(crate::icons::Icon::Lock), label, 13.5, None)
        } else {
            ui.add(egui::Button::selectable(on, RichText::new(label).color(if on { t.accent } else { t.muted })))
        };
        if r.on_hover_text(hint).clicked() {
            self.run("chat.set_sources", json!({"protect": !on}));
        }
    }

    /// The preview above the text field, while protection is on and there is text.
    pub(crate) fn privacy_preview(&mut self, ui: &mut Ui) {
        if !self.state.protection_on() || (self.composer.trim().is_empty() && self.privacy.add.is_empty()) {
            return;
        }
        let files: Vec<String> = self.pending_files.iter().map(|p| p.display().to_string()).collect();
        let params = self.privacy.params();
        let key = hash(&[&self.composer, &params.to_string(), &files.join("\n"), self.state.current.as_deref().unwrap_or("")]);
        if key != self.privacy.asked {
            self.privacy.asked = key;
            self.run("privacy.preview", json!({"text": self.composer, "files": files, "privacy": params}));
        }
        let Some(p) = self.state.privacy.preview.clone().filter(|p| p.active) else { return };
        let t = self.tokens.clone();
        let mut switch: Option<(String, usize, bool, bool)> = None;
        Frame::new().fill(t.surface_hi).corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
            let replaced = p.hits.iter().filter(|h| h.on).count();
            let unsure = p.hits.iter().filter(|h| h.ambiguous && h.on).count();
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Geht so raus").small().strong());
                let mut parts = vec![match replaced {
                    0 => "nichts ersetzt".to_string(),
                    1 => "1 Angabe ersetzt".to_string(),
                    n => format!("{n} Angaben ersetzt"),
                }];
                if unsure > 0 {
                    parts.push(format!("davon {unsure} unsicher: antippen, um es stehen zu lassen"));
                }
                ui.label(small_muted(&t, parts.join(" · ")));
            });
            if p.hits.is_empty() && p.added.is_empty() {
                ui.label(small_muted(&t, "Keine Namen oder Kontaktdaten gefunden."));
            } else {
                // The outgoing text, replacements marked.
                let size = 13.0;
                let mut job = LayoutJob::default();
                for seg in p.segments.iter() {
                    let hit = seg.hit.and_then(|i| p.hits.get(i));
                    let mut f = TextFormat { font_id: egui::FontId::proportional(size), color: t.text, ..Default::default() };
                    match hit {
                        Some(h) if h.on => {
                            f.background = t.accent_soft;
                            f.color = t.accent_ink;
                        }
                        Some(h) if h.ambiguous => {
                            f.underline = Stroke::new(1.5, t.warning);
                        }
                        Some(_) => {
                            f.strikethrough = Stroke::new(1.0, t.muted);
                            f.color = t.muted;
                        }
                        None => {}
                    }
                    job.append(&seg.text, 0.0, f);
                }
                job.wrap.max_width = ui.available_width();
                egui::ScrollArea::vertical().id_salt("privacy-preview").max_height(110.0).show(ui, |ui| {
                    ui.add(egui::Label::new(job).wrap().selectable(true));
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                    for h in &p.hits {
                        let nth = if h.n > 0 { format!(" ({}.)", h.n + 1) } else { String::new() };
                        let (text, fill, fg) = if h.on {
                            (format!("{}{nth} als {}", h.text, h.placeholder), t.accent_soft, t.accent_ink)
                        } else if h.ambiguous {
                            (format!("{}{nth}? {}", h.text, h.label), t.surface, t.warning)
                        } else {
                            (format!("{}{nth} bleibt", h.text), t.surface, t.muted)
                        };
                        let tip = if h.on {
                            format!("{} – geht als {} raus. Klicken: so lassen, wie es da steht.", h.label, h.placeholder)
                        } else if h.ambiguous {
                            format!("Vielleicht ein {} oder ein gewöhnliches Wort. Klicken: ersetzen.", h.label)
                        } else {
                            "Bleibt im Text. Klicken: doch ersetzen.".to_string()
                        };
                        let b = egui::Button::new(RichText::new(text).small().color(fg)).fill(fill).corner_radius(CornerRadius::same(10));
                        if ui.add(b).on_hover_text(tip).clicked() {
                            switch = Some((h.text.clone(), h.n, h.on, h.ambiguous));
                        }
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                for (i, a) in self.privacy.add.clone().iter().enumerate() {
                    if ui.small_button(format!("{a} ×")).on_hover_text("Von Hand ergänzt; klicken entfernt den Begriff.").clicked() {
                        self.privacy.add.remove(i);
                    }
                }
                if self.privacy.term_open {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.privacy.term).hint_text("Name oder Begriff").desired_width(160.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (enter || ui.small_button("Ersetzen").clicked()) && self.privacy.term.trim().chars().count() >= 2 {
                        let term = self.privacy.term.trim().to_string();
                        if !self.privacy.add.contains(&term) {
                            self.privacy.add.push(term);
                        }
                        self.privacy.term.clear();
                        self.privacy.term_open = false;
                    }
                } else if ui
                    .small_button("+ Begriff ersetzen")
                    .on_hover_text("Einen Namen, den AIS.ChatCraft nicht erkannt hat, in diesem Chat immer ersetzen")
                    .clicked()
                {
                    self.privacy.term_open = true;
                }
            });
            for f in &p.files {
                let note = if f.checked {
                    format!("{}: Text wird mit geprüft", f.name)
                } else {
                    format!("{}: nicht geprüft (Bild oder Datei wird so hochgeladen)", f.name)
                };
                ui.label(small_muted(&t, note));
            }
            ui.label(small_muted(&t, "Spitznamen, Tippfehler und Namen in Bildern erkennt AIS.ChatCraft nicht. Prüfe den Text."));
        });
        if let Some((text, n, on, ambiguous)) = switch {
            self.privacy.toggle(&text, n, on, ambiguous);
        }
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_toggle_back_and_forth() {
        let mut c = Choices::default();
        c.toggle("Lea", 0, true, false);
        assert_eq!(c.off, vec![("lea".to_string(), 0)]);
        c.toggle("Lea", 0, false, false);
        assert!(c.off.is_empty() && c.on.is_empty());
        c.toggle("Mark", 1, false, true);
        assert_eq!(c.params()["on"][0]["n"], 1);
    }
}
