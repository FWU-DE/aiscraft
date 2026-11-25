//! Stage c with a model (#73): „Namenserkennung laden“. A GLiNER model finds names that are in
//! no list (last names without a form of address, rare first names). Owner's decision
//! (2026-11-25): download on demand, never bundled.
//!
//! - Nothing is downloaded and no native code is loaded until the person says yes in the
//!   settings (`privacy.ner_install` with `yes: true`); before that the settings show size,
//!   source and licence ([`offer`]).
//! - Fixed addresses with pinned SHA-256 checksums: the model and its tokenizer from a pinned
//!   commit on Hugging Face (checksums from the repository's LFS metadata), ONNX Runtime from
//!   Microsoft's `onnxruntime` package on PyPI (checksum of the wheel from PyPI's metadata, and
//!   of the library inside from the wheel's RECORD). A wheel is a ZIP: only the library is taken
//!   out of it. A wrong checksum keeps nothing.
//! - Everything lives in `<data>/ner`; the library and the model are checked again before they
//!   are loaded, once per run.
//! - The model runs only with the engine feature `ner` (gline-rs on ort with `load-dynamic`:
//!   ONNX Runtime is loaded from the downloaded file at run time, never linked or packaged).
//!   Without the feature the settings say so and nothing is offered.
//! - Hits are unsure like those of the first-name list: replaced by default, the preview lets the
//!   person keep one ([`super::detect::Detector::with_model`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{Ctx, NoticeLevel};

/// Finds names in a text: byte ranges. The real one is the GLiNER model; tests use their own.
pub type NameModel = Arc<dyn Fn(&str) -> Vec<(usize, usize)> + Send + Sync>;

/// Bump when the files change: an older installation is then offered again.
const LAYOUT: u32 = 1;
const MARKER: &str = "installed.json";
/// Hugging Face repository and commit of the model (an ONNX export of urchade/gliner_multi_pii-v1).
const MODEL_REPO: &str = "onnx-community/gliner_multi_pii-v1";
const MODEL_REV: &str = "2e0397a7e8a250d76c37122232b3cbde42c8d629";


/// One file to download.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Part {
    /// As people read it ("Modell", "Laufzeit").
    pub what: String,
    pub url: String,
    /// SHA-256 of what is downloaded, lower-case hex.
    pub sha256: String,
    pub size: u64,
    /// Name under `<data>/ner`.
    pub file: String,
    /// The download is a ZIP (a wheel): take only this entry out of it.
    pub extract: Option<String>,
    /// SHA-256 and size of the file kept (the entry), when it differs from the download.
    pub file_sha256: Option<String>,
    pub file_size: Option<u64>,
}

impl Part {
    fn kept_sha256(&self) -> &str {
        self.file_sha256.as_deref().unwrap_or(&self.sha256)
    }
}

fn hf(file: &str, sha256: &str, size: u64, to: &str, what: &str) -> Part {
    Part {
        what: what.into(),
        url: format!("https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REV}/{file}"),
        sha256: sha256.into(),
        size,
        file: to.into(),
        extract: None,
        file_sha256: None,
        file_size: None,
    }
}

