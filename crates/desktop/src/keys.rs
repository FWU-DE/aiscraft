//! Keyboard shortcuts for one program: „Strg+S“, „Enter“, „Umschalt+Tab“.
//!
//! Only what works inside a program: no Windows key (Win+R opens „Ausführen“), no switching or
//! closing other windows' worlds (Alt+Tab, Alt+Esc), no Strg+Alt+Entf or Strg+Umschalt+Esc.

use aiscraft_core::{Error, Result};

/// One shortcut, pressed `times` times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// A named key in UI Automation's spelling (`ENTER`, `F5`) or one character.
    pub key: String,
    pub times: u32,
}

const NAMED: &[(&[&str], &str)] = &[
    (&["enter", "eingabe", "return", "eingabetaste"], "ENTER"),
    (&["tab", "tabulator"], "TAB"),
    (&["esc", "escape"], "ESC"),
    (&["backspace", "rücktaste", "zurück"], "BACKSPACE"),
    (&["delete", "del", "entf", "entfernen"], "DELETE"),
    (&["insert", "einfg", "einfügen"], "INSERT"),
    (&["home", "pos1"], "HOME"),
    (&["end", "ende"], "END"),
    (&["pageup", "bildauf", "bild auf", "page up"], "PAGEUP"),
    (&["pagedown", "bildab", "bild ab", "page down"], "PAGEDOWN"),
    (&["up", "hoch", "pfeil hoch", "nach oben"], "UP"),
    (&["down", "runter", "pfeil runter", "nach unten"], "DOWN"),
    (&["left", "links", "pfeil links"], "LEFT"),
    (&["right", "rechts", "pfeil rechts"], "RIGHT"),
    (&["space", "leertaste", "leer"], "SPACE"),
];

/// Parse „Strg+Umschalt+S“ (German or English names); `times` from 1 to 20.
pub fn parse_keys(text: &str, times: u32) -> Result<KeyChord> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 40 {
        return Err(Error::invalid("Kein gültiges Tastenkürzel (z. B. „Strg+S“ oder „Enter“)"));
    }
    if !(1..=20).contains(&times) {
        return Err(Error::invalid("times: 1 bis 20"));
    }
    let parts: Vec<String> = text.split('+').map(|p| p.trim().to_lowercase()).collect();
    let (mods, key) = match parts.split_last() {
        Some((k, m)) if !k.is_empty() => (m, k.clone()),
        // „Strg++“: the plus key itself.
        _ if text.ends_with("++") => (parts.get(..parts.len().saturating_sub(2)).unwrap_or(&[]), "+".to_string()),
        _ => return Err(Error::invalid("Kein gültiges Tastenkürzel")),
    };
    let mut chord = KeyChord { ctrl: false, alt: false, shift: false, key: String::new(), times };
    for m in mods {
        match m.as_str() {
            "strg" | "ctrl" | "control" | "steuerung" => chord.ctrl = true,
            "alt" => chord.alt = true,
            "shift" | "umschalt" | "umsch" | "hochstelltaste" => chord.shift = true,
            "win" | "windows" | "super" | "cmd" | "meta" => {
                return Err(Error::refused("Die Windows-Taste ist gesperrt: sie wirkt auf das ganze System, nicht auf ein Programm."));
            }
            "altgr" => {
                chord.ctrl = true;
                chord.alt = true;
            }
            other => return Err(Error::invalid(format!("Unbekannte Taste „{other}“"))),
        }
    }
    if matches!(key.as_str(), "win" | "windows" | "super" | "cmd" | "meta") {
        return Err(Error::refused("Die Windows-Taste ist gesperrt: sie wirkt auf das ganze System, nicht auf ein Programm."));
    }
    chord.key = if let Some((_, name)) = NAMED.iter().find(|(names, _)| names.contains(&key.as_str())) {
        (*name).to_string()
    } else if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()).filter(|n| (1..=12).contains(n)) {
        format!("F{n}")
    } else if key.chars().count() == 1 && key.chars().all(|c| !c.is_control()) {
        key.clone()
    } else {
        return Err(Error::invalid(format!("Unbekannte Taste „{key}“")));
    };
    // Shortcuts that leave the program or reach the system.
    let system = (chord.ctrl && chord.alt && chord.key == "DELETE")
        || (chord.ctrl && chord.shift && chord.key == "ESC")
        || (chord.alt && matches!(chord.key.as_str(), "TAB" | "ESC"))
        || (chord.ctrl && chord.key == "ESC");
    if system {
        return Err(Error::refused("Dieses Tastenkürzel wirkt auf das System oder andere Fenster und ist gesperrt."));
    }
    Ok(chord)
}

impl KeyChord {
    /// The spelling of UI Automation's keyboard (`{ctrl}{shift}s`, `{enter 3}`).
    pub fn to_uia(&self) -> String {
        let mut out = String::new();
        if self.ctrl {
            out.push_str("{ctrl}");
        }
        if self.alt {
            out.push_str("{alt}");
        }
        if self.shift {
            out.push_str("{shift}");
        }
        let single = self.key.chars().count() == 1;
        let key = if single && "{}()".contains(self.key.as_str()) { format!("{{{}}}", self.key) } else { self.key.clone() };
        if single {
            out.push_str(&key.repeat(self.times as usize));
        } else if self.times > 1 {
            out.push_str(&format!("{{{} {}}}", self.key, self.times));
        } else {
            out.push_str(&format!("{{{}}}", self.key));
        }
        out
    }

    /// For people: „Strg+Umschalt+S“.
    pub fn shown(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Strg".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        if self.shift {
            parts.push("Umschalt".to_string());
        }
        parts.push(if self.key.chars().count() == 1 { self.key.to_uppercase() } else { self.key.clone() });
        let s = parts.join("+");
        if self.times > 1 { format!("{s} ×{}", self.times) } else { s }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_in_german_and_english() {
        let k = parse_keys("Strg+Umschalt+S", 1).unwrap();
        assert!(k.ctrl && k.shift && !k.alt && k.key == "s");
        assert_eq!(k.to_uia(), "{ctrl}{shift}s");
        assert_eq!(k.shown(), "Strg+Umschalt+S");
        assert_eq!(parse_keys("Enter", 3).unwrap().to_uia(), "{ENTER 3}");
        assert_eq!(parse_keys("ctrl+a", 1).unwrap().to_uia(), "{ctrl}a");
        assert_eq!(parse_keys("Pos1", 1).unwrap().key, "HOME");
        assert_eq!(parse_keys("F5", 1).unwrap().to_uia(), "{F5}");
        assert_eq!(parse_keys("Strg+(", 1).unwrap().to_uia(), "{ctrl}{(}");
        assert_eq!(parse_keys("Strg++", 1).unwrap().key, "+");
    }

    #[test]
    fn system_shortcuts_and_the_windows_key_are_refused() {
        for bad in ["Win+R", "Windows", "Strg+Alt+Entf", "Strg+Umschalt+Esc", "Alt+Tab", "Alt+Esc", "Strg+Esc"] {
            let e = parse_keys(bad, 1).unwrap_err();
            assert_eq!(e.kind, aiscraft_core::ErrorKind::Refused, "{bad}");
        }
        for bad in ["", "Strg+", "Hyper+X", "F13", "Enter", "x".repeat(50).as_str()] {
            if bad == "Enter" {
                assert!(parse_keys(bad, 0).is_err() && parse_keys(bad, 21).is_err());
            } else {
                assert!(parse_keys(bad, 1).is_err(), "{bad}");
            }
        }
    }
}
