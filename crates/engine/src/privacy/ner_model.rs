//! The GLiNER model through gline-rs on ONNX Runtime (engine feature `ner`, #73). ONNX Runtime
//! is loaded from the downloaded library (ort's `load-dynamic`), only from [`super::ner::model`],
//! which runs after the person installed it and the files' checksums were checked again.
//!
//! ort and gline-rs can panic inside (a library that does not load, an unexpected tensor); every
//! call into them runs under `catch_unwind`, so a broken model means "no model hits", never a
//! crash.

use std::path::Path;
use std::sync::Mutex;

use gliner::model::input::text::TextInput;
use gliner::model::params::Parameters;
use gliner::model::pipeline::span::SpanMode;
use gliner::model::GLiNER;
use orp::params::RuntimeParameters;

/// The label the model looks for. GLiNER is zero-shot; "person" is what the PII model was trained
/// with for names.
const LABELS: [&str; 1] = ["person"];
/// A piece of text per model input: GLiNER reads at most 384 words at a time.
const CHUNK_BYTES: usize = 1200;
const CHUNKS_PER_RUN: usize = 8;
/// A longer text is looked at up to here (the lists and rules still cover all of it).
const MAX_BYTES: usize = 200_000;

pub(crate) struct Model {
    /// ONNX Runtime sessions run one call at a time here; requests are short.
    gliner: Mutex<GLiNER<SpanMode>>,
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "unbekannter Fehler".into())
}

impl Model {
    pub(crate) fn load(library: &Path, tokenizer: &Path, model: &Path) -> std::result::Result<Model, String> {
        let lib = library.display().to_string();
        let r = std::panic::catch_unwind(|| -> std::result::Result<GLiNER<SpanMode>, String> {
            ort::init_from(lib.clone()).with_name("aiscraft-ner").commit().map_err(|e| e.to_string())?;
            let threads = std::thread::available_parallelism().map(|n| n.get().clamp(1, 4)).unwrap_or(2);
            GLiNER::<SpanMode>::new(Parameters::default(), RuntimeParameters::default().with_threads(threads), tokenizer, model).map_err(|e| e.to_string())
        });
        match r {
            Ok(Ok(g)) => Ok(Model { gliner: Mutex::new(g) }),
            Ok(Err(e)) => Err(format!("Die Namenserkennung ließ sich nicht starten: {e}")),
            Err(p) => Err(format!("Die Namenserkennung ließ sich nicht starten ({lib}): {}", panic_text(&*p))),
        }
    }

    /// Byte ranges of names in `text`.
    pub(crate) fn find(&self, text: &str) -> Vec<(usize, usize)> {
        let text = text.get(..floor(text, text.len().min(MAX_BYTES))).unwrap_or("");
        let chunks = chunks(text);
        let mut out = Vec::new();
        for batch in chunks.chunks(CHUNKS_PER_RUN) {
            let pieces: Vec<&str> = batch.iter().map(|(s, e)| text.get(*s..*e).unwrap_or("")).collect();
            let Ok(input) = TextInput::from_str(&pieces, &LABELS) else { continue };
            let g = self.gliner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.inference(input)));
            drop(g);
            let output = match r {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => {
                    log::warn!("Namenserkennung: {e}");
                    continue;
                }
                Err(p) => {
                    log::warn!("Namenserkennung: {}", panic_text(&*p));
                    continue;
                }
            };
            for (seq, spans) in output.spans.iter().enumerate() {
                let Some((base, end)) = batch.get(seq) else { continue };
                for s in spans {
                    let (a, b) = s.offsets();
                    let (a, b) = (base + a, base + b);
                    if a < b && b <= *end && text.is_char_boundary(a) && text.is_char_boundary(b) {
                        out.push((a, b));
                    }
                }
            }
        }
        out
    }
}

/// The largest char boundary at or below `i`.
fn floor(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Pieces of at most [`CHUNK_BYTES`], cut after a line or sentence end where possible, else at a
/// space, never inside a character.
fn chunks(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        // A character has at most 4 bytes, so this is always past `at`.
        let mut end = floor(text, (at + CHUNK_BYTES).min(text.len()));
        if end <= at {
            break;
        }
        if end < text.len() {
            let window = text.get(at..end).unwrap_or("");
            // The cut characters are ASCII: one byte, so `+ 1` stays on a boundary.
            if let Some(i) = window.rfind(['\n', '.', '!', '?']).filter(|i| *i > CHUNK_BYTES / 3).or_else(|| window.rfind(' ').filter(|i| *i > 0)) {
                end = at + i + 1;
            }
        }
        if text.get(at..end).is_some_and(|s| s.chars().any(char::is_alphabetic)) {
            out.push((at, end));
        }
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cover_the_text_on_char_boundaries() {
        let text = "Gestern war Klaus wieder unruhig. ".repeat(200) + &"ä".repeat(3000);
        let c = chunks(&text);
        assert!(c.len() > 4);
        assert_eq!(c.first().map(|x| x.0), Some(0));
        for w in c.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        assert!(c.iter().all(|(a, b)| a < b && b - a <= CHUNK_BYTES && text.is_char_boundary(*a) && text.is_char_boundary(*b)));
        assert!(chunks("").is_empty());
    }

    /// The real model, when installed: `AISCRAFT_NER_DIR=<data>/ner cargo test -p aiscraft-engine
    /// --features ner -- --ignored real_model`. Never downloads.
    #[test]
    #[ignore]
    fn real_model_finds_last_names_without_a_form_of_address() {
        let dir = std::path::PathBuf::from(std::env::var("AISCRAFT_NER_DIR").unwrap());
        let m = Model::load(&dir.join(super::super::ner::library_name()), &dir.join("tokenizer.json"), &dir.join("model.onnx")).unwrap();
        let text = "Gestern hat Weber im Unterricht gefehlt. Klaus war wieder unruhig.";
        let hits: Vec<&str> = m.find(text).into_iter().map(|(a, b)| &text[a..b]).collect();
        assert!(hits.contains(&"Weber"), "{hits:?}");
        assert!(hits.contains(&"Klaus"), "{hits:?}");
    }
}
