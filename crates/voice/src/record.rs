//! Microphone recording with cpal.
//!
//! The cpal stream lives on its own thread (streams are not `Send` on every platform). The audio
//! callback only converts and downmixes; resampling and buffering happen on that thread, so the
//! UI thread is never blocked except for the short wait while the device opens.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use aiscraft_core::{Error, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample};

use crate::resample::{Resampler, downmix, rms, to_i16};
use crate::wav::encode_wav;

/// The sample rate recordings are delivered in.
pub const SPEECH_RATE: u32 = 16_000;

#[derive(Debug, Clone)]
pub struct RecordOptions {
    /// Recording stops by itself after this long.
    pub max_duration: Duration,
    /// Output sample rate (16 kHz for speech recognition).
    pub sample_rate: u32,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self { max_duration: Duration::from_secs(10 * 60), sample_rate: SPEECH_RATE }
    }
}

#[derive(Default)]
struct Shared {
    /// RMS level of the last device buffer, as f32 bits.
    level: AtomicU32,
    /// Output samples so far.
    samples: AtomicU64,
    stop: AtomicBool,
    capped: AtomicBool,
    finished: AtomicBool,
    error: Mutex<Option<String>>,
}

impl Shared {
    fn set_error(&self, msg: String) {
        let mut e = self.error.lock().unwrap_or_else(PoisonError::into_inner);
        if e.is_none() {
            *e = Some(msg);
        }
    }
}

/// A running recording from the default input device.
pub struct Recorder {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<Vec<i16>>>,
    sample_rate: u32,
}

fn device_error(e: cpal::Error) -> Error {
    use cpal::ErrorKind as K;
    match e.kind() {
        K::PermissionDenied => Error::refused("Kein Zugriff auf das Mikrofon. Bitte den Zugriff in den Systemeinstellungen erlauben."),
        K::DeviceBusy => Error::refused("Das Mikrofon wird gerade von einer anderen Anwendung benutzt."),
        K::DeviceNotAvailable | K::HostUnavailable => Error::not_found("Kein Mikrofon gefunden. Bitte ein Mikrofon anschließen."),
        K::UnsupportedConfig => Error::unsupported(format!("Das Mikrofon unterstützt das Aufnahmeformat nicht: {e}")),
        _ => Error::network(format!("Mikrofon-Fehler: {e}")),
    }
}

impl Recorder {
    /// Open the default input device and start recording. Returns once the device is open.
    pub fn start(opts: RecordOptions) -> Result<Recorder> {
        if opts.sample_rate == 0 {
            return Err(Error::invalid("Abtastrate muss größer als null sein"));
        }
        let sample_rate = opts.sample_rate;
        let shared = Arc::new(Shared::default());
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("aiscraft-recorder".into())
            .spawn(move || run(opts, worker_shared, ready_tx))
            .map_err(|e| Error::storage(format!("Aufnahme-Thread: {e}")))?;
        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = worker.join();
                return Err(e);
            }
            Err(_) => {
                shared.stop.store(true, Ordering::SeqCst);
                return Err(Error::network("Das Mikrofon reagiert nicht."));
            }
        }
        Ok(Recorder { shared, worker: Some(worker), sample_rate })
    }

    /// Current input level (RMS of the last buffer, 0..1). Speech is typically 0.02..0.3.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    /// Recorded length so far.
    pub fn elapsed(&self) -> Duration {
        Duration::from_secs_f64(self.shared.samples.load(Ordering::Relaxed) as f64 / f64::from(self.sample_rate.max(1)))
    }

    /// Whether recording is still going (false after the time cap, a device error or `stop`).
    pub fn is_recording(&self) -> bool {
        !self.shared.finished.load(Ordering::SeqCst)
    }

    /// Whether recording stopped because it hit `max_duration`.
    pub fn hit_limit(&self) -> bool {
        self.shared.capped.load(Ordering::SeqCst)
    }

    /// A device error that ended the recording early (e.g. the microphone was unplugged).
    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Stop and return the recording as a 16-bit mono WAV file. If the device failed midway, what
    /// was recorded until then is returned; an empty recording is an error.
    pub fn stop(mut self) -> Result<Vec<u8>> {
        let samples = self.finish()?;
        if samples.is_empty() {
            return Err(match self.error() {
                Some(e) => Error::network(e),
                None => Error::invalid("Die Aufnahme ist leer."),
            });
        }
        encode_wav(&samples, self.sample_rate, 1)
    }

    /// Stop and throw the recording away.
    pub fn cancel(mut self) {
        let _ = self.finish();
    }

    fn finish(&mut self) -> Result<Vec<i16>> {
        self.shared.stop.store(true, Ordering::SeqCst);
        let worker = self.worker.take().ok_or_else(|| Error::invalid("Aufnahme bereits beendet"))?;
        worker.join().map_err(|_| Error::storage("Der Aufnahme-Thread ist abgestürzt."))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        // Do not wait for the thread here; it notices the flag within ~50 ms and exits.
        self.shared.stop.store(true, Ordering::SeqCst);
    }
}

