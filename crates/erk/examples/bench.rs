//! M5's measurements (p2-incremental §4, B1–B11) against the full
//! recompute every frame does today: the baseline M5's incremental steps
//! are measured against, and the numbers its acceptance is set from.
//!
//! ```text
//! cargo run --release -p erk --example bench            # every scenario
//! cargo run --release -p erk --example bench -- B1 B6   # some of them
//! ```
//!
//! Each scenario builds its page in a windowless app, warms it up, then
//! times `tick` after each change: the frame prepared and painted on the
//! CPU. It prints the median, the 95th percentile and the slowest frame,
//! and what the last frame did (elements styled, boxes laid out,
//! paragraphs shaped, display list items).
//!
//! An example, not engine code: it reads the clock, which the core never
//! does.

use std::time::{Duration, Instant};

use erk::{App, Config, FrameStats, FrameTimings, Mutation, Node, Ref};

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

fn app(html: &str) -> App {
    let mut app = App::headless(Config {
        width: WIDTH,
        height: HEIGHT,
        system_fonts: false,
        ..Config::default()
    })
    .expect("a windowless app");
    app.load_html(html);
    app.tick(0);
    app
}

fn node(app: &App, selector: &str) -> Node {
    app.query(None, selector)
        .expect("a valid selector")
        .unwrap_or_else(|| panic!("no {selector}"))
}

/// `rows` rows of a div and four spans of text: five elements a row.
fn rows(rows: usize) -> String {
    let mut html = String::from(
        "<style>body { margin: 0; font-family: 'Noto Sans'; font-size: 12px } \
         .row { display: flex; gap: 4px } .on { color: #dc2626 }</style><body>",
    );
    for i in 0..rows {
        html.push_str(&format!(
            "<div class=row><span id=s{i}>satır {i}</span><span>bir</span><span>iki</span><span>üç</span></div>"
        ));
    }
    html
}

struct Result {
    name: &'static str,
    what: &'static str,
    times: Vec<Duration>,
    stages: Vec<FrameTimings>,
    stats: FrameStats,
}

/// Time `runs` frames, each after `change` with the run's number.
fn measure(
    app: &mut App,
    runs: usize,
    name: &'static str,
    what: &'static str,
    mut change: impl FnMut(&mut App, usize),
) -> Result {
    // Warm up: caches and the frame thread.
    for run in 0..3 {
        change(app, run);
        app.tick(0);
    }
    let mut times = Vec::with_capacity(runs);
    let mut stages = Vec::with_capacity(runs);
    for run in 0..runs {
        change(app, run + 3);
        let start = Instant::now();
        app.tick(0);
        times.push(start.elapsed());
        stages.extend(app.last_frame_timings());
    }
    Result {
        name,
        what,
        times,
        stages,
        stats: app.frame_stats(),
    }
}

fn b1() -> Result {
    // 2000 rows: 10 000 elements; one text changes, as a keystroke would.
    let mut app = app(&rows(2000));
    let span = node(&app, "#s1000");
    measure(
        &mut app,
        30,
        "B1",
        "10 000 elements, one text changes",
        |app, run| {
            app.set_text(span, &format!("satır {run}")).unwrap();
        },
    )
}

fn b2() -> Result {
    let mut app = app(&rows(2000));
    let spans: Vec<Node> = (0..100)
        .map(|i| node(&app, &format!("#s{}", i * 20)))
        .collect();
    measure(
        &mut app,
        30,
        "B2",
        "100 texts change in one frame",
        |app, run| {
            for (i, span) in spans.iter().enumerate() {
                app.set_text(*span, &format!("{run}.{i}")).unwrap();
            }
        },
    )
}

