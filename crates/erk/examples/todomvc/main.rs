//! TodoMVC in a window, its host written in Rust on Erk's API (M4.5). The
//! page and the logic are in `todos.rs`; `tests/todomvc.rs` drives them
//! without a window.
//!
//! ```text
//! cargo run -p erk --example todomvc
//! ```

mod todos;

use std::process::ExitCode;

use erk::{App, Config};

fn main() -> ExitCode {
    let mut app = match App::new(Config {
        width: 520,
        height: 560,
        title: "Erk — TodoMVC".to_owned(),
        ..Config::default()
    }) {
        Ok(app) => app,
        Err(status) => {
            eprintln!("cannot make the app: {status:?}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(status) = todos::mount(&mut app) {
        eprintln!("cannot wire the page up: {status:?}");
        return ExitCode::FAILURE;
    }
    match app.run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