fn build_stream<T>(device: &cpal::Device, config: cpal::StreamConfig, tx: mpsc::SyncSender<Vec<f32>>, shared: Arc<Shared>) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels;
    let err_shared = shared.clone();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let floats: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
                let mono = downmix(&floats, channels);
                shared.level.store(rms(&mono).to_bits(), Ordering::Relaxed);
                // If the worker is behind, drop the buffer rather than block the audio thread.
                let _ = tx.try_send(mono);
            },
            move |e: cpal::Error| {
                use cpal::ErrorKind as K;
                match e.kind() {
                    K::Xrun | K::RealtimeDenied => log::debug!("Mikrofon: {e}"),
                    _ => {
                        log::warn!("Mikrofon-Fehler: {e}");
                        err_shared.set_error(format!("Das Mikrofon ist ausgefallen: {e}"));
                    }
                }
            },
            None,
        )
        .map_err(device_error)
}

fn open(shared: &Arc<Shared>, tx: mpsc::SyncSender<Vec<f32>>) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device =
        host.default_input_device().ok_or_else(|| Error::not_found("Kein Mikrofon gefunden. Bitte ein Mikrofon anschließen oder den Zugriff erlauben."))?;
    let supported = device.default_input_config().map_err(device_error)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let rate = config.sample_rate;
    let s = shared.clone();
    let stream = match format {
        SampleFormat::I8 => build_stream::<i8>(&device, config, tx, s),
        SampleFormat::I16 => build_stream::<i16>(&device, config, tx, s),
        SampleFormat::I32 => build_stream::<i32>(&device, config, tx, s),
        SampleFormat::U8 => build_stream::<u8>(&device, config, tx, s),
        SampleFormat::U16 => build_stream::<u16>(&device, config, tx, s),
        SampleFormat::U32 => build_stream::<u32>(&device, config, tx, s),
        SampleFormat::F32 => build_stream::<f32>(&device, config, tx, s),
        SampleFormat::F64 => build_stream::<f64>(&device, config, tx, s),
        other => Err(Error::unsupported(format!("Mikrofon-Format {other} wird nicht unterstützt"))),
    }?;
    stream.play().map_err(device_error)?;
    Ok((stream, rate))
}

fn run(opts: RecordOptions, shared: Arc<Shared>, ready: mpsc::SyncSender<Result<()>>) -> Vec<i16> {
    // About 10 s of buffers of headroom between the audio callback and this thread.
    let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(512);
    let (stream, device_rate) = match open(&shared, tx) {
        Ok(s) => s,
        Err(e) => {
            shared.finished.store(true, Ordering::SeqCst);
            let _ = ready.send(Err(e));
            return Vec::new();
        }
    };
    let _ = ready.send(Ok(()));
    let cap = (opts.max_duration.as_secs_f64() * f64::from(opts.sample_rate)) as usize;
    let mut resampler = Resampler::new(device_rate, opts.sample_rate);
    let mut mono = Vec::new();
    let mut out: Vec<i16> = Vec::new();
    let mut push = |buf: &[f32], out: &mut Vec<i16>| -> bool {
        mono.clear();
        resampler.process(buf, &mut mono);
        let room = cap.saturating_sub(out.len());
        out.extend(mono.iter().take(room).map(|s| to_i16(*s)));
        shared.samples.store(out.len() as u64, Ordering::Relaxed);
        out.len() >= cap
    };
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        if shared.error.lock().unwrap_or_else(PoisonError::into_inner).is_some() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(buf) => {
                if push(&buf, &mut out) {
                    shared.capped.store(true, Ordering::SeqCst);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    drop(stream);
    // Buffers that arrived before the stream closed.
    while let Ok(buf) = rx.try_recv() {
        if push(&buf, &mut out) {
            shared.capped.store(true, Ordering::SeqCst);
            break;
        }
    }
    shared.level.store(0f32.to_bits(), Ordering::Relaxed);
    shared.finished.store(true, Ordering::SeqCst);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a microphone"]
    fn records_from_default_device() {
        let r = Recorder::start(RecordOptions::default()).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        assert!(r.is_recording());
        let wav = r.stop().unwrap();
        let w = crate::wav::decode_wav(&wav).unwrap();
        assert_eq!((w.sample_rate, w.channels), (SPEECH_RATE, 1));
        assert!(w.samples.len() > 4000, "{}", w.samples.len());
    }

    #[test]
    #[ignore = "needs a microphone"]
    fn stops_at_cap() {
        let r = Recorder::start(RecordOptions { max_duration: Duration::from_millis(300), ..Default::default() }).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        assert!(!r.is_recording());
        assert!(r.hit_limit());
        let w = crate::wav::decode_wav(&r.stop().unwrap()).unwrap();
        assert_eq!(w.samples.len(), 4800);
    }
}
