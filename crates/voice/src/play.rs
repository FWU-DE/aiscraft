//! Audio playback with rodio: whole MP3/WAV files, or MP3 that arrives in chunks (streaming TTS).
//!
//! The output device is opened on its own thread, which keeps it alive until the `Player` is
//! dropped. Streamed audio is decoded on a separate thread into short sample buffers that are
//! queued on the rodio player, so the audio callback never waits for the network.

use std::collections::VecDeque;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, Source};

/// Streamed audio is refused beyond this size.
pub const STREAM_CAP: usize = 64 * 1024 * 1024;

struct Current {
    player: Arc<rodio::Player>,
    feeds: Vec<Arc<FeedShared>>,
}

/// Plays audio on the default output device.
pub struct Player {
    mixer: rodio::mixer::Mixer,
    current: Mutex<Current>,
    volume: Mutex<f32>,
    // Dropping this ends the device thread, which closes the output device.
    _keepalive: mpsc::Sender<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn decode_error(e: rodio::decoder::DecoderError) -> Error {
    Error::invalid(format!("Audio kann nicht abgespielt werden: {e}"))
}

impl Player {
    /// Open the default output device.
    pub fn new() -> Result<Player> {
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<rodio::mixer::Mixer>>(1);
        let (keep_tx, keep_rx) = mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("aiscraft-audio-out".into())
            .spawn(move || match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(mut sink) => {
                    sink.log_on_drop(false);
                    let _ = ready_tx.send(Ok(sink.mixer().clone()));
                    // Blocks until the Player (holding the sender) is dropped.
                    let _ = keep_rx.recv();
                    drop(sink);
                }
                Err(e) => {
                    let msg = match e {
                        rodio::DeviceSinkError::NoDevice => "Kein Lautsprecher gefunden.".to_string(),
                        other => format!("Lautsprecher kann nicht geöffnet werden: {other}"),
                    };
                    let _ = ready_tx.send(Err(Error::not_found(msg)));
                }
            })
            .map_err(|e| Error::storage(format!("Audio-Thread: {e}")))?;
        let mixer = match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(r) => r?,
            Err(_) => return Err(Error::network("Der Lautsprecher reagiert nicht.")),
        };
        let player = Arc::new(rodio::Player::connect_new(&mixer));
        Ok(Player { mixer, current: Mutex::new(Current { player, feeds: Vec::new() }), volume: Mutex::new(1.0), _keepalive: keep_tx })
    }

    /// Stop what is playing and start a fresh queue.
    fn fresh(&self) -> Arc<rodio::Player> {
        let mut cur = lock(&self.current);
        for f in cur.feeds.drain(..) {
            f.cancel();
        }
        cur.player.stop();
        let player = Arc::new(rodio::Player::connect_new(&self.mixer));
        player.set_volume(*lock(&self.volume));
        cur.player = player.clone();
        player
    }

    /// Play a complete MP3 or WAV file, replacing whatever is playing.
    pub fn play(&self, bytes: Vec<u8>) -> Result<()> {
        if bytes.is_empty() {
            return Err(Error::invalid("Keine Audiodaten erhalten."));
        }
        let len = bytes.len() as u64;
        let decoder = Decoder::builder().with_data(Cursor::new(bytes)).with_byte_len(len).build().map_err(decode_error)?;
        self.fresh().append(decoder);
        Ok(())
    }

    /// Start playing audio that arrives in pieces, replacing whatever is playing. Push the
    /// network chunks into the returned feed; call `finish` when the response is complete.
    pub fn stream(&self) -> StreamFeed {
        let player = self.fresh();
        let shared = Arc::new(FeedShared::default());
        lock(&self.current).feeds.push(shared.clone());
        let worker = shared.clone();
        let spawned = std::thread::Builder::new().name("aiscraft-audio-decode".into()).spawn(move || {
            decode_feed(&worker, &player);
        });
        if let Err(e) = spawned {
            shared.fail(format!("Audio-Thread: {e}"));
            shared.cancel();
        }
        StreamFeed { shared }
    }

    /// Stop playback and cancel streams still arriving.
    pub fn stop(&self) {
        let _ = self.fresh();
    }

    pub fn pause(&self) {
        lock(&self.current).player.pause();
    }

    pub fn resume(&self) {
        lock(&self.current).player.play();
    }

    pub fn is_paused(&self) -> bool {
        lock(&self.current).player.is_paused()
    }

    /// Something is queued or still arriving (a stream counts until it is decoded), and not paused.
    pub fn is_playing(&self) -> bool {
        let cur = lock(&self.current);
        let busy = !cur.player.empty() || cur.feeds.iter().any(|f| !f.done());
        busy && !cur.player.is_paused()
    }

    /// Volume factor, 1.0 = unchanged.
    pub fn set_volume(&self, v: f32) {
        let v = if v.is_finite() { v.clamp(0.0, 4.0) } else { 1.0 };
        *lock(&self.volume) = v;
        lock(&self.current).player.set_volume(v);
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let cur = lock(&self.current);
        for f in &cur.feeds {
            f.cancel();
        }
        cur.player.stop();
    }
}

