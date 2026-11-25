//! Unpacking a connector package or plug-in into a staging folder. Everything in it is hostile:
//! sizes and counts are capped, names are checked (no absolute paths, no `..`, no drive letters),
//! ZIP links are refused, and links in a folder count only when they stay inside it.

use std::path::{Path, PathBuf};

use aiscraft_core::{Error, Result};

/// Largest package file read.
pub(crate) const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;
/// Most bytes all files of a package may unpack to.
pub(crate) const MAX_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;
/// Most files and folders in a package.
pub(crate) const MAX_ENTRIES: u64 = 20_000;
const MAX_NAME: usize = 512;
const MAX_DEPTH: usize = 32;

fn zip_err(e: craft_zip::Error) -> Error {
    if e.is_limit() {
        Error::invalid(format!("Das Paket ist zu groß oder hat zu viele Dateien ({e})"))
    } else {
        Error::invalid(format!("Das ist kein lesbares Paket ({e})"))
    }
}

/// Why an entry name is refused, if it is.
pub(crate) fn bad_name(name: &str) -> Option<String> {
    let shown: String = name.chars().filter(|c| !c.is_control()).take(80).collect();
    if name.is_empty() || name.len() > MAX_NAME {
        return Some(format!("Dateiname zu lang oder leer: {shown}"));
    }
    if name.starts_with('/') || name.contains('\\') || name.contains(':') || name.chars().any(char::is_control) {
        return Some(format!("unzulässiger Pfad: {shown}"));
    }
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.split('/').any(|p| !segment_ok(p)) || trimmed.split('/').count() > MAX_DEPTH {
        return Some(format!("unzulässiger Pfad: {shown}"));
    }
    None
}

/// Device names Windows reserves in every folder (`NUL`, `COM1.txt` …).
const WINDOWS_DEVICES: &[&str] = &["con", "prn", "aux", "nul", "conin$", "conout$"];

/// One segment of an entry name. Windows drops trailing dots and spaces (`.. ` and `...` become
/// `..`, `a.` becomes `a`), so such segments are refused, like dot-only segments, leading
/// spaces, characters Windows does not allow, device names and `.git` (hooks run code). Other
/// names starting with a dot stay allowed: plug-ins carry `.claude-plugin/` and `.mcp.json`,
/// bundles `node_modules/.bin`.
fn segment_ok(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    let device = WINDOWS_DEVICES.contains(&stem)
        || ((stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.chars().count() == 4
            && stem.chars().nth(3).is_some_and(|c| c.is_ascii_digit() || "¹²³".contains(c)));
    !p.is_empty()
        && !p.chars().all(|c| c == '.' || c == ' ')
        && !p.ends_with(['.', ' '])
        && !p.starts_with(' ')
        && !p.contains(['<', '>', '"', '|', '?', '*'])
        && !device
        && lower != ".git"
}

/// Unpack ZIP bytes (an `.mcpb` or a zipped plug-in) into `dest`, which must not exist yet.
pub(crate) fn unzip(bytes: &[u8], dest: &Path) -> Result<()> {
    if bytes.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(Error::invalid(format!("Das Paket ist größer als {} MB", MAX_PACKAGE_BYTES / (1024 * 1024))));
    }
    let mut zip = craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ENTRIES, max_total_bytes: MAX_UNPACKED_BYTES }).map_err(zip_err)?;
    if zip.has_duplicate_names() {
        return Err(Error::invalid("Das Paket enthält eine Datei doppelt"));
    }
    if zip.has_overlapping_files().map_err(zip_err)? {
        return Err(Error::invalid("Das Paket ist manipuliert (Dateien überlappen sich)"));
    }
    crate::disk::create_dir(dest)?;
    for i in 0..zip.len() {
        let name = zip.entry_name(i).map_err(zip_err)?.unwrap_or_default();
        if zip.entry_is_symlink(i).map_err(zip_err)?.unwrap_or(false) {
            return Err(Error::invalid(format!("Das Paket enthält eine Verknüpfung ({}); das ist nicht erlaubt", name.chars().take(80).collect::<String>())));
        }
        if let Some(why) = bad_name(&name) {
            return Err(Error::invalid(format!("Das Paket enthält einen {why}")));
        }
        let target = dest.join(name.trim_end_matches('/'));
        if zip.entry_is_dir(i).map_err(zip_err)?.unwrap_or(false) {
            crate::disk::create_dir(&target)?;
            continue;
        }
        let data = zip.read(&name, MAX_PACKAGE_BYTES).map_err(zip_err)?.unwrap_or_default();
        if let Some(parent) = target.parent() {
            crate::disk::create_dir(parent)?;
        }
        crate::disk::write_new(&target, &data, &name)?;
    }
    Ok(())
}

/// Copy a plug-in folder into `dest`. Links are followed only when they lead to a place inside
/// the folder (as Agent Plugins asks); others are left out and named in the result.
pub(crate) fn copy_folder(src: &Path, dest: &Path) -> Result<Vec<String>> {
    let root = std::fs::canonicalize(src).map_err(|e| Error::not_found(format!("{}: {e}", src.display())))?;
    if !root.is_dir() {
        return Err(Error::invalid("Das ist kein Ordner"));
    }
    crate::disk::create_dir(dest)?;
    let mut skipped = Vec::new();
    let mut budget = (MAX_ENTRIES, MAX_UNPACKED_BYTES);
    copy_into(&root, &root, dest, 0, &mut budget, &mut skipped)?;
    Ok(skipped)
}

