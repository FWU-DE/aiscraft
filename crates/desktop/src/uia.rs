//! Windows: UI Automation through the `uiautomation` crate, on one thread of its own (COM).
//!
//! Reading walks the control view. Steps use patterns (Invoke, Toggle, SelectionItem,
//! ExpandCollapse, Value); the pointer never moves. Keys and typed text need the window in front:
//! it is brought there, and nothing is sent unless the keyboard focus really is in that window.

use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use aiscraft_core::{Error, Result};
use uiautomation::inputs::Keyboard;
use uiautomation::patterns::{
    UIExpandCollapsePattern, UIInvokePattern, UISelectionItemPattern, UITextPattern, UITogglePattern, UIValuePattern, UIWindowPattern,
};
use uiautomation::types::{ControlType, ExpandCollapseState, Handle, WindowVisualState};
use uiautomation::{UIAutomation, UIElement, UITreeWalker};

use crate::{Act, Control, DesktopAutomation, KeyChord, MAX_CONTROLS, MAX_DEPTH, MAX_DOC_TEXT, MAX_TEXT, Program, Snapshot, WindowInfo, cap};

/// How long one call may take before the caller gives up (a program that hangs).
const TIMEOUT: Duration = Duration::from_secs(30);

struct Worker {
    auto: UIAutomation,
    walker: UITreeWalker,
    sys: sysinfo::System,
}

type Job = Box<dyn FnOnce(std::result::Result<&mut Worker, &Error>) + Send>;

pub(crate) struct Uia {
    tx: Mutex<Option<mpsc::Sender<Job>>>,
}

fn uia_err(what: &str, e: uiautomation::Error) -> Error {
    Error::unsupported(format!("{what}: {e}"))
}

impl Uia {
    pub(crate) fn new() -> Uia {
        Uia { tx: Mutex::new(None) }
    }

    fn sender(&self) -> Result<mpsc::Sender<Job>> {
        let mut guard = self.tx.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tx) = guard.as_ref() {
            return Ok(tx.clone());
        }
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("aiscraft-uia".into())
            .spawn(move || {
                let worker = UIAutomation::new().and_then(|auto| {
                    let walker = auto.get_control_view_walker()?;
                    Ok(Worker { auto, walker, sys: sysinfo::System::new() })
                });
                let mut worker = worker.map_err(|e| Error::unsupported(format!("Die Bedienungshilfen von Windows (UI Automation) sind nicht erreichbar: {e}")));
                for job in rx {
                    match worker.as_mut() {
                        Ok(w) => job(Ok(w)),
                        Err(e) => job(Err(e)),
                    }
                }
            })
            .map_err(|e| Error::unsupported(format!("Bedienungshilfen: {e}")))?;
        *guard = Some(tx.clone());
        Ok(tx)
    }

    fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut Worker) -> Result<R> + Send + 'static) -> Result<R> {
        let (rtx, rrx) = mpsc::channel();
        let job: Job = Box::new(move |w| {
            let r = match w {
                Ok(w) => f(w),
                Err(e) => Err(e.clone()),
            };
            let _ = rtx.send(r);
        });
        if self.sender()?.send(job).is_err() {
            // The thread ended; start a new one next time.
            *self.tx.lock().unwrap_or_else(PoisonError::into_inner) = None;
            return Err(Error::unsupported("Die Bedienungshilfen von Windows antworten nicht mehr; bitte erneut versuchen."));
        }
        rrx.recv_timeout(TIMEOUT).unwrap_or_else(|_| Err(Error::busy("Das Programm antwortet nicht (30 s). Ist es beschäftigt oder hängt es?")))
    }
}

fn hwnd_of(id: &str) -> Result<isize> {
    id.strip_prefix('w').and_then(|n| n.parse::<isize>().ok()).filter(|n| *n != 0).ok_or_else(|| Error::invalid(format!("Kein gültiges Fenster: {id}")))
}

