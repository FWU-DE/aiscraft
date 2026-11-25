//! Repository chores: `cargo xtask <command>`.
//!
//! `cargo xtask version set <version>` sets `[workspace.package] version` in Cargo.toml and the
//! workspace's own entries in Cargo.lock (so `cargo build --locked` still passes). The Craft
//! suite's shared release workflow calls it when a run builds another version than Cargo.toml's,
//! e.g. the nightly `0.1.1-nightly.20261126`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["version", "set", version] => set_version(&root(), version),
        ["version", "get"] => read_version(&root()).map(|v| println!("{v}")),
        _ => Err("usage: cargo xtask version set <version> | cargo xtask version get".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Semver without build metadata: `1.2.3` or `1.2.3-rc.1`.
fn valid(version: &str) -> bool {
    let (core, pre) = version.split_once('-').map_or((version, None), |(c, p)| (c, Some(p)));
    let numbers: Vec<&str> = core.split('.').collect();
    let number = |n: &&str| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && (n.len() == 1 || !n.starts_with('0'));
    let ident = |i: &str| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    numbers.len() == 3 && numbers.iter().all(number) && pre.is_none_or(|p| p.split('.').all(ident))
}

/// The `version = "…"` line inside `[workspace.package]`, as (line index, value).
fn workspace_version_line(toml: &str) -> Option<(usize, String)> {
    let mut in_pkg = false;
    for (i, line) in toml.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_pkg = t == "[workspace.package]";
            continue;
        }
        if in_pkg && let Some(rest) = t.strip_prefix("version") {
            let value = rest.trim_start().strip_prefix('=')?.trim().trim_matches('"');
            return Some((i, value.to_string()));
        }
    }
    None
}

fn read_version(root: &Path) -> Result<String, String> {
    let toml = read(&root.join("Cargo.toml"))?;
    workspace_version_line(&toml).map(|(_, v)| v).ok_or_else(|| "no [workspace.package] version in Cargo.toml".into())
}

fn set_version(root: &Path, version: &str) -> Result<(), String> {
    let version = version.trim_start_matches('v');
    if !valid(version) {
        return Err(format!("'{version}' is not a version like 1.2.3 or 1.2.3-rc.1"));
    }
    let toml_path = root.join("Cargo.toml");
    let toml = read(&toml_path)?;
    let (index, old) = workspace_version_line(&toml).ok_or("no [workspace.package] version in Cargo.toml")?;
    let toml = replace_line(&toml, index, &format!("version = \"{version}\""));
    write(&toml_path, &toml)?;

    // Workspace members are the lock entries without a `source` (registry and git crates have one).
    let lock_path = root.join("Cargo.lock");
    let lock = read(&lock_path)?;
    let old_line = format!("version = \"{old}\"");
    let new_line = format!("version = \"{version}\"");
    let mut out = String::with_capacity(lock.len());
    let mut changed = 0;
    for (n, block) in lock.split("\n[[package]]\n").enumerate() {
        if n > 0 {
            out.push_str("\n[[package]]\n");
        }
        let ours = !block.lines().any(|l| l.starts_with("source = "));
        if n > 0 && ours && block.lines().any(|l| l == old_line) {
            out.push_str(&block.replacen(&old_line, &new_line, 1));
            changed += 1;
        } else {
            out.push_str(block);
        }
    }
    write(&lock_path, &out)?;
    println!("version {old} -> {version} (Cargo.toml, {changed} entries in Cargo.lock)");
    Ok(())
}

fn replace_line(text: &str, index: usize, new: &str) -> String {
    let mut out: Vec<&str> = text.lines().collect();
    if let Some(line) = out.get_mut(index) {
        *line = new;
    }
    let mut joined = out.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

/// The file with `\n` line ends (a Windows checkout may have CRLF; `write` restores them).
fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map(|t| t.replace("\r\n", "\n")).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes `text` (with `\n` line ends) in the line ends the file has now.
fn write(path: &Path, text: &str) -> Result<(), String> {
    let crlf = std::fs::read_to_string(path).is_ok_and(|t| t.contains("\r\n"));
    let text = if crlf { text.replace('\n', "\r\n") } else { text.to_string() };
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        for ok in ["0.1.0", "1.20.3", "0.1.1-nightly.20261126", "1.0.0-rc.1"] {
            assert!(valid(ok), "{ok}");
        }
        for bad in ["", "1.2", "01.2.3", "1.2.3-", "1.2.3-a..b", "1.2.3+meta", "v1.2.3", "1.2.3-ä"] {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn sets_toml_and_only_our_lock_entries() {
        let dir = std::env::temp_dir().join(format!("aiscraft-xtask-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"aiscraft-core\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"other\"\nversion = \"0.1.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )
        .unwrap();
        set_version(&dir, "v0.1.1-nightly.20261126").unwrap();
        let toml = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(toml.contains("[workspace.package]\nversion = \"0.1.1-nightly.20261126\""), "{toml}");
        assert!(toml.contains("[dependencies]\nversion = \"0.1.0\""), "{toml}");
        let lock = std::fs::read_to_string(dir.join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"aiscraft-core\"\nversion = \"0.1.1-nightly.20261126\""), "{lock}");
        assert!(lock.contains("name = \"other\"\nversion = \"0.1.0\""), "{lock}");
        assert!(lock.starts_with("version = 4\n"));
        assert!(set_version(&dir, "kaputt").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_crlf_line_ends() {
        let dir = std::env::temp_dir().join(format!("aiscraft-xtask-crlf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[workspace.package]\r\nversion = \"0.1.0\"\r\n").unwrap();
        std::fs::write(dir.join("Cargo.lock"), "version = 4\r\n\r\n[[package]]\r\nname = \"a\"\r\nversion = \"0.1.0\"\r\n").unwrap();
        set_version(&dir, "0.2.0").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("Cargo.toml")).unwrap(), "[workspace.package]\r\nversion = \"0.2.0\"\r\n");
        assert_eq!(std::fs::read_to_string(dir.join("Cargo.lock")).unwrap(), "version = 4\r\n\r\n[[package]]\r\nname = \"a\"\r\nversion = \"0.2.0\"\r\n");
        let _ = std::fs::remove_dir_all(dir);
    }
}
