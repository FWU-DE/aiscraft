//! Secrets (session cookies, API keys) live in the OS keychain under the backend id. Where no
//! keychain is reachable they go to a file only the owner can read, and the app says so.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use aiscraft_core::{Error, Result};

const SERVICE: &str = "aiscraft";

/// Windows' Credential Manager holds at most 2560 bytes per entry, and keyring stores the password
/// as UTF-16 there: 1280 code units. An AIS.chat session cookie is longer, so a long value goes into
/// numbered parts (`<id>#1`, `<id>#2`, …) with a marker under `<id>` that says how many there are.
/// The limit leaves room below the platform's; it applies on every system so one format reads back
/// everywhere. Values stored before this format are read unchanged.
const CHUNK_UTF16_UNITS: usize = 1000;
const CHUNK_MARKER: &str = "aiscraft-chunks:v1:";
/// A value over this many parts is refused (about 1 MB); nothing a backend sends us is that long.
const MAX_CHUNKS: usize = 1000;

/// The raw per-entry operations of a keychain, so the chunking can be tested without one.
trait RawStore {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

struct OsKeychain;

impl OsKeychain {
    fn entry(key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, key).map_err(|e| Error::storage(format!("Schlüsselbund: {e}")))
    }
}

impl RawStore for OsKeychain {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::storage(format!("Schlüsselbund: {e}"))),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        Self::entry(key)?.set_password(value).map_err(|e| Error::storage(format!("Schlüsselbund: {e}")))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::storage(format!("Schlüsselbund: {e}"))),
        }
    }
}

fn chunk_key(id: &str, n: usize) -> String {
    format!("{id}#{n}")
}

/// How many parts the head value of `id` announces; `None` for a plain value.
fn chunk_count(head: &str) -> Option<usize> {
    head.strip_prefix(CHUNK_MARKER)?.parse().ok()
}

/// Splits `value` at char boundaries into parts of at most `CHUNK_UTF16_UNITS` UTF-16 code units.
fn split_chunks(value: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut units = 0;
    for c in value.chars() {
        if units + c.len_utf16() > CHUNK_UTF16_UNITS {
            parts.push(std::mem::take(&mut part));
            units = 0;
        }
        part.push(c);
        units += c.len_utf16();
    }
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

fn chunked_get(store: &impl RawStore, id: &str) -> Result<Option<String>> {
    let Some(head) = store.get(id)? else { return Ok(None) };
    let Some(n) = chunk_count(&head) else { return Ok(Some(head)) };
    if n > MAX_CHUNKS {
        return Err(Error::storage(format!("Schlüsselbund: Eintrag „{id}“ ist beschädigt. Bitte neu anmelden.")));
    }
    let mut value = String::new();
    for i in 1..=n {
        match store.get(&chunk_key(id, i))? {
            Some(part) => value.push_str(&part),
            None => return Err(Error::storage(format!("Schlüsselbund: Teil {i} von {n} für „{id}“ fehlt. Bitte neu anmelden."))),
        }
    }
    Ok(Some(value))
}

fn chunked_set(store: &impl RawStore, id: &str, value: Option<&str>) -> Result<()> {
    let old_parts = match store.get(id)? {
        Some(head) => chunk_count(&head).unwrap_or(0).min(MAX_CHUNKS),
        None => 0,
    };
    let new_parts = match value {
        Some(v) if v.encode_utf16().count() > CHUNK_UTF16_UNITS => {
            let parts = split_chunks(v);
            if parts.len() > MAX_CHUNKS {
                return Err(Error::storage("Schlüsselbund: Der Wert ist zu lang zum Speichern."));
            }
            // Parts first, the head last: a crash in between leaves the old head, never a head
            // that points at missing parts.
            for (i, part) in parts.iter().enumerate() {
                store.set(&chunk_key(id, i + 1), part)?;
            }
            store.set(id, &format!("{CHUNK_MARKER}{}", parts.len()))?;
            parts.len()
        }
        Some(v) => {
            store.set(id, v)?;
            0
        }
        None => {
            store.delete(id)?;
            0
        }
    };
    for i in new_parts + 1..=old_parts {
        store.delete(&chunk_key(id, i))?;
    }
    Ok(())
}

pub enum Secrets {
    Keychain,
    File(PathBuf),
    Memory(Mutex<BTreeMap<String, String>>),
}

impl Secrets {
    pub fn memory() -> Self {
        Secrets::Memory(Mutex::new(BTreeMap::new()))
    }

    /// Prefer the keychain; fall back to `fallback` when it cannot be reached.
    pub fn detect(fallback: PathBuf) -> (Self, Option<String>) {
        match keyring::Entry::new(SERVICE, "__probe__").and_then(|e| match e.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e),
        }) {
            Ok(()) => (Secrets::Keychain, None),
            Err(e) => (
                Secrets::File(fallback.clone()),
                Some(format!("Schlüsselbund nicht erreichbar ({e}). Zugangsdaten liegen in {} (nur für dich lesbar).", fallback.display())),
            ),
        }
    }

    pub fn get(&self, id: &str) -> Result<Option<String>> {
        if let Ok(v) = std::env::var(env_name(id))
            && !v.is_empty()
        {
            return Ok(Some(v));
        }
        match self {
            Secrets::Keychain => chunked_get(&OsKeychain, id),
            Secrets::File(p) => Ok(read_file(p)?.remove(id)),
            Secrets::Memory(m) => Ok(m.lock().unwrap_or_else(PoisonError::into_inner).get(id).cloned()),
        }
    }

    pub fn set(&self, id: &str, value: Option<&str>) -> Result<()> {
        match self {
            Secrets::Keychain => chunked_set(&OsKeychain, id, value),
            Secrets::File(p) => {
                let mut all = read_file(p)?;
                match value {
                    Some(v) => all.insert(id.to_string(), v.to_string()),
                    None => all.remove(id),
                };
                write_file(p, &all)
            }
            Secrets::Memory(m) => {
                let mut m = m.lock().unwrap_or_else(PoisonError::into_inner);
                match value {
                    Some(v) => m.insert(id.to_string(), v.to_string()),
                    None => m.remove(id),
                };
                Ok(())
            }
        }
    }
}

