//! Malformed input never makes the renderer panic or crash. An embedded UI
//! engine renders whatever its host hands it; a broken page must render as
//! well as it can, never take the application down.
//!
//! Generated pages come from a small fixed-seed generator, so every run and
//! every machine sees the same inputs. A page that once broke the renderer
//! goes into `tests/robustness/` and stays there. Deeper, open-ended search
//! is the fuzz target's job (`fuzz/`).

use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use erk_renderer::{FromRenderer, ToRenderer, render_html, spawn};

const WIDTH: u16 = 320;
const HEIGHT: u16 = 240;

/// xorshift64*: tiny, deterministic, good enough to vary test inputs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const TAGS: &[&str] = &[
    "div", "p", "span", "b", "h1", "h2", "section", "ul", "li", "table", "td", "tr", "a", "em",
    "pre", "button", "input", "img", "br", "template", "svg", "math", "select", "option",
];

const DECLARATIONS: &[&str] = &[
    "width: -50px",
    "width: 1e30px",
    "height: 1e30px",
    "width: calc(100% - 1e9px)",
    "margin: -1000000px",
    "padding: 50%",
    "padding: -10px",
    "font-size: 0",
    "font-size: 100000px",
    "font-size: -4px",
    "line-height: -3",
    "line-height: 0",
    "line-height: 1e20",
    "display: flex; flex-wrap: wrap",
    "display: grid; grid-template-columns: repeat(1000, 1fr)",
    "display: none",
    "display: inline",
    "display: inline-block",
    "position: absolute; inset: 0",
    "position: fixed; z-index: -5; bottom: -1e9px",
    "position: relative; top: 1e9px; z-index: 2147483647",
    "position: absolute; width: 50%; right: -50%",
    "position: sticky; top: 3px",
    "vertical-align: super",
    "vertical-align: 1e9px",
    "vertical-align: -5000%; line-height: 1e9",
    "vertical-align: top; display: inline-block; height: 1e9px",
    "vertical-align: middle text-bottom",
    "display: inline-flex; padding: 50%",
    "display: inline-block; overflow: hidden; height: 1e9px",
    "padding: 0 1e9px; background: #abc",
    "margin: 0 -1e9px; background: red",
    "border: 7px solid; padding: 3px 9px; background: #fde68a",
    "display: contents",
    "position: absolute; left: 1e10px; top: -1e10px",
    "float: left",
    "color: #zzz",
    "color: rgb(300, -5, 1e9)",
    "background: red",
    "visibility: hidden",
    "text-align: justify",
    "text-align: center",
    "white-space: pre",
    "letter-spacing: 1e6px",
    "border: 1e9px solid red",
    "border: 3px solid",
    "--x: var(--x)",
    "width: var(--missing)",
    "}",
    "{{{",
    ";;;",
];

const TEXT: &[&str] = &[
    "merhaba",
    "İstanbul ılık şişe göç üzüm ağaç",
    " ",
    "\n\t  \n",
    "&amp;&lt;&#0;&#xD800;&#x110000;&bogus;",
    "e\u{301}\u{301}\u{301}\u{301}\u{301}",
    "\u{200b}\u{200d}\u{feff}",
    "👩‍👩‍👧‍👦🇹🇷",
    "مرحبا עולם",
    "\u{1}\u{7}\u{1b}\u{7f}",
    "uzunbirkelimeboşluksuzvekırılmasıimkânsızgibigörünenbirdizi",
    "<",
    ">",
    "</",
];

/// A page of random elements, styles and text: unclosed and stray tags,
/// out-of-range and malformed CSS, and awkward text.
fn page(rng: &mut Rng) -> String {
    let mut html = String::from("<!DOCTYPE html><html><head><style>");
    for _ in 0..rng.below(6) {
        let tag = rng.pick(TAGS);
        let declaration = rng.pick(DECLARATIONS);
        html.push_str(&format!("{tag} {{ {declaration} }} "));
    }
    html.push_str("</style></head><body>");
    let mut open = Vec::new();
    for _ in 0..rng.below(60) {
        match rng.below(6) {
            0 | 1 => {
                let tag = rng.pick(TAGS);
                let declaration = rng.pick(DECLARATIONS);
                html.push_str(&format!("<{tag} style=\"{declaration}\">"));
                open.push(tag);
            }
            2 => {
                // A close tag, matching or not.
                let tag = if rng.below(2) == 0 {
                    open.pop().unwrap_or("div")
                } else {
                    rng.pick(TAGS)
                };
                html.push_str(&format!("</{tag}>"));
            }
            3 => html.push_str(&format!("<{} ", rng.pick(TAGS))),
            _ => html.push_str(rng.pick(TEXT)),
        }
    }
    // Leave the rest open: the parser closes what it can.
    html
}

fn failures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/robustness-failures")
}

/// The renderer thread's stack (erk-renderer, thread.rs): deep documents
/// need it, and a test thread's default would overflow first.
const RENDERER_STACK: usize = 16 * 1024 * 1024;

/// Render `html` on a thread with the renderer's stack and report a panic as
/// an error, keeping the page so it can be added to the corpus.
fn renders(name: &str, html: &str) -> Result<(), String> {
    let page = html.to_owned();
    let result = std::thread::Builder::new()
        .stack_size(RENDERER_STACK)
        .spawn(move || {
            render_html(&page, WIDTH, HEIGHT);
        })
        .expect("a render thread starts")
        .join();
    match result {
        Ok(()) => Ok(()),
        Err(_) => {
            let dir = failures_dir();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(format!("{name}.html"));
            std::fs::write(&path, html).unwrap();
            Err(format!(
                "{name} panicked; the page is in {}",
                path.display()
            ))
        }
    }
}

#[test]
fn generated_malformed_pages_render_without_panicking() {
    let mut rng = Rng(0x0e2c_2026_0930_0001);
    let failures: Vec<String> = (0..300)
        .filter_map(|n| renders(&format!("generated-{n}"), &page(&mut rng)).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn the_crash_corpus_renders_without_panicking() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/robustness");
    let mut failures = Vec::new();
    let mut pages = 0;
    for entry in std::fs::read_dir(&dir).expect("tests/robustness exists") {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let html = std::fs::read_to_string(&path).expect("corpus pages are UTF-8");
        pages += 1;
        if let Err(failure) = renders(&name, &html) {
            failures.push(failure);
        }
    }
    assert!(pages > 0, "the corpus is empty");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn deep_nesting_does_not_overflow_the_renderer_thread() {
    // Through the real renderer thread: a stack overflow there aborts the
    // whole process, which no catch_unwind can turn into an error.
    let html = format!(
        "<body>{}derin{}",
        "<div>".repeat(5000),
        "</div>".repeat(5000)
    );
    let (to, from, renderer) = spawn();
    to.send(ToRenderer::Load { html }).unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: HEIGHT,
    })
    .unwrap();
    match from.recv_timeout(Duration::from_secs(120)) {
        Ok(FromRenderer::Frame(frame)) => assert_eq!(frame.width(), WIDTH),
        Err(RecvTimeoutError::Timeout) => panic!("no frame for a deeply nested page"),
        Err(RecvTimeoutError::Disconnected) => panic!("the renderer thread died"),
    }
    to.send(ToRenderer::Shutdown).unwrap();
    renderer.join().unwrap();
}