/// The file name of the ONNX Runtime library on this system.
pub(crate) fn library_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// ONNX Runtime for this computer: Microsoft's wheel on PyPI (cp312; the library inside is the
/// same for every Python version). Checksums: PyPI's JSON API (`digests.sha256` of the wheel)
/// and the wheel's RECORD (the library). `None`: no package for this system.
fn runtime_part() -> Option<Part> {
    let (path, wheel_sha, wheel_size, entry, lib_sha, lib_size) = if cfg!(all(windows, target_arch = "x86_64")) {
        (
            "dd/80/76979e0b744307d488c79e41051117634b956612cc731f1028eb17ee7294/onnxruntime-1.20.1-cp312-cp312-win_amd64.whl",
            "19c2d843eb074f385e8bbb753a40df780511061a63f9def1b216bf53860223fb",
            11_331_482,
            "onnxruntime/capi/onnxruntime.dll",
            "c0926f3b9a7f166e13630f600d78c155562d808936ca3c3a9e0449c755ec7e39",
            13_557_792,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        (
            "47/42/2f71f5680834688a9c81becbe5c5bb996fd33eaed5c66ae0606c3b1d6a02/onnxruntime-1.20.1-cp312-cp312-manylinux_2_27_x86_64.manylinux_2_28_x86_64.whl",
            "bb71a814f66517a65628c9e4a2bb530a6edd2cd5d87ffa0af0f6f773a027d99e",
            13_333_903,
            "onnxruntime/capi/libonnxruntime.so.1.20.1",
            "be3dd32e78f6906fd4fa6208dbd296031fa1c3ffb42b40f706cf68d3de269f2b",
            16_559_416,
        )
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        (
            "c5/9d/a42a84e10f1744dd27c6f2f9280cc3fb98f869dd19b7cd042e391ee2ab61/onnxruntime-1.20.1-cp312-cp312-manylinux_2_27_aarch64.manylinux_2_28_aarch64.whl",
            "f1f56e898815963d6dc4ee1c35fc6c36506466eff6d16f3cb9848cea4e8c8172",
            11_952_833,
            "onnxruntime/capi/libonnxruntime.so.1.20.1",
            "421034f743da872315a6631ed02d8de6bf06273277f9bcc2de75770b1c2a5db6",
            14_041_600,
        )
    } else if cfg!(target_os = "macos") {
        (
            "e5/39/9335e0874f68f7d27103cbffc0e235e32e26759202df6085716375c078bb/onnxruntime-1.20.1-cp312-cp312-macosx_13_0_universal2.whl",
            "22b0655e2bf4f2161d52706e31f517a0e54939dc393e92577df51808a7edc8c9",
            31_007_580,
            "onnxruntime/capi/libonnxruntime.1.20.1.dylib",
            "f713f7fdd4201fee5c2867eb308ba67a36904fcf409a93bcbadf837df8386aa1",
            54_116_728,
        )
    } else {
        return None;
    };
    Some(Part {
        what: "Laufzeit".into(),
        url: format!("https://files.pythonhosted.org/packages/{path}"),
        sha256: wheel_sha.into(),
        size: wheel_size,
        file: library_name().into(),
        extract: Some(entry.into()),
        file_sha256: Some(lib_sha.into()),
        file_size: Some(lib_size),
    })
}

/// Everything „Namenserkennung laden“ downloads on this computer. Checksums of the model files:
/// `https://huggingface.co/api/models/onnx-community/gliner_multi_pii-v1/tree/main?recursive=true`
/// (`lfs.oid`), commit 2e0397a.
pub(crate) fn parts() -> Option<Vec<Part>> {
    Some(vec![
        hf("onnx/model_int8.onnx", "3efb3b91aef91ae11cd781126133063a33a2ffd7787ec73057bbd57a9781a7ab", 349_120_924, "model.onnx", "Modell"),
        hf("tokenizer.json", "914bd3c8fb7b525af9e23b60d0ec7b1248ddb2b99014efd9c02ebeb022f8cab7", 16_331_948, "tokenizer.json", "Wortliste des Modells"),
        runtime_part()?,
    ])
}

/// What the settings say before anything is downloaded.
pub const MODEL_TEXT: &str = "GLiNER multi PII v1 (Urchade Zaratiana; ONNX-Fassung von onnx-community, int8), erkennt Namen in Deutsch, Englisch, Französisch, Spanisch, Italienisch und Portugiesisch. Lizenz Apache-2.0. Quelle: huggingface.co/onnx-community/gliner_multi_pii-v1";
pub const RUNTIME_TEXT: &str =
    "ONNX Runtime 1.20.1 von Microsoft, führt das Modell auf diesem Rechner aus. Lizenz MIT. Quelle: pypi.org/project/onnxruntime (Paket von Microsoft)";

/// How the state shows the model (settings).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NerState {
    /// This build can run the model (engine feature `ner`).
    pub available: bool,
    /// There is a runtime for this computer.
    pub supported: bool,
    pub installed: bool,
    /// Bytes still to download for an installation.
    pub size: u64,
    /// Downloading: bytes so far and in all.
    pub progress: Option<(u64, u64)>,
    /// The model could not be loaded (named for the person), after an installation.
    pub problem: Option<String>,
}

enum Slot {
    NotLoaded,
    Loaded(NameModel),
    Failed(String),
}

/// What the engine holds for the model.
pub(crate) struct Runtime {
    /// `<data>/ner`; `None` for engines in memory (tests set their own).
    dir: Mutex<Option<PathBuf>>,
    /// Where downloads come from; `None`: this engine downloads nothing (tests, scripts).
    source: Mutex<Option<Vec<Part>>>,
    busy: std::sync::atomic::AtomicBool,
    model: Mutex<Slot>,
    /// Hits per text already looked at (a chat's history goes out with every request).
    cache: Mutex<HashMap<u64, Vec<(usize, usize)>>>,
}

