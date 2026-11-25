//! Small, self-contained project artwork. No paths, URLs or active image formats.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::io::Cursor;

pub const PROJECT_LOGO_SIDE: u32 = 128;
pub const PROJECT_LOGO_MAX_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectLogo {
    pub png_base64: String,
}

impl ProjectLogo {
    pub fn from_png(bytes: &[u8]) -> Result<Self, &'static str> {
        validate_png(bytes)?;
        Ok(Self {
            png_base64: STANDARD.encode(bytes),
        })
    }

    pub fn png_bytes(&self) -> Result<Vec<u8>, &'static str> {
        if self.png_base64.len() > PROJECT_LOGO_MAX_BYTES.div_ceil(3) * 4 {
            return Err("Project logo exceeds 96 KiB");
        }
        let bytes = STANDARD
            .decode(&self.png_base64)
            .map_err(|_| "Invalid project logo encoding")?;
        validate_png(&bytes)?;
        Ok(bytes)
    }
}

fn validate_png(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.is_empty() || bytes.len() > PROJECT_LOGO_MAX_BYTES {
        return Err("Project logo exceeds 96 KiB or is empty");
    }
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: 512 * 1024 });
    decoder.set_ignore_text_chunk(true);
    let mut reader = decoder
        .read_info()
        .map_err(|_| "Project logo must be a valid PNG")?;
    let info = reader.info();
    if info.width != PROJECT_LOGO_SIDE
        || info.height != PROJECT_LOGO_SIDE
        || info.animation_control.is_some()
    {
        return Err("Project logo must be a static 128 by 128 PNG");
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 128 * 128 * 8)
        .ok_or("Project logo is too large")?;
    reader
        .next_frame(&mut vec![0; size])
        .map_err(|_| "Project logo PNG is damaged")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![];
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![80; (width * height * 4) as usize])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn accepts_only_bounded_valid_square_pngs() {
        let bytes = png(128, 128);
        assert_eq!(
            ProjectLogo::from_png(&bytes).unwrap().png_bytes().unwrap(),
            bytes
        );
        assert!(ProjectLogo::from_png(&png(128, 64)).is_err());
        assert!(ProjectLogo::from_png(&png(256, 256)).is_err());
        assert!(ProjectLogo::from_png(b"<svg/>").is_err());
        assert!(ProjectLogo::from_png(&bytes[..bytes.len() / 2]).is_err());
        assert!(ProjectLogo {
            png_base64: "not base64".into()
        }
        .png_bytes()
        .is_err());
        assert!(ProjectLogo {
            png_base64: "A".repeat(PROJECT_LOGO_MAX_BYTES * 2)
        }
        .png_bytes()
        .is_err());
    }
}
