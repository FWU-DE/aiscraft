//! The real Windows backend against Editor (Notepad): open, type, read back, close the tab
//! without saving. It opens a window on the desktop and takes the keyboard focus, so it is
//! ignored; run it by hand on a Windows machine nobody is typing on:
//!
//! ```sh
//! cargo test -p aiscraft-desktop --test notepad -- --ignored
//! ```

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use aiscraft_desktop::{Act, Snapshot, parse_keys, policy};

fn wait<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        if let Some(t) = f() {
            return t;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn notepad_windows(d: &dyn aiscraft_desktop::DesktopAutomation) -> Vec<aiscraft_desktop::WindowInfo> {
    d.windows().unwrap().into_iter().filter(|w| w.programs.iter().any(|p| p.exe == "notepad.exe")).collect()
}

fn document(s: &Snapshot) -> Option<String> {
    s.controls.iter().find(|c| c.role == "Dokument" || (c.role == "Textfeld" && c.acts.contains(&Act::Type))).map(|c| c.path.clone())
}

#[test]
#[ignore = "opens Notepad on the desktop"]
fn drives_notepad_open_type_read_back_close_without_saving() {
    let d = aiscraft_desktop::system();
    d.ready().unwrap();
    let path = d.locate("notepad.exe").expect("notepad.exe found");
    assert!(policy::program_refusal(&aiscraft_desktop::Program::new(&path), r"C:\Windows").is_none(), "{path}");
    // Windows 11 opens a new tab in a Notepad window that is already open, next to the person's
    // own tabs. The test never works in someone's open Notepad.
    if !notepad_windows(d.as_ref()).is_empty() {
        eprintln!("skipped: Notepad is open; close every Notepad window first");
        return;
    }
    d.launch(&path).unwrap();
    let win = wait("a Notepad window", || notepad_windows(d.as_ref()).into_iter().next());
    let snap = wait("the text area", || d.snapshot(&win.id).ok().filter(|s| document(s).is_some()));
    let doc = document(&snap).unwrap();
    let ctl = snap.control(&doc).unwrap().clone();
    let text = format!("Hallo von AIS.ChatCraft {}", std::process::id());
    d.set_text(&win.id, &ctl, &text, true).unwrap();
    let back = wait("the text read back", || {
        let s = d.snapshot(&win.id).ok()?;
        let v = s.control(&doc)?.value.clone()?;
        v.contains(&text).then_some(v)
    });
    assert!(back.contains(&text), "{back}");
    // Close this tab (in a new window: the window), then „Nicht speichern“.
    let ctl = d.snapshot(&win.id).unwrap().control(&doc).unwrap().clone();
    d.keys(&win.id, Some(&ctl), &parse_keys("Strg+W", 1).unwrap()).unwrap();
    let dont_save = wait("the save question", || {
        for w in notepad_windows(d.as_ref()) {
            let s = d.snapshot(&w.id).ok()?;
            if let Some(c) = s.controls.iter().find(|c| {
                let n = c.name.to_lowercase();
                c.acts.contains(&Act::Press) && (n.contains("nicht speichern") || n.contains("don't save") || n.contains("don’t save"))
            }) {
                return Some((w.id.clone(), c.clone()));
            }
        }
        None
    });
    d.act(&dont_save.0, &dont_save.1, Act::Press).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    // The text is gone: no Notepad window shows it any more.
    for w in notepad_windows(d.as_ref()) {
        if let Ok(s) = d.snapshot(&w.id) {
            assert!(!s.outline().contains(&text), "the tab was closed without saving");
        }
    }
}
