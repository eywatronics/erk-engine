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
    // A node moved out of the document and back in one frame came back as
    // new: the journal missed what changed in it meanwhile (M5.9's longer
    // local run of the fuzzer).
    &[
        167, 145, 250, 170, 166, 237, 192, 49, 207, 14, 218, 103, 218, 188, 177, 250, 133, 16, 117,
        215, 120, 31, 170, 52, 181, 40, 219, 212, 243, 113, 197, 230, 96, 170, 213, 174, 230, 245,
        155, 80, 245, 129, 236, 128, 41, 89, 66, 229, 0, 119, 215, 188, 233, 101, 97, 65, 44, 10,
        62, 40, 145, 203, 183, 105, 154, 117, 54, 5, 52, 195, 183, 119, 191, 234, 131, 191, 21,
        193, 77, 224, 94, 129, 44, 133, 147, 92, 141, 199, 245, 129, 147, 29, 188, 236, 106, 186,
        223, 78, 114, 18, 90, 2, 110, 193, 65, 62, 153, 198, 248, 46, 199, 123, 92, 71, 229, 176,
        203, 23, 113, 186,
    ],
    // query_all with a detached scope panicked in the style system.
    &[
        237, 145, 7, 120, 186, 35, 112, 72, 199, 137, 74, 81, 230, 254, 171,
    ],
    // A node made in one frame and attached in the next was taken for an
    // old node whose attributes changed (M5.1's journal, found by the CI
    // fuzz job).
    &[
        54, 54, 0, 0, 115, 116, 33, 43, 136, 136, 136, 136, 136, 218, 3, 0, 203, 136, 136, 136,
        218, 54, 67, 204, 3,
    ],
    // A page loaded and grown long enough to scroll in the frame the
    // pointer moved over it: its scroll bar was left out, the bars taken
    // from the last frame's scroll containers (found by the CI fuzz job).
    &[
        249, 43, 10, 248, 247, 64, 167, 59, 10, 246, 241, 10, 10, 10, 43, 10, 10, 10, 63, 182,
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
