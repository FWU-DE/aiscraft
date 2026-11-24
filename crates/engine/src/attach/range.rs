//! "Nur Seiten 3-7": the part of a file a person picked, parsed strictly.

use aiscraft_core::{Error, Result};

/// Longest range text accepted.
const MAX_SPEC_CHARS: usize = 200;
/// At most this many comma-separated pieces ("1,3,5-8" has three).
const MAX_PIECES: usize = 50;

/// Page or slide numbers (1-based, ascending, no duplicates) from "3-7", "1, 3, 5–8".
/// Every number must exist: `total` is how many the file has.
pub fn numbers(spec: &str, total: usize, what: &str) -> Result<Vec<usize>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(Error::invalid(format!("Gib an, welche {what} du senden willst, z. B. 3-7")));
    }
    if spec.chars().count() > MAX_SPEC_CHARS {
        return Err(Error::invalid("Die Auswahl ist zu lang"));
    }
    if total == 0 {
        return Err(Error::invalid(format!("Die Datei hat keine {what}")));
    }
    let pieces: Vec<&str> = spec.split([',', ';']).map(str::trim).collect();
    if pieces.len() > MAX_PIECES {
        return Err(Error::invalid(format!("Höchstens {MAX_PIECES} Bereiche, durch Kommas getrennt")));
    }
    let mut picked = vec![false; total];
    for piece in pieces {
        let normal = piece.replace(['–', '—'], "-");
        let (a, b) = match normal.split_once('-') {
            Some((a, b)) => (number(a, spec)?, number(b, spec)?),
            None => {
                let n = number(&normal, spec)?;
                (n, n)
            }
        };
        if a == 0 || b == 0 {
            return Err(Error::invalid(format!("{what} zählen ab 1: „{spec}“")));
        }
        if a > b {
            return Err(Error::invalid(format!("„{piece}“: Der Bereich muss mit der kleineren Zahl beginnen")));
        }
        if b > total {
            return Err(Error::invalid(format!("Die Datei hat nur {total} {what}, „{piece}“ geht darüber hinaus")));
        }
        for p in picked.iter_mut().take(b).skip(a - 1) {
            *p = true;
        }
    }
    Ok(picked.iter().enumerate().filter(|(_, p)| **p).map(|(i, _)| i + 1).collect())
}

fn number(s: &str, spec: &str) -> Result<usize> {
    let s = s.trim();
    if s.is_empty() || s.len() > 7 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::invalid(format!("„{}“ verstehe ich nicht. Schreib z. B. 3-7 oder 1,4,6-8", spec.chars().take(40).collect::<String>())));
    }
    s.parse::<usize>().map_err(|_| Error::invalid("Zahl zu groß"))
}

/// Numbers as a short label: [3,4,5,7] → "3-5, 7".
pub fn label(numbers: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(&start) = numbers.get(i) {
        let mut end = start;
        while numbers.get(i + 1) == Some(&(end + 1)) {
            end += 1;
            i += 1;
        }
        out.push(if start == end { start.to_string() } else { format!("{start}-{end}") });
        i += 1;
    }
    out.join(", ")
}

/// One sheet by its 1-based number ("2") or its name ("Noten", case ignored).
pub fn sheet(spec: &str, names: &[String]) -> Result<usize> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(Error::invalid("Gib den Namen oder die Nummer des Tabellenblatts an"));
    }
    if spec.chars().count() > MAX_SPEC_CHARS {
        return Err(Error::invalid("Die Auswahl ist zu lang"));
    }
    if let Some(i) = names.iter().position(|n| n.trim().to_lowercase() == spec.to_lowercase()) {
        return Ok(i);
    }
    if spec.len() <= 7 && spec.bytes().all(|b| b.is_ascii_digit()) {
        let n: usize = spec.parse().unwrap_or(0);
        if n >= 1 && n <= names.len() {
            return Ok(n - 1);
        }
        return Err(Error::invalid(format!("Die Datei hat {} Tabellenblätter, nicht {n}", names.len())));
    }
    let list: Vec<&str> = names.iter().take(12).map(String::as_str).collect();
    Err(Error::invalid(format!("Kein Tabellenblatt „{}“. Vorhanden: {}", spec.chars().take(40).collect::<String>(), list.join(", "))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ranges_are_strict() {
        assert_eq!(numbers("3-5", 10, "Seiten").unwrap(), [3, 4, 5]);
        assert_eq!(numbers(" 1, 3 , 5–6;5 ", 10, "Seiten").unwrap(), [1, 3, 5, 6]);
        assert_eq!(numbers("7", 7, "Seiten").unwrap(), [7]);
        for bad in ["", "0", "0-2", "5-3", "1-11", "a", "1-", "-3", "1--3", "3 4", "99999999999999", "1.5", "1-999999999"] {
            assert!(numbers(bad, 10, "Seiten").is_err(), "{bad}");
        }
        let many = (1..=60).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        assert!(numbers(&many, 100, "Seiten").is_err());
        assert!(numbers("1", 0, "Seiten").is_err());
        assert!(numbers(&"1".repeat(500), 10, "Seiten").is_err());
        assert!(numbers("1-11", 10, "Seiten").unwrap_err().message.contains("nur 10 Seiten"));
    }

    #[test]
    fn labels_and_sheets() {
        assert_eq!(label(&[3, 4, 5, 7]), "3-5, 7");
        assert_eq!(label(&[1]), "1");
        let names = vec!["Noten".to_string(), "Liste".to_string()];
        assert_eq!(sheet("noten", &names).unwrap(), 0);
        assert_eq!(sheet("2", &names).unwrap(), 1);
        assert!(sheet("3", &names).is_err());
        assert!(sheet("0", &names).is_err());
        assert!(sheet("Fehlt", &names).unwrap_err().message.contains("Noten, Liste"));
    }
}
