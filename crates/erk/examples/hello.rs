//! Erk from Rust: an app without a window opens a page, finds its button,
//! subscribes to clicks on it and clicks it; the callback changes the page.
//! The same steps as `examples/c/hello.c`, and CI runs both (M3's
//! acceptance). With `--window` it opens the page in a window instead, and
//! the button counts real clicks.
//!
//! ```text
//! cargo run -p erk --example hello
//! cargo run -p erk --example hello -- --window
//! ```

use std::cell::Cell;
use std::process::ExitCode;
use std::rc::Rc;

use erk::{App, Config, EventKind};

const PAGE: &str = r#"<body style="margin: 0; font-family: sans-serif">
  <button id="b" style="display: block; margin: 16px; padding: 8px 16px">Tıkla</button>
  <p id="label" style="margin: 16px">Henüz tıklanmadı.</p>"#;

fn main() -> ExitCode {
    let window = std::env::args().any(|arg| arg == "--window");
    let config = Config {
        width: 320,
        height: 200,
        title: "Erk — merhaba".to_owned(),
        // The same text on every machine, as a test wants.
        system_fonts: window,
        ..Config::default()
    };
    let made = if window {
        App::new(config)
    } else {
        App::headless(config)
    };
    let mut app = match made {
        Ok(app) => app,
        Err(status) => {
            eprintln!("cannot make the app: {status:?}");
            return ExitCode::FAILURE;
        }
    };
    app.load_html(PAGE);
    let (Ok(Some(button)), Ok(Some(label))) = (app.query(None, "#b"), app.query(None, "#label"))
    else {
        eprintln!("the page has no button or label");
        return ExitCode::FAILURE;
    };

    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    let subscribed = app.on(button, EventKind::Click, move |cx, _| {
        counted.set(counted.get() + 1);
        let text = format!("{} kez tıklandı.", counted.get());
        if let Err(status) = cx.set_text(label, &text) {
            eprintln!("cannot change the label: {status:?}");
        }
    });
    if let Err(status) = subscribed {
        eprintln!("cannot subscribe: {status:?}");
        return ExitCode::FAILURE;
    }

    if window {
        return match app.run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }

    // Without a window: paint once, so input has a frame to hit, then click
    // the middle of the button's box.
    app.tick(0);
    let Ok(found) = app.node_box(button) else {
        eprintln!("the button has no box");
        return ExitCode::FAILURE;
    };
    app.click(found.x + found.width / 2.0, found.y + found.height / 2.0);
    app.tick(1);
    match (clicks.get(), app.text(label)) {
        (1, Ok(text)) if text == "1 kez tıklandı." => {
            println!("ok: {text}");
            ExitCode::SUCCESS
        }
        (count, text) => {
            eprintln!("expected one click and its label, got {count} and {text:?}");
            ExitCode::FAILURE
        }
    }
}
