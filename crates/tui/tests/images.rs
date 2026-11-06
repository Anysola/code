//! Inline image previews: composer thumbnails, user-message and tool-result images under
//! `session.image_preview`, the size rule, `No preview`, and the viewer a click opens.

use std::sync::Arc;

use anysola_tui::composer::AtomKind;
use anysola_tui::dialog::Dialog;
use anysola_tui::preview::{Set, Source, size, viewer};
use anysola_tui::state::{Route, Session, State};
use anysola_tui::theme::{Catalog, Mode as ThemeMode};
use anysola_tui::transcript::model::{Assistant, Item, Part, Status, Tool};
use anysola_tui::view::{Drawn, Hit, hit_at};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn draw(state: &State) -> (Buffer, Drawn) {
    let theme = Catalog::bundled().load("anysola", ThemeMode::Dark).unwrap();
    let mut buffer = Buffer::empty(Rect::new(0, 0, state.width, state.height));
    let drawn = anysola_tui::view::draw(state, &theme, &mut buffer);
    (buffer, drawn)
}

/// A `w`×`h` PNG, red on the left half and blue on the right, as base64.
fn png(w: u32, h: u32) -> String {
    use base64::Engine;
    let picture = image::RgbaImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

const CORRUPT: &str = "bm90IGFuIGltYWdl";

fn session(width: u16, height: u16, items: Vec<Item>) -> State {
    let mut state = State::new("0.5.1");
    state.width = width;
    state.height = height;
    state.route = Route::Session("ses_a".into());
    state.sessions.insert(
        "ses_a".into(),
        Session {
            id: "ses_a".into(),
            title: "Images".into(),
            directory: "/home/user/my-app".into(),
            items,
            ..Session::default()
        },
    );
    state
}

fn attach(state: &mut State, images: &[&str]) {
    for (i, data) in images.iter().enumerate() {
        state.composer.insert_atom(
            &format!("[Image {}]", i + 1),
            AtomKind::Attachment("image/png".into(), data.to_string(), "shot.png".into()),
        );
        state.composer.insert(" ");
    }
}

/// Rows holding half-block picture cells, and the most such cells on one row.
fn picture_rows(buffer: &Buffer) -> (Vec<u16>, usize) {
    let mut rows = Vec::new();
    let mut most = 0;
    for y in 0..buffer.area.height {
        let n = (0..buffer.area.width)
            .filter(|&x| matches!(buffer[(x, y)].symbol(), "▀" | "▄"))
            .filter(|&x| {
                // The test pictures are red and blue; the theme draws nothing without green.
                matches!(buffer[(x, y)].fg, ratatui::style::Color::Rgb(r, 0, b) if u16::from(r) + u16::from(b) >= 200)
            })
            .count();
        if n > 0 {
            rows.push(y);
            most = most.max(n);
        }
    }
    (rows, most)
}

fn find(buffer: &Buffer, text: &str) -> Option<(u16, u16)> {
    for y in 0..buffer.area.height {
        let cells: Vec<&str> = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let line = cells.concat();
        if let Some(byte) = line.find(text) {
            let mut seen = 0;
            for (x, cell) in cells.iter().enumerate() {
                if seen >= byte {
                    return Some((x as u16, y));
                }
                seen += cell.len();
            }
        }
    }
    None
}

#[test]
fn composer_thumbnails_need_the_setting() {
    let data = png(40, 20);
    let mut state = session(120, 36, Vec::new());
    attach(&mut state, &[&data]);
    let (buffer, drawn) = draw(&state);
    assert!(picture_rows(&buffer).0.is_empty());
    assert!(!drawn.hits.iter().any(|(_, h)| matches!(h, Hit::Image(..))));
}

#[test]
fn composer_shows_three_thumbnails_and_more() {
    let data = png(40, 20);
    let mut state = session(120, 36, Vec::new());
    state.settings.prompt_image_preview = true;
    attach(&mut state, &[&data, &data, &data, &data, &data]);
    let (buffer, drawn) = draw(&state);
    let (w, h) = size(36);
    assert_eq!((w, h), (16, 8));
    let (rows, most) = picture_rows(&buffer);
    // 40×20 pixels fit 16 columns and 8 pixel rows: four cell rows of three 16-cell tiles.
    assert_eq!(rows.len(), 4);
    assert_eq!(most, 3 * w as usize);
    let (mx, my) = find(&buffer, "+2 more").expect("+2 more tile");
    assert_eq!(
        hit_at(&drawn.hits, (mx, my)),
        Some(&Hit::Image(Set::Composer, 3))
    );
    // The thumbnails sit above the textarea: the chips come after them.
    let (_, chips) = find(&buffer, "[Image 1]").expect("chips");
    assert!(rows.iter().all(|&y| y < chips));
    let y = rows[0];
    let first = (0..120).find(|&x| buffer[(x, y)].symbol() == "▀").unwrap();
    assert_eq!(
        hit_at(&drawn.hits, (first, y)),
        Some(&Hit::Image(Set::Composer, 0))
    );
    assert_eq!(
        hit_at(&drawn.hits, (first + w + 1, y)),
        Some(&Hit::Image(Set::Composer, 1))
    );
}

#[test]
fn composer_corrupt_image_has_no_preview() {
    let mut state = session(120, 36, Vec::new());
    state.settings.prompt_image_preview = true;
    attach(&mut state, &[CORRUPT]);
    let (buffer, drawn) = draw(&state);
    let (x, y) = find(&buffer, "No preview").expect("No preview");
    assert_eq!(
        hit_at(&drawn.hits, (x, y)),
        Some(&Hit::Image(Set::Composer, 0))
    );
}

#[test]
fn thumbnail_size_follows_the_terminal_height() {
    // A square picture fills its box's height: `clamp(floor(h / 4), 4, 8)` rows.
    let data = png(32, 32);
    for (height, rows) in [(20, 5), (24, 6), (40, 8), (16, 4)] {
        let mut state = session(120, height, Vec::new());
        state.settings.prompt_image_preview = true;
        attach(&mut state, &[&data]);
        let (buffer, _) = draw(&state);
        let (found, most) = picture_rows(&buffer);
        assert_eq!(found.len(), rows, "height {height}");
        // Square in pixels: twice as many columns as rows.
        assert_eq!(most, 2 * rows, "height {height}");
    }
}

fn tool_with_image(data: &str) -> Item {
    Item::Assistant(Assistant {
        id: "msg_a".into(),
        agent: "build".into(),
        model: "Model".into(),
        parts: vec![Part::Tool(Tool {
            call_id: "call_x".into(),
            name: "screenshot".into(),
            input: serde_json::json!({"url": "https://example.com"}),
            status: Status::Completed,
            output: String::new(),
            metadata: Default::default(),
            error: None,
            started: 0,
            awaiting: false,
            images: vec![Arc::new(Source {
                mime: "image/png".into(),
                data: data.into(),
                label: "shot.png".into(),
            })],
        })],
        ..Assistant::default()
    })
}

#[test]
fn tool_result_images_need_the_setting() {
    let data = png(40, 20);
    let mut state = session(120, 36, vec![tool_with_image(&data)]);
    let (buffer, _) = draw(&state);
    assert!(find(&buffer, "screenshot").is_some());
    assert!(picture_rows(&buffer).0.is_empty());

    state.settings.image_preview = true;
    let (buffer, drawn) = draw(&state);
    let (_, read) = find(&buffer, "screenshot").unwrap();
    let (rows, _) = picture_rows(&buffer);
    assert_eq!(rows.len(), 4);
    assert!(rows[0] > read);
    let x = (0..120)
        .find(|&x| buffer[(x, rows[0])].symbol() == "▀")
        .unwrap();
    assert_eq!(
        hit_at(&drawn.hits, (x, rows[0])),
        Some(&Hit::Image(Set::Tool("call_x".into()), 0))
    );
}

#[test]
fn user_message_images_sit_above_the_text() {
    let data = png(40, 20);
    let user = Item::User {
        id: "msg_u".into(),
        text: "what is in this picture".into(),
        agent: "build".into(),
        files: vec!["shot.png".into()],
        images: vec![Arc::new(Source {
            mime: "image/png".into(),
            data: data.clone(),
            label: "shot.png".into(),
        })],
        created: 0,
    };
    let mut state = session(120, 36, vec![user]);
    let (buffer, _) = draw(&state);
    assert!(picture_rows(&buffer).0.is_empty());
    state.settings.image_preview = true;
    let (buffer, drawn) = draw(&state);
    let (rows, _) = picture_rows(&buffer);
    let (_, text) = find(&buffer, "what is in this picture").unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|&y| y < text));
    let x = (0..120)
        .find(|&x| buffer[(x, rows[0])].symbol() == "▀")
        .unwrap();
    assert_eq!(
        hit_at(&drawn.hits, (x, rows[0])),
        Some(&Hit::Image(Set::User(0), 0))
    );
}

