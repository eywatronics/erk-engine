//! Any bytes, read as a script of host calls, run without a panic (M4.2):
//! changes, batches, stale ids, frames, inputs and callbacks that change
//! the document under the event. The interpreter is the one the fixed-seed
//! test runs (`crates/erk/tests/script/`); a finding goes into that test's
//! `FOUND` list.
//!
//! Each input gets a new windowless app on this thread, as a host's UI
//! thread; the engine prepares frames on its own long-lived thread, so no
//! thread is made per input.

#![no_main]

use erk::{App, Config};
use libfuzzer_sys::fuzz_target;

#[path = "../../crates/erk/tests/script/mod.rs"]
mod script;

fuzz_target!(|data: &[u8]| {
    let mut app = App::headless(Config {
        width: 64,
        height: 48,
        system_fonts: false,
        ..Config::default()
    })
    .expect("a windowless app needs nothing from the system");
    app.load_html("<ul id=a><li class=a>bir<li>iki</ul><p tabindex=0>p</p>");
    app.tick(0);
    script::run(&mut app, data);
});