fn b3() -> Result {
    // 1000 levels; ten above the leaf a box of fixed size with
    // `contain: size layout` (a boundary from M5.4 on).
    let mut html =
        String::from("<style>.on { color: #dc2626 } div { padding-left: 0 }</style><body>");
    for depth in 0..1000 {
        if depth == 990 {
            html.push_str("<div style='contain: size layout; width: 300px; height: 40px'>");
        } else {
            html.push_str("<div>");
        }
    }
    html.push_str("<span id=leaf>yaprak</span>");
    html.push_str(&"</div>".repeat(1000));
    let mut app = app(&html);
    let leaf = node(&app, "#leaf");
    measure(
        &mut app,
        30,
        "B3",
        "1000 levels, a class on the leaf",
        |app, run| {
            let _ = if run % 2 == 0 {
                app.add_class(leaf, "on")
            } else {
                app.remove_class(leaf, "on")
            };
        },
    )
}

fn b4() -> Result {
    // `.card:has(input:checked)` was the case, but Stylo 0.20 does not
    // parse `:has()` in Servo mode (M5.3): the card carries the class, and
    // what it holds follows it.
    let mut html = String::from(
        "<style>.card { padding: 4px; margin: 2px; background: #f1f5f9 } \
         .card.checked { background: #bbf7d0 } .checked span { font-weight: bold }</style><body>",
    );
    for i in 0..1000 {
        html.push_str(&format!(
            "<div class=card id=c{i}><span>kart {i}</span></div>"
        ));
    }
    let mut app = app(&html);
    let mark = node(&app, "#c500");
    measure(
        &mut app,
        30,
        "B4",
        "a class on one of 1000 cards, its content following",
        |app, run| {
            let _ = if run % 2 == 0 {
                app.add_class(mark, "checked")
            } else {
                app.remove_class(mark, "checked")
            };
        },
    )
}

fn b5() -> Result {
    let mut html = rows(2000);
    html.push_str(
        "<div style='contain: size layout; width: 200px; height: 40px'><span id=inside>içeride</span></div>",
    );
    let mut app = app(&html);
    let inside = node(&app, "#inside");
    measure(
        &mut app,
        30,
        "B5",
        "a change inside contain: size layout, 10 000 elements",
        |app, run| {
            app.set_text(inside, &format!("içeride {run}")).unwrap();
        },
    )
}

fn b6() -> Result {
    // A caret's blink on a 5000-element page: a 1 × 16 px box that shows
    // and hides.
    let mut html = rows(1000);
    html.push_str("<span id=caret style='display: inline-block; width: 1px; height: 16px; background: #111'></span>");
    let mut app = app(&html);
    let caret = node(&app, "#caret");
    measure(
        &mut app,
        30,
        "B6",
        "a caret blinks on 5000 elements",
        |app, run| {
            let visibility = if run % 2 == 0 { "hidden" } else { "visible" };
            app.set_attr(
            caret,
            "style",
            &format!("display: inline-block; width: 1px; height: 16px; background: #111; visibility: {visibility}"),
        )
        .unwrap();
        },
    )
}

fn b7() -> Result {
    // Dragging: a box follows the pointer every frame, at 120 Hz the
    // budget is 8.3 ms.
    let mut html = rows(1000);
    html.push_str("<div id=drag style='position: absolute; left: 0; top: 0; width: 40px; height: 40px; background: #2563eb'></div>");
    let mut app = app(&html);
    let drag = node(&app, "#drag");
    measure(
        &mut app,
        120,
        "B7",
        "a box dragged across 5000 elements (p99 below)",
        |app, run| {
            let (x, y) = (run * 3 % 700, run * 2 % 500);
            app.set_attr(
            drag,
            "style",
            &format!("position: absolute; left: {x}px; top: {y}px; width: 40px; height: 40px; background: #2563eb"),
        )
        .unwrap();
        },
    )
}

/// 500 paragraphs of 20 words: 10 000 words, as a long document.
fn words() -> String {
    let mut html = String::from(
        "<style>body { margin: 8px; font-family: 'Noto Sans'; font-size: 13px; width: 760px }</style><body>",
    );
    for i in 0..500 {
        html.push_str(&format!("<p id=p{i}>"));
        for w in 0..20 {
            html.push_str(&format!("kelime{} ", i * 20 + w));
        }
        html.push_str("</p>");
    }
    html
}

