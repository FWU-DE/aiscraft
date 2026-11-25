//! Templates for recurring tasks (#54): a text with fields such as `{{Klasse}}` or `{{Thema}}`,
//! filled in a small form before sending, optionally aimed at an AIS.chat assistant. Kept on this
//! device (settings), never on a server. Filling only builds the text: it goes into the composer
//! for a look, or is sent at once when the person asks for that.

use aiscraft_core::{Error, Persona, PersonaKind, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, opt_str, req_str};
use crate::{Ctx, new_id, now_ms};

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "template.save",
        label: "Vorlage speichern",
        group: "Chat",
        shortcut: None,
        params: "id?: string (ändern), title: string, text: string (Felder als {{Klasse}} oder {{Niveau=mittel}}), persona?: {kind, id, name} (AIS.chat-Assistent als Ziel), backend?: string",
        palette: false,
        run: |c, p| Box::pin(async move { save(&c, &p) }),
    },
    CommandSpec {
        id: "template.delete",
        label: "Vorlage löschen",
        group: "Chat",
        shortcut: None,
        params: "id: string",
        palette: false,
        run: |c, p| Box::pin(async move { delete(&c, &p) }),
    },
    CommandSpec {
        id: "template.use",
        label: "Vorlage verwenden",
        group: "Chat",
        shortcut: None,
        params: "id: string, values: {Feld: Wert}, send?: bool (sonst ins Eingabefeld). Fehlende Felder werden genannt",
        palette: false,
        run: |c, p| Box::pin(use_template(c, p)),
    },
];

const KEY: &str = "templates";
/// Set once the starter templates were offered, so deleting them all keeps them gone.
const SEEDED: &str = "templates_seeded";
pub const MAX_TEMPLATES: usize = 100;
const MAX_TEXT: usize = 20_000;
const MAX_TITLE: usize = 80;
const MAX_FIELDS: usize = 20;
const MAX_VALUE: usize = 4000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub id: String,
    pub title: String,
    /// The text with fields in double braces: `{{Klasse}}`, or `{{Niveau=mittel}}` with a default.
    pub text: String,
    /// An assistant, character or learning scenario the filled text goes to (AIS.chat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<Persona>,
    /// The backend the persona belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// What was filled in last time, per field, offered again.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub last: std::collections::BTreeMap<String, String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One field of a template, in the order it first appears.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default: String,
}

impl Template {
    pub fn fields(&self) -> Vec<Field> {
        fields(&self.text)
    }
}

/// Text the engine puts into the composer (filled templates, connector results #53); `seq`
/// grows with every fill, so the UI takes each one once.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ComposerFill {
    pub seq: u64,
    pub text: String,
    /// Replace what is in the composer (templates) instead of adding to it (connector results).
    #[serde(default)]
    pub replace: bool,
}

/// Put `text` into the composer for a look before sending.
pub(crate) fn fill_composer(c: &Ctx, text: String, replace: bool) {
    c.update(|s| {
        let seq = s.composer_fill.as_ref().map_or(1, |f| f.seq + 1);
        s.composer_fill = Some(ComposerFill { seq, text, replace });
    });
}

/// The fields of a text, in order, each once: `{{Name}}` or `{{Name=Vorgabe}}`.
pub fn fields(text: &str) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let after = rest.get(start + 2..).unwrap_or("");
        let Some(end) = after.find("}}") else { break };
        let inner = after.get(..end).unwrap_or("");
        rest = after.get(end + 2..).unwrap_or("");
        let (name, default) = match inner.split_once('=') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (inner.trim(), ""),
        };
        if name.is_empty() || name.contains('{') || name.chars().count() > 40 {
            continue;
        }
        if !out.iter().any(|f| f.name.eq_ignore_ascii_case(name)) && out.len() < MAX_FIELDS {
            out.push(Field { name: name.to_string(), default: default.to_string() });
        }
    }
    out
}

/// The text with every field replaced by its value; fields without a value or default are listed.
pub fn fill(text: &str, values: &std::collections::BTreeMap<String, String>) -> std::result::Result<String, Vec<String>> {
    let value_of = |name: &str| values.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    let mut missing = Vec::new();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(rest.get(..start).unwrap_or(""));
        let after = rest.get(start + 2..).unwrap_or("");
        let Some(end) = after.find("}}") else {
            out.push_str(rest.get(start..).unwrap_or(""));
            rest = "";
            break;
        };
        let inner = after.get(..end).unwrap_or("");
        let (name, default) = match inner.split_once('=') {
            Some((n, d)) => (n.trim(), d.trim()),
            None => (inner.trim(), ""),
        };
        if name.is_empty() || name.contains('{') || name.chars().count() > 40 {
            out.push_str("{{");
            out.push_str(inner);
            out.push_str("}}");
        } else {
            match value_of(name).or_else(|| (!default.is_empty()).then(|| default.to_string())) {
                Some(v) => out.push_str(&v),
                None => {
                    if !missing.iter().any(|m: &String| m.eq_ignore_ascii_case(name)) {
                        missing.push(name.to_string());
                    }
                }
            }
        }
        rest = after.get(end + 2..).unwrap_or("");
    }
    out.push_str(rest);
    if missing.is_empty() { Ok(out) } else { Err(missing) }
}

