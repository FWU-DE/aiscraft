//! The AISCraft engine (layer 2). It owns the state, the store, the secrets and the backends,
//! and exposes one registry of commands. The egui shell, the CLI and the JSON control channel
//! all dispatch the same commands by id, so everything a person can do, an agent can do.

mod artifacts;
mod chat;
mod commands;
mod files;
mod secrets;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use aiscraft_backends::{Backend, SecretSink};
use aiscraft_core::{BackendConfig, BackendKind, Error, Result};
use aiscraft_store::Store;
use futures::future::BoxFuture;
use serde_json::Value;

pub use aiscraft_backends::SESSION_COOKIE;
pub use commands::{CommandSpec, commands};
pub use artifacts::{doc_text, markdown_html};
pub use files::{MAX_FILE_BYTES, mime_of};
pub use secrets::Secrets;
pub use state::*;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) struct Inner {
    handle: tokio::runtime::Handle,
    store: Mutex<Store>,
    state: Mutex<Arc<State>>,
    secrets: Secrets,
    backends: Mutex<HashMap<String, Arc<dyn Backend>>>,
    listeners: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
    revision: tokio::sync::watch::Sender<u64>,
    /// Per-conversation cancel signals of running generations.
    cancels: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    next_notice: std::sync::atomic::AtomicU64,
}

/// What commands and tasks hold. It never owns the runtime, so it can be dropped anywhere.
#[derive(Clone)]
pub struct Ctx(Arc<Inner>);

/// The handle surfaces hold. Owns the async runtime; create it outside of any runtime.
#[derive(Clone)]
pub struct Engine {
    ctx: Ctx,
    _rt: Arc<tokio::runtime::Runtime>,
}

pub struct Options {
    /// Where the database (and the fallback secrets file) live. `None` = in memory, for tests.
    pub data_dir: Option<PathBuf>,
    pub secrets: Option<Secrets>,
}

impl Options {
    pub fn default_dirs() -> Self {
        let dir = directories::ProjectDirs::from("ai", "uds", "aiscraft").map(|d| d.data_dir().to_path_buf());
        Self { data_dir: dir, secrets: None }
    }
}

impl Engine {
    pub fn open(opts: Options) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("aiscraft-engine")
            .build()
            .map_err(|e| Error::storage(format!("Laufzeit: {e}")))?;
        let (store, secrets, secret_notice) = match &opts.data_dir {
            Some(dir) => {
                let store = Store::open(&dir.join("aiscraft.db"))?;
                let (secrets, notice) = match opts.secrets {
                    Some(s) => (s, None),
                    None => Secrets::detect(dir.join("secrets.json")),
                };
                (store, secrets, notice)
            }
            None => (Store::open_in_memory()?, opts.secrets.unwrap_or_else(Secrets::memory), None),
        };
        let (tx, _) = tokio::sync::watch::channel(0);
        let ctx = Ctx(Arc::new(Inner {
            handle: rt.handle().clone(),
            store: Mutex::new(store),
            state: Mutex::new(Arc::new(State::default())),
            secrets,
            backends: Mutex::new(HashMap::new()),
            listeners: Mutex::new(Vec::new()),
            revision: tx,
            cancels: Mutex::new(HashMap::new()),
            next_notice: std::sync::atomic::AtomicU64::new(1),
        }));
        ctx.load()?;
        if let Some(n) = secret_notice {
            ctx.notice(NoticeLevel::Warning, n);
        }
        let engine = Self { ctx, _rt: Arc::new(rt) };
        engine.ctx.spawn_startup_checks();
        Ok(engine)
    }

    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// The current state. Cheap: an `Arc` that stays valid while the engine moves on.
    pub fn state(&self) -> Arc<State> {
        self.ctx.state()
    }

    /// Run a command and wait for it (CLI, control channel, tests).
    pub fn call(&self, id: &str, params: Value) -> Result<Value> {
        let fut = self.ctx.run(id, params);
        self._rt.block_on(fut)
    }

    /// Run a command in the background; failures become notices (UI).
    pub fn dispatch(&self, id: &str, params: Value) {
        let ctx = self.ctx.clone();
        let fut = ctx.run(id, params);
        let id = id.to_string();
        self.ctx.0.handle.spawn(async move {
            if let Err(e) = fut.await
                && e.kind != aiscraft_core::ErrorKind::Cancelled
            {
                log::warn!("{id}: {e}");
                ctx.notice(NoticeLevel::Error, e.message);
            }
        });
    }

    /// Call `f` after every state change (the UI asks egui to repaint).
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        self.ctx.0.listeners.lock().unwrap_or_else(PoisonError::into_inner).push(Box::new(f));
    }

    /// Block until `pred` holds for the state or the timeout passes. For CLI and tests.
    pub fn wait_for(&self, timeout: Duration, pred: impl Fn(&State) -> bool) -> bool {
        let mut rx = self.ctx.0.revision.subscribe();
        let ctx = self.ctx.clone();
        self._rt.block_on(async move {
            let deadline = tokio::time::Instant::now() + timeout;
            loop {
                if pred(&ctx.state()) {
                    return true;
                }
                if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
                    return pred(&ctx.state());
                }
            }
        })
    }

    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self._rt.block_on(f)
    }
}

