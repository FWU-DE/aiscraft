//! Speaking and dictating (Nod's "voice in and out").
//!
//! Audio devices live on one thread of their own; commands talk to it over a channel. Reading
//! aloud uses AIS.chat's speech endpoint when signed in there, else an OpenAI-compatible backend
//! with a key. Dictation needs a transcription endpoint (OpenAI or a local server such as
//! speaches). Engines in memory (tests, scripts) never touch a microphone or speakers.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use aiscraft_core::{BackendKind, Error, Result};
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::opt_str;

enum AudioCmd {
    StartRecording(mpsc::Sender<Result<()>>),
    StopRecording(mpsc::Sender<Result<Vec<u8>>>),
    CancelRecording,
    Play(Vec<u8>, mpsc::Sender<Result<()>>),
    StopPlaying,
    IsPlaying(mpsc::Sender<bool>),
}

#[derive(Default)]
pub(crate) struct Voice {
    tx: Mutex<Option<mpsc::Sender<AudioCmd>>>,
    /// Microphone level 0..1, as f32 bits, for the meter.
    pub level: Arc<AtomicU32>,
    /// Test devices for the voice mode (#40): set only by tests, never a real microphone there.
    pub(crate) test_devices: Mutex<Option<Arc<dyn crate::voice_mode::Devices>>>,
    /// The voice mode's stop and "next" signals while it runs.
    pub(crate) mode: Mutex<Option<Arc<crate::voice_mode::Signals>>>,
}

impl Voice {
    fn sender(&self) -> mpsc::Sender<AudioCmd> {
        let mut guard = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tx) = guard.as_ref() {
            return tx.clone();
        }
        let (tx, rx) = mpsc::channel::<AudioCmd>();
        let level = self.level.clone();
        let _ = std::thread::Builder::new().name("aischatcraft-audio".into()).spawn(move || audio_thread(rx, level));
        *guard = Some(tx.clone());
        tx
    }
}

fn audio_thread(rx: mpsc::Receiver<AudioCmd>, level: Arc<AtomicU32>) {
    let mut recorder: Option<aiscraft_voice::Recorder> = None;
    let mut player: Option<aiscraft_voice::Player> = None;
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(80)) {
            Ok(AudioCmd::StartRecording(reply)) => {
                let r = aiscraft_voice::Recorder::start(aiscraft_voice::RecordOptions::default());
                let _ = reply.send(r.map(|r| {
                    recorder = Some(r);
                }));
            }
            Ok(AudioCmd::StopRecording(reply)) => {
                let out = recorder.take().ok_or_else(|| Error::invalid("Es läuft keine Aufnahme")).and_then(|r| r.stop());
                level.store(0f32.to_bits(), Ordering::Relaxed);
                let _ = reply.send(out);
            }
            Ok(AudioCmd::CancelRecording) => {
                if let Some(r) = recorder.take() {
                    r.cancel();
                }
                level.store(0f32.to_bits(), Ordering::Relaxed);
            }
            Ok(AudioCmd::Play(bytes, reply)) => {
                if player.is_none() {
                    player = aiscraft_voice::Player::new().ok();
                }
                let out = match player.as_ref() {
                    Some(p) => p.play(bytes),
                    None => Err(Error::not_found("Kein Lautsprecher verfügbar")),
                };
                let _ = reply.send(out);
            }
            Ok(AudioCmd::StopPlaying) => {
                if let Some(p) = player.as_ref() {
                    p.stop();
                }
            }
            Ok(AudioCmd::IsPlaying(reply)) => {
                let _ = reply.send(player.as_ref().is_some_and(|p| p.is_playing()));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if let Some(r) = recorder.as_ref() {
            level.store(r.level().to_bits(), Ordering::Relaxed);
        }
    }
}

pub(crate) fn devices_allowed(c: &Ctx) -> Result<()> {
    if c.0.browser_allowed && std::env::var_os("AISCRAFT_NO_DEVICES").is_none() {
        Ok(())
    } else {
        Err(Error::unsupported("In dieser Umgebung nutzt AIS.ChatCraft weder Mikrofon noch Lautsprecher (Tests, Skripte)"))
    }
}

/// Where speech comes from: ("ais", base, cookie) or ("openai", base, key).
pub(crate) fn tts_provider(c: &Ctx) -> Result<(BackendKind, String, String)> {
    let s = c.state();
    // AIS.chat only where its speech feature is on for the person's state.
    // The same rule decides `State::speech_available` (the „Vorlesen“ button).
    if let Some(b) = s.backends.iter().find(|b| b.ais_speech())
        && let Some(cookie) = c.0.secrets.get(&b.config.id)?
    {
        return Ok((BackendKind::AisChat, b.config.base_url.clone(), cookie));
    }
    openai_with_key(c).ok_or_else(|| Error::unsupported("Zum Vorlesen bei AIS.chat anmelden oder ein OpenAI-kompatibles Backend mit Schlüssel hinzufügen"))
}

pub(crate) fn openai_with_key(c: &Ctx) -> Option<(BackendKind, String, String)> {
    c.state()
        .backends
        .iter()
        .filter(|b| b.openai_speech())
        .find_map(|b| c.0.secrets.get(&b.config.id).ok().flatten().map(|k| (BackendKind::OpenAi, b.config.base_url.clone(), k)))
}

