//! How long `render_html` takes for a page: the first call, which includes
//! one-time setup such as loading the embedded fonts, and the median of the
//! calls after it.
//!
//! ```text
//! cargo run --release -p erk-renderer --example measure -- examples/perf/nodes-1000.html [runs]
//! ```
//!
//! An example, not engine code: it may read the file and the clock, which
//! the engine core never does.

use std::time::{Duration, Instant};

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: measure <page.html> [runs]");
    let runs: usize = args
        .next()
        .map_or(20, |n| n.parse().expect("runs must be a number"));
    let html = std::fs::read_to_string(&path).expect("the page is readable");

    let time = || {
        let start = Instant::now();
        std::hint::black_box(erk_renderer::render_html(&html, WIDTH, HEIGHT));
        start.elapsed()
    };
    let first = time();
    let mut rest: Vec<Duration> = (0..runs).map(|_| time()).collect();
    rest.sort();

    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!("{path} at {WIDTH}x{HEIGHT}");
    println!("  first call      {:8.2} ms", ms(first));
    if let (Some(min), Some(max)) = (rest.first(), rest.last()) {
        println!("  median of {runs:<4}  {:8.2} ms", ms(rest[rest.len() / 2]));
        println!("  min / max       {:8.2} / {:.2} ms", ms(*min), ms(*max));
    }
}
