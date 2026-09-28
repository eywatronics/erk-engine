//! Erk Engine shell: opens a local HTML file in a window, or paints it to a
//! PNG without one.
//!
//! ```text
//! erk <file.html>
//! erk --screenshot <out.png> <file.html>
//! ```
//!
//! The shell reads the file; the renderer, on its own thread, only ever
//! receives the document's text. In the target architecture the renderer is
//! a sandboxed process with no disk access, and this split is already that
//! shape.

mod window;

use std::path::PathBuf;
use std::process::ExitCode;

use erk_renderer::{FromRenderer, ToRenderer};

/// Screenshot size, the same as the golden images.
const SCREENSHOT_WIDTH: u16 = 800;
const SCREENSHOT_HEIGHT: u16 = 600;

enum Command {
    Window { page: PathBuf },
    Screenshot { out: PathBuf, page: PathBuf },
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Command, String> {
    match (args.next(), args.next(), args.next(), args.next()) {
        (Some(flag), Some(out), Some(page), None) if flag == "--screenshot" => {
            Ok(Command::Screenshot {
                out: out.into(),
                page: page.into(),
            })
        }
        (Some(page), None, None, None) if !page.starts_with("--") => {
            Ok(Command::Window { page: page.into() })
        }
        _ => {
            Err("usage: erk <file.html>\n       erk --screenshot <out.png> <file.html>".to_owned())
        }
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
        Command::Window { page } => {
            read_page(&page).and_then(|html| window::run(&page, html).map_err(|e| e.to_string()))
        }
        Command::Screenshot { out, page } => {
            read_page(&page).and_then(|html| screenshot(html, &out))
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

fn read_page(page: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(page).map_err(|e| format!("cannot read {}: {e}", page.display()))
}

/// Paint `html` through the renderer thread, exactly as the window does, and
/// write the frame as a PNG.
fn screenshot(html: String, out: &PathBuf) -> Result<(), String> {
    let (to, from, handle) = erk_renderer::spawn();
    let send = |message| {
        to.send(message)
            .map_err(|_| "the renderer stopped".to_owned())
    };
    send(ToRenderer::Load { html })?;
    send(ToRenderer::Resize {
        width: SCREENSHOT_WIDTH,
        height: SCREENSHOT_HEIGHT,
    })?;
    let FromRenderer::Frame(frame) = from
        .recv()
        .map_err(|_| "the renderer stopped before painting".to_owned())?;
    send(ToRenderer::Shutdown)?;
    handle
        .join()
        .map_err(|_| "the renderer thread panicked".to_owned())?;

    let png = frame.to_png().ok_or("the frame has no pixels")?;
    std::fs::write(out, png).map_err(|e| format!("cannot write {}: {e}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn a_single_path_opens_a_window() {
        assert!(matches!(parse(&["sayfa.html"]), Ok(Command::Window { .. })));
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
