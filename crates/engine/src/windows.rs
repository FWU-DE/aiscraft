//! Contents in their own window (#75): a document, whiteboard, worksheet or H5P content lifted
//! out of the panel next to the chat. The engine keeps which contents are out, one window per
//! content, and where each window last stood; the UI opens the native windows (egui viewports in
//! the same process, no web engine). Learning apps stay in the preview window (`aiscraft-preview`).

use std::collections::BTreeMap;

use aiscraft_core::{ArtifactKind, Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::opt_str;

/// Where the windows stood, by artifact id (settings table).
const KEY: &str = "artifact_windows";
/// Places kept; the oldest go when there are more.
const MAX_PLACES: usize = 200;
/// Windows open at once.
const MAX_WINDOWS: usize = 16;

/// One content in its own window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetachedView {
    pub id: String,
    /// Goes up each time the window is asked to the front (a second „In eigenem Fenster“).
    pub focus: u64,
    /// Where the window stood last time, in points; `None` the first time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<WindowPlace>,
    /// Whiteboard presentation: full screen, no toolbar; Esc ends it.
    #[serde(default)]
    pub presenting: bool,
}

/// Position (outer, top left) and inner size of a window, in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// When it was stored (ms), to drop the oldest.
    #[serde(default)]
    pub at: i64,
}

fn places(c: &Ctx) -> BTreeMap<String, WindowPlace> {
    c.store(|st| Ok(st.setting(KEY)?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default())).unwrap_or_default()
}

/// The artifact a command is about: `id`, else (palette) for `detach` the one in the panel, for
/// `attach` the newest window.
fn target(c: &Ctx, p: &Value, newest_window: bool) -> Result<String> {
    let s = c.state();
    let id =
        opt_str(p, "id").filter(|i| !i.is_empty()).or_else(|| if newest_window { s.detached.last().map(|d| d.id.clone()) } else { s.open_artifact.clone() });
    let Some(id) = id else {
        return Err(Error::invalid(if newest_window {
            "Kein Inhalt ist in einem eigenen Fenster."
        } else {
            "Kein Dokument offen. Öffne zuerst ein Dokument oder Whiteboard."
        }));
    };
    if !s.artifacts.iter().any(|a| a.id == id) {
        return Err(Error::not_found("Artefakt nicht gefunden"));
    }
    Ok(id)
}

/// `artifact.detach`: into its own window; the panel closes. A second call for the same content
/// brings its window to the front (and switches the presentation on or off with `present`).
pub(crate) async fn detach(c: Ctx, p: Value) -> Result<Value> {
    let id = target(&c, &p, false)?;
    let kind = c.state().artifacts.iter().find(|a| a.id == id).map(|a| a.kind);
    if kind == Some(ArtifactKind::App) {
        return Err(Error::invalid("Lern-Apps öffnen im Vorschaufenster („▶ Öffnen“), nicht in einem eigenen Fenster."));
    }
    let present = p.get("present").and_then(Value::as_bool);
    if present == Some(true) && kind != Some(ArtifactKind::Board) {
        return Err(Error::invalid("Präsentieren geht nur mit einem Whiteboard."));
    }
    let place = places(&c).get(&id).copied();
    let out = c.update(|s| {
        if s.open_artifact.as_deref() == Some(&id) {
            s.open_artifact = None;
        }
        if let Some(d) = s.detached.iter_mut().find(|d| d.id == id) {
            d.focus = d.focus.wrapping_add(1);
            if let Some(p) = present {
                d.presenting = p;
            }
            return Ok(json!({"id": id, "already": true, "focus": d.focus, "presenting": d.presenting}));
        }
        if s.detached.len() >= MAX_WINDOWS {
            return Err(Error::invalid(format!("Höchstens {MAX_WINDOWS} eigene Fenster gleichzeitig. Schließe zuerst eines.")));
        }
        let presenting = present.unwrap_or(false);
        s.detached.push(DetachedView { id: id.clone(), focus: 0, place, presenting });
        Ok(json!({"id": id, "already": false, "focus": 0, "presenting": presenting}))
    })?;
    Ok(out)
}

/// `artifact.attach`: the window closes; the content goes back into the panel when its chat is
/// open (or it belongs to no chat and the panel is free), otherwise it simply stays saved.
pub(crate) async fn attach(c: Ctx, p: Value) -> Result<Value> {
    let id = target(&c, &p, true)?;
    let back = c.update(|s| {
        let before = s.detached.len();
        s.detached.retain(|d| d.id != id);
        if s.detached.len() == before {
            return Err(Error::invalid("Dieser Inhalt ist in keinem eigenen Fenster."));
        }
        let chat = s.artifacts.iter().find(|a| a.id == id).and_then(|a| a.conversation_id.clone());
        let back = match chat {
            Some(chat) => s.current.as_deref() == Some(chat.as_str()),
            None => s.open_artifact.is_none(),
        };
        if back {
            s.open_artifact = Some(id.clone());
        }
        Ok(back)
    })?;
    Ok(json!({"id": id, "in_panel": back}))
}

/// `artifact.window`: remember where the window stands, for the next time it opens.
pub(crate) async fn place(c: Ctx, p: Value) -> Result<Value> {
    let id = opt_str(&p, "id").filter(|i| !i.is_empty()).ok_or_else(|| Error::invalid("Parameter id (Text) fehlt"))?;
    if !c.state().artifacts.iter().any(|a| a.id == id) {
        return Err(Error::not_found("Artefakt nicht gefunden"));
    }
    let num = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
    let (Some(x), Some(y), Some(width), Some(height)) = (num("x"), num("y"), num("width"), num("height")) else {
        return Err(Error::invalid("Parameter x, y, width und height (Zahlen) fehlen"));
    };
    // Off by more than any screen is not a place, and a window needs room for its toolbar.
    let place = WindowPlace {
        x: x.clamp(-100_000.0, 100_000.0),
        y: y.clamp(-100_000.0, 100_000.0),
        width: width.clamp(360.0, 16_000.0),
        height: height.clamp(280.0, 16_000.0),
        at: crate::now_ms(),
    };
    let mut all = places(&c);
    all.insert(id.clone(), place);
    while all.len() > MAX_PLACES {
        let Some(oldest) = all.iter().min_by_key(|(_, p)| p.at).map(|(k, _)| k.clone()) else { break };
        all.remove(&oldest);
    }
    c.store(|st| st.set_setting(KEY, &serde_json::to_value(&all)?))?;
    c.update(|s| {
        if let Some(d) = s.detached.iter_mut().find(|d| d.id == id) {
            d.place = Some(place);
        }
    });
    Ok(json!({"id": id, "place": place}))
}

/// A deleted artifact's window closes, and its place is forgotten.
pub(crate) fn forget(c: &Ctx, id: &str) {
    c.update(|s| s.detached.retain(|d| d.id != id));
    let mut all = places(c);
    if all.remove(id).is_some() {
        let _ = c.store(|st| st.set_setting(KEY, &serde_json::to_value(&all)?));
    }
}
