use factory_core::{PromptImage, IMAGE_BYTES};
use image::ImageEncoder;

pub enum Content {
    Image(PromptImage),
    Text(String),
}

pub fn read() -> Result<Content, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| format!("Zwischenablage nicht erreichbar: {e}"))?;
    if let Ok(image) = clipboard.get_image() {
        // Bound raw pixel processing as well as the persisted PNG size.
        if image.bytes.len() > 64 * 1024 * 1024 {
            return Err("Bild zu gross (maximal 64 MiB Pixeldaten)".into());
        }
        let width = u32::try_from(image.width).map_err(|_| "Bildbreite zu gross")?;
        let height = u32::try_from(image.height).map_err(|_| "Bildhoehe zu gross")?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&image.bytes, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|e| e.to_string())?;
        if png.len() > IMAGE_BYTES {
            return Err("Bild zu gross (maximal 4 MiB als PNG)".into());
        }
        return PromptImage::from_bytes("Zwischenablage.png".into(), &png)
            .map(Content::Image)
            .map_err(str::to_string);
    }
    clipboard
        .get_text()
        .map(Content::Text)
        .map_err(|_| "Kein Bild oder Text in der Zwischenablage".into())
}