impl Ctx {
    pub fn state(&self) -> Arc<State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub(crate) fn update<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let (r, rev) = {
            let mut guard = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
            let s = Arc::make_mut(&mut guard);
            let r = f(s);
            s.revision = s.revision.wrapping_add(1);
            (r, s.revision)
        };
        self.0.revision.send_replace(rev);
        for l in self.0.listeners.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            l();
        }
        r
    }

    pub(crate) fn store<R>(&self, f: impl FnOnce(&Store) -> Result<R>) -> Result<R> {
        let guard = self.0.store.lock().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }

    pub(crate) fn spawn<F>(&self, fut: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.0.handle.spawn(fut);
    }

    pub fn notice(&self, level: NoticeLevel, text: impl Into<String>) -> u64 {
        self.notice_with(level, text, None)
    }

    pub(crate) fn notice_with(&self, level: NoticeLevel, text: impl Into<String>, action: Option<NoticeAction>) -> u64 {
        let id = self.0.next_notice.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let text = text.into();
        self.update(|s| {
            s.notices.push(Notice { id, level, text, action });
            let excess = s.notices.len().saturating_sub(6);
            s.notices.drain(..excess);
        });
        if level != NoticeLevel::Error {
            let ctx = self.clone();
            self.spawn(async move {
                tokio::time::sleep(Duration::from_secs(if level == NoticeLevel::Warning { 12 } else { 6 })).await;
                ctx.update(|s| s.notices.retain(|n| n.id != id));
            });
        }
        id
    }

    pub(crate) fn backend(&self, id: &str) -> Result<Arc<dyn Backend>> {
        self.0.backends.lock().unwrap_or_else(PoisonError::into_inner).get(id).cloned().ok_or_else(|| Error::not_found(format!("Kein Backend {id}")))
    }

    pub(crate) fn secret_sink(&self) -> SecretSink {
        let ctx = self.clone();
        Arc::new(move |id: &str, secret: String| {
            if let Err(e) = ctx.0.secrets.set(id, Some(&secret)) {
                log::warn!("Sitzung für {id} nicht gespeichert: {e}");
            }
        })
    }

    /// (Re)build the live backend for a config and refresh its view.
    pub(crate) fn install_backend(&self, config: BackendConfig) -> Result<()> {
        let secret = self.0.secrets.get(&config.id)?;
        let has_secret = secret.is_some();
        let backend = aiscraft_backends::build(config.clone(), secret, self.secret_sink())?;
        let caps = backend.capabilities();
        self.0.backends.lock().unwrap_or_else(PoisonError::into_inner).insert(config.id.clone(), backend);
        self.update(|s| {
            let models = s.backend(&config.id).map(|b| b.models.clone()).unwrap_or_default();
            let view = BackendView { config: config.clone(), capabilities: caps, has_secret, status: BackendStatus::Unknown, models };
            match s.backends.iter_mut().find(|b| b.config.id == config.id) {
                Some(b) => *b = view,
                None => s.backends.push(view),
            }
        });
        Ok(())
    }

    fn load(&self) -> Result<()> {
        let (mut backends, conversations, projects, memory, settings, artifacts) = self.store(|st| {
            let settings: Settings = st.setting("settings")?.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            Ok((st.backends()?, st.conversations(500)?, st.projects()?, st.memory()?, settings, st.artifacts()?))
        })?;
        if backends.is_empty() {
            let ais = BackendConfig {
                id: "ais-chat".into(),
                name: "AIS.chat".into(),
                kind: BackendKind::AisChat,
                base_url: BackendKind::AisChat.default_base_url().into(),
                default_model: None,
                send_context: false,
            };
            self.store(|st| st.save_backend(&ais))?;
            backends.push(ais);
        }
        let selection = self.store(|st| Ok(st.setting("selection")?.and_then(|v| serde_json::from_value::<Selection>(v).ok())))?;
        self.update(|s| {
            s.conversations = conversations;
            s.projects = projects;
            s.memory = memory;
            s.settings = settings;
            s.artifacts = artifacts;
        });
        for b in &backends {
            if let Err(e) = self.install_backend(b.clone()) {
                self.notice(NoticeLevel::Warning, format!("{}: {e}", b.name));
            }
        }
        let first = backends.first().map(|b| b.id.clone()).unwrap_or_default();
        self.update(|s| {
            s.selection = selection.filter(|sel| s.backend(&sel.backend_id).is_some()).unwrap_or(Selection { backend_id: first, ..Default::default() });
        });
        Ok(())
    }

    fn spawn_startup_checks(&self) {
        for b in self.state().backends.iter().filter(|b| b.has_secret || matches!(b.config.kind, BackendKind::OpenAi | BackendKind::ClaudeCli)) {
            let ctx = self.clone();
            let id = b.config.id.clone();
            self.spawn(async move {
                let _ = commands::refresh_backend(&ctx, &id).await;
            });
        }
    }

    /// Run a command by id.
    pub fn run(&self, id: &str, params: Value) -> BoxFuture<'static, Result<Value>> {
        match commands::find(id) {
            Some(spec) => (spec.run)(self.clone(), params),
            None => {
                let id = id.to_string();
                Box::pin(async move { Err(Error::not_found(format!("Unbekannter Befehl: {id}"))) })
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn chat_system_prompt_for_test(c: &Ctx, conv: &aiscraft_core::Conversation, send: bool, takes: bool) -> Option<String> {
    chat::system_prompt_pub(c, conv, send, takes)
}
