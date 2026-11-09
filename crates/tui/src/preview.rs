//! Inline image previews (`routes/session/index.tsx:2632-2719`, `component/prompt/index.tsx:1713-1780`,
//! `component/dialog-image-preview.tsx`). Every terminal gets the same drawing: a cell is `▀`
//! with the upper pixel as foreground and the lower one as background, the picture fitted into
//! the box with its aspect kept. Images are decoded once; the fitted cell grids are cached per
//! image and size, so a frame only copies spans.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::transcript::markdown::Line;

/// An image to preview: its type, base64 bytes and the label the viewer's footer shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub mime: String,
    pub data: String,
    pub label: String,
}

impl Source {
    /// A `data:<mime>;base64,<bytes>` URI as a source; anything else (a media reference that is
    /// not inline) is not previewable.
    pub fn from_uri(uri: &str, label: String) -> Option<Source> {
        let rest = uri.strip_prefix("data:")?;
        let (head, data) = rest.split_once(',')?;
        let mime = head.strip_suffix(";base64")?;
        Some(Source {
            mime: mime.to_string(),
            data: data.to_string(),
            label,
        })
    }
}

/// Where a set of previews came from: the viewer pages through the same set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Set {
    /// The composer's image attachments.
    Composer,
    /// The user message at this index of the open session's items.
    User(usize),
    /// A tool call's result images, by call ID.
    Tool(String),
}

/// Thumbnails drawn side by side at most; the rest are a `+N more` tile.
pub const MAX_TILES: usize = 3;

/// Thumbnail box in cells for a terminal `height` rows tall: height
/// `clamp(floor(height / 4), 4, 8)`, width twice that (`routes/session/index.tsx:2647`).
pub fn size(height: u16) -> (u16, u16) {
    let h = (height / 4).clamp(4, 8);
    (2 * h, h)
}

/// A clickable thumbnail of a strip: its column offset, width and the image it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    pub x: u16,
    pub w: u16,
    pub set: Set,
    pub index: usize,
}

/// A fitted picture: `rows` lines of `cols` cells.
#[derive(Debug, PartialEq)]
pub struct Grid {
    pub cols: u16,
    pub rows: u16,
    pub lines: Vec<Line>,
}

type Decoded = Option<Arc<image::RgbaImage>>;

struct Caches {
    decoded: HashMap<u64, Decoded>,
    grids: HashMap<(u64, u16, u16), Option<Arc<Grid>>>,
    /// Pictures decoding on a thread.
    loading: std::collections::HashSet<u64>,
}

fn caches() -> &'static Mutex<Caches> {
    static CACHES: OnceLock<Mutex<Caches>> = OnceLock::new();
    CACHES.get_or_init(|| {
        Mutex::new(Caches {
            decoded: HashMap::new(),
            grids: HashMap::new(),
            loading: Default::default(),
        })
    })
}

/// Decoded pictures larger than this on a side are scaled down once after decoding: no box a
/// terminal can show needs more.
const MAX_SIDE: u32 = 1024;
const MAX_DECODED: usize = 32;
const MAX_GRIDS: usize = 256;
/// Base64 up to this long decodes while the frame is drawn; a larger picture (up to 20 MiB)
/// decodes on a thread, with `Loading…` in its place until it lands.
const INLINE_DECODE: usize = 512 * 1024;

/// A cheap identity for base64 data: its length and a hash of its ends and a sample of its
/// middle, so a frame never hashes megabytes.
fn fingerprint(data: &str) -> u64 {
    let bytes = data.as_bytes();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.len().hash(&mut h);
    let edge = 4096.min(bytes.len());
    bytes[..edge].hash(&mut h);
    bytes[bytes.len() - edge..].hash(&mut h);
    let step = (bytes.len() / 512).max(1);
    for b in bytes.iter().step_by(step) {
        b.hash(&mut h);
    }
    h.finish()
}