#[derive(Default)]
struct Segment {
    data: Vec<u8>,
    ended: bool,
}

#[derive(Default)]
struct FeedState {
    segments: VecDeque<Segment>,
    /// Segments already handed to the decoder and dropped from `segments`.
    consumed: usize,
    finished: bool,
    cancelled: bool,
    decoded: bool,
    total: usize,
    error: Option<String>,
}

#[derive(Default)]
struct FeedShared {
    state: Mutex<FeedState>,
    cv: Condvar,
}

impl FeedShared {
    fn cancel(&self) {
        lock(&self.state).cancelled = true;
        self.cv.notify_all();
    }
    fn fail(&self, msg: String) {
        let mut st = lock(&self.state);
        if st.error.is_none() {
            st.error = Some(msg);
        }
    }
    fn done(&self) -> bool {
        let st = lock(&self.state);
        st.decoded || st.cancelled
    }
}

/// Feeds streamed audio to a `Player`. Each segment is one audio file (e.g. one TTS response per
/// sentence chunk); segments play back to back. Dropping the feed finishes it.
pub struct StreamFeed {
    shared: Arc<FeedShared>,
}

impl StreamFeed {
    /// Append bytes of the current segment.
    pub fn push(&self, chunk: &[u8]) -> Result<()> {
        let mut st = lock(&self.shared.state);
        if st.cancelled {
            return Err(Error::cancelled());
        }
        if st.finished {
            return Err(Error::invalid("Die Wiedergabe ist bereits abgeschlossen."));
        }
        if st.total.saturating_add(chunk.len()) > STREAM_CAP {
            return Err(Error::refused("Die Audioantwort ist zu groß."));
        }
        st.total += chunk.len();
        if st.segments.back().is_none_or(|s| s.ended) {
            st.segments.push_back(Segment::default());
        }
        if let Some(seg) = st.segments.back_mut() {
            seg.data.extend_from_slice(chunk);
        }
        drop(st);
        self.shared.cv.notify_all();
        Ok(())
    }

    /// The current segment (audio file) is complete; the next `push` starts a new one.
    pub fn end_segment(&self) {
        if let Some(seg) = lock(&self.shared.state).segments.back_mut() {
            seg.ended = true;
        }
        self.shared.cv.notify_all();
    }

    /// No more audio will come.
    pub fn finish(&self) {
        {
            let mut st = lock(&self.shared.state);
            if let Some(seg) = st.segments.back_mut() {
                seg.ended = true;
            }
            st.finished = true;
        }
        self.shared.cv.notify_all();
    }

    /// Stopped by `Player::stop` or a newer `play`/`stream`.
    pub fn is_cancelled(&self) -> bool {
        lock(&self.shared.state).cancelled
    }

    /// A segment that could not be decoded (it is skipped; later segments still play).
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.state).error.clone()
    }
}

impl Drop for StreamFeed {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Blocking reader over the front segment of a feed.
struct SegmentReader {
    shared: Arc<FeedShared>,
    index: usize,
    pos: usize,
}

impl Read for SegmentReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut st = lock(&self.shared.state);
        loop {
            if st.cancelled {
                return Ok(0);
            }
            let rel = self.index.saturating_sub(st.consumed);
            let Some(seg) = st.segments.get(rel) else { return Ok(0) };
            if self.pos < seg.data.len() {
                let avail = seg.data.get(self.pos..).unwrap_or_default();
                let n = avail.len().min(buf.len());
                if let (Some(dst), Some(src)) = (buf.get_mut(..n), avail.get(..n)) {
                    dst.copy_from_slice(src);
                }
                self.pos += n;
                return Ok(n);
            }
            if seg.ended {
                return Ok(0);
            }
            st = self.shared.cv.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Seek for SegmentReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let len = {
            let st = lock(&self.shared.state);
            let rel = self.index.saturating_sub(st.consumed);
            st.segments.get(rel).filter(|s| s.ended).map(|s| s.data.len())
        };
        let target = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(d) => self.pos as i128 + i128::from(d),
            SeekFrom::End(d) => match len {
                Some(l) => l as i128 + i128::from(d),
                None => return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Länge noch unbekannt")),
            },
        };
        let target = usize::try_from(target).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Position ungültig"))?;
        self.pos = target;
        Ok(target as u64)
    }
}

/// Wait for segment `index` to exist (true) or the feed to end without it (false).
fn wait_segment(shared: &FeedShared, index: usize) -> bool {
    let mut st = lock(&shared.state);
    loop {
        if st.cancelled {
            return false;
        }
        if index < st.consumed + st.segments.len() {
            return true;
        }
        if st.finished {
            return false;
        }
        st = shared.cv.wait(st).unwrap_or_else(PoisonError::into_inner);
    }
}

