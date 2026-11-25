//! The setup of „Namen schützen“ (#73): a step-by-step dialog the first time protection is
//! switched on (the switch next to „Quellen“, or a first class list in the settings), and again
//! from Einstellungen › Namen schützen („Einrichtung erneut durchlaufen“). What it does and what
//! stays on the device, class lists, the optional model download (only after an explicit yes),
//! the switch of the chat and what always happens, and a summary. Every step can be skipped;
//! „Einrichtung abbrechen“ puts the switch and the lists back as they were. Finishing calls
//! `privacy.onboarding_done`, so switching on later does not ask again. The engine does the work;
//! this only shows and dispatches commands.

use egui::{CornerRadius, Frame, Margin, RichText, Ui};
use serde_json::json;

use crate::app::AisCraftApp;
use crate::dialogs::{Dialog, SettingsTab};
use crate::widgets::{accent_button, card, link_button, muted, small_muted};

/// The steps, as their titles say them.
pub(crate) const STEPS: [&str; 5] =
    ["So schützt AIS.ChatCraft Namen", "Klassenlisten (freiwillig)", "Namenserkennung (freiwillig)", "Weitere Einstellungen", "Zusammenfassung"];

/// Where the setup stands, and what „Einrichtung abbrechen“ puts back.
#[derive(Debug, Clone, Default)]
pub struct PrivacySetup {
    pub step: usize,
    /// Names typed or pasted, one person per line, not saved yet.
    pub pasted: String,
    /// The pasted list is only for the open project.
    pub project_only: bool,
    /// The chat's switch before the setup: `None` automatic.
    before_protect: Option<bool>,
    /// The lists that existed before; lists added during the setup go again on cancel.
    before_lists: Vec<String>,
    /// Opened from the settings: back there when done.
    from_settings: bool,
}

/// What the setup ends with.
enum Close {
    Done,
    Cancel,
}

impl AisCraftApp {
    /// A setup that starts at `step`, remembering the switch and the lists as they are now.
    pub(crate) fn privacy_setup(&self, step: usize, from_settings: bool) -> Dialog {
        Dialog::PrivacySetup(Box::new(PrivacySetup {
            step: step.min(STEPS.len() - 1),
            before_protect: self.state.protect_choice(),
            before_lists: self.state.privacy.lists.iter().map(|l| l.id.clone()).collect(),
            from_settings,
            ..Default::default()
        }))
    }

    /// Puts back what the setup changed: the chat's switch and lists added meanwhile. A model
    /// download that started keeps going (it was asked for explicitly; „Entfernen“ deletes it).
    pub(crate) fn privacy_setup_cancel(&self, s: &PrivacySetup) {
        if self.state.protect_choice() != s.before_protect {
            self.run("chat.set_sources", json!({"protect": s.before_protect}));
        }
        for l in self.state.privacy.lists.iter().filter(|l| !s.before_lists.contains(&l.id)) {
            self.run("privacy.list_remove", json!({"id": l.id}));
        }
    }

    fn save_pasted(&self, s: &mut PrivacySetup) {
        if s.pasted.trim().is_empty() {
            return;
        }
        let project = self.state.selection.project_id.clone().filter(|_| s.project_only);
        // Without a project `null`: the list is for all chats.
        self.run("privacy.list_add", json!({"text": s.pasted, "name": "Eingefügte Namen", "project": project}));
        s.pasted.clear();
    }

