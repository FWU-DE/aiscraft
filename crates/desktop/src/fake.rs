//! A desktop in memory for tests: windows with controls, programs to start, and a log of every
//! step. No real window is read or touched.

use std::sync::{Mutex, PoisonError};

use aiscraft_core::{Error, Result};

use crate::{Act, Control, DesktopAutomation, KeyChord, Program, Snapshot, WindowInfo};

#[derive(Default)]
struct Desk {
    windows: Vec<(WindowInfo, Vec<Control>)>,
    /// Installed programs: file name → path, and the window a start opens.
    installed: Vec<(String, String, String)>,
    log: Vec<String>,
    reads: usize,
    next: u32,
}

/// The fake desktop. Every look at the desktop (listing windows, reading one) counts in
/// [`FakeDesktop::reads`]; every step
/// that changes something is in [`FakeDesktop::log`].
#[derive(Default)]
pub struct FakeDesktop {
    desk: Mutex<Desk>,
    /// `Err` from [`DesktopAutomation::ready`] (e.g. running with administrator rights).
    pub not_ready: Mutex<Option<String>>,
    /// The next step (press, text, keys) fails with this error, e.g. a timeout.
    pub next_error: Mutex<Option<Error>>,
}

pub fn control(path: &str, role: &str, name: &str, value: Option<&str>, acts: &[Act]) -> Control {
    Control {
        path: path.into(),
        depth: path.split('.').count(),
        role: role.into(),
        name: name.into(),
        value: value.map(str::to_string),
        enabled: true,
        focused: false,
        password: false,
        acts: acts.to_vec(),
        class: String::new(),
        runtime_id: format!("rt-{path}"),
    }
}

impl FakeDesktop {
    pub fn new() -> FakeDesktop {
        FakeDesktop::default()
    }

    fn desk(&self) -> std::sync::MutexGuard<'_, Desk> {
        self.desk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A window of the program at `path`; returns its id.
    pub fn add_window(&self, path: &str, title: &str, controls: Vec<Control>) -> String {
        let mut d = self.desk();
        d.next += 1;
        let id = format!("w{}", d.next);
        let info = WindowInfo { id: id.clone(), title: title.into(), pid: 1000 + d.next, programs: vec![Program::new(path)], class: String::new() };
        d.windows.push((info, controls));
        id
    }

    /// A program that can be started; a start opens a window titled `title` with a text area.
    pub fn install(&self, path: &str, title: &str) {
        let p = Program::new(path);
        self.desk().installed.push((p.exe, path.into(), title.into()));
    }

    /// Change a control's value from outside (the person typed meanwhile).
    pub fn set_value(&self, window: &str, control: &str, value: &str) {
        let mut d = self.desk();
        if let Some(c) = d.windows.iter_mut().find(|w| w.0.id == window).and_then(|w| w.1.iter_mut().find(|c| c.path == control)) {
            c.value = Some(value.into());
        }
    }

    /// Put another element at a control's place that looks the same (same role, name, value):
    /// only its runtime id differs.
    pub fn swap_element(&self, window: &str, control: &str) {
        let mut d = self.desk();
        if let Some(c) = d.windows.iter_mut().find(|w| w.0.id == window).and_then(|w| w.1.iter_mut().find(|c| c.path == control)) {
            c.runtime_id.push_str("-neu");
        }
    }

    /// Give a control the keyboard focus (and mark it a password field).
    pub fn focus(&self, window: &str, control: &str, password: bool) {
        let mut d = self.desk();
        if let Some(w) = d.windows.iter_mut().find(|w| w.0.id == window) {
            for c in w.1.iter_mut() {
                c.focused = c.path == control;
                if c.path == control {
                    c.password = password;
                }
            }
        }
    }

    pub fn log(&self) -> Vec<String> {
        self.desk().log.clone()
    }

    /// The windows, without counting as a look at the desktop.
    pub fn windows_now(&self) -> Vec<WindowInfo> {
        self.desk().windows.iter().map(|w| w.0.clone()).collect()
    }

    pub fn reads(&self) -> usize {
        self.desk().reads
    }

    pub fn value(&self, window: &str, control: &str) -> Option<String> {
        self.desk().windows.iter().find(|w| w.0.id == window).and_then(|w| w.1.iter().find(|c| c.path == control)).and_then(|c| c.value.clone())
    }

