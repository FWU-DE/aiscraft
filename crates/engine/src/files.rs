//! Turning a file on disk into an attachment.

use std::path::Path;

use aiscraft_core::{Error, Result};
use base64::Engine as _;

pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
pub(crate) const MAX_INLINE_TEXT_CHARS: usize = 300_000;

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

/// Keep a copy of an attached file in AIS.ChatCraft's data folder and return its path.
pub fn keep_copy(dir: &Path, original: &Path, bytes: &[u8]) -> Result<std::path::PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| Error::io("Anhänge-Ordner", &e))?;
    let ext = original.extension().and_then(|e| e.to_str()).filter(|e| e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin");
    let path = dir.join(format!("{}.{ext}", uuid::Uuid::new_v4()));
    // A fresh name, created exclusively: never replaces an existing attachment.
    crate::disk::write_new(&path, bytes, "Anhang speichern")?;
    Ok(path)
}

/// Images and PDFs are not kept in the database; read them back for a request.
pub fn hydrate(messages: &mut [aiscraft_core::Message]) {
    // Only what the person attached; files the assistant created stay on disk.
    for m in messages.iter_mut().filter(|m| m.role == aiscraft_core::Role::User) {
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
    use aiscraft_core::Attachment;

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
            thinking: String::new(),
        };
        hydrate(std::slice::from_mut(&mut m));
        assert_eq!(m.attachments[0].data_base64.as_deref(), Some(base64::engine::general_purpose::STANDARD.encode(png).as_str()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
