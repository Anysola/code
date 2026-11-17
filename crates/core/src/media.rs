//! Image normalization before images reach the model (tool results and prompt attachments), with
//! the rules 0.5.1 applied through photon: images within the limits pass unchanged (large opaque
//! PNGs become JPEG when that is smaller); larger ones are scaled to fit and re-encoded, shrinking
//! by 0.75 per step until the base64 size fits. Images of a type the models don't take are
//! converted to PNG first (and then follow the same rules); ones that can't be decoded are refused.

use std::io::Cursor;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde_json::Value;

use crate::tool::ToolContent;

const PNG_PASSTHROUGH_MAX_BASE64_BYTES: usize = 384 * 1024;
const SMALL_PNG_MAX_PIXELS: u64 = 256 * 256;
const JPEG_QUALITIES: [u8; 4] = [80, 70, 55, 40];
const SIZE_STEPS: usize = 32;

/// Media the read tool returns as files; MCP results with these types are sent the same way.
pub const READABLE: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "application/pdf",
];

/// The image types the models take; images of other types are converted or refused.
pub const ACCEPTED: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Debug, Clone, Copy, PartialEq)]
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
            max_width: 2000,
            max_height: 2000,
            max_base64_bytes: 5 * 1024 * 1024,
        }
    }
}

impl Limits {
    /// `media.image.{auto_resize,max_width,max_height,max_base64_bytes}` over the defaults.
    pub fn from_config(media: Option<&Value>) -> Limits {
        let mut limits = Limits::default();
        let Some(image) = media.and_then(|m| m.get("image")) else {
            return limits;
        };
        let number = |key: &str| image.get(key).and_then(Value::as_u64);
        if let Some(value) = image.get("auto_resize").and_then(Value::as_bool) {
            limits.auto_resize = value;
        }
        if let Some(value) = number("max_width") {
            limits.max_width = value as u32;
        }
        if let Some(value) = number("max_height") {
            limits.max_height = value as u32;
        }
        if let Some(value) = number("max_base64_bytes") {
            limits.max_base64_bytes = value as usize;
        }
        limits
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImageError {
    Decode,
    Size {
        width: u32,
        height: u32,
        bytes: usize,
        limits: Limits,
    },
}

impl ImageError {
    /// The error text of a prompt attachment that cannot be used.
    pub fn message(&self, resource: &str) -> String {
        match self {
            ImageError::Decode => format!("Image could not be decoded: {resource}"),
            ImageError::Size {
                width,
                height,
                bytes,
                limits,
            } => format!(
                "Image {resource} is {width}x{height} with base64 size {bytes}, exceeding configured limits {}x{}/{} bytes",
                limits.max_width, limits.max_height, limits.max_base64_bytes
            ),
        }
    }
}

fn base64_len(bytes: usize) -> usize {
    bytes.div_ceil(3) * 4
}

/// More than 5% of an evenly spaced grid of up to 64×64 pixels has alpha below 250.
fn has_transparency(pixels: &RgbaImage) -> bool {
    let (width, height) = pixels.dimensions();
    let columns = width.min(64);
    let rows = height.min(64);
    let mut transparent = 0u32;
    for row in 0..rows {
        let y = (((row as f64 + 0.5) * height as f64) / rows as f64).floor() as u32;
        for column in 0..columns {
            let x = (((column as f64 + 0.5) * width as f64) / columns as f64).floor() as u32;
            if pixels.get_pixel(x, y)[3] < 250 {
                transparent += 1;
            }
        }
    }
    transparent as f64 / (columns * rows) as f64 > 0.05
}

fn encode_png(image: &RgbaImage) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let _ = image.write_to(&mut out, ImageFormat::Png);
    out.into_inner()
}

fn encode_jpeg(image: &RgbaImage, quality: u8) -> Vec<u8> {
    let rgb = DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut out = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
    let _ = rgb.write_with_encoder(encoder);
    out
}

/// The sizes tried, largest first: the fitted size, then 0.75 steps, without repeats.
fn sizes(width: u32, height: u32, limits: &Limits) -> Vec<(u32, u32)> {
    let scale = 1f64
        .min(limits.max_width as f64 / width as f64)
        .min(limits.max_height as f64 / height as f64);
    let first = (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    );
    let mut out = vec![first];
    for _ in 1..SIZE_STEPS {
        let (w, h) = *out.last().unwrap_or(&first);
        let shrink = |v: u32| {
            if v == 1 {
                1
            } else {
                ((v as f64 * 0.75).floor() as u32).max(1)
            }
        };
        let next = (shrink(w), shrink(h));
        if !out.contains(&next) {
            out.push(next);
        }
    }
    out
}

/// Normalizes base64 image data; `Ok(None)` means unchanged, `Ok(Some((mime, base64)))` replaced.
/// The bytes decide the type: data of an accepted type under another name is relabeled, other
/// decodable data (BMP) is converted, and anything else (SVG, TIFF, HEIC, AVIF, ICO, ...) fails
/// with [`ImageError::Decode`].
pub fn normalize(
    mime: &str,
    data: &str,
    limits: &Limits,
) -> Result<Option<(String, String)>, ImageError> {
    let bytes = STANDARD.decode(data).map_err(|_| ImageError::Decode)?;
    let format = image::guess_format(&bytes).map_err(|_| ImageError::Decode)?;
    let decoded =
        image::load_from_memory_with_format(&bytes, format).map_err(|_| ImageError::Decode)?;
    let actual = match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Gif => "image/gif",
        ImageFormat::WebP => "image/webp",
        _ => {
            let png = STANDARD.encode(encode_png(&decoded.to_rgba8()));
            return Ok(Some(
                fit("image/png", &png, &decoded, limits)?
                    .unwrap_or_else(|| ("image/png".to_string(), png)),
            ));
        }
    };
    Ok(match fit(actual, data, &decoded, limits)? {
        None if !mime.eq_ignore_ascii_case(actual) => Some((actual.to_string(), data.to_string())),
        fitted => fitted,
    })
}

