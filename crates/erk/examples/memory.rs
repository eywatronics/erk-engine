//! B16 (M5.13): where Erk's memory goes. Each page is opened in a
//! windowless app (CPU raster), drawn, scrolled 30 notches and left idle;
//! the live and peak heap is read after each step, and what is live at
//! the end is split by where it was allocated: the DOM, Stylo, Taffy,
//! Parley and the fonts, Erk's own layout, text and display list, the
//! raster.
//!
//! ```text
//! $env:CARGO_PROFILE_RELEASE_DEBUG = "line-tables-only"
//! cargo run --release -p erk --example memory              # both pages
//! cargo run --release -p erk --example memory -- cards     # one of them
//! ```
//!
//! The split needs the frames' names: build with line tables, as above.
//! The heap is what the program asked for (dhat counts each allocation as
//! requested, not as the allocator rounds it), so it is below the process's
//! private bytes: those add the allocator's own overhead, the stacks, the
//! code and, on the GPU path, the driver. dhat writes each page's profile to
//! `target/dhat-<page>.json`, for its viewer.
//!
//! An example, not engine code: it reads files, which the core never does.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use erk::{App, Config, Input};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

/// The pages of the 2026-10-12 measurement, and how many elements each
/// has.
const PAGES: &[(&str, &str)] = &[("small", "m2-demo.html"), ("cards", "erk-10000.html")];

/// What a frame's function name says allocated: the first frame from the
/// allocation outwards that belongs to one of these, in this order of
/// prefixes. Containers, the standard library and the profiler belong to
/// none: an allocation is the crate's that asked for it.
const PARTS: &[(&str, &[&str])] = &[
    ("DOM (erk-dom)", &["erk_dom::"]),
    (
        "HTML parser",
        &["html5ever::", "markup5ever::", "tendril::"],
    ),
    ("change journal", &["erk_invalidation::"]),
    ("styles (erk-style)", &["erk_style::"]),
    (
        "styles (Stylo)",
        &["style::", "selectors::", "stylo", "style_traits::"],
    ),
    ("layout tree (Taffy)", &["taffy::"]),
    (
        "text (Parley, fonts)",
        &[
            "parley::",
            "swash::",
            "skrifa::",
            "read_fonts::",
            "harfrust::",
            "fontique::",
            "icu_",
        ],
    ),
    ("text (Erk)", &["erk_renderer::text"]),
    ("layout (Erk)", &["erk_renderer::layout"]),
    (
        "display list",
        &[
            "erk_renderer::display",
            "erk_renderer::list",
            "erk_renderer::damage",
        ],
    ),
    (
        "raster",
        &[
            "erk_renderer::raster",
            "erk_renderer::tables",
            "vello_cpu::",
            "vello_common::",
            "vello_hybrid::",
            "wgpu",
        ],
    ),
    ("engine (other)", &["erk_renderer::"]),
    ("erk", &["erk::"]),
];

/// Frames that say nothing about who wanted the memory.
const GENERIC: &[&str] = &[
    "alloc::",
    "core::",
    "std::",
    "hashbrown::",
    "smallvec::",
    "servo_arc::",
    "dhat::",
    "string_cache::",
    "indexmap::",
    "arrayvec::",
    "bitflags::",
    "rustc_hash::",
];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn mb(bytes: usize) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// The heap as dhat counts it now: live bytes and blocks, and the peak.
fn heap() -> dhat::HeapStats {
    dhat::HeapStats::get()
}

fn row(step: &str, stats: &dhat::HeapStats) {
    println!(
        "| {step} | {} | {} | {} |",
        mb(stats.curr_bytes),
        stats.curr_blocks,
        mb(stats.max_bytes)
    );
}

