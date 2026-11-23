//! Transitions drawn at fixed times on the host's clock (M5.12): golden
//! images of the standard timing functions and of `opacity`, `transform`
//! and `color` a quarter, half and three quarters of the way.
//!
//! Bless changed golden images with `ERK_BLESS=1` (exactly `1`) and commit
//! them with the change that caused them.

use std::path::PathBuf;

use erk::{App, Config};

const MS: u64 = 1_000_000;

/// Seven boxes going from blue to red, one per timing function; a box
/// fading; a box moving and turning; a paragraph changing colour. Each
/// takes a second once the body is `on`.
const PAGE: &str = "<!doctype html><style>\
    body { margin: 0; background: #ffffff; font-family: 'Noto Sans'; font-size: 14px }\
    .curves { display: flex; gap: 6px; padding: 8px }\
    .curves div { width: 40px; height: 40px; background-color: rgb(37, 99, 235);\
      transition: background-color 1s }\
    .on .curves div { background-color: rgb(220, 38, 38) }\
    #linear { transition-timing-function: linear }\
    #ease { transition-timing-function: ease }\
    #ease-in { transition-timing-function: ease-in }\
    #ease-out { transition-timing-function: ease-out }\
    #ease-in-out { transition-timing-function: ease-in-out }\
    #steps { transition-timing-function: steps(4, end) }\
    #bezier { transition-timing-function: cubic-bezier(0.1, 0.7, 0.3, 1) }\
    #fade { margin: 8px; width: 120px; height: 30px; background: #16a34a;\
      transition: opacity 1s linear }\
    .on #fade { opacity: 0.1 }\
    #turn { margin: 8px 8px 8px 40px; width: 60px; height: 30px; background: #9333ea;\
      transition: transform 1s ease-in-out }\
    .on #turn { transform: translateX(160px) rotate(90deg) }\
    #text { margin: 8px; color: rgb(0, 0, 0); transition: color 1s linear }\
    .on #text { color: rgb(220, 38, 38) }\
    </style><body><div class=curves><div id=linear></div><div id=ease></div>\
    <div id=ease-in></div><div id=ease-out></div><div id=ease-in-out></div>\
    <div id=steps></div><div id=bezier></div></div>\
    <div id=fade></div><div id=turn></div><p id=text>Geçişler saate göre</p>";

fn showing(html: &str) -> App {
    let mut app = App::headless(Config {
        width: 360,
        height: 240,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.load_html(html);
    app.set_verifying(true);
    app.tick(0);
    app
}

fn png(app: &App) -> Vec<u8> {
    app.frame()
        .expect("a headless frame")
        .to_png()
        .expect("pixels")
}

/// `app`'s frame against the golden image `name`.
fn check_golden(app: &App, name: &str) {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    let png = png(app);
    if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &png).unwrap();
        return;
    }
    let expected = std::fs::read(&golden).unwrap_or_else(|_| {
        panic!(
            "no golden image at {}; run with ERK_BLESS=1",
            golden.display()
        )
    });
    if expected != png {
        let actual = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/golden-diff/{name}.actual.png"));
        std::fs::create_dir_all(actual.parent().unwrap()).unwrap();
        std::fs::write(&actual, &png).unwrap();
        panic!(
            "{name} differs from its golden image; this frame is at {}",
            actual.display()
        );
    }
}

#[test]
fn transitions_are_drawn_where_the_clock_puts_them() {
    let mut app = showing(PAGE);
    let before = png(&app);
    let body = app.query(None, "body").unwrap().unwrap();
    // The change is drawn at 100 ms: the transitions start then, from the
    // old values.
    app.add_class(body, "on").unwrap();
    app.tick(100 * MS);
    app.verify_frame().unwrap();
    assert!(png(&app) == before, "a transition starts where it was");
    for (at, name) in [
        (350, "transitions-250ms"),
        (600, "transitions-500ms"),
        (850, "transitions-750ms"),
    ] {
        assert!(app.next_tick().is_some(), "{name}: still moving");
        app.tick(at * MS);
        app.verify_frame().unwrap();
        check_golden(&app, name);
    }
    // Ended: the page as drawn with the new values from the start.
    app.tick(1100 * MS);
    app.verify_frame().unwrap();
    assert_eq!(app.next_tick(), None, "nothing moves any more");
    let after = showing(&PAGE.replace("<body>", "<body class=on>"));
    assert!(png(&app) == png(&after), "the end is the new values");
}
