//! EmbeddingGemma 2 image preprocessing. Algorithm and tensor layout follow
//! Hugging Face Transformers.js Gemma4ImageProcessor (Apache-2.0).
//! See docs/indicizzazione-completa.md for the upstream source and model revision.
//!
//! Decoding runs in the sacrificial parser process (INV-11). The patches travel to the model
//! process as the resized 8-bit RGB values; [`Pixels::values`] turns them into the model's
//! floats (`channel / 255`) exactly as the in-process preprocessing did.
use image::{ImageReader, imageops::FilterType};
use std::io::Cursor;

pub const PATCHES: usize = 280 * 9;
pub const PATCH_VALUES: usize = 16 * 16 * 3;

#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    /// `PATCHES * PATCH_VALUES` RGB bytes, patch after patch; padding is zero.
    pub rgb: Vec<u8>,
    /// `PATCHES * 2` positions (column, row); padding is -1.
    pub positions: Vec<i64>,
    pub tokens: usize,
}

impl Pixels {
    /// The `pixel_values` input of the vision encoder.
    pub fn values(&self) -> Vec<f32> {
        self.rgb.iter().map(|channel| *channel as f32 / 255.0).collect()
    }

    /// Wire form for the parser frames: RGB bytes, then positions as little-endian i64.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgb.len() + self.positions.len() * 8);
        out.extend_from_slice(&self.rgb);
        for position in &self.positions {
            out.extend_from_slice(&position.to_le_bytes());
        }
        out
    }

    pub fn from_bytes(bytes: &[u8], tokens: usize) -> Result<Pixels, String> {
        let rgb_len = PATCHES * PATCH_VALUES;
        if bytes.len() != rgb_len + PATCHES * 2 * 8 || tokens == 0 || tokens > PATCHES / 9 {
            return Err("Image patches have an unexpected shape".into());
        }
        let positions = bytes[rgb_len..]
            .as_chunks::<8>()
            .0
            .iter()
            .map(|chunk| i64::from_le_bytes(*chunk))
            .collect();
        Ok(Pixels { rgb: bytes[..rgb_len].to_vec(), positions, tokens })
    }
}

fn target_size(width: u32, height: u32) -> Result<(u32, u32), String> {
    if width == 0 || height == 0 { return Err("Empty image".into()); }
    let factor = ((PATCHES * 16 * 16) as f64 / (width as f64 * height as f64)).sqrt();
    let mut w = ((factor * width as f64 / 48.0).floor() as u32) * 48;
    let mut h = ((factor * height as f64 / 48.0).floor() as u32) * 48;
    if h == 0 { h = 48; w = (width / height).saturating_mul(48).min(280 * 48); }
    if w == 0 { w = 48; h = (height / width).saturating_mul(48).min(280 * 48); }
    Ok((w, h))
}

pub fn prepare(bytes: &[u8]) -> Result<Pixels, String> {
    // Decode only bounded images. A tiny compressed image must not allocate an
    // arbitrarily large bitmap in the worker before its process limit catches it.
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(96 * 1024 * 1024);
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| format!("Image unavailable or exceeds decoding memory budget: {e}"))?;
    let (w, h) = target_size(decoded.width(), decoded.height())?;
    let pixels = decoded.to_rgb8();
    drop(decoded);
    let resized = image::imageops::resize(&pixels, w, h, FilterType::CatmullRom);
    drop(pixels);
    let mut rgb = vec![0u8; PATCHES * PATCH_VALUES];
    let mut positions = vec![-1; PATCHES * 2];
    let mut offset = 0;
    let mut patch = 0;
    for row in 0..h / 16 {
        for col in 0..w / 16 {
            positions[patch * 2] = col as i64;
            positions[patch * 2 + 1] = row as i64;
            patch += 1;
            for dy in 0..16 {
                for dx in 0..16 {
                    for channel in resized.get_pixel(col * 16 + dx, row * 16 + dy).0 {
                        rgb[offset] = channel;
                        offset += 1;
                    }
                }
            }
        }
    }
    Ok(Pixels { rgb, positions, tokens: patch / 9 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shapes_preserve_ratio_and_stay_within_the_token_budget() {
        for (w, h) in [(1, 1), (800, 600), (600, 800), (16384, 1), (1, 16384)] {
            let (w, h) = target_size(w, h).unwrap();
            assert!(w > 0 && h > 0 && w % 48 == 0 && h % 48 == 0);
            assert!(w as usize * h as usize / 256 <= PATCHES);
        }
        assert_eq!(target_size(100, 100).unwrap(), (768, 768));
    }
    #[test]
    fn rgb_patch_layout_and_padding_match_the_model_contract() {
        let image = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 128, 0]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let result = prepare(bytes.get_ref()).unwrap();
        assert_eq!(result.tokens, 256);
        assert_eq!(&result.positions[..4], &[0, 0, 1, 0]);
        assert_eq!(&result.positions[2304 * 2..2304 * 2 + 2], &[-1, -1]);
        let values = result.values();
        assert_eq!(values[0], 1.0);
        assert!((values[1] - 128.0 / 255.0).abs() < 0.00001);
        assert_eq!(values[2], 0.0);
        assert_eq!(values[2304 * PATCH_VALUES], 0.0);
    }
    #[test]
    fn patches_survive_the_wire_form() {
        let image = image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([x as u8, y as u8, 7]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let pixels = prepare(bytes.get_ref()).unwrap();
        assert_eq!(Pixels::from_bytes(&pixels.to_bytes(), pixels.tokens).unwrap(), pixels);
        assert!(Pixels::from_bytes(&pixels.to_bytes()[1..], pixels.tokens).is_err());
        assert!(Pixels::from_bytes(&pixels.to_bytes(), 0).is_err());
    }
}
