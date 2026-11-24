//! The AIS.ChatCraft engine (layer 2). It owns the state, the store, the secrets and the backends,
//! and exposes one registry of commands. The egui shell, the CLI and the JSON control channel
//! all dispatch the same commands by id, so everything a person can do, an agent can do.

mod artifacts;
mod attach;
mod bridge;
mod chat;
#[cfg(feature = "collab")]
mod collab;
mod commands;
mod files;
mod inbox;
pub mod learn;
mod links;
pub mod office;
mod pictures;
pub mod privacy;
mod proactive;
mod schedules;
mod secrets;
mod skills;
mod state;
mod tasks;
pub mod tools;
mod update;
mod usage;
mod vault;
mod voice;

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
pub use artifacts::{change_blocks, doc_replace, doc_text, markdown_html, merge_states};
pub use attach::{AIS_ACCEPTS, AttachmentPreview, RangeKind};
pub use chat::{LEFT_OUT, SENT_ALONG};
pub use commands::{CommandSpec, commands};
pub use files::{MAX_FILE_BYTES, mime_of};
pub use inbox::InboxItem;
pub use links::{LINK_CHECK, has_links};
pub use schedules::{Cron, Schedule, next_run as schedule_next_run, parse_when};
pub use secrets::Secrets;
pub use skills::Skill;
pub use state::*;
pub use tools::{obsidian_vaults, presets as connector_presets};
pub use update::CURRENT_VERSION;
pub use vault::{EntryInfo as VaultEntry, totp_at};

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The person at this computer, for greetings when no sign-in (SSO) names them: the full name
/// of the OS account where the system has one, else the login name. Windows, Linux and macOS.
pub fn local_user_name() -> String {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        let login = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
        let run = |cmd: &str, args: &[&str]| -> Option<String> {
            let out = std::process::Command::new(cmd).args(args).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
        };
        let full = if cfg!(target_os = "macos") {
            run("id", &["-F"])
        } else if cfg!(windows) || login.is_empty() {
            None
        } else {
            // passwd: name:x:uid:gid:GECOS:home:shell; the full name is GECOS up to the first comma.
            run("getent", &["passwd", &login])
                .and_then(|l| l.split(':').nth(4).map(|g| g.split(',').next().unwrap_or("").trim().to_string()))
                .filter(|g| !g.is_empty())
        };
        full.unwrap_or(login)
    })
    .clone()
}

/// "04.12. 07:30" in local time.
pub fn format_local(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%d.%m. %H:%M").to_string()).unwrap_or_default()
}

/// Local weekday (0 = Monday), day of month, month (1–12) and hour of a moment.
pub fn local_date_parts(ms: i64) -> Option<(u32, u32, u32, u32)> {
    use chrono::{Datelike, TimeZone, Timelike};
    let t = chrono::Local.timestamp_millis_opt(ms).single()?;
    Some((t.weekday().num_days_from_monday(), t.day(), t.month(), t.hour()))
}

