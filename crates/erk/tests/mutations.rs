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
    // An option added to or removed from a list selected or unselected
    // another, which the journal did not note: which option a list
    // shows follows from all of them (M5.10's longer local run).
    &[
        164, 113, 162, 17, 11, 249, 207, 112, 19, 231, 203, 6, 59, 143, 201, 231, 155, 246, 119,
        47, 192, 87, 215, 94, 2, 148, 205, 5, 255, 35, 184, 200, 129, 206, 241, 225, 148, 79, 145,
        2, 230, 76, 219, 75, 235, 214, 136, 14, 50, 60, 33, 238, 149, 84, 6, 94, 110, 32, 225, 149,
        180, 169, 59, 218, 83, 104, 29, 51, 19, 70, 100, 29, 186, 140, 72, 4, 33, 77, 189, 165,
        118, 85, 144, 22, 110, 194, 253, 146, 62, 247, 124, 179, 206, 128, 240, 221, 213, 235, 192,
        247, 38, 228, 165, 63, 132, 115, 33, 250, 46, 42, 125, 9, 57, 109, 202, 155, 175, 220, 103,
        135, 108, 43, 88, 45, 206, 222, 34, 153, 148, 26, 19, 189, 159, 67, 54, 188, 225, 245, 144,
        185, 225, 42, 120, 205, 15, 178, 145, 161, 20, 185, 17, 160, 73, 133, 151, 102, 104, 69,
        237, 168, 197, 88, 96, 243, 146, 7, 48, 234, 102, 13, 229, 2, 119, 24, 151, 71, 158, 107,
        200, 252, 68, 139, 136, 70, 86, 24, 97, 166, 116, 141, 174, 77, 240, 195, 31, 57, 159, 110,
        220, 3,
    ],
    // A radio button the host put in another kept the box it had
    // elsewhere: a checkbox is a leaf Taffy never enters, so what it held
    // was built but never laid out again (M5.10's longer local run).
    &[
        185, 187, 85, 34, 111, 123, 23, 87, 161, 205, 254, 236, 128, 43, 202, 231, 136, 109, 109,
        181, 188, 222, 175, 222, 120, 41, 72, 176, 99, 143, 136, 141, 229, 199, 115, 101, 98, 209,
        122, 216, 43, 137, 59, 170, 161, 149, 114, 201, 220, 101, 34, 234, 212, 31, 43, 128, 5,
        120, 203, 211, 140, 37, 77, 163, 7, 228, 11, 214, 254, 39, 203, 101, 83, 228, 207, 91, 64,
        235, 88, 43, 20, 140, 50, 123, 46, 46, 113, 132, 219, 34, 114, 213, 39, 96, 91, 111, 174,
        224, 135, 208, 48, 43, 179, 144, 147, 104, 220, 141,
    ],
];

/// The page every script starts from.
const START: &str = "<ul id=a><li class=a>bir<li>iki</ul><p tabindex=0>p</p>";

/// The same page where everything transitions (M5.12), for long enough
/// to run through most of a script and end in some: a class, an
/// attribute, the pointer or the focus starts one, and the frames after it
/// are checked against the oracle while it runs.
const TRANSITIONS: &str = "<style>* { transition: all 120ms linear }\
    .a, .b, :hover, :focus, [data-x] { color: red; opacity: 0.5; transform: translateX(3px) }\
    </style><ul id=a><li class=a>bir<li>iki</ul><p tabindex=0>p</p>";

fn app() -> App {
    app_with(START)
}

fn app_with(html: &str) -> App {
    let mut app = App::headless(Config {
        width: 64,
        height: 48,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.load_html(html);
    app.tick(0);
    app
}

#[test]
fn random_scripts_of_host_calls_never_panic() {
    let mut rng = Rng(0x00e4_4b1e_5eed_0042);
    for run in 0..200 {
        let script: Vec<u8> = (0..160).map(|_| rng.next() as u8).collect();
        if run % 2 == 0 {
            script::run(&mut app(), &script);
            continue;
        }
        // Each script starts with every element's transition just begun:
        // a script rarely makes the change that starts one and then ticks
        // while it runs.
        let mut app = app_with(TRANSITIONS);
        for node in app.query_all(None, "*").unwrap() {
            app.add_class(node, "b").unwrap();
        }
        app.tick(0);
        assert!(app.next_tick().is_some(), "the transitions run");
        script::run(&mut app, &script);
    }
}

#[test]
fn scripts_that_once_broke_the_engine_never_panic() {
    for script in FOUND {
        script::run(&mut app(), script);
    }
}