impl Worker {
    fn program(&mut self, pid: u32) -> Program {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
        let p = Pid::from_u32(pid);
        self.sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing().with_exe(UpdateKind::Always));
        match self.sys.process(p) {
            Some(proc_) => match proc_.exe() {
                Some(path) => Program::new(&path.display().to_string()),
                // Without a path the program is not recognised (and so refused).
                None => Program { exe: proc_.name().to_string_lossy().to_lowercase(), path: String::new() },
            },
            None => Program { exe: format!("pid {pid}"), path: String::new() },
        }
    }

    fn info(&mut self, el: &UIElement) -> Result<WindowInfo> {
        let handle: isize = el.get_native_window_handle().map_err(|e| uia_err("Fenster", e))?.into();
        let pid = el.get_process_id().map_err(|e| uia_err("Fenster", e))?;
        let title = el.get_name().unwrap_or_default();
        let mut programs = vec![self.program(pid)];
        // Store apps: the host owns the frame, the app the content. Judge both.
        if programs.first().is_some_and(|p| crate::policy::is_host(&p.exe)) {
            let mut pids = Vec::new();
            let mut child = self.walker.get_first_child(el).ok();
            let mut n = 0;
            while let Some(c) = child {
                n += 1;
                if let Ok(cp) = c.get_process_id()
                    && cp != pid
                    && !pids.contains(&cp)
                {
                    pids.push(cp);
                }
                if n > 50 {
                    break;
                }
                child = self.walker.get_next_sibling(&c).ok();
            }
            for cp in pids {
                let p = self.program(cp);
                programs.push(p);
            }
        }
        Ok(WindowInfo { id: format!("w{handle}"), title, pid, programs })
    }

    fn window(&mut self, id: &str) -> Result<UIElement> {
        let h = hwnd_of(id)?;
        self.auto.element_from_handle(Handle::from(h)).map_err(|_| Error::not_found("Dieses Fenster gibt es nicht mehr. Bitte die Programme neu auflisten."))
    }

    fn resolve(&self, window: &UIElement, path: &str) -> Result<UIElement> {
        let mut el = window.clone();
        for part in path.split('.') {
            let i: usize = part.parse().ok().filter(|i| *i >= 1).ok_or_else(|| Error::invalid(format!("Kein gültiges Steuerelement: {path}")))?;
            let mut child = self.walker.get_first_child(&el).map_err(|_| Error::not_found(format!("Steuerelement {path} gibt es nicht (mehr)")))?;
            for _ in 1..i {
                child = self.walker.get_next_sibling(&child).map_err(|_| Error::not_found(format!("Steuerelement {path} gibt es nicht (mehr)")))?;
            }
            el = child;
        }
        Ok(el)
    }

    fn control(&self, el: &UIElement, path: String, depth: usize) -> Control {
        let kind = el.get_control_type().ok();
        let password = el.is_password().unwrap_or(false);
        let mut acts = Vec::new();
        if el.get_pattern::<UIInvokePattern>().is_ok() {
            acts.push(Act::Press);
        }
        if el.get_pattern::<UITogglePattern>().is_ok() {
            acts.push(Act::Toggle);
        }
        if el.get_pattern::<UISelectionItemPattern>().is_ok() {
            acts.push(Act::Select);
        }
        if el.get_pattern::<UIExpandCollapsePattern>().is_ok() {
            acts.push(Act::Expand);
        }
        let value_pattern = el.get_pattern::<UIValuePattern>().ok();
        let writable = value_pattern.as_ref().is_some_and(|v| !v.is_readonly().unwrap_or(true));
        let texty = matches!(kind, Some(ControlType::Edit) | Some(ControlType::Document));
        if writable || (texty && el.is_keyboard_focusable().unwrap_or(false)) {
            acts.push(Act::Type);
        }
        let value = if password {
            None
        } else {
            let v = value_pattern.and_then(|v| v.get_value().ok()).filter(|v| !v.is_empty());
            match v {
                Some(v) => Some(cap(&v, MAX_DOC_TEXT)),
                None if texty => el
                    .get_pattern::<UITextPattern>()
                    .ok()
                    .and_then(|t| t.get_document_range().ok())
                    .and_then(|r| r.get_text(MAX_DOC_TEXT as i32 + 1).ok())
                    .map(|t| cap(&t, MAX_DOC_TEXT)),
                None => None,
            }
        };
        Control {
            path,
            depth,
            role: kind.map(role).unwrap_or("Element").to_string(),
            name: cap(&el.get_name().unwrap_or_default(), MAX_TEXT),
            value,
            enabled: el.is_enabled().unwrap_or(false),
            focused: el.has_keyboard_focus().unwrap_or(false),
            password,
            acts,
        }
    }

    fn walk(&self, el: &UIElement, prefix: &str, depth: usize, out: &mut Vec<Control>, truncated: &mut bool) {
        if depth > MAX_DEPTH {
            *truncated = true;
            return;
        }
        let mut child = self.walker.get_first_child(el).ok();
        let mut i = 0usize;
        while let Some(c) = child {
            if out.len() >= MAX_CONTROLS {
                *truncated = true;
                return;
            }
            i += 1;
            let path = if prefix.is_empty() { i.to_string() } else { format!("{prefix}.{i}") };
            out.push(self.control(&c, path.clone(), depth));
            self.walk(&c, &path, depth + 1, out, truncated);
            child = self.walker.get_next_sibling(&c).ok();
        }
    }

    /// Bring the window to the front, focus the element, and check the keyboard focus is inside
    /// the window. Nothing is typed otherwise.
    fn focus_inside(&self, window: &UIElement, target: &UIElement) -> Result<()> {
        if let Ok(wp) = window.get_pattern::<UIWindowPattern>()
            && wp.get_window_visual_state().ok() == Some(WindowVisualState::Minimized)
        {
            let _ = wp.set_window_visual_state(WindowVisualState::Normal);
        }
        let _ = window.set_focus();
        target.set_focus().map_err(|e| uia_err("Das Fenster ließ sich nicht nach vorn holen", e))?;
        std::thread::sleep(Duration::from_millis(150));
        let hwnd: isize = window.get_native_window_handle().map(Into::into).unwrap_or(0);
        let focused = self.auto.get_focused_element().map_err(|e| uia_err("Fokus", e))?;
        let root = self.auto.get_root_element().map_err(|e| uia_err("Fokus", e))?;
        let raw = self.auto.get_raw_view_walker().map_err(|e| uia_err("Fokus", e))?;
        let mut el = focused;
        for _ in 0..200 {
            let h: isize = el.get_native_window_handle().map(Into::into).unwrap_or(0);
            if h != 0 && h == hwnd {
                return Ok(());
            }
            match raw.get_parent(&el) {
                Ok(p) if !self.auto.compare_elements(&p, &root).unwrap_or(true) => el = p,
                _ => break,
            }
        }
        Err(Error::refused("Das Fenster hat den Fokus nicht bekommen (ein anderes Fenster ist vorn). Nichts getippt; bitte erneut versuchen."))
    }
}

