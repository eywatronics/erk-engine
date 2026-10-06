//! The counter demo's host (M2.4): the first host logic, written against
//! the `erk` API as any host writes it.
//!
//! When a page has a `#count` element and an `#increment` (and perhaps a
//! `#decrement`) button, the host keeps the number: a click on a button
//! changes it, and the host sets `#count`'s text. Erk only shows it. Pages
//! without them are left alone.

use std::cell::Cell;
use std::rc::Rc;

use erk::{App, EventKind};

/// Run the counter on `app`'s page, if it has one.
pub(crate) fn install(app: &mut App) {
    let find = |app: &App, id: &str| app.query(None, &format!("#{id}")).ok().flatten();
    let (Some(count), Some(increment)) = (find(app, "count"), find(app, "increment")) else {
        return;
    };
    let value = Rc::new(Cell::new(0_i64));
    let buttons = [(Some(increment), 1), (find(app, "decrement"), -1)];
    for (button, step) in buttons {
        let Some(button) = button else {
            continue;
        };
        let value = value.clone();
        // A click anywhere inside the button bubbles up to it.
        let _ = app.on(button, EventKind::Click, move |cx, _| {
            value.set(value.get() + step);
            let _ = cx.set_text(count, &value.get().to_string());
        });
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use erk::{Config, Input, Modifiers, PointerButton, PointerInput, PointerKind};

    use super::*;

    const PAGE: &str = include_str!("../../../examples/counter.html");

    fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("valid PNG");
        let mut pixels = vec![0; reader.output_buffer_size().expect("sized PNG")];
        let info = reader.next_frame(&mut pixels).expect("PNG frame");
        pixels.truncate(info.buffer_size());
        (info.width, info.height, pixels)
    }

    /// The middle of the element `selector` finds, where the last frame put
    /// it.
    fn middle(app: &App, selector: &str) -> (f32, f32) {
        let node = app.query(None, selector).unwrap().unwrap();
        let found = app.node_box(node).unwrap();
        (found.x + found.width / 2.0, found.y + found.height / 2.0)
    }

    fn counter() -> App {
        let mut app = App::headless(Config {
            system_fonts: false,
            ..Config::default()
        })
        .unwrap();
        app.load_html(PAGE);
        install(&mut app);
        // Input is hit-tested against the last frame painted.
        app.tick(0);
        app
    }

    /// The acceptance item: clicks reach the host, the host counts, and the
    /// frame showing the new number is the golden image.
    #[test]
    fn clicks_count_and_the_page_shows_the_number() {
        let mut app = counter();
        let (increment, decrement) = (middle(&app, "#increment"), middle(&app, "#decrement"));
        // Up three times, down once: 2.
        for (turn, (x, y)) in [increment, increment, increment, decrement]
            .into_iter()
            .enumerate()
        {
            app.click(x, y);
            app.tick(turn as u64 + 1);
        }
        let count = app.query(None, "#count").unwrap().unwrap();
        assert_eq!(app.text(count).unwrap(), "2");
        // Leave the pointer's state out of the image: move it away.
        app.input(Input::Pointer(PointerInput {
            kind: PointerKind::Leave,
            x: 0.0,
            y: 0.0,
            button: PointerButton::None,
            modifiers: Modifiers::default(),
        }));
        app.tick(10);
        let frame = app.frame().expect("a frame");
        assert!(
            frame.display_list().contains("\"2\""),
            "{}",
            frame.display_list()
        );

        let actual = frame.to_png().unwrap();
        let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/counter-2.png");
        if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
            std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
            std::fs::write(&golden, &actual).unwrap();
        } else {
            let expected = std::fs::read(&golden).unwrap_or_else(|_| {
                panic!(
                    "no golden image at {}; run with ERK_BLESS=1",
                    golden.display()
                )
            });
            if decode(&expected) != decode(&actual) {
                let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-actual");
                std::fs::create_dir_all(&out).unwrap();
                std::fs::write(out.join("counter-2.png"), &actual).unwrap();
                panic!("the counter differs from {}", golden.display());
            }
        }
    }

    #[test]
    fn a_click_beside_the_buttons_counts_nothing() {
        let mut app = counter();
        let (x, y) = middle(&app, "#increment");
        app.click(x + 200.0, y);
        let count = app.query(None, "#count").unwrap().unwrap();
        assert_eq!(app.text(count).unwrap(), "0");
    }

    #[test]
    fn a_page_without_the_counter_is_left_alone() {
        let mut app = App::headless(Config {
            system_fonts: false,
            ..Config::default()
        })
        .unwrap();
        app.load_html(r#"<button id="increment">+</button>"#);
        install(&mut app);
        app.tick(0);
        app.click(10.0, 10.0);
        let button = app.query(None, "#increment").unwrap().unwrap();
        assert_eq!(app.text(button).unwrap(), "+");
    }
}