const CACHE_ENTRIES: usize = 4096;

impl Runtime {
    pub(crate) fn new(data_dir: Option<&Path>) -> Runtime {
        let real = data_dir.is_some() && !cfg!(test);
        Runtime {
            dir: Mutex::new(data_dir.map(|d| d.join("ner"))),
            source: Mutex::new(if real { parts() } else { None }),
            busy: std::sync::atomic::AtomicBool::new(false),
            model: Mutex::new(Slot::NotLoaded),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn dir(&self) -> Option<PathBuf> {
        self.dir.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn source(&self) -> Option<Vec<Part>> {
        self.source.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// Test hook: downloads come from `parts` (a local server) into `dir`.
    #[cfg(test)]
    pub(crate) fn set_source(&self, dir: PathBuf, parts: Vec<Part>) {
        *self.dir.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(dir);
        *self.source.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(parts);
    }

    /// Test hook: this stands in for the model (no native code in tests).
    #[cfg(test)]
    pub(crate) fn set_model(&self, model: Option<NameModel>) {
        *self.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = match model {
            Some(m) => Slot::Loaded(m),
            None => Slot::NotLoaded,
        };
        self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    }
}

/// What is written last: an installation is complete only with it.
#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    layout: u32,
    files: Vec<(String, String)>,
}

fn read_marker(dir: &Path) -> Option<Marker> {
    let bytes = std::fs::read(dir.join(MARKER)).ok()?;
    let m: Marker = serde_json::from_slice(&bytes).ok()?;
    (m.layout == LAYOUT && m.files.iter().all(|(f, _)| dir.join(f).is_file())).then_some(m)
}

fn installed(c: &Ctx) -> bool {
    c.0.ner.dir().is_some_and(|d| read_marker(&d).is_some())
}

fn status(c: &Ctx) -> NerState {
    let rt = &c.0.ner;
    let parts = rt.source().or_else(parts);
    let problem = match &*rt.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
        Slot::Failed(e) => Some(e.clone()),
        _ => None,
    };
    NerState {
        available: cfg!(feature = "ner"),
        supported: parts.is_some(),
        installed: installed(c),
        size: parts.map(|p| p.iter().map(|x| x.size).sum()).unwrap_or(0),
        progress: c.state().privacy.ner.progress,
        problem,
    }
}

pub(crate) fn refresh(c: &Ctx) {
    let s = status(c);
    c.update(|st| st.privacy.ner = s);
}

fn mb(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1_000_000))
}