/// The part a program point's frames belong to.
fn part(frames: &[&str]) -> &'static str {
    for frame in frames {
        // "0x…: name (file:line:col)", the name maybe "<T as Trait>::…".
        let name = frame.split_once(": ").map_or(*frame, |(_, name)| name);
        let name = name.trim_start_matches('<');
        if GENERIC.iter().any(|generic| name.starts_with(generic)) {
            continue;
        }
        if let Some((part, _)) = PARTS
            .iter()
            .find(|(_, prefixes)| prefixes.iter().any(|prefix| name.starts_with(prefix)))
        {
            return part;
        }
    }
    "other"
}

/// What was live when the profile was written (`eb`) and at the peak
/// (`gb`), by part.
fn split(profile: &Path) -> (BTreeMap<&'static str, u64>, BTreeMap<&'static str, u64>) {
    let text = std::fs::read_to_string(profile).expect("dhat wrote its profile");
    let json: serde_json::Value = serde_json::from_str(&text).expect("dhat's JSON");
    let table: Vec<&str> = json["ftbl"]
        .as_array()
        .expect("a frame table")
        .iter()
        .map(|frame| frame.as_str().unwrap_or(""))
        .collect();
    let mut live = BTreeMap::new();
    let mut peak = BTreeMap::new();
    for point in json["pps"].as_array().expect("program points") {
        let frames: Vec<&str> = point["fs"]
            .as_array()
            .expect("frames")
            .iter()
            .filter_map(|at| table.get(at.as_u64()? as usize).copied())
            .collect();
        let part = part(&frames);
        *live.entry(part).or_insert(0) += point["eb"].as_u64().unwrap_or(0);
        *peak.entry(part).or_insert(0) += point["gb"].as_u64().unwrap_or(0);
    }
    (live, peak)
}

fn measure(name: &str, file: &str) {
    let html = std::fs::read_to_string(repo().join("examples").join(file)).expect("the page");
    let profile = repo().join(format!("target/dhat-{name}.json"));
    let profiler = dhat::Profiler::builder()
        .file_name(&profile)
        .trim_backtraces(Some(24))
        .build();
    println!("\n### {name}: {file}\n");
    println!("| Step | Live heap | Live blocks | Peak so far |");
    println!("|---|---|---|---|");
    let mut app = App::headless(Config {
        width: WIDTH,
        height: HEIGHT,
        system_fonts: false,
        ..Config::default()
    })
    .expect("a windowless app");
    row("app made", &heap());
    app.load_html(&html);
    row("page parsed", &heap());
    app.tick(0);
    row("first frame", &heap());
    let elements = app.query_all(None, "*").expect("a selector").len();
    for notch in 0..30u64 {
        app.input(Input::Wheel {
            dx: 0.0,
            dy: 100.0,
            x: 400.0,
            y: 300.0,
        });
        app.tick((notch + 1) * 16_000_000);
    }
    row("scrolled 30 notches", &heap());
    for turn in 0..10u64 {
        app.tick(1_000_000_000 + turn * 16_000_000);
    }
    let idle = heap();
    row("idle", &idle);
    println!(
        "\n{elements} elements; live heap per element {:.1} KB.",
        idle.curr_bytes as f64 / elements as f64 / 1024.0
    );
    // The profile is written now, with the app still open: what it holds
    // is what is live.
    drop(profiler);
    let (live, peak) = split(&profile);
    let total: u64 = live.values().sum();
    let mut parts: Vec<_> = live.iter().collect();
    parts.sort_by(|a, b| b.1.cmp(a.1));
    println!("\n| Part | Live at the end | Share | At the peak |");
    println!("|---|---|---|---|");
    for (part, bytes) in parts {
        println!(
            "| {part} | {} | {:.0}% | {} |",
            mb(*bytes as usize),
            *bytes as f64 * 100.0 / total.max(1) as f64,
            mb(peak.get(part).copied().unwrap_or(0) as usize)
        );
    }
    drop(app);
}

fn main() {
    let wanted: Vec<String> = std::env::args().skip(1).collect();
    println!("B16: {WIDTH}x{HEIGHT}, windowless, CPU raster, embedded fonts");
    for (name, file) in PAGES {
        if wanted.is_empty() || wanted.iter().any(|w| w == name) {
            measure(name, file);
        }
    }
}
