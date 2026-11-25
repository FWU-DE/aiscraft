//! What reaches beyond AIS.ChatCraft's own data: local files (#25). Everything from there is
//! untrusted.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

/// Runtime state of the extensions, held by the engine.
pub(crate) struct Runtime {
    /// The app's data folder; `None` for engines in memory (tests, scripts).
    pub(crate) data_dir: Option<PathBuf>,
    /// Test hook: where local files may work instead of the home folder and external drives.
    local_roots: Mutex<Option<Vec<PathBuf>>>,
}

impl Runtime {
    pub(crate) fn new(data_dir: Option<PathBuf>) -> Self {
        Self { data_dir, local_roots: Mutex::new(None) }
    }

    pub(crate) fn local_roots(&self) -> Option<Vec<PathBuf>> {
        self.local_roots.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    #[cfg(test)]
    pub(crate) fn set_local_roots(&self, roots: Vec<PathBuf>) {
        *self.local_roots.lock().unwrap_or_else(PoisonError::into_inner) = Some(roots);
    }
}

/// Load what the extensions remember (at start).
pub(crate) fn load(c: &crate::Ctx) {
    crate::local_files::load(c);
}
