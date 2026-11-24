use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const TASK_FILES_CAPABILITY: &str = "task-files-v1";
pub const TASK_FILE_BYTES: usize = 4 * 1024 * 1024;

/// Explicit user-provided input, never an arbitrary path on a runner host.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptFile {
    pub name: String,
    pub mime: String,
    pub data: String,
}
impl PromptFile {
    pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > TASK_FILE_BYTES {
            return Err("Dateien insgesamt höchstens 4 MiB");
        }
        if name.is_empty()
            || name.len() > 255
            || name.contains(['/', '\\'])
            || name.chars().any(char::is_control)
        {
            return Err("Ungültiger Dateiname");
        }
        let mime = match name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "pdf" => "application/pdf",
            "json" => "application/json",
            "csv" => "text/csv",
            "md" => "text/markdown",
            "txt" | "rs" | "py" | "js" | "ts" | "toml" | "yaml" | "yml" | "log" => "text/plain",
            _ => "application/octet-stream",
        };
        Ok(Self {
            name,
            mime: mime.into(),
            data: STANDARD.encode(bytes),
        })
    }
    pub fn uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, self.data)
    }
}

pub fn validate_files(files: &[PromptFile]) -> Result<(), &'static str> {
    if files.len() > 16 {
        return Err("Höchstens 16 Dateien pro Aufgabe");
    }
    let mut total = 0;
    for file in files {
        if file.data.len() > TASK_FILE_BYTES * 4 / 3 + 4 {
            return Err("Datei zu groß");
        }
        let bytes = STANDARD
            .decode(&file.data)
            .map_err(|_| "Ungültige Dateidaten")?;
        total += bytes.len();
        if total > TASK_FILE_BYTES {
            return Err("Dateien insgesamt höchstens 4 MiB");
        }
        if PromptFile::from_bytes(file.name.clone(), &bytes)? != *file {
            return Err("Dateityp oder Kodierung stimmt nicht");
        }
    }
    Ok(())
}
pub fn files_digest(files: &[PromptFile]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(files).expect("file serialization"))
    )
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptImage {
    pub name: String,
    pub mime: String,
    pub data: String,
}
impl PromptImage {
    pub fn from_bytes(name: String, bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > IMAGE_BYTES {
            return Err("Bilder insgesamt hoechstens 4 MiB");
        }
        if name.len() > 255 || name.chars().any(char::is_control) {
            return Err("Ungueltiger Bildname");
        }
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg"
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            "image/gif"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else {
            return Err("PNG, JPEG, GIF oder WebP erforderlich");
        };
        Ok(Self {
            name,
            mime: mime.into(),
            data: STANDARD.encode(bytes),
        })
    }
    pub fn uri(&self) -> String {
        format!("data:{};base64,{}", self.mime, self.data)
    }
}
pub fn validate_images(images: &[PromptImage]) -> Result<(), &'static str> {
    if images.len() > 4 {
        return Err("Hoechstens vier Bilder pro Aufgabe");
    }
    let mut size = 0;
    for image in images {
        if image.data.len() > IMAGE_BYTES * 4 / 3 + 4 {
            return Err("Bild zu gross");
        }
        let bytes = STANDARD
            .decode(&image.data)
            .map_err(|_| "Ungueltige Bilddaten")?;
        size += bytes.len();
        if size > IMAGE_BYTES {
            return Err("Bilder insgesamt hoechstens 4 MiB");
        }
        if PromptImage::from_bytes(image.name.clone(), &bytes)? != *image {
            return Err("Bildtyp oder Kodierung stimmt nicht");
        }
    }
    Ok(())
}
pub fn image_digest(images: &[PromptImage]) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(images).expect("image serialization"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_files_have_safe_names_limits_and_exact_bytes() {
        let file = PromptFile::from_bytes("notizen.md".into(), "Grüße 界".as_bytes()).unwrap();
        validate_files(std::slice::from_ref(&file)).unwrap();
        assert!(file.uri().starts_with("data:text/markdown;base64,"));
        assert_eq!(STANDARD.decode(&file.data).unwrap(), "Grüße 界".as_bytes());
        for name in ["../escape", "foo/bar", "foo\\bar", "", "line\n"] {
            assert!(PromptFile::from_bytes(name.into(), b"x").is_err());
        }
        let mut corrupt = file.clone();
        corrupt.mime = "text/plain".into();
        assert!(validate_files(&[corrupt]).is_err());
        assert!(PromptFile::from_bytes("large.pdf".into(), &vec![0; TASK_FILE_BYTES + 1]).is_err());
        assert_ne!(files_digest(&[file]), files_digest(&[]));
    }
}