/// Size, source and licence, for the question before the download.
pub(crate) fn offer(c: &Ctx) -> Value {
    let s = status(c);
    json!({
        "available": s.available,
        "supported": s.supported,
        "installed": s.installed,
        "size": s.size,
        "size_text": mb(s.size),
        "model": MODEL_TEXT,
        "runtime": RUNTIME_TEXT,
        "folder": c.0.ner.dir().map(|d| d.display().to_string()),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
    }
    Ok(hex(&h.finalize()))
}

/// The newest progress shown at most every 4 MB.
fn show_progress(c: &Ctx, done: u64, total: u64) {
    c.update(|s| s.privacy.ner.progress = Some((done, total)));
}

/// Download one part into `dir` and check it. On any error nothing of it stays.
async fn fetch(c: &Ctx, client: &reqwest::Client, dir: &Path, part: &Part, before: u64, total: u64) -> Result<()> {
    let target = dir.join(&part.file);
    if target.is_file() && sha256_file(&target).ok().as_deref() == Some(part.kept_sha256()) {
        return Ok(());
    }
    let host = reqwest::Url::parse(&part.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    let net = |e: reqwest::Error| Error::network(format!("{} von {host} nicht geladen: {e}. Internetverbindung prüfen und noch einmal versuchen.", part.what));
    let mut resp = client.get(&part.url).send().await.map_err(net)?;
    if !resp.status().is_success() {
        return Err(Error::network(format!("{} von {host} nicht geladen: Antwort {}.", part.what, resp.status())));
    }
    if resp.content_length().is_some_and(|n| n != part.size) {
        return Err(Error::protocol(format!("{} von {host} hat nicht die erwartete Größe; nichts installiert.", part.what)));
    }
    let stage_name = if part.extract.is_some() { format!("{}.download", part.file) } else { part.file.clone() };
    let mut staged = crate::disk::Staged::create(&dir.join(&stage_name))?;
    let mut hash = Sha256::new();
    let mut got: u64 = 0;
    let mut shown: u64 = 0;
    while let Some(chunk) = resp.chunk().await.map_err(net)? {
        got += chunk.len() as u64;
        if got > part.size {
            return Err(Error::protocol(format!("{} von {host} ist größer als erwartet; nichts installiert.", part.what)));
        }
        hash.update(&chunk);
        staged.write(&chunk)?;
        if got - shown >= 4_000_000 {
            shown = got;
            show_progress(c, before + got, total);
        }
    }
    let sum = hex(&hash.finalize());
    if got != part.size || sum != part.sha256 {
        return Err(Error::protocol(format!(
            "Die Prüfsumme von „{}“ stimmt nicht (erwartet {}, erhalten {sum}). Die Datei wurde verworfen, nichts installiert.",
            part.what, part.sha256
        )));
    }
    match &part.extract {
        None => staged.finish(&target),
        Some(entry) => {
            let bytes = std::fs::read(staged.path()).map_err(|e| Error::io(staged.path().display(), &e))?;
            let max = part.file_size.unwrap_or(200_000_000);
            let mut zip = craft_zip::Archive::open(&bytes, craft_zip::Limits { max_entries: 10_000, max_total_bytes: max.saturating_add(1) })
                .map_err(|e| Error::protocol(format!("{}: kein lesbares Paket ({e})", part.what)))?;
            let lib = zip
                .read(entry, max)
                .map_err(|e| Error::protocol(format!("{}: {e}", part.what)))?
                .ok_or_else(|| Error::protocol(format!("{}: {entry} fehlt im Paket", part.what)))?;
            drop(staged);
            let kept = hex(&Sha256::digest(&lib));
            if kept != part.kept_sha256() || part.file_size.is_some_and(|n| n != lib.len() as u64) {
                return Err(Error::protocol(format!("Die Prüfsumme der Bibliothek in „{}“ stimmt nicht; nichts installiert.", part.what)));
            }
            crate::disk::write(&target, &lib)
        }
    }
}

/// Download and check every part into the folder, then mark the installation complete.
pub(crate) async fn install_parts(c: &Ctx, dir: &Path, parts: &[Part]) -> Result<()> {
    crate::disk::create_dir(dir)?;
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(60))
        .user_agent(concat!("AIS.ChatCraft/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::network(e.to_string()))?;
    let total: u64 = parts.iter().map(|p| p.size).sum();
    let mut before = 0;
    for p in parts {
        fetch(c, &client, dir, p, before, total).await?;
        before += p.size;
        show_progress(c, before, total);
    }
    let marker = Marker { layout: LAYOUT, files: parts.iter().map(|p| (p.file.clone(), p.kept_sha256().to_string())).collect() };
    crate::disk::write(&dir.join(MARKER), &serde_json::to_vec_pretty(&marker)?)
}

// ---- using the model ------------------------------------------------------------------------

/// The model for detection, loaded on first use after the person installed it (the yes was
/// given then). `None`: not installed, not available or it failed (the settings say why).
pub(crate) fn model(c: &Ctx) -> Option<NameModel> {
    let rt = &c.0.ner;
    {
        let slot = rt.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*slot {
            Slot::Loaded(m) => return Some(cached(c, m.clone())),
            Slot::Failed(_) => return None,
            Slot::NotLoaded => {}
        }
    }
    let dir = rt.dir()?;
    read_marker(&dir)?;
    // Checked against the pinned checksums, not against what the folder says about itself.
    let expected: Vec<(String, String)> = rt.source().or_else(parts)?.iter().map(|p| (p.file.clone(), p.kept_sha256().to_string())).collect();
    let loaded = load(&dir, &expected);
    let mut slot = rt.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    match loaded {
        Ok(m) => {
            *slot = Slot::Loaded(m.clone());
            drop(slot);
            Some(cached(c, m))
        }
        Err(e) => {
            log::warn!("Namenserkennung: {e}");
            *slot = Slot::Failed(e);
            drop(slot);
            refresh(c);
            None
        }
    }
}

/// The same text twice (a chat's history) runs the model once.
fn cached(c: &Ctx, m: NameModel) -> NameModel {
    let ctx = c.clone();
    Arc::new(move |text: &str| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let key = h.finish();
        if let Some(hit) = ctx.0.ner.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key) {
            return hit.clone();
        }
        let hits = m(text);
        let mut cache = ctx.0.ner.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= CACHE_ENTRIES {
            cache.clear();
        }
        cache.insert(key, hits.clone());
        hits
    })
}

/// Check the files again (they are about to run as native code) and load them.
fn load(dir: &Path, expected: &[(String, String)]) -> std::result::Result<NameModel, String> {
    for (file, sum) in expected {
        let got = sha256_file(&dir.join(file)).map_err(|e| format!("{file}: {e}"))?;
        if &got != sum {
            return Err(format!("„{file}“ wurde verändert. Bitte die Namenserkennung entfernen und neu laden."));
        }
    }
    load_model(dir)
}

#[cfg(feature = "ner")]
fn load_model(dir: &Path) -> std::result::Result<NameModel, String> {
    let m = super::ner_model::Model::load(&dir.join(library_name()), &dir.join("tokenizer.json"), &dir.join("model.onnx"))?;
    let m = Arc::new(m);
    Ok(Arc::new(move |text: &str| m.find(text)))
}

#[cfg(not(feature = "ner"))]
fn load_model(_dir: &Path) -> std::result::Result<NameModel, String> {
    Err("Diese Ausgabe von AIS.ChatCraft kann die Namenserkennung nicht ausführen.".into())
}

// ---- commands -------------------------------------------------------------------------------

/// Size, source, licence and whether it is installed.
pub(crate) async fn status_cmd(c: Ctx, _p: Value) -> Result<Value> {
    refresh(&c);
    Ok(offer(&c))
}

/// Downloads only with `yes: true`, the person's answer to the question with size, source and
/// licence. Without it: the question's content, nothing else.
pub(crate) async fn install(c: Ctx, p: Value) -> Result<Value> {
    if p.get("yes").and_then(Value::as_bool) != Some(true) {
        let mut o = offer(&c);
        if let Some(m) = o.as_object_mut() {
            m.insert("needs_yes".into(), json!(true));
        }
        return Ok(o);
    }
    if !cfg!(feature = "ner") && !cfg!(test) {
        return Err(Error::unsupported("Diese Ausgabe von AIS.ChatCraft kann die Namenserkennung nicht ausführen; es wird nichts geladen."));
    }
    let rt = &c.0.ner;
    let (Some(dir), Some(parts)) = (rt.dir(), rt.source()) else {
        return Err(Error::unsupported(if runtime_part().is_none() {
            "Für dieses System gibt es die Namenserkennung nicht."
        } else {
            "Diese Sitzung lädt nichts aus dem Internet (Test- oder Skriptmodus)."
        }));
    };
    if rt.busy.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::busy("Die Namenserkennung wird schon geladen."));
    }
    let r = install_parts(&c, &dir, &parts).await;
    rt.busy.store(false, std::sync::atomic::Ordering::SeqCst);
    c.update(|s| s.privacy.ner.progress = None);
    *rt.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Slot::NotLoaded;
    refresh(&c);
    r?;
    c.notice(NoticeLevel::Success, "Namenserkennung geladen. Sie läuft ab jetzt bei „Namen schützen“ auf diesem Rechner mit.");
    Ok(offer(&c))
}

