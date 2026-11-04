//! Image normalization before images reach the model (prompt attachments and tool results), as
//! 0.5.1's photon-normalize did: images within the limits pass through (large PNGs become JPEG
//! when that is smaller and they are opaque); bigger ones are scaled to fit, then shrunk by 0.75
//! per step until an encoding fits the base64 byte limit.

use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageFormat};
use serde_json::Value;

const PNG_PASSTHROUGH_MAX_BASE64_BYTES: usize = 384 * 1024;
const SMALL_PNG_MAX_PIXELS: u64 = 256 * 256;
const JPEG_QUALITIES: [u8; 4] = [80, 70, 55, 40];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub auto_resize: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub max_base64_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            auto_resize: true,
            max_width: 2_000,
            max_height: 2_000,
            max_base64_bytes: 5 * 1024 * 1024,
        }
    }
}

impl Limits {
    /// `media.image.{auto_resize,max_width,max_height,max_base64_bytes}` over the defaults.
    pub fn from_config(config: &crate::config::Config) -> Limits {
        let image = config.get("media").and_then(|m| m.get("image"));
        let number = |key: &str| image.and_then(|i| i.get(key)).and_then(Value::as_u64);
        let defaults = Limits::default();
        Limits {
            auto_resize: image
                .and_then(|i| i.get("auto_resize"))
                .and_then(Value::as_bool)
                .unwrap_or(defaults.auto_resize),
            max_width: number("max_width").map_or(defaults.max_width, |v| v as u32),
            max_height: number("max_height").map_or(defaults.max_height, |v| v as u32),
            max_base64_bytes: number("max_base64_bytes")
                .map_or(defaults.max_base64_bytes, |v| v as usize),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageError {
    #[error("Image could not be decoded: {0}")]
    Decode(String),
    #[error(
        "Image {resource} is {width}x{height} with base64 size {bytes}, exceeding configured limits {max_width}x{max_height}/{max_bytes} bytes"
    )]
    Size {
        resource: String,
        width: u32,
        height: u32,
        bytes: usize,
        max_width: u32,
        max_height: u32,
        max_bytes: usize,
    },
}

fn base64_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// More than 5% of an evenly spaced grid of up to 64×64 pixels has alpha below 250.
fn transparent(image: &DynamicImage) -> bool {
    if !image.color().has_alpha() {
        return false;
    }
    let (width, height) = image.dimensions();
    let columns = width.min(64);
    let rows = height.min(64);
    let mut count = 0;
    for row in 0..rows {
        let y = (((row as f64 + 0.5) * height as f64) / rows as f64).floor() as u32;
        for column in 0..columns {
            let x = (((column as f64 + 0.5) * width as f64) / columns as f64).floor() as u32;
            if image.get_pixel(x, y)[3] < 250 {
                count += 1;
            }
        }
    }
    count as f64 / (columns * rows) as f64 > 0.05
}

fn encode(image: &DynamicImage, mime: &str, quality: u8) -> Option<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    if mime == "image/png" {
        image.write_to(&mut out, ImageFormat::Png).ok()?;
    } else {
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        image.to_rgb8().write_with_encoder(encoder).ok()?;
    }
    Some(out.into_inner())
}

/// The image as it should be sent: `(mime, base64)`, unchanged when it already qualifies.
pub fn normalize(
    resource: &str,
    mime: &str,
    data: &str,
    limits: &Limits,
) -> Result<(String, String), ImageError> {
    let decode = || ImageError::Decode(resource.to_string());
    let bytes = STANDARD.decode(data.trim()).map_err(|_| decode())?;
    let image = image::load_from_memory(&bytes).map_err(|_| decode())?;
    let (width, height) = image.dimensions();
    let size = data.len();
    let fits =
        width <= limits.max_width && height <= limits.max_height && size <= limits.max_base64_bytes;
    let small_png = size <= PNG_PASSTHROUGH_MAX_BASE64_BYTES
        || u64::from(width) * u64::from(height) <= SMALL_PNG_MAX_PIXELS;
    let unchanged = || Ok((mime.to_string(), data.to_string()));
    if fits && (!limits.auto_resize || mime != "image/png" || small_png) {
        return unchanged();
    }
    let too_big = || ImageError::Size {
        resource: resource.to_string(),
        width,
        height,
        bytes: size,
        max_width: limits.max_width,
        max_height: limits.max_height,
        max_bytes: limits.max_base64_bytes,
    };
    if !limits.auto_resize {
        return Err(too_big());
    }
    let alpha = mime != "image/jpeg" && transparent(&image);
    if fits {
        if alpha {
            return unchanged();
        }
        return match encode(&image, "image/jpeg", JPEG_QUALITIES[0]) {
            Some(jpeg) if base64_len(jpeg.len()) < size => {
                Ok(("image/jpeg".into(), STANDARD.encode(jpeg)))
            }
            _ => unchanged(),
        };
    }
    let scale = 1f64
        .min(limits.max_width as f64 / width as f64)
        .min(limits.max_height as f64 / height as f64);
    let mut sizes: Vec<(u32, u32)> = Vec::new();
    let mut current = (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    );
    for _ in 0..32 {
        if !sizes.contains(&current) {
            sizes.push(current);
        }
        let shrink = |v: u32| {
            if v == 1 {
                1
            } else {
                ((v as f64 * 0.75).floor() as u32).max(1)
            }
        };
        current = (shrink(current.0), shrink(current.1));
    }
    for (w, h) in sizes {
        let resized = if (w, h) == (width, height) {
            image.clone()
        } else {
            image.resize_exact(w, h, FilterType::Lanczos3)
        };
        let png_first = alpha || u64::from(w) * u64::from(h) <= SMALL_PNG_MAX_PIXELS;
        let mut encoders: Vec<(&str, u8)> =
            JPEG_QUALITIES.iter().map(|q| ("image/jpeg", *q)).collect();
        if png_first {
            encoders.insert(0, ("image/png", 0));
        } else {
            encoders.push(("image/png", 0));
        }
        for (target, quality) in encoders {
            if let Some(candidate) = encode(&resized, target, quality)
                && base64_len(candidate.len()) <= limits.max_base64_bytes
            {
                return Ok((target.to_string(), STANDARD.encode(candidate)));
            }
        }
    }
    Err(too_big())
}