fn role(t: ControlType) -> &'static str {
    match t {
        ControlType::Button => "Schaltfläche",
        ControlType::Calendar => "Kalender",
        ControlType::CheckBox => "Kontrollkästchen",
        ControlType::ComboBox => "Auswahlfeld",
        ControlType::Edit => "Textfeld",
        ControlType::Hyperlink => "Link",
        ControlType::Image => "Bild",
        ControlType::ListItem => "Listeneintrag",
        ControlType::List => "Liste",
        ControlType::Menu => "Menü",
        ControlType::MenuBar => "Menüleiste",
        ControlType::MenuItem => "Menüpunkt",
        ControlType::ProgressBar => "Fortschritt",
        ControlType::RadioButton => "Optionsfeld",
        ControlType::ScrollBar => "Bildlaufleiste",
        ControlType::Slider => "Schieberegler",
        ControlType::Spinner => "Drehfeld",
        ControlType::StatusBar => "Statusleiste",
        ControlType::Tab => "Registerkarten",
        ControlType::TabItem => "Registerkarte",
        ControlType::Text => "Text",
        ControlType::ToolBar => "Symbolleiste",
        ControlType::ToolTip => "Hinweis",
        ControlType::Tree => "Baum",
        ControlType::TreeItem => "Baumeintrag",
        ControlType::Group => "Gruppe",
        ControlType::Thumb => "Griff",
        ControlType::DataGrid => "Tabelle",
        ControlType::DataItem => "Tabellenzeile",
        ControlType::Document => "Dokument",
        ControlType::SplitButton => "Teilungsschaltfläche",
        ControlType::Window => "Fenster",
        ControlType::Pane => "Bereich",
        ControlType::Header => "Kopfzeile",
        ControlType::HeaderItem => "Spaltenkopf",
        ControlType::Table => "Tabelle",
        ControlType::TitleBar => "Titelleiste",
        ControlType::Separator => "Trennlinie",
        ControlType::SemanticZoom => "Zoom",
        ControlType::AppBar => "App-Leiste",
        _ => "Element",
    }
}

impl DesktopAutomation for Uia {
    fn ready(&self) -> Result<()> {
        if check_elevation::is_elevated().unwrap_or(true) {
            return Err(Error::refused(
                "AIS.ChatCraft läuft mit Administratorrechten (oder das ließ sich nicht prüfen). Dann bedient es keine Programme; bitte ohne Administratorrechte starten.",
            ));
        }
        Ok(())
    }

    fn windows(&self) -> Result<Vec<WindowInfo>> {
        self.ready()?;
        self.run(|w| {
            let root = w.auto.get_root_element().map_err(|e| uia_err("Desktop", e))?;
            let mut out = Vec::new();
            let mut child = w.walker.get_first_child(&root).ok();
            let mut n = 0;
            while let Some(c) = child {
                n += 1;
                if n > 300 {
                    break;
                }
                if !c.get_name().unwrap_or_default().trim().is_empty()
                    && let Ok(info) = w.info(&c)
                    && info.id != "w0"
                {
                    out.push(info);
                }
                child = w.walker.get_next_sibling(&c).ok();
            }
            Ok(out)
        })
    }