    fn with_control<R>(&self, window: &str, control: &str, f: impl FnOnce(&mut Control) -> Result<R>) -> Result<R> {
        let mut d = self.desk();
        let w = d.windows.iter_mut().find(|w| w.0.id == window).ok_or_else(|| Error::not_found("Dieses Fenster gibt es nicht mehr"))?;
        let c = w.1.iter_mut().find(|c| c.path == control).ok_or_else(|| Error::not_found(format!("Steuerelement {control} gibt es nicht")))?;
        f(c)
    }

    fn take_error(&self) -> Result<()> {
        match self.next_error.lock().unwrap_or_else(PoisonError::into_inner).take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn ensure_ready(&self) -> Result<()> {
        match self.not_ready.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            Some(why) => Err(Error::refused(why)),
            None => Ok(()),
        }
    }
}

impl DesktopAutomation for FakeDesktop {
    fn ready(&self) -> Result<()> {
        self.ensure_ready()
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        self.ensure_ready()?;
        let mut d = self.desk();
        d.reads += 1;
        Ok(d.windows.iter().map(|w| w.0.clone()).collect())
    }

    fn locate(&self, exe: &str) -> Option<String> {
        self.desk().installed.iter().find(|i| i.0 == exe.to_lowercase()).map(|i| i.1.clone())
    }

    fn launch(&self, path: &str) -> Result<()> {
        self.ensure_ready()?;
        let title = {
            let mut d = self.desk();
            let title =
                d.installed.iter().find(|i| i.1 == path).map(|i| i.2.clone()).ok_or_else(|| Error::not_found(format!("{path} ist nicht installiert")))?;
            d.log.push(format!("launch {path}"));
            title
        };
        self.add_window(path, &title, vec![control("1", "Dokument", "Textbereich", Some(""), &[Act::Type])]);
        Ok(())
    }

    fn snapshot(&self, window: &str) -> Result<Snapshot> {
        self.ensure_ready()?;
        let mut d = self.desk();
        d.reads += 1;
        let (info, controls) = d.windows.iter().find(|w| w.0.id == window).cloned().ok_or_else(|| Error::not_found("Dieses Fenster gibt es nicht mehr"))?;
        Ok(Snapshot { window: info, controls, truncated: false })
    }

    fn act(&self, window: &str, control: &Control, act: Act) -> Result<()> {
        self.take_error()?;
        self.ensure_ready()?;
        self.with_control(window, &control.path, |c| {
            if !c.same_element(control) {
                return Err(Error::refused("An dieser Stelle steht jetzt ein anderes Steuerelement; nichts gemacht."));
            }
            if !c.acts.contains(&act) {
                return Err(Error::unsupported(format!("„{}“ kann nicht {}", c.name, act.label())));
            }
            Ok(())
        })?;
        self.desk().log.push(format!("{} {window} {}", act.label(), control.path));
        Ok(())
    }

    fn set_text(&self, window: &str, control: &Control, text: &str, replace: bool) -> Result<()> {
        self.take_error()?;
        self.ensure_ready()?;
        let text = crate::typeable(text)?;
        self.with_control(window, &control.path, |c| {
            if !c.same_element(control) {
                return Err(Error::refused("An dieser Stelle steht jetzt ein anderes Steuerelement; nichts getippt."));
            }
            if c.password {
                return Err(Error::refused("In Passwortfelder tippt AIS.ChatCraft nie."));
            }
            if !c.acts.contains(&Act::Type) {
                return Err(Error::unsupported(format!("„{}“ nimmt keinen Text", c.name)));
            }
            let old = c.value.clone().unwrap_or_default();
            c.value = Some(if replace { text.clone() } else { format!("{old}{text}") });
            Ok(())
        })?;
        self.desk().log.push(format!("text {window} {} {text}", control.path));
        Ok(())
    }

    fn keys(&self, window: &str, control: Option<&Control>, keys: &KeyChord) -> Result<()> {
        self.take_error()?;
        self.ensure_ready()?;
        if let Some(control) = control {
            self.with_control(window, &control.path, |c| {
                if !c.same_element(control) {
                    return Err(Error::refused("An dieser Stelle steht jetzt ein anderes Steuerelement; keine Tasten gesendet."));
                }
                Ok(())
            })?;
        }
        let mut d = self.desk();
        if !d.windows.iter().any(|w| w.0.id == window) {
            return Err(Error::not_found("Dieses Fenster gibt es nicht mehr"));
        }
        d.log.push(format!("keys {window} {} {}", control.map(|c| c.path.as_str()).unwrap_or("-"), keys.to_uia()));
        Ok(())
    }

    fn close(&self, window: &str) -> Result<()> {
        let mut d = self.desk();
        d.windows.retain(|w| w.0.id != window);
        d.log.push(format!("close {window}"));
        Ok(())
    }
}