/// Delete the downloaded files. A library already loaded stays in memory until the app restarts,
/// but is not used any more.
pub(crate) async fn remove(c: Ctx, _p: Value) -> Result<Value> {
    let rt = &c.0.ner;
    if rt.busy.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::busy("Die Namenserkennung wird gerade geladen."));
    }
    let removed = match rt.dir() {
        Some(dir) if dir.exists() => {
            // The marker first: a half-removed folder is never "installed".
            let _ = std::fs::remove_file(dir.join(MARKER));
            std::fs::remove_dir_all(&dir).map_err(|e| Error::io(dir.display(), &e))?;
            true
        }
        _ => false,
    };
    // Without the files `model` finds nothing to load; a new installation loads it again.
    *rt.model.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Slot::NotLoaded;
    rt.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
    refresh(&c);
    Ok(json!({"removed": removed}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_platform_part_is_pinned() {
        let parts = parts().unwrap_or_default();
        for p in &parts {
            assert!(p.url.starts_with("https://"), "{}", p.url);
            assert_eq!(p.sha256.len(), 64);
            assert!(p.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
            assert!(p.size > 0);
        }
        if let Some(rt) = runtime_part() {
            assert!(rt.extract.is_some() && rt.file_sha256.as_ref().is_some_and(|s| s.len() == 64));
            assert!(rt.url.contains("onnxruntime-1.20.1-"));
        }
        assert!(parts.iter().any(|p| p.url.contains(MODEL_REV)));
    }
}
