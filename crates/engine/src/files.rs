//! Turning a file on disk into an attachment.

use std::path::Path;

use aiscraft_core::{Attachment, Error, Result};
use base64::Engine as _;

pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_INLINE_TEXT_CHARS: usize = 300_000;

pub fn mime_of(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "txt" | "rs" | "py" | "js" | "ts" | "tsx" | "toml" | "yaml" | "yml" | "xml" | "sh" | "c" | "h" | "cpp" | "java" | "go" | "rb" | "css" | "sql"
        | "tex" => "text/plain",
        _ => "application/octet-stream",
    }
}

pub fn is_text(mime: &str) -> bool {
    mime.starts_with("text/") || mime == "application/json"
}

pub fn is_image(mime: &str) -> bool {
    mime.starts_with("image/")
}

/// Read a file. `inline_binary` keeps images and PDFs as base64 for backends that take them inline.
pub fn load(path: &Path, inline_binary: bool) -> Result<(Attachment, Vec<u8>)> {
    let meta = std::fs::metadata(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(Error::invalid(format!("{} ist keine Datei", path.display())));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(Error::invalid(format!("{} ist größer als 25 MB", path.display())));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    let mime = mime_of(path);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("datei").to_string();
    let mut a = Attachment { name, mime: mime.to_string(), size: meta.len(), local_path: Some(path.display().to_string()), ..Default::default() };
    if is_text(mime) {
        let text = String::from_utf8_lossy(&bytes);
        a.text = Some(text.chars().take(MAX_INLINE_TEXT_CHARS).collect());
    } else if inline_binary && (is_image(mime) || mime == "application/pdf") {
        a.data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
    }
    Ok((a, bytes))
}

/// Keep a copy of an attached file in AIS.ChatCraft's data folder and return its path.
pub fn keep_copy(dir: &Path, original: &Path, bytes: &[u8]) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| Error::storage(format!("Anhänge-Ordner: {e}")))?;
    let ext = original.extension().and_then(|e| e.to_str()).filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin");
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    // A fresh name, created exclusively: never replaces an existing attachment.
    craft_atomic::write_new(&path, bytes).map_err(|e| Error::storage(format!("Anhang speichern: {e}")))?;
    Ok(path)
}

/// Images and PDFs are not kept in the database; read them back for a request.
pub fn hydrate(messages: &mut [aiscraft_core::Message]) {
    for m in messages.iter_mut() {
        for a in m.attachments.iter_mut() {
            if a.data_base64.is_some() || a.remote_id.is_some() || !(is_image(&a.mime) || a.mime == "application/pdf") {
                continue;
            }
            let Some(p) = a.local_path.as_deref() else { continue };
            let Ok(meta) = std::fs::metadata(p) else { continue };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            if let Ok(bytes) = std::fs::read(p) {
                a.data_base64 = Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_and_hydrates_images() {
        let dir = std::env::temp_dir().join(format!("aiscraft-files-test-{}", std::process::id()));
        let png = [0x89u8, b'P', b'N', b'G', 1, 2, 3];
        let copy = keep_copy(&dir, Path::new("/x/Foto.PNG"), &png).unwrap();
        let mut m = aiscraft_core::Message {
            id: "m".into(),
            conversation_id: "c".into(),
            role: aiscraft_core::Role::User,
            content: String::new(),
            created_at: 0,
            model: None,
            attachments: vec![Attachment {
                name: "Foto.PNG".into(),
                mime: "image/png".into(),
                local_path: Some(copy.display().to_string()),
                ..Default::default()
            }],
            activity: vec![],
            error: None,
            usage: None,
            tool_calls: vec![],
            tool_result: None,
        };
        hydrate(std::slice::from_mut(&mut m));
        assert_eq!(m.attachments[0].data_base64.as_deref(), Some(base64::engine::general_purpose::STANDARD.encode(png).as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