pub(crate) async fn speak(c: Ctx, p: Value) -> Result<Value> {
    let text = match (opt_str(&p, "message_id"), opt_str(&p, "text")) {
        (_, Some(t)) => t,
        (Some(mid), None) => {
            let s = c.state();
            s.messages.iter().find(|m| m.id == mid).map(|m| m.content.clone()).ok_or_else(|| Error::not_found("Nachricht nicht gefunden"))?
        }
        (None, None) => return Err(Error::invalid("message_id oder text fehlt")),
    };
    let spoken = aiscraft_voice::speakable(&text);
    if spoken.trim().is_empty() {
        return Err(Error::invalid("Nichts zum Vorlesen"));
    }
    devices_allowed(&c)?;
    let (kind, base, secret) = tts_provider(&c)?;
    let stream = match kind {
        BackendKind::AisChat => aiscraft_voice::speak_ais(&base, &secret, &spoken, None, None).await?,
        _ => aiscraft_voice::speak_openai(&base, Some(&secret), "gpt-4o-mini-tts", "alloy", &spoken).await?,
    };
    let audio = aiscraft_voice::collect_audio(stream).await?;
    let (tx, rx) = mpsc::channel();
    c.0.voice.sender().send(AudioCmd::Play(audio, tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
    rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))??;
    Ok(json!({}))
}

pub(crate) async fn stop_speaking(c: Ctx, _p: Value) -> Result<Value> {
    devices_allowed(&c)?;
    let _ = c.0.voice.sender().send(AudioCmd::StopPlaying);
    Ok(json!({}))
}

pub(crate) async fn record_start(c: Ctx, _p: Value) -> Result<Value> {
    devices_allowed(&c)?;
    if openai_with_key(&c).is_none() {
        return Err(Error::unsupported("Zum Diktieren ein OpenAI-kompatibles Backend mit Transkription hinzufügen (OpenAI oder lokal, z. B. speaches)"));
    }
    let (tx, rx) = mpsc::channel();
    c.0.voice.sender().send(AudioCmd::StartRecording(tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
    rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))??;
    c.update(|s| s.recording = true);
    Ok(json!({}))
}

pub(crate) async fn record_stop(c: Ctx, p: Value) -> Result<Value> {
    devices_allowed(&c)?;
    c.update(|s| s.recording = false);
    if p.get("cancel").and_then(Value::as_bool) == Some(true) {
        let _ = c.0.voice.sender().send(AudioCmd::CancelRecording);
        return Ok(json!({}));
    }
    let (tx, rx) = mpsc::channel();
    c.0.voice.sender().send(AudioCmd::StopRecording(tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
    let wav = rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))??;
    let (_, base, key) = openai_with_key(&c).ok_or_else(|| Error::unsupported("Kein Backend für Transkription"))?;
    let text = aiscraft_voice::transcribe_openai(&base, Some(&key), "whisper-1", wav, Some("de")).await?;
    Ok(json!({"text": text}))
}

/// The real microphone and speakers, through the audio thread.
pub(crate) struct RealDevices(pub Ctx);

impl crate::voice_mode::Devices for RealDevices {
    fn start(&self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.0.0.voice.sender().send(AudioCmd::StartRecording(tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
        rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))?
    }
    fn level(&self) -> f32 {
        level(&self.0)
    }
    fn stop(&self) -> Result<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        self.0.0.voice.sender().send(AudioCmd::StopRecording(tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
        rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))?
    }
    fn cancel(&self) {
        let _ = self.0.0.voice.sender().send(AudioCmd::CancelRecording);
    }
    fn play(&self, audio: Vec<u8>) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.0.0.voice.sender().send(AudioCmd::Play(audio, tx)).map_err(|_| Error::protocol("Audio nicht verfügbar"))?;
        rx.recv().map_err(|_| Error::protocol("Audio nicht verfügbar"))?
    }
    fn playing(&self) -> bool {
        let (tx, rx) = mpsc::channel();
        if self.0.0.voice.sender().send(AudioCmd::IsPlaying(tx)).is_err() {
            return false;
        }
        rx.recv().unwrap_or(false)
    }
    fn stop_playing(&self) {
        let _ = self.0.0.voice.sender().send(AudioCmd::StopPlaying);
    }
}

/// Speech for `text` from the configured provider (AIS.chat or OpenAI-compatible).
pub(crate) async fn synthesize(c: &Ctx, text: &str) -> Result<Vec<u8>> {
    let spoken = aiscraft_voice::speakable(text);
    if spoken.trim().is_empty() {
        return Err(Error::invalid("Nichts zum Vorlesen"));
    }
    let (kind, base, secret) = tts_provider(c)?;
    let stream = match kind {
        BackendKind::AisChat => aiscraft_voice::speak_ais(&base, &secret, &spoken, None, None).await?,
        _ => aiscraft_voice::speak_openai(&base, Some(&secret), "gpt-4o-mini-tts", "alloy", &spoken).await?,
    };
    aiscraft_voice::collect_audio(stream).await
}

/// The microphone level for the meter, 0..1.
pub fn level(c: &Ctx) -> f32 {
    f32::from_bits(c.0.voice.level.load(Ordering::Relaxed))
}