fn decode(data: &str) -> Decoded {
    use base64::Engine;
    let compact: String;
    let data = if data.contains(char::is_whitespace) {
        compact = data.chars().filter(|c| !c.is_whitespace()).collect();
        &compact
    } else {
        data
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .ok()?;
    let picture = image::load_from_memory(&bytes).ok()?;
    let picture = if picture.width() > MAX_SIDE || picture.height() > MAX_SIDE {
        picture.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        picture
    };
    let rgba = picture.into_rgba8();
    (rgba.width() > 0 && rgba.height() > 0).then(|| Arc::new(rgba))
}

fn decoded(key: u64, data: &str) -> Decoded {
    if let Some(found) = lock().decoded.get(&key) {
        return found.clone();
    }
    // Decode outside the lock: a large picture takes a while.
    let picture = decode(data);
    store(&mut lock(), key, picture.clone());
    picture
}

fn store(caches: &mut Caches, key: u64, picture: Decoded) {
    if caches.decoded.len() >= MAX_DECODED {
        caches.decoded.clear();
    }
    caches.decoded.insert(key, picture);
}

/// The decoded picture for a frame: `None` while a large one decodes on a thread (started on
/// first sight).
fn decoded_for_frame(key: u64, data: &str) -> Option<Decoded> {
    if data.len() <= INLINE_DECODE {
        return Some(decoded(key, data));
    }
    let mut caches = lock();
    if let Some(found) = caches.decoded.get(&key) {
        return Some(found.clone());
    }
    if caches.loading.insert(key) {
        let data = data.to_string();
        std::thread::spawn(move || {
            let picture = decode(&data);
            let mut caches = lock();
            caches.loading.remove(&key);
            store(&mut caches, key, picture);
        });
    }
    None
}

/// Pictures are decoding: the app redraws until they land.
pub fn decoding() -> bool {
    !lock().loading.is_empty()
}

fn lock() -> std::sync::MutexGuard<'static, Caches> {
    caches().lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the base64 `data` decodes to a picture.
pub fn decodes(data: &str) -> bool {
    decoded(fingerprint(data), data).is_some()
}

/// The picture in `data` fitted into `w`×`h` cells (aspect kept, never empty); `None` when it
/// does not decode.
pub fn grid(data: &str, w: u16, h: u16) -> Option<Arc<Grid>> {
    fitted(data, w, h).flatten()
}

/// [`grid`], or `None` while the picture decodes.
fn fitted(data: &str, w: u16, h: u16) -> Option<Option<Arc<Grid>>> {
    if w == 0 || h == 0 {
        return Some(None);
    }
    let key = fingerprint(data);
    if let Some(found) = lock().grids.get(&(key, w, h)) {
        return Some(found.clone());
    }
    let grid = decoded_for_frame(key, data)?.map(|picture| Arc::new(fit(&picture, w, h)));
    let mut caches = lock();
    if caches.grids.len() >= MAX_GRIDS {
        caches.grids.clear();
    }
    caches.grids.insert((key, w, h), grid.clone());
    Some(grid)
}

/// Scales the picture into `w` columns and `2h` pixel rows and pairs the rows into half blocks.
fn fit(picture: &image::RgbaImage, w: u16, h: u16) -> Grid {
    let (iw, ih) = (f64::from(picture.width()), f64::from(picture.height()));
    let (bw, bh) = (f64::from(w), f64::from(h) * 2.0);
    let scale = (bw / iw).min(bh / ih);
    let pw = ((iw * scale).round() as u32).clamp(1, u32::from(w));
    let ph = ((ih * scale).round() as u32).clamp(1, u32::from(h) * 2);
    let scaled = image::imageops::resize(picture, pw, ph, image::imageops::FilterType::Triangle);
    let rows = ph.div_ceil(2);
    let pixel = |x: u32, y: u32| -> Option<Color> {
        if y >= ph {
            return None;
        }
        let p = scaled.get_pixel(x, y).0;
        (p[3] >= 128).then_some(Color::Rgb(p[0], p[1], p[2]))
    };
    let lines = (0..rows)
        .map(|r| {
            (0..pw)
                .map(|x| match (pixel(x, 2 * r), pixel(x, 2 * r + 1)) {
                    (Some(top), Some(bottom)) => Span::styled("▀", Style::new().fg(top).bg(bottom)),
                    (Some(top), None) => Span::styled("▀", Style::new().fg(top)),
                    (None, Some(bottom)) => Span::styled("▄", Style::new().fg(bottom)),
                    (None, None) => Span::raw(" "),
                })
                .collect()
        })
        .collect();
    Grid {
        cols: pw as u16,
        rows: rows as u16,
        lines,
    }
}

/// The picture centred in a `w`×`h` box, or `fallback` (`No preview`, or `Loading…` while a
/// large picture decodes) centred: always `h` lines of `w` cells.
pub fn tile(data: &str, w: u16, h: u16, fallback: Style) -> Vec<Line> {
    match fitted(data, w, h) {
        None => label_tile("Loading…", w, h, fallback),
        Some(Some(grid)) => {
            let left = (w - grid.cols) / 2;
            let top = (h - grid.rows) / 2;
            (0..h)
                .map(|y| {
                    let mut line: Line = Vec::new();
                    match y.checked_sub(top).and_then(|r| grid.lines.get(r as usize)) {
                        Some(row) => {
                            pad(&mut line, left);
                            line.extend(row.iter().cloned());
                            pad(&mut line, w - left - grid.cols);
                        }
                        None => pad(&mut line, w),
                    }
                    line
                })
                .collect()
        }
        Some(None) => label_tile("No preview", w, h, fallback),
    }
}

/// `text` centred in a `w`×`h` box of blanks.
fn label_tile(text: &str, w: u16, h: u16, style: Style) -> Vec<Line> {
    let text = crate::text::clip(text, w as usize);
    let tw = crate::text::width(&text) as u16;
    (0..h)
        .map(|y| {
            let mut line: Line = Vec::new();
            if y == (h.saturating_sub(1)) / 2 {
                let left = (w - tw) / 2;
                pad(&mut line, left);
                line.push(Span::styled(text.clone(), style));
                pad(&mut line, w - left - tw);
            } else {
                pad(&mut line, w);
            }
            line
        })
        .collect()
}

fn pad(line: &mut Line, n: u16) {
    if n > 0 {
        line.push(Span::raw(" ".repeat(n as usize)));
    }
}

/// Up to three thumbnails of the base64 `images` side by side, one column apart, then a `+N more` tile:
/// `h` lines and the tiles' click areas. `error` styles `No preview`, `muted` the more tile.
pub fn strip(
    images: &[&str],
    (w, h): (u16, u16),
    set: &Set,
    error: Style,
    muted: Style,
) -> (Vec<Line>, Vec<Tile>) {
    let mut lines: Vec<Line> = vec![Vec::new(); h as usize];
    let mut tiles = Vec::new();
    let mut x = 0;
    let mut add = |cells: Vec<Line>, index: usize, x: &mut u16| {
        for (line, cells) in lines.iter_mut().zip(cells) {
            if *x > 0 {
                pad(line, 1);
            }
            line.extend(cells);
        }
        if *x > 0 {
            *x += 1;
        }
        tiles.push(Tile {
            x: *x,
            w,
            set: set.clone(),
            index,
        });
        *x += w;
    };
    for (index, data) in images.iter().take(MAX_TILES).enumerate() {
        let style = if fitted(data, w, h).is_some() {
            error
        } else {
            muted
        };
        add(tile(data, w, h, style), index, &mut x);
    }
    if images.len() > MAX_TILES {
        let more = format!("+{} more", images.len() - MAX_TILES);
        add(label_tile(&more, w, h, muted), MAX_TILES, &mut x);
    }
    (lines, tiles)
}

/// The images of `set` in the current state.
pub fn images(state: &crate::state::State, set: &Set) -> Vec<Arc<Source>> {
    use crate::transcript::model::{Item, Part};
    match set {
        Set::Composer => state.composer.images(),
        Set::User(index) => match state.session().and_then(|s| s.items.get(*index)) {
            Some(Item::User { images, .. }) => images.clone(),
            _ => Vec::new(),
        },
        Set::Tool(call_id) => state
            .session()
            .and_then(|s| {
                s.items.iter().rev().find_map(|item| match item {
                    Item::Assistant(a) => a.parts.iter().find_map(|p| match p {
                        Part::Tool(t) if &t.call_id == call_id => Some(t.images.clone()),
                        _ => None,
                    }),
                    _ => None,
                })
            })
            .unwrap_or_default(),
    }
}

/// The image viewer on image `index` (wrapping) of `set`: `Image <i> of <n>`, the picture
/// fitted to the dialog, footer `← previous` / label (or `No preview` in red) / `next →`
/// (`component/dialog-image-preview.tsx:33-82`). `None` when the set has no images.
pub fn viewer(state: &crate::state::State, set: &Set, index: usize) -> Option<crate::dialog::Body> {
    use crate::dialog::{Body, BodyKind, Size, Tone};
    let images = images(state, set);
    if images.is_empty() {
        return None;
    }
    let index = index % images.len();
    let source = images[index].clone();
    let label = if decodes(&source.data) {
        let label = if source.label.is_empty() {
            format!("Image {}", index + 1)
        } else {
            source.label.clone()
        };
        (label, Tone::Base)
    } else {
        ("No preview".into(), Tone::Error)
    };
    let mut body = Body::new(
        BodyKind::Image {
            set: set.clone(),
            index,
            source,
        },
        &format!("Image {} of {}", index + 1, images.len()),
        Vec::new(),
    );
    body.size = Size::XLarge;
    body.footer = vec![
        ("← previous  ".into(), Tone::Muted),
        label,
        ("  next →".into(), Tone::Muted),
    ];
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> String {
        use base64::Engine;
        let mut bytes = Vec::new();
        let picture = image::RgbaImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        image::DynamicImage::ImageRgba8(picture)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn size_follows_the_terminal_height() {
        assert_eq!(size(10), (8, 4));
        assert_eq!(size(24), (12, 6));
        assert_eq!(size(36), (16, 8));
        assert_eq!(size(80), (16, 8));
    }

    #[test]
    fn fits_keeping_the_aspect() {
        // 40×20 pixels into 16×8 cells (16×16 pixels): 16 columns, 8 pixel rows = 4 cell rows.
        let grid = grid(&png(40, 20), 16, 8).unwrap();
        assert_eq!((grid.cols, grid.rows), (16, 4));
        let first = &grid.lines[0][0];
        assert_eq!(first.content, "▀");
        assert_eq!(first.style.fg, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(first.style.bg, Some(Color::Rgb(255, 0, 0)));
        let last = grid.lines[0].last().unwrap();
        assert_eq!(last.style.fg, Some(Color::Rgb(0, 0, 255)));
        let tile = tile(&png(40, 20), 16, 8, Style::new());
        assert_eq!(tile.len(), 8);
        for line in &tile {
            let w: usize = line.iter().map(|s| crate::text::width(&s.content)).sum();
            assert_eq!(w, 16);
        }
    }

    #[test]
    fn large_pictures_decode_off_the_frame() {
        use base64::Engine;
        // Noise does not compress: the PNG is well past the inline limit.
        let mut seed = 7u32;
        let picture = image::RgbaImage::from_fn(400, 400, |_, _| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let [r, g, b, _] = seed.to_le_bytes();
            image::Rgba([r, g, b, 255])
        });
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(picture)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert!(data.len() > INLINE_DECODE);
        let text = |lines: Vec<Line>| -> String {
            lines
                .iter()
                .flatten()
                .map(|s| s.content.to_string())
                .collect()
        };
        assert_eq!(text(tile(&data, 16, 8, Style::new())).trim(), "Loading…");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while decoding() {
            assert!(std::time::Instant::now() < deadline, "still decoding");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(grid(&data, 16, 8).is_some());
    }

    #[test]
    fn corrupt_data_has_no_preview() {
        assert!(grid("bm90IGFuIGltYWdl", 8, 4).is_none());
        let tile = tile("bm90IGFuIGltYWdl", 8, 4, Style::new());
        let text: String = tile.iter().flatten().map(|s| s.content.as_ref()).collect();
        assert!(text.trim().starts_with("No"), "{text:?}");
    }

    #[test]
    fn data_uris() {
        let s = Source::from_uri("data:image/png;base64,AAAA", "x".into()).unwrap();
        assert_eq!((s.mime.as_str(), s.data.as_str()), ("image/png", "AAAA"));
        assert!(Source::from_uri("file:///a.png", "x".into()).is_none());
    }
}