    fn locate(&self, exe: &str) -> Option<String> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let exe = exe.trim();
        if exe.is_empty() || exe.contains(['\\', '/', ':', '"']) {
            return None;
        }
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
            if let Ok(k) = RegKey::predef(hive).open_subkey(&key)
                && let Ok(v) = k.get_value::<String, _>("")
            {
                let p = v.trim().trim_matches('"').to_string();
                if std::path::Path::new(&p).is_file() {
                    return Some(p);
                }
            }
        }
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let candidates = [format!(r"{windir}\System32\{exe}"), format!(r"{windir}\{exe}"), format!(r"{local}\Microsoft\WindowsApps\{exe}")];
        // App execution aliases are links that `is_file` does not follow; `symlink_metadata` sees them.
        candidates.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
    }

    fn launch(&self, path: &str) -> Result<()> {
        self.ready()?;
        std::process::Command::new(path).spawn().map(|_| ()).map_err(|e| Error::unsupported(format!("{path} ließ sich nicht starten: {e}")))
    }

    fn snapshot(&self, window: &str) -> Result<Snapshot> {
        self.ready()?;
        let id = window.to_string();
        self.run(move |w| {
            let el = w.window(&id)?;
            let info = w.info(&el)?;
            let mut controls = Vec::new();
            let mut truncated = false;
            w.walk(&el, "", 1, &mut controls, &mut truncated);
            Ok(Snapshot { window: info, controls, truncated })
        })
    }

    fn act(&self, window: &str, control: &str, act: Act) -> Result<()> {
        self.ready()?;
        let (id, path) = (window.to_string(), control.to_string());
        self.run(move |w| {
            let win = w.window(&id)?;
            let el = w.resolve(&win, &path)?;
            let r = match act {
                Act::Press => el.get_pattern::<UIInvokePattern>().and_then(|p| p.invoke()),
                Act::Toggle => el.get_pattern::<UITogglePattern>().and_then(|p| p.toggle()),
                Act::Select => el.get_pattern::<UISelectionItemPattern>().and_then(|p| p.select()),
                Act::Expand => el.get_pattern::<UIExpandCollapsePattern>().and_then(|p| match p.get_state() {
                    Ok(ExpandCollapseState::Expanded) => p.collapse(),
                    _ => p.expand(),
                }),
                Act::Type => return Err(Error::invalid("Für Text computer_type verwenden")),
            };
            r.map_err(|e| uia_err(&format!("Das Steuerelement kann nicht {}", act.label()), e))
        })
    }

    fn set_text(&self, window: &str, control: &str, text: &str, replace: bool) -> Result<()> {
        self.ready()?;
        let (id, path, text) = (window.to_string(), control.to_string(), text.to_string());
        self.run(move |w| {
            let win = w.window(&id)?;
            let el = w.resolve(&win, &path)?;
            if el.is_password().unwrap_or(true) {
                return Err(Error::refused("In Passwortfelder tippt AIS.ChatCraft nie."));
            }
            if replace
                && let Ok(v) = el.get_pattern::<UIValuePattern>()
                && !v.is_readonly().unwrap_or(true)
                && v.set_value(&text).is_ok()
            {
                return Ok(());
            }
            w.focus_inside(&win, &el)?;
            let kb = Keyboard::new().interval(2);
            if replace {
                kb.send_keys("{ctrl}a{DELETE}").map_err(|e| uia_err("Tippen", e))?;
            }
            kb.send_text(&text).map_err(|e| uia_err("Tippen", e))
        })
    }

    fn keys(&self, window: &str, control: Option<&str>, keys: &KeyChord) -> Result<()> {
        self.ready()?;
        let (id, path, keys) = (window.to_string(), control.map(str::to_string), keys.to_uia());
        self.run(move |w| {
            let win = w.window(&id)?;
            let el = match &path {
                Some(p) => w.resolve(&win, p)?,
                None => win.clone(),
            };
            w.focus_inside(&win, &el)?;
            Keyboard::new().interval(20).send_keys(&keys).map_err(|e| uia_err("Tasten", e))
        })
    }

    fn close(&self, window: &str) -> Result<()> {
        self.ready()?;
        let id = window.to_string();
        self.run(move |w| {
            let win = w.window(&id)?;
            win.get_pattern::<UIWindowPattern>().and_then(|p| p.close()).map_err(|e| uia_err("Schließen", e))
        })
    }
}