/// [`normalize`] for data of an accepted type, `mime` being the one its bytes have.
fn fit(
    mime: &str,
    data: &str,
    decoded: &DynamicImage,
    limits: &Limits,
) -> Result<Option<(String, String)>, ImageError> {
    let (width, height) = (decoded.width(), decoded.height());
    let size = data.len();
    let fits =
        width <= limits.max_width && height <= limits.max_height && size <= limits.max_base64_bytes;
    let small_png = size <= PNG_PASSTHROUGH_MAX_BASE64_BYTES
        || (width as u64) * (height as u64) <= SMALL_PNG_MAX_PIXELS;
    if fits && (!limits.auto_resize || mime != "image/png" || small_png) {
        return Ok(None);
    }
    if !limits.auto_resize {
        return Err(ImageError::Size {
            width,
            height,
            bytes: size,
            limits: *limits,
        });
    }
    let pixels = decoded.to_rgba8();
    let transparent = mime != "image/jpeg" && has_transparency(&pixels);
    if fits {
        if transparent {
            return Ok(None);
        }
        let jpeg = encode_jpeg(&pixels, JPEG_QUALITIES[0]);
        return Ok((base64_len(jpeg.len()) < size)
            .then(|| ("image/jpeg".to_string(), STANDARD.encode(jpeg))));
    }
    for (w, h) in sizes(width, height, limits) {
        let resized = if (w, h) == (width, height) {
            pixels.clone()
        } else {
            image::imageops::resize(&pixels, w, h, FilterType::Lanczos3)
        };
        let png_first = transparent || (w as u64) * (h as u64) <= SMALL_PNG_MAX_PIXELS;
        let mut encoders: Vec<(&str, Option<u8>)> = JPEG_QUALITIES
            .iter()
            .map(|q| ("image/jpeg", Some(*q)))
            .collect();
        if png_first {
            encoders.insert(0, ("image/png", None));
        } else {
            encoders.push(("image/png", None));
        }
        for (mime, quality) in encoders {
            let candidate = match quality {
                Some(quality) => encode_jpeg(&resized, quality),
                None => encode_png(&resized),
            };
            if base64_len(candidate.len()) <= limits.max_base64_bytes {
                return Ok(Some((mime.to_string(), STANDARD.encode(candidate))));
            }
        }
    }
    Err(ImageError::Size {
        width,
        height,
        bytes: size,
        limits: *limits,
    })
}