/// Normalizes the image items of a tool result. Images that cannot be decoded or made small
/// enough are dropped and counted in a closing note.
pub fn normalize_tool_content(
    content: Vec<crate::tool::ToolContent>,
    limits: &Limits,
) -> Vec<crate::tool::ToolContent> {
    use crate::tool::ToolContent;
    let (mut decode, mut size) = (0, 0);
    let mut out: Vec<ToolContent> = content
        .into_iter()
        .filter_map(|item| {
            let ToolContent::File { uri, mime, name } = &item else {
                return Some(item);
            };
            if !mime.starts_with("image/") {
                return Some(item);
            }
            let Some(data) = uri
                .strip_prefix("data:")
                .and_then(|rest| rest.split_once(";base64,"))
                .map(|(_, data)| data)
            else {
                return Some(item);
            };
            let resource = name
                .clone()
                .unwrap_or_else(|| format!("{mime} tool output"));
            match normalize(&resource, mime, data, limits) {
                Ok((mime, data)) => Some(ToolContent::File {
                    uri: format!("data:{mime};base64,{data}"),
                    mime,
                    name: name.clone(),
                }),
                Err(ImageError::Decode(_)) => {
                    decode += 1;
                    None
                }
                Err(ImageError::Size { .. }) => {
                    size += 1;
                    None
                }
            }
        })
        .collect();
    for (count, text) in [
        (decode, "could not be decoded."),
        (size, "could not be resized below the image size limit."),
    ] {
        if count > 0 {
            let plural = if count == 1 { "" } else { "s" };
            out.push(ToolContent::text(format!(
                "[{count} image{plural} omitted: {text}]"
            )));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn png(image: DynamicImage) -> String {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        STANDARD.encode(out.into_inner())
    }

    fn noise(width: u32, height: u32) -> DynamicImage {
        let mut seed: u32 = 7;
        DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |_, _| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let b = seed.to_le_bytes();
            Rgb([b[1], b[2], b[3]])
        }))
    }

    fn dimensions(data: &str) -> (u32, u32) {
        image::load_from_memory(&STANDARD.decode(data).unwrap())
            .unwrap()
            .dimensions()
    }

    #[test]
    fn small_images_pass_through() {
        let data = png(noise(10, 10));
        let limits = Limits::default();
        assert_eq!(
            normalize("a.png", "image/png", &data, &limits).unwrap(),
            ("image/png".into(), data)
        );
    }

    #[test]
    fn oversized_noise_becomes_a_fitting_jpeg() {
        let data = png(noise(3000, 1000));
        let (mime, out) = normalize("n.png", "image/png", &data, &Limits::default()).unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(dimensions(&out), (2000, 667));
        assert!(out.len() <= 5 * 1024 * 1024);
    }

    #[test]
    fn large_opaque_png_within_limits_turns_jpeg() {
        let data = png(noise(800, 800));
        assert!(data.len() > PNG_PASSTHROUGH_MAX_BASE64_BYTES);
        let (mime, out) = normalize("n.png", "image/png", &data, &Limits::default()).unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(out.len() < data.len());
        let clear = DynamicImage::ImageRgba8(RgbaImage::from_pixel(800, 800, Rgba([1, 2, 3, 0])));
        let mut noisy = clear.to_rgba8();
        for (x, y, p) in noise(800, 800).to_rgba8().enumerate_pixels() {
            let mut q = *p;
            q[3] = 0;
            noisy.put_pixel(x, y, q);
        }
        let data = png(DynamicImage::ImageRgba8(noisy));
        let (mime, _) = normalize("t.png", "image/png", &data, &Limits::default()).unwrap();
        assert_eq!(mime, "image/png", "transparency keeps PNG");
    }

    #[test]
    fn failures() {
        assert_eq!(
            normalize("x.png", "image/png", "bm90IGFuIGltYWdl", &Limits::default()).unwrap_err(),
            ImageError::Decode("x.png".into())
        );
        let limits = Limits {
            auto_resize: false,
            max_width: 5,
            ..Limits::default()
        };
        let error = normalize("big.png", "image/png", &png(noise(10, 10)), &limits).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Image big.png is 10x10 with base64 size")
        );
        let content = vec![
            crate::tool::ToolContent::text("t"),
            crate::tool::ToolContent::File {
                uri: "data:image/png;base64,bm90IGFuIGltYWdl".into(),
                mime: "image/png".into(),
                name: None,
            },
        ];
        let out = normalize_tool_content(content, &Limits::default());
        assert_eq!(
            out.last().unwrap(),
            &crate::tool::ToolContent::text("[1 image omitted: could not be decoded.]")
        );
        assert_eq!(out.len(), 2);
    }
}