/// Starters for a fresh profile; the person may change or delete them.
fn starters() -> Vec<Template> {
    let now = now_ms();
    let t = |title: &str, text: &str| Template {
        id: new_id(),
        title: title.into(),
        text: text.into(),
        persona: None,
        backend: None,
        last: Default::default(),
        created_at: now,
        updated_at: now,
    };
    vec![
        t(
            "Brief an die Eltern",
            "Schreibe einen freundlichen, gut verständlichen Elternbrief für die Klasse {{Klasse}} zum Anlass: {{Anlass}}. Wichtige Angaben: {{Angaben}}. Ton: {{Ton=freundlich und sachlich}}. Höchstens eine Seite.",
        ),
        t(
            "Unterrichtsstunde planen",
            "Erstelle einen Stundenentwurf für {{Fach}} in Klasse {{Klasse}} zum Thema {{Thema}}. Dauer: {{Dauer=45 Minuten}}. Niveau: {{Niveau=gemischt}}. Mit Lernziel, Ablauf in Phasen (Zeit, Inhalt, Sozialform, Material) und einer Idee zur Sicherung.",
        ),
        t(
            "Feedback zu einer Schülerarbeit",
            "Formuliere ein wertschätzendes, konkretes Feedback zu dieser Arbeit in {{Fach}} (Klasse {{Klasse}}). Kriterien: {{Kriterien}}. Nenne zwei Stärken und einen nächsten Schritt.\n\nArbeit:\n{{Text der Arbeit}}",
        ),
    ]
}

pub(crate) fn load(c: &Ctx) -> Vec<Template> {
    c.store(|st| {
        let saved: Option<Vec<Template>> = st.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok());
        match saved {
            Some(list) => Ok(list),
            None if st.setting(SEEDED)?.is_none() => {
                let list = starters();
                st.set_setting(KEY, &serde_json::to_value(&list)?)?;
                st.set_setting(SEEDED, &json!(true))?;
                Ok(list)
            }
            None => Ok(Vec::new()),
        }
    })
    .unwrap_or_default()
}

fn save_all(c: &Ctx, mut all: Vec<Template>) -> Result<()> {
    all.sort_by_key(|a| a.title.to_lowercase());
    c.store(|st| {
        st.set_setting(KEY, &serde_json::to_value(&all)?)?;
        st.set_setting(SEEDED, &json!(true))
    })?;
    c.update(|s| s.templates = all);
    Ok(())
}

fn persona_param(p: &Value) -> Result<Option<Persona>> {
    match p.get("persona") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let kind = v
                .get("kind")
                .and_then(Value::as_str)
                .and_then(PersonaKind::parse)
                .ok_or_else(|| Error::invalid("persona.kind muss assistant, character oder learning_scenario sein"))?;
            let id = v.get("id").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| Error::invalid("persona.id fehlt"))?;
            let name = v.get("name").and_then(Value::as_str).unwrap_or("").chars().take(MAX_TITLE).collect();
            Ok(Some(Persona { kind, id: id.to_string(), name }))
        }
    }
}

/// `template.save`: a new template, or changes to one (`id`).
pub(crate) fn save(c: &Ctx, p: &Value) -> Result<Value> {
    let title: String = req_str(p, "title")?.trim().chars().take(MAX_TITLE).collect();
    let text = req_str(p, "text")?;
    if title.is_empty() || text.trim().is_empty() {
        return Err(Error::invalid("Titel und Text der Vorlage dürfen nicht leer sein"));
    }
    if text.chars().count() > MAX_TEXT {
        return Err(Error::invalid("Der Text der Vorlage ist zu lang (höchstens 20 000 Zeichen)"));
    }
    let persona = persona_param(p)?;
    let backend = opt_str(p, "backend").filter(|b| !b.is_empty());
    let mut all = c.state().templates.clone();
    let now = now_ms();
    let id = match opt_str(p, "id").filter(|s| !s.is_empty()) {
        Some(id) => {
            let t = all.iter_mut().find(|t| t.id == id).ok_or_else(|| Error::not_found("Diese Vorlage gibt es nicht"))?;
            t.title = title;
            t.text = text;
            t.persona = persona;
            t.backend = backend;
            t.updated_at = now;
            id
        }
        None => {
            if all.len() >= MAX_TEMPLATES {
                return Err(Error::invalid("Höchstens 100 Vorlagen. Lösche eine, die du nicht mehr brauchst."));
            }
            let id = new_id();
            all.push(Template { id: id.clone(), title, text, persona, backend, last: Default::default(), created_at: now, updated_at: now });
            id
        }
    };
    let fields = all.iter().find(|t| t.id == id).map(Template::fields).unwrap_or_default();
    save_all(c, all)?;
    Ok(json!({"id": id, "fields": fields}))
}

