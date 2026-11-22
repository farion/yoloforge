use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub const IMAGE_BYTES: usize = 4 * 1024 * 1024;
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
