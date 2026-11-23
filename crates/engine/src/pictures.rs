//! Pictures of assistants, characters and learning scenarios. AIS.chat signs its picture URLs
//! for a short while only, so the cache is keyed by backend, kind and id, never by URL, and a
//! file older than [`MAX_AGE`] is fetched again (the old one stays if that fails).

use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::time::Duration;

use aiscraft_core::{Error, PersonaKind, Result};
use serde_json::{Value, json};

use crate::commands::{opt_str, req_str};
use crate::{Ctx, picture_key};

const MAX_AGE: Duration = Duration::from_secs(12 * 3600);

/// At most this many downloads at once; a grid of cards asks for many.
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect()
}

fn fresh(path: &Path) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < MAX_AGE)
}

/// `https://host/path?q` → `<origin>/path?q`.
fn rebase(url: &str, origin: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let path = rest.find('/').and_then(|i| rest.get(i..)).unwrap_or("/");
    format!("{origin}{path}")
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| Error::storage("Bildordner fehlt"))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::storage(format!("Bildordner {}: {e}", dir.display())))?;
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes).map_err(|e| Error::storage(format!("Bild speichern: {e}")))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::storage(format!("Bild speichern: {e}")))
}

fn record(c: &Ctx, key: &str, path: Option<&PathBuf>) {
    let value = path.map(|p| p.display().to_string());
    c.update(|s| {
        s.persona_pictures.insert(key.to_string(), value);
    });
}

/// `persona.picture`: the cached picture file of one persona, fetched when missing or stale.
pub(crate) async fn picture(c: Ctx, p: Value) -> Result<Value> {
    let backend = opt_str(&p, "backend").unwrap_or_else(|| c.state().selection.backend_id.clone());
    let kind = PersonaKind::parse(&req_str(&p, "kind")?).ok_or_else(|| Error::invalid("kind muss assistant, character oder learning_scenario sein"))?;
    let id = req_str(&p, "id")?;
    let force = p.get("fresh").and_then(Value::as_bool).unwrap_or(false);
    let state = c.state();
    let info = state
        .backend(&backend)
        .and_then(|b| b.personas.as_ref())
        .and_then(|l| l.iter().find(|x| x.kind == kind && x.id == id))
        .cloned()
        .ok_or_else(|| Error::not_found(format!("{} nicht gefunden. Erst die Liste laden (persona.refresh).", kind.label())))?;
    let key = picture_key(&backend, kind, &info.id);
    let path = c.0.pictures_dir.join(format!("{}-{}-{}.img", safe(&backend), kind.as_str(), safe(&info.id)));
    let Some(url) = info.picture_url.clone() else {
        let _ = std::fs::remove_file(&path);
        record(&c, &key, None);
        return Ok(json!({"path": null}));
    };
    if !force && fresh(&path) {
        record(&c, &key, Some(&path));
        return Ok(json!({"path": path.display().to_string(), "cached": true}));
    }
    let origin = c.0.picture_origin.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let target = match origin {
        Some(o) => rebase(&url, &o),
        None if c.0.pictures_online => url,
        None => {
            if path.is_file() {
                record(&c, &key, Some(&path));
                return Ok(json!({"path": path.display().to_string(), "cached": true, "stale": true}));
            }
            record(&c, &key, None);
            return Err(Error::unsupported("Ohne Datenordner lädt AIS.ChatCraft keine Bilder aus dem Netz"));
        }
    };
    let fetched = {
        let _slot = SLOTS.acquire().await.map_err(|_| Error::new(aiscraft_core::ErrorKind::Cancelled, "Beendet"))?;
        aiscraft_backends::picture::download(&target).await
    };
    match fetched.and_then(|(bytes, ctype)| write_atomic(&path, &bytes).map(|()| (bytes.len(), ctype))) {
        Ok((n, ctype)) => {
            record(&c, &key, Some(&path));
            Ok(json!({"path": path.display().to_string(), "cached": false, "bytes": n, "content_type": ctype}))
        }
        Err(e) if path.is_file() => {
            log::debug!("Bild {key}: {e}");
            record(&c, &key, Some(&path));
            Ok(json!({"path": path.display().to_string(), "cached": true, "stale": true}))
        }
        Err(e) => {
            record(&c, &key, None);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebase_keeps_path_and_query() {
        assert_eq!(rebase("https://s3.example/b/avatar_1?X-Amz-Signature=a", "http://127.0.0.1:9"), "http://127.0.0.1:9/b/avatar_1?X-Amz-Signature=a");
        assert_eq!(rebase("https://s3.example", "http://127.0.0.1:9"), "http://127.0.0.1:9/");
        assert_eq!(safe("ais-chat/../x"), "ais-chat____x");
    }
}