pub(crate) fn delete(c: &Ctx, p: &Value) -> Result<Value> {
    let id = req_str(p, "id")?;
    let mut all = c.state().templates.clone();
    let before = all.len();
    all.retain(|t| t.id != id);
    if all.len() == before {
        return Err(Error::not_found("Diese Vorlage gibt es nicht"));
    }
    save_all(c, all)?;
    Ok(json!({}))
}

/// `template.use`: fill the fields; the text goes into the composer, or is sent (`send`), to the
/// template's assistant when it has one.
pub(crate) async fn use_template(c: Ctx, p: Value) -> Result<Value> {
    let id = req_str(&p, "id")?;
    let t = c.state().templates.iter().find(|t| t.id == id).cloned().ok_or_else(|| Error::not_found("Diese Vorlage gibt es nicht"))?;
    let mut values = std::collections::BTreeMap::new();
    if let Some(obj) = p.get("values").and_then(Value::as_object) {
        for (k, v) in obj.iter().take(MAX_FIELDS * 2) {
            let v = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            values.insert(k.chars().take(40).collect::<String>(), v.chars().take(MAX_VALUE).collect::<String>());
        }
    }
    let text = fill(&t.text, &values).map_err(|missing| Error::invalid(format!("Bitte ausfüllen: {}", missing.join(", "))))?;
    // Remember what was filled in, for next time (only fields the template has).
    let names: Vec<String> = t.fields().into_iter().map(|f| f.name).collect();
    let mut all = c.state().templates.clone();
    if let Some(x) = all.iter_mut().find(|x| x.id == id) {
        x.last = values.into_iter().filter(|(k, v)| !v.trim().is_empty() && names.iter().any(|n| n.eq_ignore_ascii_case(k))).collect();
    }
    save_all(&c, all)?;
    let send = p.get("send").and_then(Value::as_bool).unwrap_or(false);
    if let Some(persona) = &t.persona {
        // A new chat with the assistant; the text goes along at once or into the composer.
        let mut params = serde_json::Map::new();
        params.insert("kind".into(), json!(persona.kind.as_str()));
        params.insert("id".into(), json!(persona.id));
        if let Some(b) = &t.backend {
            params.insert("backend".into(), json!(b));
        }
        if send {
            params.insert("text".into(), json!(text));
        }
        let params = Value::Object(params);
        let out = c.run("persona.start", params).await?;
        if !send {
            fill_composer(&c, text.clone(), true);
        }
        return Ok(json!({"text": text, "sent": send, "chat": out}));
    }
    if send {
        let out = c.run("chat.send", json!({"text": text})).await?;
        return Ok(json!({"text": text, "sent": true, "chat": out}));
    }
    fill_composer(&c, text.clone(), true);
    Ok(json!({"text": text, "sent": false}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_found_once_in_order_with_defaults() {
        let f = fields("Klasse {{Klasse}}, {{ Thema }}, {{Niveau=mittel}}, wieder {{klasse}}, kaputt {{}} {{a{b}} und {{offen");
        let names: Vec<_> = f.iter().map(|x| (x.name.as_str(), x.default.as_str())).collect();
        assert_eq!(names, [("Klasse", ""), ("Thema", ""), ("Niveau", "mittel")]);
    }

    #[test]
    fn filling_names_what_is_missing_and_keeps_odd_braces() {
        let mut v = std::collections::BTreeMap::new();
        v.insert("klasse".to_string(), "7b".to_string());
        assert_eq!(fill("{{Klasse}} / {{Thema}} / {{Ton=nett}}", &v), Err(vec!["Thema".to_string()]));
        v.insert("Thema".to_string(), "Igel".to_string());
        assert_eq!(fill("{{Klasse}} / {{Thema}} / {{Ton=nett}} / {{}} / {{x", &v).unwrap(), "7b / Igel / nett / {{}} / {{x");
        // Hostile input never panics.
        let long = "{{".repeat(5000) + &"}}".repeat(10);
        let _ = fields(&long);
        let _ = fill(&long, &v);
        let _ = fill("ä{{ü}}ö", &v);
    }
}
