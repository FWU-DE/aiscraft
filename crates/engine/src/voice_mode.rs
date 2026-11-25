//! Voice mode (#40): a hands-free conversation. Listen until the person stops speaking, write
//! down what was said, send it, read the answer aloud, listen again, until the person ends it
//! (button, `voice.mode {on: false}`, or saying „Sprachmodus beenden“). Dictation and reading
//! aloud exist separately; this joins them into one loop.
//!
//! The end of a turn comes from the microphone level: speech, then a pause. The loop counts level
//! readings, so tests drive it with test devices that play back a list of levels; tests never
//! touch a real microphone or speakers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{Ctx, NoticeLevel};

/// A level above this counts as speech.
const SPEECH_LEVEL: f32 = 0.04;
/// Readings of quiet after speech that end a turn (about 1.2 s).
const SILENCE_READINGS: u32 = 12;
/// Readings without any speech before the turn starts over (about 15 s).
const IDLE_READINGS: u32 = 150;
/// Longest turn, in readings (about 60 s).
const MAX_READINGS: u32 = 600;
/// Turns in a row without speech that end the voice mode.
const MAX_IDLE_TURNS: u32 = 3;
const POLL: Duration = Duration::from_millis(100);

pub(crate) static COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: "voice.mode",
        label: "Sprachmodus",
        group: "Chat",
        shortcut: None,
        params: "on?: bool (Standard: an). Zuhören, antworten, vorlesen, wieder zuhören, bis du beendest",
        palette: true,
        run: |c, p| Box::pin(async move { set(&c, &p) }),
    },
    CommandSpec {
        id: "voice.mode_next",
        label: "Sprachmodus: weiter",
        group: "Chat",
        shortcut: None,
        params: "Beim Zuhören: ich bin fertig, jetzt senden. Beim Vorlesen: aufhören und wieder zuhören",
        palette: false,
        run: |c, _| Box::pin(async move { next(&c) }),
    },
];

/// The audio devices the loop uses: the real ones in the app, test devices in tests.
pub(crate) trait Devices: Send + Sync {
    fn start(&self) -> Result<()>;
    fn level(&self) -> f32;
    /// The recording as WAV.
    fn stop(&self) -> Result<Vec<u8>>;
    fn cancel(&self);
    fn play(&self, audio: Vec<u8>) -> Result<()>;
    fn playing(&self) -> bool;
    fn stop_playing(&self);
    /// Check the level every millisecond instead of every 100 ms (scripted test devices).
    fn fast(&self) -> bool {
        false
    }
}

/// Devices for UI tests: a microphone that hears nothing, speakers that play nothing.
struct Silent;

impl Devices for Silent {
    fn start(&self) -> Result<()> {
        Ok(())
    }
    fn level(&self) -> f32 {
        0.0
    }
    fn stop(&self) -> Result<Vec<u8>> {
        aiscraft_voice::encode_wav(&[], 16_000, 1)
    }
    fn cancel(&self) {}
    fn play(&self, _: Vec<u8>) -> Result<()> {
        Ok(())
    }
    fn playing(&self) -> bool {
        false
    }
    fn stop_playing(&self) {}
}

impl crate::Engine {
    /// UI tests: the voice mode uses devices that hear and play nothing, never a real
    /// microphone or speakers. Only for engines without a data folder.
    pub fn use_test_audio(&self) -> Result<()> {
        if self.ctx().0.pictures_online {
            return Err(Error::refused("Nur ohne Datenordner (Tests, Vorschau) erlaubt"));
        }
        *self.ctx().0.voice.test_devices.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(Silent));
        Ok(())
    }
}

/// The voice mode as the UI shows it; `None` in the state when it is off.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceModeView {
    /// "listening", "thinking", "speaking".
    pub phase: String,
    /// What was understood last.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub heard: String,
    /// Turns sent so far.
    pub turns: u32,
}

impl VoiceModeView {
    pub fn label(&self) -> &'static str {
        match self.phase.as_str() {
            "listening" => "Ich höre zu …",
            "thinking" => "Einen Moment …",
            "speaking" => "Ich lese vor …",
            _ => "Sprachmodus",
        }
    }
}

#[derive(Default)]
pub(crate) struct Signals {
    stop: AtomicBool,
    next: AtomicBool,
    /// Readings between level checks; tests make it short.
    pub(crate) fast: AtomicBool,
}

fn devices(c: &Ctx) -> Result<Arc<dyn Devices>> {
    if let Some(d) = c.0.voice.test_devices.lock().unwrap_or_else(PoisonError::into_inner).clone() {
        return Ok(d);
    }
    crate::voice::devices_allowed(c)?;
    Ok(Arc::new(crate::voice::RealDevices(c.clone())))
}