/// When something happened, as people say it: "heute, 10:42", "gestern" or "03.11.".
pub fn friendly_time(ms: i64, now: i64) -> String {
    use chrono::TimeZone;
    let (Some(t), Some(n)) = (chrono::Local.timestamp_millis_opt(ms).single(), chrono::Local.timestamp_millis_opt(now).single()) else {
        return String::new();
    };
    match (n.date_naive() - t.date_naive()).num_days() {
        0 => format!("heute, {}", t.format("%H:%M")),
        1 => "gestern".to_string(),
        _ => t.format("%d.%m.").to_string(),
    }
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
    tools: tools::ToolRuntime,
    bridge: bridge::Bridge,
    voice: voice::Voice,
    #[cfg(feature = "collab")]
    collab: collab::Hub,
    /// Copies of attached files, so they stay readable for later requests.
    pub(crate) attachments_dir: PathBuf,
    /// The browser profile for signing in to AIS.chat.
    pub(crate) browser_profile: PathBuf,
    /// May this engine start a browser? Only the real app (with a data folder) does; engines in
    /// memory (tests, scripts) never open windows or start browsers.
    pub(crate) browser_allowed: bool,
    /// Copies of learning apps and printed worksheets to open (`<data>/open`).
    pub(crate) open_dir: PathBuf,
    /// The HTML preview program (`aiscraft-preview`), set by the app when it is installed.
    pub(crate) preview: Mutex<Option<PathBuf>>,
    /// Running previews; they exit when their stdin (held here) closes.
    pub(crate) previews: Mutex<Vec<std::process::Child>>,
    /// Cached pictures of assistants, characters and scenarios (`<data>/personas`).
    pub(crate) pictures_dir: PathBuf,
    /// Per-backend cache kept across restarts (AIS.chat action ids, the persona list), so the
    /// app does not search the web app's JS and reload all lists on every start. None in tests.
    pub(crate) cache_dir: Option<PathBuf>,
    /// Engines in memory fetch pictures only from this local server (tests, the demo).
    pub(crate) picture_origin: Mutex<Option<String>>,
    /// Only the real app (with a data folder) fetches pictures from the network.
    pub(crate) pictures_online: bool,
    /// Test hook: the page reader may reach local addresses (the tests' scripted servers).
    /// Never set outside tests; the real app refuses loopback and private addresses.
    pub(crate) web_local_ok: std::sync::atomic::AtomicBool,
    /// Where the update check reads releases from; off for engines in memory.
    pub(crate) update_feed: Mutex<update::Feed>,
    /// Placeholder mappings of chats read so far (#55), so drawing never waits for the database.
    pub(crate) privacy_maps: Mutex<HashMap<String, Arc<privacy::Mapping>>>,
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
            tools: Default::default(),
            bridge: Default::default(),
            voice: Default::default(),
            #[cfg(feature = "collab")]
            collab: Default::default(),
            browser_allowed: opts.data_dir.is_some() && std::env::var_os("AISCRAFT_NO_BROWSER").is_none(),
            browser_profile: opts
                .data_dir
                .clone()
                .map(|d| d.join("browser-profile"))
                .unwrap_or_else(|| std::env::temp_dir().join(format!("aiscraft-browser-{}", std::process::id()))),
            picture_origin: Mutex::new(None),
            pictures_online: opts.data_dir.is_some(),
            web_local_ok: std::sync::atomic::AtomicBool::new(false),
            update_feed: Mutex::new(if opts.data_dir.is_some() && !cfg!(test) && std::env::var_os("AISCRAFT_NO_UPDATE_CHECK").is_none() {
                update::Feed::GitHub
            } else {
                update::Feed::Off
            }),
            cache_dir: opts.data_dir.clone().map(|d| d.join("cache")),
            pictures_dir: opts
                .data_dir
                .clone()
                .map(|d| d.join("personas"))
                .unwrap_or_else(|| std::env::temp_dir().join(format!("aiscraft-personas-{}", std::process::id()))),
            open_dir: opts
                .data_dir
                .clone()
                .map(|d| d.join("open"))
                .unwrap_or_else(|| std::env::temp_dir().join(format!("aiscraft-open-{}", std::process::id()))),
            preview: Mutex::new(None),
            privacy_maps: Mutex::new(HashMap::new()),
            previews: Mutex::new(Vec::new()),
            attachments_dir: opts
                .data_dir
                .clone()
                .map(|d| d.join("attachments"))
                .unwrap_or_else(|| std::env::temp_dir().join(format!("aiscraft-attachments-{}", std::process::id()))),
        }));
        ctx.load()?;
        if let Some(n) = secret_notice {
            ctx.notice(NoticeLevel::Warning, n);
        }
        let engine = Self { ctx, _rt: Arc::new(rt) };
        engine.ctx.spawn_startup_checks();
        {
            let _guard = engine._rt.enter();
            tools::load(&engine.ctx);
            let schedules = schedules::load(&engine.ctx);
            let skills = skills::load(&engine.ctx);
            let inbox = inbox::load(&engine.ctx);
            engine.ctx.update(|s| {
                s.schedules = schedules;
                s.skills = skills;
                s.inbox = inbox;
            });
            schedules::start(&engine.ctx);
            vault::refresh(&engine.ctx);
            privacy::refresh(&engine.ctx);
            update::start(&engine.ctx);
            #[cfg(feature = "collab")]
            collab::load(&engine.ctx);
        }
        Ok(engine)
    }

    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    /// Where the HTML preview program is (`aiscraft-preview`, feature `preview` of the app).
    /// Without it, learning apps open in the system browser.
    pub fn set_preview_program(&self, path: Option<PathBuf>) {
        *self.ctx.0.preview.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = path;
    }

    /// The current state. Cheap: an `Arc` that stays valid while the engine moves on.
    pub fn state(&self) -> Arc<State> {
        self.ctx.state()
    }

    /// The open chat's messages as people read them: placeholders of „Namen schützen“ translated
    /// back (#55). Drawing, copying and editing use these; the state keeps what the backend saw.
    pub fn shown_messages(&self, state: &State) -> Vec<aiscraft_core::Message> {
        privacy::shown_messages(&self.ctx, state)
    }

    /// Placeholders of chat `conv` in `text` translated back (titles, single texts).
    pub fn reveal(&self, conv: &str, text: &str) -> String {
        privacy::reveal(&self.ctx, conv, text)
    }

    /// The microphone level while dictating, 0..1.
    pub fn mic_level(&self) -> f32 {
        voice::level(&self.ctx)
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

    /// Run a command in the background without a notice on failure (pictures, prefetches).
    pub fn dispatch_quiet(&self, id: &str, params: Value) {
        let fut = self.ctx.run(id, params);
        let id = id.to_string();
        self.ctx.0.handle.spawn(async move {
            if let Err(e) = fut.await {
                log::debug!("{id}: {e}");
            }
        });
    }

    /// Fetch persona pictures from a local server instead of their own hosts, keeping path and
    /// query (tests and the offscreen demo). Only `http://127.0.0.1:…` and `http://localhost:…`.
    pub fn set_picture_origin(&self, origin: Option<&str>) -> Result<()> {
        let origin = match origin {
            Some(o) if o.starts_with("http://127.0.0.1:") || o.starts_with("http://localhost:") => Some(o.trim_end_matches('/').to_string()),
            Some(_) => return Err(Error::invalid("Nur ein lokaler Server (http://127.0.0.1:…) ist erlaubt")),
            None => None,
        };
        *self.ctx.0.picture_origin.lock().unwrap_or_else(PoisonError::into_inner) = origin;
        Ok(())
    }

    /// Let the page reader reach local addresses (tests and the offscreen demo read pages from
    /// their scripted servers). Only for engines in memory; the real app always refuses them.
    pub fn allow_local_web(&self) -> Result<()> {
        if self.ctx.0.pictures_online {
            return Err(Error::refused("Nur ohne Datenordner (Tests, Vorschau) erlaubt"));
        }
        self.ctx.0.web_local_ok.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    /// Hand the update check a release list (JSON as GitHub's `/releases` sends it) or the error
    /// it should report, instead of asking GitHub (tests and the offscreen demo). Only for
    /// engines in memory; the real app always asks GitHub.
    pub fn set_update_feed(&self, feed: std::result::Result<String, String>) -> Result<()> {
        if self.ctx.0.pictures_online {
            return Err(Error::refused("Nur ohne Datenordner (Tests, Vorschau) erlaubt"));
        }
        *self.ctx.0.update_feed.lock().unwrap_or_else(PoisonError::into_inner) = update::Feed::Scripted(feed);
        Ok(())
    }

    /// Where persona pictures are cached.
    pub fn pictures_dir(&self) -> PathBuf {
        self.ctx.0.pictures_dir.clone()
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

    /// Desktop notifications only from the real app; engines in memory (tests, scripts) stay quiet.
    pub(crate) fn desktop_allowed(&self) -> bool {
        self.0.browser_allowed
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
        let cached = self.read_backend_cache(&config.id);
        if let Some(actions) = cached.actions {
            backend.seed_actions(actions);
        }
        let caps = backend.capabilities();
        self.0.backends.lock().unwrap_or_else(PoisonError::into_inner).insert(config.id.clone(), backend);
        self.update(|s| {
            let models = s.backend(&config.id).map(|b| b.models.clone()).unwrap_or_default();
            let quota = s.backend(&config.id).and_then(|b| b.quota);
            let personas = s.backend(&config.id).and_then(|b| b.personas.clone()).or(cached.personas);
            let image_models = s.backend(&config.id).map(|b| b.image_models.clone()).unwrap_or_default();
            let view = BackendView {
                config: config.clone(),
                capabilities: caps,
                has_secret,
                status: BackendStatus::Unknown,
                models,
                quota,
                personas,
                personas_error: None,
                image_models,
            };
            match s.backends.iter_mut().find(|b| b.config.id == config.id) {
                Some(b) => *b = view,
                None => s.backends.push(view),
            }
        });
        Ok(())
    }

    fn cache_file(&self, backend: &str, what: &str) -> Option<PathBuf> {
        let safe: String = backend.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        self.0.cache_dir.as_ref().map(|d| d.join(format!("{safe}.{what}.json")))
    }

    /// What an earlier run remembered for a backend; nothing when missing, too big or unreadable.
    fn read_backend_cache(&self, backend: &str) -> BackendCache {
        fn read<T: serde::de::DeserializeOwned>(path: Option<PathBuf>, cap: u64) -> Option<T> {
            let path = path?;
            if std::fs::metadata(&path).ok()?.len() > cap {
                return None;
            }
            serde_json::from_slice(&std::fs::read(path).ok()?).ok()
        }
        BackendCache {
            actions: read::<std::collections::BTreeMap<String, String>>(self.cache_file(backend, "actions"), 256 * 1024)
                .map(|m| m.into_iter().take(200).filter(|(k, v)| k.len() <= 100 && v.len() <= 100).collect()),
            personas: read(self.cache_file(backend, "personas"), 16 * 1024 * 1024),
        }
    }

    /// Keep a backend's action ids and persona list for the next start. Failures only log: the
    /// cache is a speed-up, never needed.
    pub(crate) fn save_backend_cache(&self, backend: &str) {
        let Some(dir) = &self.0.cache_dir else { return };
        let Ok(b) = self.backend(backend) else { return };
        let actions = b.known_actions();
        let personas = self.state().backend(backend).and_then(|v| v.personas.clone());
        let mut files: Vec<(Option<PathBuf>, Vec<u8>)> = Vec::new();
        if !actions.is_empty() {
            files.push((self.cache_file(backend, "actions"), serde_json::to_vec(&actions).unwrap_or_default()));
        }
        if let Some(list) = personas {
            files.push((self.cache_file(backend, "personas"), serde_json::to_vec(&list).unwrap_or_default()));
        }
        if files.is_empty() {
            return;
        }
        if let Err(e) = std::fs::create_dir_all(dir) {
            log::warn!("Zwischenspeicher {}: {e}", dir.display());
            return;
        }
        for (path, bytes) in files {
            if let Some(path) = path
                && !bytes.is_empty()
                && std::fs::read(&path).ok().as_deref() != Some(&bytes[..])
                && let Err(e) = craft_atomic::atomic_write(&path, &bytes)
            {
                log::warn!("Zwischenspeicher {}: {e}", path.display());
            }
        }
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
            let kind = b.config.kind;
            self.spawn(async move {
                let r = commands::refresh_backend(&ctx, &id).await;
                // An expired AIS.chat session: sign in again without a window if the browser
                // profile still has the identity provider's session. Never opens a window.
                if kind == BackendKind::AisChat
                    && r.as_ref().is_err_and(|e| e.kind == aiscraft_core::ErrorKind::Auth)
                    && ctx.0.browser_profile.join("Default").is_dir()
                    && commands::browser_login(ctx.clone(), serde_json::json!({"id": id, "silent": true, "window": false})).await.is_ok()
                {
                    ctx.notice(NoticeLevel::Info, "AIS.chat-Sitzung automatisch erneuert");
                }
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

/// What an earlier run remembered for one backend.
#[derive(Default)]
struct BackendCache {
    actions: Option<std::collections::BTreeMap<String, String>>,
    personas: Option<Vec<aiscraft_core::PersonaInfo>>,
}