/// Normalizes the base64 images of a tool result; dropped images are counted in a trailing note.
pub fn normalize_content(content: Vec<ToolContent>, limits: &Limits) -> Vec<ToolContent> {
    let mut decode = 0;
    let mut size = 0;
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
            match normalize(mime, data, limits) {
                Ok(None) => Some(item),
                Ok(Some((mime, data))) => Some(ToolContent::File {
                    uri: format!("data:{mime};base64,{data}"),
                    mime,
                    name: name.clone(),
                }),
                Err(ImageError::Decode) => {
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

    fn png(width: u32, height: u32, noise: bool, alpha: u8) -> String {
        let mut seed = 12345u32;
        let image = RgbaImage::from_fn(width, height, |x, y| {
            if noise {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let v = seed.to_be_bytes();
                image::Rgba([v[0], v[1], v[2], alpha])
            } else {
                image::Rgba([(x % 256) as u8, (y % 256) as u8, 0, alpha])
            }
        });
        STANDARD.encode(encode_png(&image))
    }

    fn dimensions(data: &str) -> (u32, u32) {
        let image = image::load_from_memory(&STANDARD.decode(data).unwrap()).unwrap();
        (image.width(), image.height())
    }

    #[test]
    fn small_images_pass() {
        let data = png(10, 10, false, 255);
        assert_eq!(normalize("image/png", &data, &Limits::default()), Ok(None));
    }

    #[test]
    fn wide_noise_png_becomes_fitted_jpeg() {
        let data = png(3000, 1000, true, 255);
        let (mime, out) = normalize("image/png", &data, &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(dimensions(&out), (2000, 667));
        assert!(out.len() <= 5 * 1024 * 1024);
    }

    #[test]
    fn large_opaque_png_within_limits_becomes_jpeg() {
        let data = png(800, 800, true, 255);
        let (mime, out) = normalize("image/png", &data, &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            (mime.as_str(), dimensions(&out)),
            ("image/jpeg", (800, 800))
        );
        let transparent = png(800, 800, true, 10);
        assert_eq!(
            normalize("image/png", &transparent, &Limits::default()),
            Ok(None)
        );
    }

    #[test]
    fn shrinks_until_the_bytes_fit() {
        let limits = Limits {
            max_base64_bytes: 40_000,
            ..Limits::default()
        };
        let data = png(600, 600, true, 255);
        let (_, out) = normalize("image/png", &data, &limits).unwrap().unwrap();
        assert!(out.len() <= 40_000);
        assert!(dimensions(&out).0 < 600);
        let fixed = Limits {
            auto_resize: false,
            max_width: 100,
            ..Limits::default()
        };
        assert_eq!(
            normalize("image/png", &data, &fixed)
                .unwrap_err()
                .message("a.png"),
            format!(
                "Image a.png is 600x600 with base64 size {}, exceeding configured limits 100x2000/5242880 bytes",
                data.len()
            )
        );
    }

    fn encoded(width: u32, height: u32, alpha: u8, format: ImageFormat) -> String {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 0, alpha])
        });
        let mut out = Cursor::new(Vec::new());
        match format {
            ImageFormat::Jpeg => DynamicImage::ImageRgba8(image)
                .to_rgb8()
                .write_to(&mut out, format)
                .unwrap(),
            _ => image.write_to(&mut out, format).unwrap(),
        }
        STANDARD.encode(out.into_inner())
    }

    fn format(data: &str) -> ImageFormat {
        image::guess_format(&STANDARD.decode(data).unwrap()).unwrap()
    }

    #[test]
    fn types_the_model_does_not_take_become_png() {
        let bmp = encoded(40, 30, 255, ImageFormat::Bmp);
        let (mime, out) = normalize("image/bmp", &bmp, &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(
            (format(&out), dimensions(&out)),
            (ImageFormat::Png, (40, 30))
        );
        // Converting is not resizing: it happens with auto_resize off too.
        let fixed = Limits {
            auto_resize: false,
            ..Limits::default()
        };
        let (mime, _) = normalize("image/x-ms-bmp", &bmp, &fixed).unwrap().unwrap();
        assert_eq!(mime, "image/png");
        // Too large, it is scaled like any other image.
        let large = encoded(3000, 100, 255, ImageFormat::Bmp);
        let (mime, out) = normalize("image/bmp", &large, &Limits::default())
            .unwrap()
            .unwrap();
        assert!(ACCEPTED.contains(&mime.as_str()));
        assert_eq!(dimensions(&out), (2000, 67));
        assert!(matches!(
            normalize("image/bmp", &large, &fixed),
            Err(ImageError::Size { .. })
        ));
    }

    #[test]
    fn images_that_cannot_be_decoded_are_refused() {
        let svg = STANDARD.encode(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>"#,
        );
        // TIFF, HEIC, AVIF and ICO are recognized but not decoded with this build's features.
        let tiff = STANDARD.encode(b"II*\0\x08\0\0\0\0\0\0\0");
        let ico = STANDARD.encode(b"\0\0\x01\0\x01\0\x10\x10\0\0\x01\0\x20\0\0\0\0\0");
        for (mime, data) in [
            ("image/svg+xml", &svg),
            ("image/tiff", &tiff),
            ("image/x-icon", &ico),
        ] {
            assert_eq!(
                normalize(mime, data, &Limits::default()),
                Err(ImageError::Decode),
                "{mime}"
            );
        }
    }

    #[test]
    fn the_bytes_decide_the_type() {
        let jpeg = encoded(10, 10, 255, ImageFormat::Jpeg);
        assert_eq!(normalize("image/jpeg", &jpeg, &Limits::default()), Ok(None));
        assert_eq!(normalize("image/JPEG", &jpeg, &Limits::default()), Ok(None));
        for named in ["image/jpg", "image/png"] {
            assert_eq!(
                normalize(named, &jpeg, &Limits::default()),
                Ok(Some(("image/jpeg".to_string(), jpeg.clone()))),
                "{named}"
            );
        }
    }

    #[test]
    fn size_steps() {
        let limits = Limits::default();
        let steps = sizes(4000, 10, &limits);
        assert_eq!(&steps[..3], &[(2000, 5), (1500, 3), (1125, 2)]);
        assert!(steps.len() <= SIZE_STEPS);
    }

    #[test]
    fn tool_content_notes() {
        let content = vec![
            ToolContent::text("Image read successfully"),
            ToolContent::File {
                uri: "data:image/png;base64,AAAA".into(),
                mime: "image/png".into(),
                name: None,
            },
        ];
        assert_eq!(
            normalize_content(content, &Limits::default()),
            vec![
                ToolContent::text("Image read successfully"),
                ToolContent::text("[1 image omitted: could not be decoded.]"),
            ]
        );
    }
}
