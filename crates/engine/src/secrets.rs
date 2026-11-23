//! Secrets (session cookies, API keys) live in the OS keychain under the backend id. Where no
//! keychain is reachable they go to a file only the owner can read, and the app says so.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use aiscraft_core::{Error, Result};

const SERVICE: &str = "aiscraft";

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
            Secrets::Keychain => {
                let entry = keyring::Entry::new(SERVICE, id).map_err(|e| Error::storage(format!("Schlüsselbund: {e}")))?;
                match entry.get_password() {
                    Ok(v) => Ok(Some(v)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(Error::storage(format!("Schlüsselbund: {e}"))),
                }
            }
            Secrets::File(p) => Ok(read_file(p)?.remove(id)),
            Secrets::Memory(m) => Ok(m.lock().unwrap_or_else(PoisonError::into_inner).get(id).cloned()),
        }
    }

    pub fn set(&self, id: &str, value: Option<&str>) -> Result<()> {
        match self {
            Secrets::Keychain => {
                let entry = keyring::Entry::new(SERVICE, id).map_err(|e| Error::storage(format!("Schlüsselbund: {e}")))?;
                match value {
                    Some(v) => entry.set_password(v).map_err(|e| Error::storage(format!("Schlüsselbund: {e}"))),
                    None => match entry.delete_credential() {
                        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                        Err(e) => Err(Error::storage(format!("Schlüsselbund: {e}"))),
                    },
                }
            }
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

fn write_file(p: &PathBuf, all: &BTreeMap<String, String>) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::storage(e.to_string()))?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(all)?).map_err(|e| Error::storage(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| Error::storage(e.to_string()))?;
    }
    std::fs::rename(&tmp, p).map_err(|e| Error::storage(e.to_string()))
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
}