fn b12() -> Result {
    // A keystroke: one character more at the end of a paragraph in the
    // middle of a 10 000-word document (the host has no text field before
    // M5.8; this is what typing into one does to the document).
    let mut app = app(&words());
    let p = node(&app, "#p250");
    let start = app.text(p).unwrap();
    measure(
        &mut app,
        30,
        "B12",
        "one character typed into a paragraph of 10 000 words",
        |app, run| {
            let mut text = start.clone();
            text.extend(std::iter::repeat_n('a', run + 1));
            app.set_text(p, &text).unwrap();
        },
    )
}

fn b13() -> Result {
    // A change only paint needs: an inline colour, 10 000 elements.
    let mut app = app(&rows(2000));
    let span = node(&app, "#s1000");
    measure(
        &mut app,
        30,
        "B13",
        "an inline style colour changes, 10 000 elements",
        |app, run| {
            let color = if run % 2 == 0 { "#dc2626" } else { "#2563eb" };
            app.set_attr(span, "style", &format!("color: {color}"))
                .unwrap();
        },
    )
}

fn b2b() -> Result {
    // Typing: the same text a hundred times in a frame, one change left.
    let mut app = app(&rows(2000));
    let span = node(&app, "#s1000");
    measure(
        &mut app,
        30,
        "B2b",
        "one text set 100 times in one frame",
        |app, run| {
            for i in 0..100 {
                app.set_text(span, &format!("{run}.{i}")).unwrap();
            }
        },
    )
}

fn b10() -> Result {
    // 100 transactions in a frame; before M5.1 each is applied at once.
    let mut app = app(&rows(2000));
    let spans: Vec<Node> = (0..100)
        .map(|i| node(&app, &format!("#s{}", i * 20)))
        .collect();
    measure(
        &mut app,
        30,
        "B10",
        "100 batches of one change each, one frame",
        |app, run| {
            for (i, span) in spans.iter().enumerate() {
                app.apply(&[Mutation::SetText(Ref::Node(*span), format!("{run}:{i}"))])
                    .unwrap();
            }
        },
    )
}

/// A scenario's name and how to run it.
type Scenario = (&'static str, fn() -> Result);

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

fn main() {
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    let all: [Scenario; 11] = [
        ("B1", b1),
        ("B2", b2),
        ("B2b", b2b),
        ("B3", b3),
        ("B4", b4),
        ("B5", b5),
        ("B6", b6),
        ("B7", b7),
        ("B10", b10),
        ("B12", b12),
        ("B13", b13),
    ];
    println!("{WIDTH}x{HEIGHT}, incremental style, full layout and display list, CPU raster");
    println!(
        "| | Scenario | median | p95 | p99 | max | style | layout | display list | raster | styled | laid out | shaped | items | recorded | changes |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (name, run) in all {
        if !wanted.is_empty() && !wanted.iter().any(|w| w == name) {
            continue;
        }
        let mut result = run();
        result.times.sort();
        let t = &result.times;
        let s = result.stats;
        // The median of each stage on its own.
        let stage = |pick: fn(&FrameTimings) -> u64| {
            let mut ns: Vec<u64> = result.stages.iter().map(pick).collect();
            ns.sort_unstable();
            ns.get(ns.len() / 2).map_or(0.0, |ns| *ns as f64 / 1e6)
        };
        println!(
            "| {} | {} | {:.2} ms | {:.2} ms | {:.2} ms | {:.2} ms | {:.2} | {:.2} | {:.2} | {:.2} | {} | {} | {} | {} | {} | {} |",
            result.name,
            result.what,
            ms(percentile(t, 0.5)),
            ms(percentile(t, 0.95)),
            ms(percentile(t, 0.99)),
            ms(*t.last().expect("runs")),
            stage(|f| f.style_ns),
            stage(|f| f.layout_ns),
            stage(|f| f.display_list_ns),
            stage(|f| f.raster_ns),
            s.styled,
            s.laid_out,
            s.shaped,
            s.items,
            s.recorded,
            s.changes
        );
    }
    println!("B8 (accessibility) comes with M5.11, B9 (incremental against full) once there is an");
    println!("incremental path, B11 (tile sizes) with M5.6.");
}
