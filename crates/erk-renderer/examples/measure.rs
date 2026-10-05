//! How long a frame of a page takes: the first call, which includes one-time
//! setup such as loading the embedded fonts, and the median of the calls
//! after it.
//!
//! ```text
//! cargo run --release -p erk-renderer --example measure -- examples/perf/nodes-1000.html [runs]
//! cargo run --release -p erk-renderer --example measure -- --frames examples/perf/long-page.html [runs]
//! ```
//!
//! By default each call is `render_html`: parse, style, lay out and paint.
//! With `--frames` the page goes to the renderer thread once and each frame
//! is a repaint of the document it keeps (M2): style, lay out and paint, no
//! parsing. That is the cost of every state change in M2, the baseline M5's
//! incremental work is measured against.
//!
//! An example, not engine code: it may read the file and the clock, which
//! the engine core never does.

use std::time::{Duration, Instant};

use erk_renderer::{FromRenderer, ToRenderer};

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let frames = args.first().is_some_and(|arg| arg == "--frames");
    if frames {
        args.remove(0);
    }
    let path = args
        .first()
        .expect("usage: measure [--frames] <page.html> [runs]")
        .clone();
    let runs: usize = args
        .get(1)
        .map_or(20, |n| n.parse().expect("runs must be a number"));
    let html = std::fs::read_to_string(&path).expect("the page is readable");

    let (first, mut rest) = if frames {
        time_frames(html, runs)
    } else {
        let time = || {
            let start = Instant::now();
            std::hint::black_box(erk_renderer::render_html(&html, WIDTH, HEIGHT));
            start.elapsed()
        };
        let first = time();
        (first, (0..runs).map(|_| time()).collect())
    };
    rest.sort();

    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let mode = if frames {
        "repaint of the kept document"
    } else {
        "render_html (parse included)"
    };
    println!("{path} at {WIDTH}x{HEIGHT}, {mode}");
    println!("  first call      {:8.2} ms", ms(first));
    if let (Some(min), Some(max)) = (rest.first(), rest.last()) {
        println!("  median of {runs:<4}  {:8.2} ms", ms(rest[rest.len() / 2]));
        println!("  min / max       {:8.2} / {:.2} ms", ms(*min), ms(*max));
    }
}

/// The first frame after loading `html` on the renderer thread, and `runs`
/// repaints after it: each `Resize` to the same size makes the thread paint
/// the document it keeps again.
fn time_frames(html: String, runs: usize) -> (Duration, Vec<Duration>) {
    let (to, from, renderer) = erk_renderer::spawn();
    let frame = |message: ToRenderer| {
        let start = Instant::now();
        to.send(message).expect("the renderer thread runs");
        loop {
            if let FromRenderer::Frame(frame) = from.recv().expect("the renderer thread runs") {
                std::hint::black_box(frame);
                return start.elapsed();
            }
        }
    };
    to.send(ToRenderer::Load { html })
        .expect("the renderer thread runs");
    let resize = || ToRenderer::Resize {
        width: WIDTH,
        height: HEIGHT,
    };
    let first = frame(resize());
    let rest = (0..runs).map(|_| frame(resize())).collect();
    to.send(ToRenderer::Shutdown)
        .expect("the renderer thread runs");
    renderer.join().expect("the renderer thread ends cleanly");
    (first, rest)
}
