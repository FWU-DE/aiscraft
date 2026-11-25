//! Computer use (#97), as in n8n's Nod: the model operates programs on the person's own computer
//! through the system's accessibility APIs, like a screen reader. Windows first (UI Automation);
//! macOS and Linux say „noch nicht verfügbar“.
//!
//! This crate knows windows, controls and keys. It never moves the pointer and never captures the
//! screen. What may run when (trust check, approvals, the request a program was opened for) is the
//! engine's business (`engine/src/computer.rs`); the hard block list lives here
//! ([`policy`]) so every backend and the engine share it.

use std::sync::Arc;

use aiscraft_core::{Error, Result};
use serde::Serialize;

pub mod fake;
mod keys;
pub mod policy;
#[cfg(windows)]
mod uia;

pub use keys::{KeyChord, parse_keys};

/// Most controls one read lists.
pub const MAX_CONTROLS: usize = 400;
/// Deepest level of the control tree a read walks.
pub const MAX_DEPTH: usize = 30;
/// Most characters of a control's name or value.
pub const MAX_TEXT: usize = 300;
/// Most characters of a document's or text field's text.
pub const MAX_DOC_TEXT: usize = 8000;
/// Most characters of a whole read.
pub const MAX_OUTLINE: usize = 40_000;
/// Most characters of the excerpt the trust check sees.
pub const MAX_EXCERPT: usize = 3000;
/// Most characters typed in one step.
pub const MAX_TYPE: usize = 5000;
/// Characters typed per keyboard step; the focus is checked again before each.
pub const TYPE_CHUNK: usize = 200;

/// A program behind a window: its file name (lowercase, `notepad.exe`) and full path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Program {
    pub exe: String,
    pub path: String,
}

impl Program {
    pub fn new(path: &str) -> Program {
        let exe = path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
        Program { exe, path: path.to_string() }
    }
}

/// A top-level window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowInfo {
    /// Stable while the window lives (`w` and the window handle on Windows).
    pub id: String,
    pub title: String,
    pub pid: u32,
    /// The program that owns the window first, then programs that show content in it (an app
    /// hosted by ApplicationFrameHost). An empty path means the program could not be read.
    pub programs: Vec<Program>,
    /// The window class (`#32770` for dialogs, `wndclass_desked_gsk` for the Office VBA editor).
    #[serde(skip)]
    pub class: String,
}

impl WindowInfo {
    /// The program people know the window by (not the host).
    pub fn program(&self) -> Option<&Program> {
        self.programs.iter().find(|p| !policy::is_host(&p.exe)).or_else(|| self.programs.first())
    }
}

/// What a control can do, through UI Automation patterns (never a click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Act {
    /// Invoke: a button, a menu item, a link.
    Press,
    /// Toggle: a check box, a toggle button.
    Toggle,
    /// SelectionItem: an entry of a list, a tab, a radio button.
    Select,
    /// ExpandCollapse: a menu, a combo box, a tree item.
    Expand,
    /// Value or text input.
    Type,
}

impl Act {
    pub fn label(self) -> &'static str {
        match self {
            Act::Press => "drücken",
            Act::Toggle => "umschalten",
            Act::Select => "auswählen",
            Act::Expand => "auf-/zuklappen",
            Act::Type => "Text",
        }
    }
}

/// One control of a window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Control {
    /// Position in the control tree, 1-based per level: `2.1.3`.
    pub path: String,
    pub depth: usize,
    /// The kind in German (Schaltfläche, Textfeld, Dokument …).
    pub role: String,
    pub name: String,
    pub value: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub password: bool,
    pub acts: Vec<Act>,
    /// The class name (`SHELLDLL_DefView` marks a file dialog's folder view).
    #[serde(skip)]
    pub class: String,
    /// UI Automation's runtime id: the same element, not just the same place. A step checks it
    /// right before it acts.
    #[serde(skip)]
    pub runtime_id: String,
}

impl Control {
    /// Whether `other` (read just now) is the element this control describes.
    pub fn same_element(&self, other: &Control) -> bool {
        self.path == other.path && self.role == other.role && self.name == other.name && self.runtime_id == other.runtime_id
    }
}

