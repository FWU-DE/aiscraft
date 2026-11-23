//! Records 3 seconds from the default microphone, prints the level, then plays it back.
//!
//! `cargo run -p aiscraft-voice --example mic_check`

use std::time::Duration;

use aiscraft_voice::{Player, RecordOptions, Recorder};

fn main() {
    if let Err(e) = run() {
        eprintln!("Fehler: {e}");
        std::process::exit(1);
    }
}

fn run() -> aiscraft_core::Result<()> {
    println!("Aufnahme läuft (3 s) …");
    let rec = Recorder::start(RecordOptions::default())?;
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        let level = rec.level();
        let bar = "#".repeat(((level * 200.0).min(50.0)) as usize);
        println!("{:>5.3} {bar}", level);
        if !rec.is_recording() {
            break;
        }
    }
    if let Some(e) = rec.error() {
        eprintln!("Mikrofon: {e}");
    }
    let wav = rec.stop()?;
    println!("{} Bytes aufgenommen, Wiedergabe …", wav.len());
    let player = Player::new()?;
    player.play(wav)?;
    while player.is_playing() {
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