/// `AISCRAFT_SECRET_<ID>` overrides the store (for CI and scripted use).
fn env_name(id: &str) -> String {
    format!("AISCRAFT_SECRET_{}", id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect::<String>())
}

fn read_file(p: &PathBuf) -> Result<BTreeMap<String, String>> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(serde_json::from_str(&s).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(Error::storage(format!("{}: {e}", p.display()))),
    }
}

fn write_file(p: &std::path::Path, all: &BTreeMap<String, String>) -> Result<()> {
    if let Some(dir) = p.parent() {
        crate::disk::create_dir(dir)?;
    }
    // Crash-safe replacement, staged with mode 0600 before any byte is written.
    crate::disk::write_private(p, &serde_json::to_vec_pretty(all)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trip_and_permissions() {
        let dir = std::env::temp_dir().join(format!("aiscraft-secrets-{}", std::process::id()));
        let p = dir.join("secrets.json");
        let s = Secrets::File(p.clone());
        s.set("b1", Some("geheim")).unwrap();
        assert_eq!(s.get("b1").unwrap().as_deref(), Some("geheim"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        s.set("b1", None).unwrap();
        assert_eq!(s.get("b1").unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A keychain with Windows' limit: at most 2560 bytes of UTF-16 per entry.
    #[derive(Default)]
    struct LimitedKeychain(Mutex<BTreeMap<String, String>>);

    impl RawStore for LimitedKeychain {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<()> {
            if value.encode_utf16().count() * 2 > 2560 {
                return Err(Error::storage("longer than platform limit of 2560 chars"));
            }
            self.0.lock().unwrap().insert(key.to_string(), value.to_string());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn long_secrets_fit_a_keychain_with_a_size_limit() {
        let kc = LimitedKeychain::default();
        // A session cookie as long as an Auth.js JWE, with a few non-BMP chars for good measure.
        let long: String = "eyJhbGciOiJkaXIiLCJlbmMiOiJBMjU2Q0JDLUhTNTEyIn0..".repeat(80) + "😀🎓ä";
        assert!(long.encode_utf16().count() > 1280);
        chunked_set(&kc, "ais-chat", Some(&long)).unwrap();
        assert_eq!(chunked_get(&kc, "ais-chat").unwrap().as_deref(), Some(long.as_str()));

        // A shorter value replaces it and leaves no stale parts behind.
        let mid = "x".repeat(1500);
        chunked_set(&kc, "ais-chat", Some(&mid)).unwrap();
        assert_eq!(chunked_get(&kc, "ais-chat").unwrap().as_deref(), Some(mid.as_str()));
        assert_eq!(kc.0.lock().unwrap().len(), 3);

        chunked_set(&kc, "ais-chat", Some("kurz")).unwrap();
        assert_eq!(chunked_get(&kc, "ais-chat").unwrap().as_deref(), Some("kurz"));
        assert_eq!(kc.0.lock().unwrap().len(), 1);

        chunked_set(&kc, "ais-chat", Some(&long)).unwrap();
        chunked_set(&kc, "ais-chat", None).unwrap();
        assert_eq!(chunked_get(&kc, "ais-chat").unwrap(), None);
        assert!(kc.0.lock().unwrap().is_empty());
    }

    #[test]
    fn a_broken_chunked_entry_says_to_sign_in_again() {
        let kc = LimitedKeychain::default();
        kc.set("b", &format!("{CHUNK_MARKER}3")).unwrap();
        kc.set("b#1", "a").unwrap();
        let err = chunked_get(&kc, "b").unwrap_err().to_string();
        assert!(err.contains("neu anmelden"), "{err}");
        kc.set("b", &format!("{CHUNK_MARKER}999999999")).unwrap();
        assert!(chunked_get(&kc, "b").is_err());
        // A value stored before chunking existed reads back unchanged.
        kc.set("alt", "sk-alt").unwrap();
        assert_eq!(chunked_get(&kc, "alt").unwrap().as_deref(), Some("sk-alt"));
    }
}
