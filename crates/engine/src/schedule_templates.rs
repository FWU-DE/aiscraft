//! Briefing templates for schedules (#27, Nod's call brief and price watch): „Jeden Morgen um 7:
//! Termine heute, offene Mails“ with one click. A template names what it needs (a calendar or
//! mail connector, a model that reads web pages) and says what is missing instead of creating a
//! schedule that cannot work.

use aiscraft_core::{Error, Result};
use serde::Serialize;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{opt_str, req_str};

/// Something a template needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// A connector with the owner's calendar.
    Calendar,
    /// A connector with the owner's mail.
    Mail,
    /// A model that can read web pages (the built-in reader needs tool calls).
    Web,
}

impl Need {
    pub fn label(self) -> &'static str {
        match self {
            Need::Calendar => "Kalender",
            Need::Mail => "E-Mail",
            Need::Web => "Webseiten lesen",
        }
    }

    /// Words in a connector's name or its tools that show it brings this.
    fn words(self) -> &'static [&'static str] {
        match self {
            Need::Calendar => &["calendar", "kalender", "termin", "event"],
            Need::Mail => &["mail", "outlook", "inbox", "postfach"],
            Need::Web => &[],
        }
    }

    /// What to do about it, for people.
    fn fix(self) -> &'static str {
        match self {
            Need::Calendar | Need::Mail => "einen Konnektor unter Einstellungen › Konnektoren hinzufügen",
            Need::Web => "ein Modell mit Werkzeugen wählen (etwa Claude über API oder einen OpenAI-kompatiblen Dienst) und Werkzeuge einschalten",
        }
    }
}

pub struct Template {
    pub id: &'static str,
    pub title: &'static str,
    /// `{url}` is replaced by the page the owner gives.
    pub prompt: &'static str,
    pub cron: &'static str,
    pub rhythm: &'static str,
    pub watch: bool,
    pub needs: &'static [Need],
}

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "morgen",
        title: "Morgenüberblick",
        prompt: "Gib mir einen kurzen Überblick für heute: meine Termine heute mit Uhrzeit und die Mails, die noch eine Antwort von mir brauchen. Als kurze Liste, das Dringende zuerst.",
        cron: "0 7 * * 1-5",
        rhythm: "werktags um 7 Uhr",
        watch: false,
        needs: &[Need::Calendar, Need::Mail],
    },
    Template {
        id: "woche",
        title: "Wochenvorschau",
        prompt: "Was steht in der kommenden Woche an? Nenne meine Termine Tag für Tag und alles, was ich vorbereiten muss.",
        cron: "0 18 * * 0",
        rhythm: "sonntags um 18 Uhr",
        watch: false,
        needs: &[Need::Calendar],
    },
    Template {
        id: "mails",
        title: "Mails des Tages",
        prompt: "Fasse die Mails zusammen, die seit gestern gekommen sind: die wichtigsten zuerst, je ein Satz, und was ich tun soll.",
        cron: "0 15 * * 1-5",
        rhythm: "werktags um 15 Uhr",
        watch: false,
        needs: &[Need::Mail],
    },
    Template {
        id: "vertretung",
        title: "Vertretungsplan beobachten",
        prompt: "Lies den Vertretungsplan unter {url} und nenne die Änderungen, die mich betreffen.",
        cron: "*/30 6-15 * * 1-5",
        rhythm: "werktags alle 30 Minuten von 6 bis 15 Uhr, meldet nur Änderungen",
        watch: true,
        needs: &[Need::Web],
    },
];

/// Whether this need is met now, for schedules with the selected service.
fn met(s: &crate::State, need: Need) -> bool {
    match need {
        Need::Web => s.settings.tools && s.backend(&s.selection.backend_id).is_some_and(|b| b.capabilities.web_search || b.capabilities.tools),
        _ => {
            s.settings.tools
                && s.connectors.iter().filter(|k| k.enabled).any(|k| {
                    let mut text = k.name.to_lowercase();
                    for t in &k.tools {
                        text.push(' ');
                        text.push_str(&format!("{} {} {}", t.name, t.title, t.description).to_lowercase());
                    }
                    need.words().iter().any(|w| text.contains(w))
                })
        }
    }
}

fn missing(s: &crate::State, t: &Template) -> Vec<Need> {
    t.needs.iter().copied().filter(|n| !met(s, *n)).collect()
}

/// A template as the settings show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TemplateView {
    pub id: &'static str,
    pub title: &'static str,
    pub prompt: &'static str,
    pub cron: &'static str,
    pub rhythm: &'static str,
    pub watch: bool,
    pub needs: Vec<&'static str>,
    /// What is missing, with what to do about it; empty when the template can be used.
    pub missing: Vec<String>,
    /// The prompt needs the address of a page.
    pub needs_url: bool,
}

/// Every template, with what is missing for it now.
pub fn template_views(s: &crate::State) -> Vec<TemplateView> {
    TEMPLATES
        .iter()
        .map(|t| TemplateView {
            id: t.id,
            title: t.title,
            prompt: t.prompt,
            cron: t.cron,
            rhythm: t.rhythm,
            watch: t.watch,
            needs: t.needs.iter().map(|n| n.label()).collect(),
            missing: missing(s, t).iter().map(|n| format!("{}: {}", n.label(), n.fix())).collect(),
            needs_url: t.prompt.contains("{url}"),
        })
        .collect()
}

pub(crate) async fn list(c: Ctx, _p: Value) -> Result<Value> {
    Ok(serde_json::to_value(template_views(&c.state()))?)
}

/// Create a schedule from a template; refuses, naming what is missing, when a need is not met.
pub(crate) async fn create(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "template")?;
    let t = TEMPLATES.iter().find(|t| t.id == id).ok_or_else(|| Error::not_found(format!("Unbekannte Vorlage {}", id.chars().take(40).collect::<String>())))?;
    let gaps = missing(&c.state(), t);
    if !gaps.is_empty() {
        let what: Vec<String> = gaps.iter().map(|n| format!("{}: {}", n.label(), n.fix())).collect();
        return Err(Error::invalid(format!("Für „{}“ fehlt noch etwas. {}.", t.title, what.join("; "))));
    }
    let mut prompt = opt_str(&p, "prompt").filter(|x| !x.trim().is_empty()).unwrap_or_else(|| t.prompt.to_string());
    if prompt.contains("{url}") {
        let url = opt_str(&p, "url").ok_or_else(|| Error::invalid(format!("Für „{}“ fehlt die Adresse der Seite (url)", t.title)))?;
        let url = crate::chat::check_url(&url)?;
        prompt = prompt.replace("{url}", &url);
    }
    crate::schedules::create(
        c,
        json!({
            "title": opt_str(&p, "title").filter(|x| !x.trim().is_empty()).unwrap_or_else(|| t.title.to_string()),
            "prompt": prompt,
            "cron": opt_str(&p, "cron").filter(|x| !x.trim().is_empty()).unwrap_or_else(|| t.cron.to_string()),
            "watch": t.watch,
        }),
    )
    .await
}