/// Text that may be typed: no control characters (they would type as shortcuts: U+0013 is
/// Strg+S, a line feed is Strg+Enter, which sends a mail in Outlook), except line breaks and
/// tabs, which are sent as plain Enter and Tab. `\r\n` and `\r` become `\n`.
pub fn typeable(text: &str) -> Result<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if let Some(bad) = text
        .chars()
        .find(|c| (c.is_control() && *c != '\n' && *c != '\t') || matches!(*c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
    {
        return Err(Error::refused(format!(
            "Der Text enthält ein Steuerzeichen (U+{:04X}); das würde als Tastenkürzel ankommen. Nur Text, Zeilenumbrüche und Tabulatoren.",
            bad as u32
        )));
    }
    if text.chars().count() > MAX_TYPE {
        return Err(Error::invalid(format!("Der Text ist länger als {MAX_TYPE} Zeichen.")));
    }
    Ok(text)
}

/// What a read of a window found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Snapshot {
    pub window: WindowInfo,
    pub controls: Vec<Control>,
    /// More controls than [`MAX_CONTROLS`] or deeper than [`MAX_DEPTH`].
    pub truncated: bool,
}

pub fn cap(s: &str, n: usize) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

fn one_line(s: &str, n: usize) -> String {
    cap(&s.replace(['\r', '\n', '\t'], " "), n)
}

impl Control {
    /// One line of the outline. `text` caps name and value.
    fn line(&self, text: usize, with_focus: bool) -> String {
        let mut l = format!("{}[{}] {}", "  ".repeat(self.depth.saturating_sub(1)), self.path, self.role);
        if !self.name.is_empty() {
            l.push_str(&format!(" „{}“", one_line(&self.name, MAX_TEXT)));
        }
        if self.password {
            l.push_str(" (Passwortfeld, Inhalt verborgen)");
        } else if let Some(v) = self.value.as_deref().filter(|v| !v.is_empty()) {
            l.push_str(&format!(" = „{}“", one_line(v, text)));
        }
        if !self.enabled {
            l.push_str(" (gesperrt)");
        }
        if with_focus && self.focused {
            l.push_str(" (Fokus)");
        }
        if !self.acts.is_empty() && self.enabled {
            l.push_str(&format!(" → {}", self.acts.iter().map(|a| a.label()).collect::<Vec<_>>().join(", ")));
        }
        l
    }

    fn parent_path(&self) -> &str {
        self.path.rsplit_once('.').map(|(p, _)| p).unwrap_or("")
    }
}

impl Snapshot {
    pub fn control(&self, path: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.path == path)
    }

    /// A password field has the keyboard focus (no picture, no typing).
    pub fn password_focused(&self) -> bool {
        self.controls.iter().any(|c| c.password && c.focused)
    }

    fn heading(&self) -> String {
        let program = self.window.program().map(|p| p.exe.clone()).unwrap_or_default();
        format!("Fenster {} „{}“ ({program})", self.window.id, one_line(&self.window.title, MAX_TEXT))
    }

    /// The whole window as an indented outline, for the model.
    pub fn outline(&self) -> String {
        let mut out = format!("{}\n", self.heading());
        for c in &self.controls {
            let line = c.line(MAX_DOC_TEXT, true);
            if out.len() + line.len() > MAX_OUTLINE {
                out.push_str("… (gekürzt)\n");
                return out;
            }
            out.push_str(&line);
            out.push('\n');
        }
        if self.controls.is_empty() {
            out.push_str("(keine lesbaren Steuerelemente)\n");
        }
        if self.truncated {
            out.push_str(&format!("… (gekürzt: höchstens {MAX_CONTROLS} Steuerelemente)\n"));
        }
        out
    }

    /// What the trust check sees and what must not change before a step runs: the window, the
    /// control, its parent, its neighbours and first children. Without a control, the start of
    /// the window. Focus is left out: it moves when the person answers in AIS.ChatCraft.
    pub fn excerpt(&self, control: Option<&str>) -> String {
        // The window and the control itself always fit; the neighbourhood fills what is left.
        let mut head = self.heading();
        if let Some(target) = control.and_then(|p| self.control(p)) {
            head.push_str(&format!("\nDieses Steuerelement: {}", target.line(1200, false).trim_start()));
        }
        let head = cap(&head, MAX_EXCERPT / 2);
        let mut lines = Vec::new();
        let picked: Vec<&Control> = match control.and_then(|p| self.control(p)) {
            Some(target) => {
                let parent = target.parent_path();
                let siblings: Vec<&Control> = self.controls.iter().filter(|c| c.parent_path() == parent && c.depth == target.depth).collect();
                let at = siblings.iter().position(|c| c.path == target.path).unwrap_or(0);
                let near: Vec<&str> = siblings.iter().skip(at.saturating_sub(6)).take(13).map(|c| c.path.as_str()).collect();
                let children = format!("{}.", target.path);
                let mut kids = 0usize;
                self.controls
                    .iter()
                    .filter(|c| {
                        if c.path == parent || near.contains(&c.path.as_str()) {
                            return true;
                        }
                        if c.path.starts_with(&children) && c.depth == target.depth + 1 && kids < 10 {
                            kids += 1;
                            return true;
                        }
                        false
                    })
                    .collect()
            }
            None => self.controls.iter().take(30).collect(),
        };
        for c in picked {
            let mut l = c.line(600, false);
            if control == Some(c.path.as_str()) {
                l.push_str("   ← dieses Steuerelement");
            }
            lines.push(l);
        }
        let room = MAX_EXCERPT.saturating_sub(head.chars().count() + 1);
        format!("{head}\n{}", cap(&lines.join("\n"), room))
    }
}