fn decode_feed(shared: &Arc<FeedShared>, player: &rodio::Player) {
    let mut index = 0usize;
    while wait_segment(shared, index) {
        let reader = SegmentReader { shared: shared.clone(), index, pos: 0 };
        match Decoder::builder().with_data(reader).with_seekable(false).with_hint("mp3").with_gapless(false).build() {
            Ok(decoder) => pump(shared, player, decoder),
            Err(e) => {
                log::warn!("Audio-Segment nicht lesbar: {e}");
                shared.fail(format!("Audio kann nicht abgespielt werden: {e}"));
            }
        }
        // Drop the decoded segment's bytes.
        {
            let mut st = lock(&shared.state);
            if index >= st.consumed && st.segments.front().is_some_and(|s| s.ended) {
                st.segments.pop_front();
                st.consumed += 1;
            }
        }
        index += 1;
    }
    lock(&shared.state).decoded = true;
}

/// Move decoded samples to the player in ~100 ms buffers.
fn pump<R: Read + Seek + Send + Sync + 'static>(shared: &FeedShared, player: &rodio::Player, mut decoder: Decoder<R>) {
    loop {
        let channels = decoder.channels();
        let rate = decoder.sample_rate();
        let want = (rate.get() as usize / 10).max(256) * usize::from(channels.get());
        let buf: Vec<f32> = decoder.by_ref().take(want).collect();
        if lock(&shared.state).cancelled {
            return;
        }
        if buf.is_empty() {
            return;
        }
        let n = buf.len();
        player.append(SamplesBuffer::new(channels, rate, buf));
        if n < want {
            return;
        }
        // Do not run far ahead of playback: at most ~2 s queued.
        while player.len() > 20 {
            if lock(&shared.state).cancelled {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed() -> (Arc<FeedShared>, StreamFeed) {
        let shared = Arc::new(FeedShared::default());
        (shared.clone(), StreamFeed { shared })
    }

    #[test]
    fn segment_reader_blocks_until_data_and_ends() {
        let (shared, f) = feed();
        f.push(b"abc").unwrap();
        let mut r = SegmentReader { shared: shared.clone(), index: 0, pos: 0 };
        let mut buf = [0u8; 8];
        assert_eq!(r.read(&mut buf).unwrap(), 3);
        assert!(r.seek(SeekFrom::End(0)).is_err(), "length unknown while streaming");
        let t = std::thread::spawn(move || {
            let mut out = Vec::new();
            r.read_to_end(&mut out).unwrap();
            out
        });
        std::thread::sleep(Duration::from_millis(30));
        f.push(b"de").unwrap();
        f.end_segment();
        assert_eq!(t.join().unwrap(), b"de");
        // The next push opens segment 1.
        f.push(b"xyz").unwrap();
        f.finish();
        let mut r1 = SegmentReader { shared: shared.clone(), index: 1, pos: 0 };
        let mut out = Vec::new();
        r1.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"xyz");
        assert_eq!(r1.seek(SeekFrom::End(-1)).unwrap(), 2);
        assert!(wait_segment(&shared, 1));
        assert!(!wait_segment(&shared, 2));
        assert!(f.push(b"late").is_err());
    }

    #[test]
    fn cancel_unblocks_reader() {
        let (shared, f) = feed();
        f.push(b"a").unwrap();
        let mut r = SegmentReader { shared: shared.clone(), index: 0, pos: 1 };
        let t = std::thread::spawn(move || r.read(&mut [0u8; 4]).unwrap());
        std::thread::sleep(Duration::from_millis(30));
        shared.cancel();
        assert_eq!(t.join().unwrap(), 0);
        assert!(f.is_cancelled());
        assert!(matches!(f.push(b"b"), Err(e) if e.kind == aiscraft_core::ErrorKind::Cancelled));
    }

    #[test]
    fn decoder_reads_wav_from_feed() {
        let samples: Vec<i16> = (0..1600).map(|i| ((i % 100) * 100) as i16).collect();
        let wav = crate::wav::encode_wav(&samples, 16_000, 1).unwrap();
        let (shared, f) = feed();
        for c in wav.chunks(100) {
            f.push(c).unwrap();
        }
        f.finish();
        let reader = SegmentReader { shared, index: 0, pos: 0 };
        let decoder = Decoder::builder().with_data(reader).with_seekable(false).with_hint("wav").build().unwrap();
        assert_eq!(decoder.sample_rate().get(), 16_000);
        assert_eq!(decoder.count(), 1600);
    }

    #[test]
    fn garbage_is_an_error_not_a_crash() {
        let r = Decoder::builder().with_data(Cursor::new(vec![0x42u8; 4096])).with_byte_len(4096).build();
        assert!(r.is_err());
    }

    #[test]
    fn feed_cap() {
        let (_, f) = feed();
        assert!(f.push(&vec![0u8; STREAM_CAP + 1]).is_err());
    }

    #[test]
    #[ignore = "needs speakers"]
    fn plays_a_tone() {
        let samples: Vec<i16> = (0..8000).map(|i| ((i as f32 * 0.1).sin() * 3000.0) as i16).collect();
        let p = Player::new().unwrap();
        p.play(crate::wav::encode_wav(&samples, 16_000, 1).unwrap()).unwrap();
        assert!(p.is_playing());
        std::thread::sleep(Duration::from_millis(700));
        assert!(!p.is_playing());
    }
}
