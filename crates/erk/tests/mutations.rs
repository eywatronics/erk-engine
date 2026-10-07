//! Scripts of host calls never make the engine panic (M4.2): random
//! changes on any node the script has seen, stale ones included, between
//! frames, inputs and callbacks that change the document under the event.
//!
//! Scripts come from a fixed-seed generator, so every run and every
//! machine sees the same ones. Open-ended search is the fuzz target's job
//! (`fuzz/fuzz_targets/mutations.rs`, which runs the same interpreter); a
//! script it finds goes into `FOUND` and stays there.

mod script;

use erk::{App, Config};

/// xorshift64*, as the renderer's robustness test.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

/// Scripts that once broke the engine.
const FOUND: &[&[u8]] = &[
    // query_all with a detached scope panicked in the style system.
    &[
        237, 145, 7, 120, 186, 35, 112, 72, 199, 137, 74, 81, 230, 254, 171,
    ],
];

fn app() -> App {
    let mut app = App::headless(Config {
        width: 64,
        height: 48,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.load_html("<ul id=a><li class=a>bir<li>iki</ul><p tabindex=0>p</p>");
    app.tick(0);
    app
}

#[test]
fn random_scripts_of_host_calls_never_panic() {
    let mut rng = Rng(0x00e4_4b1e_5eed_0042);
    for _ in 0..200 {
        let script: Vec<u8> = (0..160).map(|_| rng.next() as u8).collect();
        script::run(&mut app(), &script);
    }
}

#[test]
fn scripts_that_once_broke_the_engine_never_panic() {
    for script in FOUND {
        script::run(&mut app(), script);
    }
}