fn copy_into(root: &Path, dir: &Path, dest: &Path, depth: usize, budget: &mut (u64, u64), skipped: &mut Vec<String>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::invalid("Der Ordner ist zu tief verschachtelt"));
    }
    let rd = std::fs::read_dir(dir).map_err(|e| Error::io(dir.display(), &e))?;
    for entry in rd.flatten() {
        budget.0 = budget.0.checked_sub(1).ok_or_else(|| Error::invalid(format!("Der Ordner hat mehr als {MAX_ENTRIES} Dateien")))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // `.git` and friends belong to the author's working copy, not to the plug-in.
        if name == ".git" || name == "node_modules/.cache" {
            continue;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).display().to_string();
        let Ok(real) = std::fs::canonicalize(&path) else {
            skipped.push(rel);
            continue;
        };
        if !real.starts_with(root) {
            skipped.push(rel);
            continue;
        }
        let target = dest.join(&name);
        if real.is_dir() {
            crate::disk::create_dir(&target)?;
            copy_into(root, &real, &target, depth + 1, budget, skipped)?;
        } else if real.is_file() {
            let len = std::fs::metadata(&real).map(|m| m.len()).unwrap_or(u64::MAX);
            budget.1 =
                budget.1.checked_sub(len).ok_or_else(|| Error::invalid(format!("Der Ordner ist größer als {} MB", MAX_UNPACKED_BYTES / (1024 * 1024))))?;
            let bytes = std::fs::read(&real).map_err(|e| Error::io(real.display(), &e))?;
            crate::disk::write_new(&target, &bytes, &rel)?;
        }
    }
    Ok(())
}

/// A path the package names (`server/index.js`, `./bin/x`, `${__dirname}/x`), resolved inside
/// `root` with links followed; `None` when it leads out or does not exist.
pub(crate) fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    let joined = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
    let real = std::fs::canonicalize(&joined).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    real.starts_with(&root).then_some(real)
}

/// A command is either a bare program name (looked up on PATH: `node`, `python3`, `uv`) or a file
/// inside the package. Never a shell line and never a program elsewhere on the disk.
pub(crate) fn resolve_command(root: &Path, command: &str) -> Result<String> {
    let command = command.trim();
    if command.is_empty() || command.chars().any(|c| c.is_control()) {
        return Err(Error::invalid("Das Paket nennt kein Programm"));
    }
    let bare = !command.contains('/') && !command.contains('\\') && !command.starts_with('.');
    if bare {
        if command.chars().any(|c| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '$' | '`' | '<' | '>' | '*' | '?')) {
            return Err(Error::invalid(format!("Unzulässiger Programmname: {}", command.chars().take(80).collect::<String>())));
        }
        return Ok(command.to_string());
    }
    let real = inside(root, command)
        .filter(|p| p.is_file())
        .ok_or_else(|| Error::invalid(format!("Das Programm {} liegt nicht im Paket", command.chars().take(120).collect::<String>())))?;
    Ok(real.display().to_string())
}

/// Remove a staging or plug-in folder, if it is there.
pub(crate) fn remove(dir: &Path) {
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_names() {
        for bad in ["/etc/passwd", "../x", "a/../../x", "C:/x", "a\\b", "", "a//b", "./a"] {
            assert!(bad_name(bad).is_some(), "{bad}");
        }
        // What Windows would turn into something else: trailing dots and spaces, dot-only
        // segments, device names, forbidden characters; and `.git`.
        for bad in [
            "... /x",
            "a/.../b",
            "a/.. /b",
            "a/. /b",
            "a./b",
            "server/index.js.",
            "server/index.js ",
            " a/b",
            "a/ b",
            "NUL",
            "a/con.txt",
            "lib/COM1.js",
            "lpt9",
            "a/b?.js",
            "a*/b",
            "x/.git/hooks/pre-commit",
            ".GIT/config",
        ] {
            assert!(bad_name(bad).is_some(), "{bad}");
        }
        for ok in [
            "server/index.js",
            "server/",
            ".claude-plugin/plugin.json",
            ".mcp.json",
            "node_modules/.bin/tool",
            "node_modules/.package-lock.json",
            "a.b/c.d.e",
            "console/x",
            "com/x",
            "computer.js",
            ".gitignore",
        ] {
            assert!(bad_name(ok).is_none(), "{ok}");
        }
    }

    #[test]
    fn commands_are_bare_names_or_files_inside() {
        let root = std::env::temp_dir().join(format!("aiscraft-pkg-{}", std::process::id()));
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/server"), "x").unwrap();
        assert_eq!(resolve_command(&root, "node").unwrap(), "node");
        assert!(resolve_command(&root, "node; rm -rf ~").is_err());
        assert!(Path::new(&resolve_command(&root, "./bin/server").unwrap()).ends_with("bin/server"));
        assert!(resolve_command(&root, "/bin/sh").is_err());
        assert!(resolve_command(&root, "../../bin/sh").is_err());
        assert!(resolve_command(&root, "./bin/fehlt").is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