#[test]
fn viewer_shows_the_picture_fitted_to_the_dialog() {
    let data = png(40, 20);
    let mut state = session(120, 36, Vec::new());
    attach(&mut state, &[&data, CORRUPT]);
    let body = viewer(&state, &Set::Composer, 0).expect("viewer");
    state.dialogs.push(Dialog::Body(body));
    let (buffer, drawn) = draw(&state);
    assert!(find(&buffer, "Image 1 of 2").is_some());
    assert!(find(&buffer, "[Image 1]").is_some());
    let (rows, most) = picture_rows(&buffer);
    // 2:1 picture in 110 columns × 20 rows (36 - 9 above - 7 for the panel): the height
    // bounds it, 40 pixel rows by 80 columns.
    assert_eq!(rows.len(), 20);
    assert_eq!(most, 80);
    let (px, py) = find(&buffer, "← previous").expect("footer on screen");
    assert_eq!(
        hit_at(&drawn.hits, (px, py)),
        Some(&Hit::Key("left".into()))
    );
    let (nx, ny) = find(&buffer, "next →").expect("next on screen");
    assert_eq!(
        hit_at(&drawn.hits, (nx, ny)),
        Some(&Hit::Key("right".into()))
    );

    // The second image does not decode; paging wraps around.
    let body = viewer(&state, &Set::Composer, 3).expect("viewer");
    state.dialogs.clear();
    state.dialogs.push(Dialog::Body(body));
    let (buffer, _) = draw(&state);
    assert!(find(&buffer, "Image 2 of 2").is_some());
    assert!(find(&buffer, "No preview").is_some());
    assert!(picture_rows(&buffer).0.is_empty());
}

#[test]
fn viewer_pages_through_the_set_the_click_named() {
    let data = png(40, 20);
    let mut state = session(120, 36, vec![tool_with_image(&data)]);
    state.settings.image_preview = true;
    let (_, drawn) = draw(&state);
    let Some(Hit::Image(set, index)) = drawn
        .hits
        .iter()
        .map(|(_, h)| h)
        .find(|h| matches!(h, Hit::Image(..)))
        .cloned()
    else {
        panic!("no image tile");
    };
    let body = viewer(&state, &set, index).expect("viewer");
    state.dialogs.push(Dialog::Body(body));
    let (buffer, _) = draw(&state);
    assert!(find(&buffer, "Image 1 of 1").is_some());
    assert!(find(&buffer, "shot.png").is_some());
    assert!(!picture_rows(&buffer).0.is_empty());
    // With the dialog open the tile behind it is not on top.
    let (_, drawn) = draw(&state);
    let topmost = drawn.hits.last().map(|(_, h)| h.clone());
    assert!(!matches!(topmost, Some(Hit::Image(..))));
}