/// The accessibility APIs of one system.
pub trait DesktopAutomation: Send + Sync {
    /// `Err` with a German reason when nothing can be operated here (other systems, AIS.ChatCraft
    /// running with administrator rights).
    fn ready(&self) -> Result<()>;
    /// The visible top-level windows.
    fn windows(&self) -> Result<Vec<WindowInfo>>;
    /// The full path of an installed program by its file name (`winword.exe`).
    fn locate(&self, exe: &str) -> Option<String>;
    /// Start a program (no arguments). The engine checked the path first.
    fn launch(&self, path: &str) -> Result<()>;
    /// Read a window's controls (bounded).
    fn snapshot(&self, window: &str) -> Result<Snapshot>;
    /// Press, toggle, select or expand a control through its pattern. In the same step the
    /// element at `control.path` must still be `control` ([`Control::same_element`]).
    fn act(&self, window: &str, control: &Control, act: Act) -> Result<()>;
    /// Put [`typeable`] text into a control: `replace` sets the whole value, otherwise it is typed
    /// at the cursor in chunks of [`TYPE_CHUNK`], each only while the control has the focus.
    fn set_text(&self, window: &str, control: &Control, text: &str, replace: bool) -> Result<()>;
    /// Keys to the window (or one of its controls); the window comes to the front first, and
    /// nothing is sent unless the focus really is there then.
    fn keys(&self, window: &str, control: Option<&Control>, keys: &KeyChord) -> Result<()>;
    /// Close a window (its program may ask about unsaved changes).
    fn close(&self, window: &str) -> Result<()>;
}

/// Systems without a backend yet.
pub struct Unavailable;

pub fn not_yet() -> Error {
    let os = if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    Error::unsupported(format!("Programme bedienen ist für {os} noch nicht verfügbar; es gibt es zuerst für Windows."))
}

impl DesktopAutomation for Unavailable {
    fn ready(&self) -> Result<()> {
        Err(not_yet())
    }
    fn windows(&self) -> Result<Vec<WindowInfo>> {
        Err(not_yet())
    }
    fn locate(&self, _exe: &str) -> Option<String> {
        None
    }
    fn launch(&self, _path: &str) -> Result<()> {
        Err(not_yet())
    }
    fn snapshot(&self, _window: &str) -> Result<Snapshot> {
        Err(not_yet())
    }
    fn act(&self, _window: &str, _control: &Control, _act: Act) -> Result<()> {
        Err(not_yet())
    }
    fn set_text(&self, _window: &str, _control: &Control, _text: &str, _replace: bool) -> Result<()> {
        Err(not_yet())
    }
    fn keys(&self, _window: &str, _control: Option<&Control>, _keys: &KeyChord) -> Result<()> {
        Err(not_yet())
    }
    fn close(&self, _window: &str) -> Result<()> {
        Err(not_yet())
    }
}

