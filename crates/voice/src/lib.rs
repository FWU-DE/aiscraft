//! AIS.ChatCraft speech: dictation (microphone → WAV → speech to text) and read-aloud
//! (Markdown → speakable text → text to speech → speakers).
//!
//! Devices: [`Recorder`] (cpal, 16 kHz mono) and [`Player`] (rodio, MP3/WAV, streaming).
//! Services: [`transcribe_openai`], [`speak_openai`] and [`speak_ais`] (async, reqwest).
//! Nothing here panics; device and network problems come back as `Err` with a German message.

mod http;
pub mod play;
pub mod record;
pub mod resample;
pub mod speakable;
pub mod stt;
pub mod tts;
pub mod wav;

pub use play::{Player, StreamFeed};
pub use record::{RecordOptions, Recorder, SPEECH_RATE};
pub use speakable::{CHUNK_CHARS, chunks, speakable, speakable_chunks};
pub use stt::transcribe_openai;
pub use tts::{AudioStream, collect_audio, speak_ais, speak_openai};
pub use wav::{Wav, decode_wav, encode_wav};
