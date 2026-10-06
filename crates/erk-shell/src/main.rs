//! Erk Engine shell: opens a local HTML file in a window, or paints it to a
//! PNG without one.
//!
//! ```text
//! erk [--cpu] <file.html>
//! erk --screenshot <out.png> <file.html>
//! ```
//!
//! The shell is the first host of the `erk` crate: it reads the file and
//! serves the page's resources, and runs the counter demo's logic. The
//! engine does no I/O of its own; resources, time and configuration come
//! from the host.

mod counter;
mod resources;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use erk::{App, Config, LogLevel};

use crate::resources::Provider;

/// Screenshot size, the same as the golden images.
const SCREENSHOT_WIDTH: u32 = 800;
const SCREENSHOT_HEIGHT: u32 = 600;

/// How many turns a screenshot waits for resources that arrive later.
const SCREENSHOT_TURNS: u64 = 8;

enum Command {
    /// `gpu`: draw on the GPU when the machine can (`--cpu` turns it off).
    Window {
        page: PathBuf,
        gpu: bool,
    },
    Screenshot {
        out: PathBuf,
        page: PathBuf,
    },
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    match (args.next(), args.next(), args.next(), args.next()) {
        (Some(flag), Some(out), Some(page), None) if flag == "--screenshot" => {
            Ok(Command::Screenshot {
                out: out.into(),
                page: page.into(),
            })
        }
        (Some(page), None, None, None) if !page.starts_with("--") => Ok(Command::Window {
            page: page.into(),
            gpu: true,
        }),
        (Some(flag), Some(page), None, None) if flag == "--cpu" && !page.starts_with("--") => {
            Ok(Command::Window {
                page: page.into(),
                gpu: false,
            })
        }
        _ => Err(
            "usage: erk [--cpu] <file.html>\n       erk --screenshot <out.png> <file.html>"
                .to_owned(),
        ),
    }
}

fn main() -> ExitCode {
    let command = match parse_args(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::Window { page, gpu } => {
            read_page(&page).and_then(|html| window(&page, &html, gpu))
        }
        Command::Screenshot { out, page } => {
            read_page(&page).and_then(|html| screenshot(&page, &html, &out))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("erk: {message}");
            ExitCode::FAILURE
        }
    }
}

fn read_page(page: &Path) -> Result<String, String> {
    std::fs::read_to_string(page).map_err(|e| format!("cannot read {}: {e}", page.display()))
}

/// The app's messages, on standard error: how the window draws, refused
/// resources.
fn log(_: LogLevel, message: &str) {
    eprintln!("erk: {message}");
}

/// Open `page` (already read as `html`) in a window and run until it closes.
fn window(page: &Path, html: &str, gpu: bool) -> Result<(), String> {
    let title = format!(
        "Erk — {}",
        page.file_name().map_or_else(
            || page.display().to_string(),
            |name| name.to_string_lossy().into_owned()
        )
    );
    let mut app = App::new(Config {
        title,
        gpu,
        log_level: LogLevel::Info,
        ..Config::default()
    })
    .map_err(|status| format!("cannot make the app: {status:?}"))?;
    open(&mut app, page, html);
    app.run().map_err(|error| error.to_string())
}

/// Paint `html` without a window, as the window would, and write the frame
/// as a PNG once every resource request has been answered.
fn screenshot(page: &Path, html: &str, out: &Path) -> Result<(), String> {
    let mut app = App::headless(Config {
        width: SCREENSHOT_WIDTH,
        height: SCREENSHOT_HEIGHT,
        ..Config::default()
    })
    .map_err(|status| format!("cannot make the app: {status:?}"))?;
    open(&mut app, page, html);
    for turn in 0..SCREENSHOT_TURNS {
        app.tick(turn);
        if app.frame().is_some_and(|frame| !frame.resources_pending()) {
            break;
        }
    }
    let frame = app.frame().ok_or("nothing was painted")?;
    let png = frame.to_png().ok_or("the frame has no pixels")?;
    std::fs::write(out, png).map_err(|e| format!("cannot write {}: {e}", out.display()))
}

/// Load `html`, with the page's directory as its resources and the counter
/// demo's host on the pages that have one.
fn open(app: &mut App, page: &Path, html: &str) {
    app.set_log(log);
    let provider = Provider::for_page(page);
    app.set_resource_provider(move |request, responder| provider.answer(request, responder));
    app.load_html(html);
    counter::install(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn a_single_path_opens_a_window() {
        assert!(matches!(
            parse(&["sayfa.html"]),
            Ok(Command::Window { gpu: true, .. })
        ));
        assert!(matches!(
            parse(&["--cpu", "sayfa.html"]),
            Ok(Command::Window { gpu: false, .. })
        ));
        assert!(parse(&["--cpu"]).is_err());
        assert!(parse(&["--gpu", "sayfa.html"]).is_err());
    }

    #[test]
    fn screenshot_takes_an_output_and_a_page() {
        let Ok(Command::Screenshot { out, page }) = parse(&["--screenshot", "o.png", "p.html"])
        else {
            panic!("expected a screenshot command");
        };
        assert_eq!(
            (out, page),
            (PathBuf::from("o.png"), PathBuf::from("p.html"))
        );
    }

    #[test]
    fn anything_else_is_a_usage_error() {
        for args in [
            &[][..],
            &["--screenshot"],
            &["--screenshot", "o.png"],
            &["a", "b"],
            &["--help"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }
}