/// The backend of this system: UI Automation on Windows, [`Unavailable`] elsewhere.
pub fn system() -> Arc<dyn DesktopAutomation> {
    #[cfg(windows)]
    {
        Arc::new(uia::Uia::new())
    }
    #[cfg(not(windows))]
    {
        Arc::new(Unavailable)
    }
}

/// Whether this system has a backend at all (for the settings).
pub fn supported() -> bool {
    cfg!(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snapshot {
        let c = |path: &str, depth, role: &str, name: &str, value: Option<&str>, acts: Vec<Act>| Control {
            path: path.into(),
            depth,
            role: role.into(),
            name: name.into(),
            value: value.map(str::to_string),
            enabled: true,
            focused: false,
            password: false,
            acts,
            class: String::new(),
            runtime_id: format!("rt-{path}"),
        };
        let mut pw = c("2", 1, "Textfeld", "Passwort", Some("geheim"), vec![Act::Type]);
        pw.password = true;
        pw.focused = true;
        Snapshot {
            window: WindowInfo {
                id: "w1".into(),
                title: "Brief – Editor".into(),
                pid: 7,
                programs: vec![Program::new(r"C:\Windows\notepad.exe")],
                class: String::new(),
            },
            controls: vec![
                c("1", 1, "Menüleiste", "", None, vec![]),
                c("1.1", 2, "Menüpunkt", "Datei", None, vec![Act::Expand]),
                c("1.2", 2, "Menüpunkt", "Bearbeiten", None, vec![Act::Expand]),
                pw,
                c("3", 1, "Dokument", "Textbereich", Some("Liebe Eltern,\nam Freitag"), vec![Act::Type]),
            ],
            truncated: false,
        }
    }

    #[test]
    fn outline_hides_passwords_and_excerpt_names_the_control() {
        let s = snap();
        let o = s.outline();
        assert!(o.contains("[1.1]   ") || o.contains("  [1.1] Menüpunkt „Datei“ → auf-/zuklappen"), "{o}");
        assert!(!o.contains("geheim") && o.contains("Passwortfeld"), "{o}");
        assert!(s.password_focused());
        let e = s.excerpt(Some("1.2"));
        assert!(
            e.contains("[1.2] Menüpunkt „Bearbeiten“") && e.contains("← dieses Steuerelement") && e.contains("[1.1]") && e.contains("[1] Menüleiste"),
            "{e}"
        );
        assert!(!e.contains("Textbereich"), "only the neighbourhood: {e}");
        assert!(!e.contains("Fokus"), "focus moves when the person answers: {e}");
        assert!(s.excerpt(None).contains("Liebe Eltern"));
        // A huge neighbourhood never pushes the control itself out of the excerpt.
        let mut big = s.clone();
        for i in 4..200 {
            big.controls.push(Control { path: i.to_string(), depth: 1, name: "x".repeat(500), ..s.controls[1].clone() });
        }
        let e = big.excerpt(Some("3"));
        assert!(e.contains("Dieses Steuerelement: [3] Dokument „Textbereich“") && e.chars().count() <= MAX_EXCERPT + 2, "{}", e.chars().count());
    }

    #[test]
    fn control_characters_are_never_typed() {
        assert_eq!(typeable("Milch\r\nBrot\tKäse").unwrap(), "Milch\nBrot\tKäse");
        for bad in ["a\u{13}", "\u{1}", "x\u{16}", "\u{f}", "\u{7f}", "\u{85}", "\u{202e}evil"] {
            assert_eq!(typeable(bad).unwrap_err().kind, aiscraft_core::ErrorKind::Refused, "{bad:?}");
        }
        assert!(typeable(&"a".repeat(MAX_TYPE + 1)).is_err());
        assert_eq!(Program::new(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE").exe, "winword.exe");
    }

    #[test]
    fn other_systems_say_not_yet() {
        let u = Unavailable;
        assert!(u.ready().unwrap_err().message.contains("noch nicht verfügbar"));
        assert!(u.windows().is_err() && u.locate("notepad.exe").is_none());
    }
}