fn set(c: &Ctx, p: &Value) -> Result<Value> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(true);
    if !on {
        // The loop ends at its next reading; a new start does not wait for it.
        if let Some(s) = c.0.voice.mode.lock().unwrap_or_else(PoisonError::into_inner).take() {
            s.stop.store(true, Ordering::Relaxed);
        }
        c.update(|s| s.voice_mode = None);
        return Ok(json!({"on": false}));
    }
    if c.0.voice.mode.lock().unwrap_or_else(PoisonError::into_inner).is_some() {
        return Ok(json!({"on": true}));
    }
    let dev = devices(c)?;
    if crate::voice::openai_with_key(c).is_none() {
        return Err(Error::unsupported("Für den Sprachmodus ein OpenAI-kompatibles Backend mit Transkription hinzufügen (OpenAI oder lokal, z. B. speaches)"));
    }
    crate::voice::tts_provider(c)?;
    let state = c.state();
    if state.selection.model_id.is_empty() {
        return Err(Error::invalid("Kein Modell gewählt"));
    }
    if state.recording {
        return Err(Error::invalid("Erst das Diktat beenden"));
    }
    let signals = Arc::new(Signals::default());
    signals.fast.store(dev.fast(), Ordering::Relaxed);
    *c.0.voice.mode.lock().unwrap_or_else(PoisonError::into_inner) = Some(signals.clone());
    c.update(|s| s.voice_mode = Some(VoiceModeView { phase: "listening".into(), ..Default::default() }));
    let ctx = c.clone();
    c.spawn(async move {
        let out = run(&ctx, dev.as_ref(), &signals).await;
        dev.cancel();
        dev.stop_playing();
        // Only this run's own traces: a new start may already be running.
        let mine = {
            let mut m = ctx.0.voice.mode.lock().unwrap_or_else(PoisonError::into_inner);
            let mine = m.as_ref().is_some_and(|x| Arc::ptr_eq(x, &signals));
            if mine {
                *m = None;
            }
            mine
        };
        if mine {
            ctx.update(|s| s.voice_mode = None);
        }
        match out {
            Ok(Some(why)) => {
                ctx.notice(NoticeLevel::Info, why);
            }
            Ok(None) => {}
            Err(e) => {
                ctx.notice(NoticeLevel::Error, format!("Sprachmodus beendet: {}", e.message));
            }
        }
    });
    Ok(json!({"on": true}))
}

fn next(c: &Ctx) -> Result<Value> {
    let s = c.0.voice.mode.lock().unwrap_or_else(PoisonError::into_inner).clone().ok_or_else(|| Error::invalid("Der Sprachmodus ist aus"))?;
    s.next.store(true, Ordering::Relaxed);
    Ok(json!({}))
}

fn phase(c: &Ctx, phase: &str) {
    c.update(|s| {
        if let Some(v) = s.voice_mode.as_mut() {
            v.phase = phase.into();
        }
    });
}

/// Words that end the voice mode when they are all that was said.
pub(crate) fn is_end(text: &str) -> bool {
    let t: String = text.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    ["sprachmodus beenden", "sprachmodus aus", "gespräch beenden", "ende", "tschüss", "stopp"].contains(&t.as_str())
}

async fn wait(s: &Signals) {
    tokio::time::sleep(if s.fast.load(Ordering::Relaxed) { Duration::from_millis(1) } else { POLL }).await;
}

/// The loop. `Ok(Some(text))` ends with a note for the person.
async fn run(c: &Ctx, dev: &dyn Devices, s: &Signals) -> Result<Option<String>> {
    let mut idle = 0;
    loop {
        if s.stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        phase(c, "listening");
        s.next.store(false, Ordering::Relaxed);
        dev.start()?;
        let mut heard = false;
        let mut quiet = 0;
        for n in 0..MAX_READINGS {
            wait(s).await;
            if s.stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if s.next.swap(false, Ordering::Relaxed) {
                heard = true;
                break;
            }
            if dev.level() > SPEECH_LEVEL {
                heard = true;
                quiet = 0;
            } else if heard {
                quiet += 1;
                if quiet >= SILENCE_READINGS {
                    break;
                }
            } else if n >= IDLE_READINGS {
                break;
            }
        }
        if !heard {
            dev.cancel();
            idle += 1;
            if idle >= MAX_IDLE_TURNS {
                return Ok(Some("Sprachmodus beendet: Ich habe eine Weile nichts gehört.".into()));
            }
            continue;
        }
        idle = 0;
        let wav = dev.stop()?;
        phase(c, "thinking");
        let (_, base, key) = crate::voice::openai_with_key(c).ok_or_else(|| Error::unsupported("Kein Backend für Transkription"))?;
        let text = aiscraft_voice::transcribe_openai(&base, Some(&key), "whisper-1", wav, Some("de")).await?;
        let text = text.trim().to_string();
        if s.stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if text.is_empty() {
            continue;
        }
        if is_end(&text) {
            return Ok(Some("Sprachmodus beendet.".into()));
        }
        c.update(|st| {
            if let Some(v) = st.voice_mode.as_mut() {
                v.heard = text.clone();
                v.turns += 1;
            }
        });
        let out = c.run("chat.send", json!({"text": text})).await?;
        let answer = out.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        if s.stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if answer.trim().is_empty() {
            continue;
        }
        phase(c, "speaking");
        let audio = crate::voice::synthesize(c, &answer).await?;
        dev.play(audio)?;
        s.next.store(false, Ordering::Relaxed);
        while dev.playing() {
            wait(s).await;
            if s.stop.load(Ordering::Relaxed) {
                dev.stop_playing();
                return Ok(None);
            }
            if s.next.swap(false, Ordering::Relaxed) {
                dev.stop_playing();
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn end_words_end_only_when_alone() {
        assert!(super::is_end("Sprachmodus beenden."));
        assert!(super::is_end("  Tschüss! "));
        assert!(!super::is_end("Wann endet die Stunde?"));
    }
}