    /// The dialog's body; returns whether it stays open.
    pub(crate) fn privacy_setup_body(&mut self, ui: &mut Ui, s: &mut PrivacySetup) -> bool {
        let t = self.tokens.clone();
        let last = STEPS.len() - 1;
        let step = s.step.min(last);
        ui.horizontal(|ui| {
            ui.label(crate::icons::Icon::Lock.text(15.0).color(t.accent));
            ui.label(small_muted(&t, format!("Namen schützen einrichten · Schritt {} von {}", step + 1, STEPS.len())));
        });
        // How far: one bar per step.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), egui::Sense::hover());
        let gap = 6.0;
        let w = (rect.width() - gap * (STEPS.len() as f32 - 1.0)) / STEPS.len() as f32;
        for i in 0..STEPS.len() {
            let x = rect.left() + i as f32 * (w + gap);
            let r = egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w.max(1.0), 4.0));
            ui.painter().rect_filled(r, CornerRadius::same(2), if i <= step { t.accent } else { t.opaque(t.border) });
        }
        ui.add_space(10.0);
        ui.label(crate::theme::bold(STEPS.get(step).copied().unwrap_or("")).size(24.0));
        ui.add_space(6.0);
        // The steps scroll; the buttons stay in view.
        let room = (ui.ctx().content_rect().height() - 260.0).max(160.0);
        egui::ScrollArea::vertical().id_salt("privacy-setup").max_height(room).auto_shrink([false, true]).show(ui, |ui| match step {
            0 => self.setup_intro(ui),
            1 => self.setup_lists(ui, s),
            2 => self.setup_model(ui),
            3 => self.setup_options(ui),
            _ => self.setup_summary(ui),
        });
        ui.add_space(16.0);
        let mut close = None;
        let mut go: Option<usize> = None;
        ui.horizontal(|ui| {
            if step == last {
                if accent_button(ui, &t, "Fertig").clicked() {
                    close = Some(Close::Done);
                }
            } else if accent_button(ui, &t, "Weiter").clicked() {
                if step == 1 {
                    self.save_pasted(s);
                }
                go = Some(step + 1);
            }
            if step > 0 && ui.button("Zurück").clicked() {
                go = Some(step - 1);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if link_button(ui, &t, "Einrichtung abbrechen").on_hover_text("Lässt „Namen schützen“, wie es vorher war").clicked() {
                    close = Some(Close::Cancel);
                }
                if step < last && link_button(ui, &t, "Überspringen").on_hover_text("Diesen Schritt auslassen").clicked() {
                    // Names typed in a skipped step are not saved.
                    s.pasted.clear();
                    go = Some(step + 1);
                }
            });
        });
        if let Some(n) = go {
            s.step = n.min(last);
        }
        match close {
            None => true,
            Some(how) => {
                match how {
                    Close::Done => self.run("privacy.onboarding_done", json!({})),
                    Close::Cancel => self.privacy_setup_cancel(s),
                }
                if s.from_settings {
                    self.dialog = Some(Dialog::Settings { tab: SettingsTab::Privacy, new_note: String::new() });
                }
                false
            }
        }
    }

    fn bullets(&self, ui: &mut Ui, lines: &[&str]) {
        let t = &self.tokens;
        for line in lines {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new("•").color(t.accent));
                ui.add(egui::Label::new(*line).wrap());
            });
        }
    }

    /// A lead in bold and its explanation, for things that always happen.
    fn point(&self, ui: &mut Ui, lead: &str, text: &str) {
        let t = &self.tokens;
        ui.add_space(6.0);
        ui.label(crate::theme::semibold(lead).color(t.text));
        ui.add(egui::Label::new(RichText::new(text).color(t.muted)).wrap());
    }

    fn info_box(&self, ui: &mut Ui, title: &str, text: &str) {
        let t = &self.tokens;
        Frame::new().fill(t.opaque(t.accent_soft)).corner_radius(CornerRadius::same(t.control_radius())).inner_margin(Margin::symmetric(14, 10)).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    ui.label(crate::icons::Icon::Lock.text(17.0).color(t.accent_ink));
                    ui.vertical(|ui| {
                        ui.label(crate::theme::semibold(title).color(t.accent_ink));
                        ui.add(egui::Label::new(RichText::new(text).color(t.accent_ink)).wrap());
                    });
                });
            },
        );
    }

    fn setup_intro(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(muted(
            &t,
            "Für Zeugnisbemerkungen, Elternbriefe und Förderpläne: Vor dem Senden ersetzt AIS.ChatCraft Namen und andere persönliche Angaben durch Platzhalter. Der Dienst (AIS.chat, Claude oder ein anderer Server) sieht nur die Platzhalter.",
        ));
        ui.add_space(10.0);
        card(&t).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(small_muted(&t, "Du schreibst"));
            ui.add(egui::Label::new("Lea Müller hat heute gefehlt. Mail an lea.mueller@example.org").wrap());
            ui.add_space(4.0);
            ui.label(small_muted(&t, "Raus geht"));
            ui.add(egui::Label::new(RichText::new("[S1] [N1] hat heute gefehlt. Mail an [E-Mail 1]").color(t.accent_ink)).wrap());
        });
        ui.add_space(10.0);
        ui.label(crate::theme::semibold("Was ersetzt wird"));
        self.bullets(
            ui,
            &[
                "Namen aus deinen Klassenlisten (nächster Schritt), auch einzeln und gebeugt.",
                "E-Mail-Adressen, Telefonnummern, IBAN, Geburtsdaten und Adressen.",
                "Häufige Vornamen und Namen nach einer Anrede („Frau Özdemir“), auch ohne Liste.",
                "Mit der Namenserkennung (freiwillig) auch Namen, die in keiner Liste stehen.",
            ],
        );
        ui.add_space(10.0);
        self.info_box(
            ui,
            "Was auf diesem Gerät bleibt",
            "Die Klassenlisten, die Zuordnung von Platzhalter zu Name (pro Chat, nur in der Datenbank auf diesem Gerät, in keinem Protokoll und nicht im JSON-Export) und die Namenserkennung, wenn du sie lädst. In der Antwort, beim Kopieren und beim Speichern stehen bei dir wieder die Namen.",
        );
    }

    fn setup_lists(&mut self, ui: &mut Ui, s: &mut PrivacySetup) {
        let t = self.tokens.clone();
        ui.label(muted(
            &t,
            "Mit einer Klassenliste erkennt AIS.ChatCraft die Namen deiner Schülerinnen und Schüler sicher: Vornamen und Nachnamen allein, ganze Namen, auch „Leas“ oder „Müller, Lea“. Aus „Lea Müller“ wird [S1] [N1]; dieselbe Person behält im ganzen Chat denselben Platzhalter. Eine Liste gilt für alle Chats oder nur für ein Projekt; sobald es eine passende gibt, ist der Schutz dort von selbst an. Die Liste bleibt auf diesem Gerät.",
        ));
        ui.add_space(8.0);
        for l in &self.state.privacy.lists {
            let scope = l
                .project_id
                .as_ref()
                .and_then(|p| self.state.projects.iter().find(|x| &x.id == p))
                .map(|p| format!("Projekt {}", p.name))
                .unwrap_or_else(|| "alle Chats".into());
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::icons::Icon::Users.text(15.0).color(t.accent));
                ui.label(RichText::new(&l.name).strong());
                ui.label(small_muted(&t, format!("{} Namen · {scope}", l.count)));
            });
        }
        if self.state.privacy.lists.is_empty() {
            ui.label(small_muted(&t, "Noch keine Klassenliste. Ohne Liste erkennt AIS.ChatCraft Kontaktdaten, Anreden und häufige Vornamen."));
        }
        ui.add_space(8.0);
        let project = self.state.selection.project_id.clone();
        let project_name = project.as_ref().and_then(|p| self.state.projects.iter().find(|x| &x.id == p)).map(|p| p.name.clone());
        if ui
            .button("Klassenliste einlesen …")
            .on_hover_text("CSV, Excel (xlsx, xls), OpenDocument (ods) oder Kontakte (vcf): Spalten Vorname und Nachname oder eine Person pro Zeile")
            .clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Klassenliste", &["csv", "tsv", "txt", "xlsx", "xls", "ods", "vcf"]).pick_file()
        {
            let project = project.as_ref().filter(|_| s.project_only);
            self.run("privacy.list_import", json!({"path": path.display().to_string(), "project": project}));
        }
        ui.add_space(8.0);
        ui.label(small_muted(&t, "Oder Namen einfügen, eine Person pro Zeile („Lea Müller“ oder „Müller, Lea“):"));
        ui.add(egui::TextEdit::multiline(&mut s.pasted).hint_text("Lea Müller\nSchmidt, Jonas").desired_rows(4).desired_width(f32::INFINITY));
        if let Some(name) = &project_name {
            ui.checkbox(&mut s.project_only, format!("Nur für Projekt {name}"));
        }
        if ui.add_enabled(!s.pasted.trim().is_empty(), egui::Button::new("Als Klassenliste speichern")).clicked() {
            self.save_pasted(s);
        }
    }

    fn setup_model(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        ui.label(muted(
            &t,
            "Ein Modell, das auch Namen erkennt, die in keiner Liste stehen, etwa Nachnamen ohne Anrede („Gestern hat Weber gefehlt“) oder seltene Vornamen. Es läuft auf diesem Rechner, ohne Internet. Geladen wird es nur, wenn du ausdrücklich ja sagst.",
        ));
        ui.add_space(8.0);
        self.ner_settings(ui, false);
        let s = &self.state.privacy.ner;
        if s.progress.is_some() {
            ui.add_space(6.0);
            ui.label(small_muted(&t, "Du kannst weitergehen, während es lädt. Der Stand steht auch unter Einstellungen › Namen schützen."));
        } else if s.available && s.supported && !s.installed {
            ui.add_space(6.0);
            ui.label(small_muted(&t, "Ohne Modell geht es auch: Überspringen lädt nichts. Später geht es unter Einstellungen › Namen schützen."));
        }
    }

    fn setup_options(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let mut choice = self.state.protect_choice();
        let before = choice;
        let lists = !self.state.privacy.lists.is_empty();
        ui.label(crate::theme::semibold(if self.state.current.is_some() { "In diesem Chat" } else { "Im nächsten neuen Chat" }));
        ui.add(
            egui::Label::new(RichText::new(if lists {
                "Automatisch: an, sobald eine Klassenliste für alle Chats oder für das Projekt des Chats da ist. Den Schalter „Namen schützen“ neben „Quellen“ gibt es in jedem Chat."
            } else {
                "Automatisch: an, sobald es eine passende Klassenliste gibt. Den Schalter „Namen schützen“ neben „Quellen“ gibt es in jedem Chat."
            }).color(t.muted))
            .wrap(),
        );
        ui.add_space(4.0);
        crate::widgets::segmented(ui, &t, &mut choice, [(Some(true), "An"), (None, "Automatisch"), (Some(false), "Aus")]);
        if choice != before {
            self.run("chat.set_sources", json!({"protect": choice}));
        }
        ui.add_space(8.0);
        ui.label(crate::theme::semibold("Was immer gilt"));
        self.point(
            ui,
            "Vorschau vor dem Senden",
            "Über dem Eingabefeld steht, was genau rausgeht; Ersetzungen sind markiert. Ein Klick auf einen Treffer lässt ihn stehen oder ersetzt ihn doch.",
        );
        self.point(
            ui,
            "Unsichere Treffer werden ersetzt",
            "Namen, die auch Wörter sind („Mark“, „Koch“), Namen in Kleinbuchstaben und Daten ohne „geb.“ gehen im Zweifel als Platzhalter raus. Antippen in der Vorschau lässt einen stehen.",
        );
        self.point(
            ui,
            "Regeln und Prüfsummen",
            "IBAN zählt nur mit gültiger Prüfsumme; Telefonnummern in deutschen Schreibweisen, Geburtsdaten mit „geb.“ oder im Schulalter, Adressen mit Straße und Hausnummer.",
        );
        self.point(ui, "+ Begriff ersetzen", "Einen Namen, den AIS.ChatCraft nicht erkennt, ergänzt du in der Vorschau; er wird in diesem Chat immer ersetzt.");
        self.point(
            ui,
            "Auch was mitgeht",
            "Systemanweisung, Gedächtnis, Projektdateien, angehängte Texte, Zeitpläne und Hintergrund-Aufgaben gehen ebenfalls geschützt raus. Bilder und PDF-Dateien, die als Datei hochgeladen werden, prüft AIS.ChatCraft nicht.",
        );
    }

    fn setup_summary(&mut self, ui: &mut Ui) {
        let t = self.tokens.clone();
        let lists = &self.state.privacy.lists;
        let names: usize = lists.iter().map(|l| l.count).sum();
        let lists_text = match lists.len() {
            0 => "keine; erkannt werden Kontaktdaten, Anreden und häufige Vornamen".to_string(),
            1 => format!("1 Liste mit {names} Namen"),
            n => format!("{n} Listen mit {names} Namen"),
        };
        let ner = &self.state.privacy.ner;
        let ner_text = if let Some((done, total)) = ner.progress {
            format!("wird geladen: {} von {} MB", done / 1_000_000, total.div_ceil(1_000_000))
        } else if ner.installed {
            "geladen und an".to_string()
        } else if !ner.available || !ner.supported {
            "in dieser Ausgabe nicht verfügbar".to_string()
        } else {
            "nicht geladen".to_string()
        };
        let on = self.state.protection_on();
        let switch_text = match self.state.protect_choice() {
            Some(true) => "an".to_string(),
            Some(false) => "aus".to_string(),
            None => format!("automatisch, gerade {}", if on { "an" } else { "aus" }),
        };
        let chat = if self.state.current.is_some() { "In diesem Chat" } else { "Im nächsten neuen Chat" };
        egui::Grid::new("privacy-setup-summary").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            for (k, v) in [
                ("Klassenlisten", lists_text.as_str()),
                ("Namenserkennung", ner_text.as_str()),
                (chat, switch_text.as_str()),
                ("Vorschau", "vor jedem Senden über dem Eingabefeld"),
            ] {
                ui.label(crate::theme::semibold(k));
                ui.add(egui::Label::new(v).wrap());
                ui.end_row();
            }
        });
        ui.add_space(10.0);
        ui.label(small_muted(
            &t,
            "Grenzen: Spitznamen, Tippfehler und Namen in Bildern erkennt AIS.ChatCraft nicht. Die Vorschau ist eine Hilfe, keine Garantie: Lies den Text vor dem Senden.",
        ));
        ui.add_space(4.0);
        ui.label(small_muted(
            &t,
            "Ändern lässt sich alles unter Einstellungen › Namen schützen; dort führt „Einrichtung erneut durchlaufen“ noch einmal durch diese Schritte.",
        ));
    }
}
