//! Reference tests: how close Erk's rendering is to Chrome's, per page.
//!
//! Chrome's screenshots are captured once and committed
//! (`tests/reference/chrome/`), so this test needs no browser and gives the
//! same score on every machine: Erk's output is deterministic. Erk will not
//! match Chrome pixel for pixel (antialiasing and hinting differ), so each
//! page has a similarity score, recorded to two decimals in
//! `tests/reference/expectations.txt`.
//!
//! The score must equal its expectation. Below it is a regression. Above it
//! means the expectation is stale and must be raised in the same commit:
//! otherwise an improvement could later be given back without any test
//! noticing. Lowering an expectation is checked by CI (it needs a
//! `# lowered: reason` comment on the line).
//!
//! The score counts content pixels only: pixels that differ from the canvas
//! colour at all, in either image. Counting every pixel would score a page
//! with no text drawn in the nineties. "At all" matters: a white box on an
//! off-white canvas is content, even though its colour is close.
//!
//! Run with `-- --nocapture` for the score table. Diff images go to
//! `target/reference-diff/`.
//!
//! `chrome/pages.txt` records a hash of each page as it was captured; a
//! page edited since fails the test until its reference is captured again.
//!
//! To capture Chrome references for pages that have none yet:
//! `cargo test -p erk-renderer --test chrome_reference -- --ignored capture_chrome_references`
//! After a Chrome upgrade, set `ERK_RECAPTURE_ALL=1` to recapture every page.
//! Chrome is found through `ERK_CHROME` or its default install path.
//!
//! A page is drawn at one device pixel per CSS pixel unless it says
//! otherwise with `<meta name="erk-device-scale" content="2">`: then Chrome
//! captures it with that device scale factor and Erk renders it at the same
//! scale, both into an 800 × 600 CSS pixel viewport.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;

/// Largest per-channel difference (0-255) at which an Erk pixel and a
/// Chrome pixel still count as the same: absorbs slight antialiasing
/// differences, not misplaced glyphs.
///
/// It must stay below the smallest difference between two flat colours on
/// any reference page (17: the white box on blocks.html's canvas), or a
/// missing background could pass as antialiasing. At 24 a missing white box
/// on merhaba.html (difference 21) went unnoticed.
/// `the_tolerance_cannot_hide_a_missing_background` checks this.
const TOLERANCE: u8 = 12;

/// The device scale a page asks for (see the module docs), 1 by default.
fn page_scale(html: &str) -> f32 {
    const META: &str = r#"<meta name="erk-device-scale" content=""#;
    let Some(at) = html.find(META) else {
        return 1.0;
    };
    let rest = &html[at + META.len()..];
    let value = &rest[..rest.find('"').expect("the content attribute is closed")];
    let scale: f32 = value.parse().expect("erk-device-scale is a number");
    assert!(
        scale > 0.0
            && (f32::from(WIDTH) * scale).fract() == 0.0
            && (f32::from(HEIGHT) * scale).fract() == 0.0,
        "erk-device-scale {value} must give whole device pixels"
    );
    scale
}

/// The viewport in device pixels at `scale`.
fn device_size(scale: f32) -> (u16, u16) {
    let device = |css: u16| (f32::from(css) * scale) as u16;
    (device(WIDTH), device(HEIGHT))
}

/// Largest difference, in CSS pixels, between an edge of an Erk box and
/// Chrome's. Erk rounds boxes to whole pixels; Chrome's are fractional.
const GEOMETRY_TOLERANCE: f32 = 1.0;

/// Added to a copy of each page when capturing Chrome's geometry: once the
/// fonts have loaded, it writes the border box of the body and every element
/// in it, in document order, into a `<pre>`. Only the capture tool runs this,
/// in Chrome; Erk never runs scripts. The first line reports the viewport:
/// with --dump-dom, Chrome's viewport is the window minus its frame, unlike
/// with --screenshot.
const GEOMETRY_SCRIPT: &str = r#"<script id="erk-geometry-script">
window.addEventListener('load', () => document.fonts.ready.then(() => {
  const elements = [document.body, ...document.body.querySelectorAll('*')]
    .filter((e) => e.id !== 'erk-geometry-script');
  const lines = elements.map((e, i) => {
    const r = e.getBoundingClientRect();
    return [i, e.localName, getComputedStyle(e).display, r.x, r.y, r.width, r.height].join(' ');
  });
  const pre = document.createElement('pre');
  pre.id = 'erk-geometry';
  pre.textContent = ['viewport ' + innerWidth + ' ' + innerHeight, ...lines].join('\n');
  document.body.appendChild(pre);
}));
</script>
"#;

/// Added to a copy of each page when capturing Chrome's text: once the
/// fonts have loaded, it writes where every text node under the body lies,
/// one line of the node per output line (`Range.getClientRects()`), into a
/// `<pre>`. Text nodes are numbered in document order, counting only those
/// that hold more than white space and are not inside `<script>`,
/// `<style>` or `<template>`: the numbering `erk_renderer::text_boxes`
/// uses.
const TEXT_SCRIPT: &str = r#"<script id="erk-text-script">
window.addEventListener('load', () => document.fonts.ready.then(() => {
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
    acceptNode: (node) => node.parentElement.closest('script, style, template')
      ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT,
  });
  const lines = [];
  let index = 0;
  while (walker.nextNode()) {
    const text = walker.currentNode;
    if (!/\S/.test(text.data)) continue;
    const range = document.createRange();
    range.selectNodeContents(text);
    for (const r of range.getClientRects()) {
      if (r.width > 0) lines.push([index, r.x, r.y, r.width, r.height].join(' '));
    }
    index++;
  }
  const pre = document.createElement('pre');
  pre.id = 'erk-text';
  pre.textContent = ['viewport ' + innerWidth + ' ' + innerHeight, ...lines].join('\n');
  document.body.appendChild(pre);
}));
</script>
"#;

/// Largest difference, in CSS pixels, between an edge of a text node's line
/// in Erk and in Chrome.
const TEXT_TOLERANCE: f32 = 1.0;

/// Text nodes known to lie elsewhere than in Chrome, each with its reason:
/// (page, text node, reason). The test fails on any other difference, and
/// on a listed one that has gone, so the list cannot go stale.
const KNOWN_TEXT_DIFFERENCES: &[(&str, usize, &str)] = &[
    (
        "borders",
        2,
        "the wrapped inline element's opening edge stays on this line, so the space before it is not at the line's end (M2.6)",
    ),
    (
        "borders",
        3,
        "an inline element that wraps leaves its 6px left border on the previous line (M2.6)",
    ),
    (
        "borders",
        4,
        "on the line of the wrapped inline element above, so 6px to the left too (M2.6)",
    ),
    (
        "settings",
        4,
        "position: relative on an inline element does not move its text (top: -1px; M2.7)",
    ),
];

/// A copy of `html` that loads the embedded fonts and runs `script` before
/// `</body>`. The fonts go inside `<head>`, after the doctype: anything
/// before `<!DOCTYPE html>` puts Chrome into quirks mode, where the body's
/// first child loses its top margin and every page shifts.
fn measuring_page(html: &str, font_face: &str, script: &str) -> String {
    let at = html
        .find("<head>")
        .map(|i| i + "<head>".len())
        .expect("reference pages have a <head>");
    let with_fonts = format!("{}{font_face}{}", &html[..at], &html[at..]);
    let end = with_fonts
        .rfind("</body>")
        .expect("reference pages have a </body>");
    format!("{}{script}{}", &with_fonts[..end], &with_fonts[end..])
}

/// A colour covering at least this many CSS pixels of a Chrome reference is
/// a flat colour (a background, a box), not antialiasing: about a 32 × 32
/// box. No antialiasing shade on the current pages comes close. On a page
/// drawn at device scale 2 the same area is four times as many pixels.
const FLAT_PIXELS: u32 = 1000;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn reference_dir() -> PathBuf {
    manifest().join("tests/reference")
}

/// The resource provider of the reference pages: `images/<file>` from
/// tests/reference/images, nothing else. The pages are compared with
/// Chrome, which loads the same files from beside the page.
fn provide(request: &erk_renderer::ResourceRequest) -> Option<erk_renderer::ResourceResponse> {
    let file = request.url.strip_prefix("images/")?;
    if file.contains(['/', '\\']) || file.starts_with('.') {
        return None;
    }
    let data = std::fs::read(reference_dir().join("images").join(file)).ok()?;
    let mime = match file.rsplit_once('.').map(|(_, extension)| extension) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "",
    };
    Some(erk_renderer::ResourceResponse {
        id: request.id,
        mime: mime.to_owned(),
        data,
    })
}

/// Every reference page: `(name, path)`. The pages directory may hold only
/// `.html` files, so a misnamed page cannot silently drop out of the test.
fn pages() -> Vec<(String, PathBuf)> {
    // The examples a user opens first are reference pages too: the first
    // page, and M1's acceptance mockup.
    let mut pages: Vec<(String, PathBuf)> = ["merhaba", "settings"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                manifest().join(format!("../../examples/{name}.html")),
            )
        })
        .collect();
    let mut dir: Vec<_> = std::fs::read_dir(reference_dir().join("pages"))
        .expect("tests/reference/pages exists")
        .map(|entry| entry.unwrap().path())
        .collect();
    dir.sort();
    for path in dir {
        assert!(
            path.extension().is_some_and(|ext| ext == "html"),
            "{} is not a .html file; reference pages must end in .html",
            path.display()
        );
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        pages.push((name, path));
    }
    pages
}

struct Image {
    width: u32,
    height: u32,
    /// Straight RGBA8.
    pixels: Vec<u8>,
}

fn decode(bytes: &[u8]) -> Image {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut buffer = vec![0; reader.output_buffer_size().expect("sized PNG")];
    let info = reader.next_frame(&mut buffer).expect("PNG frame");
    buffer.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => panic!("unexpected PNG colour type {other:?}"),
    };
    Image {
        width: info.width,
        height: info.height,
        pixels,
    }
}

fn encode(image: &Image) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .unwrap()
        .write_image_data(&image.pixels)
        .unwrap();
    out
}

struct Comparison {
    /// Percentage of content pixels that match.
    content_score: f64,
    /// Percentage of all pixels that match.
    overall_score: f64,
    diff: Image,
}

/// The canvas colour: the most common colour of the Chrome image. Ties are
/// broken by the colour value, so the result does not depend on hash order.
fn canvas_colour(image: &Image) -> [u8; 3] {
    let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
    for p in image.pixels.as_chunks::<4>().0 {
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(colour, count)| (count, colour))
        .map(|(colour, _)| colour)
        .unwrap_or([255, 255, 255])
}

fn channel_diff(a: &[u8], b: &[u8]) -> u8 {
    (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap()
}

fn compare(erk: &Image, chrome: &Image) -> Comparison {
    assert_eq!(
        (erk.width, erk.height),
        (chrome.width, chrome.height),
        "Erk and Chrome images differ in size"
    );
    let canvas = canvas_colour(chrome);

    let (mut matched, mut content, mut content_matched) = (0u64, 0u64, 0u64);
    let mut diff = Vec::with_capacity(erk.pixels.len());
    let pairs = erk
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(chrome.pixels.as_chunks::<4>().0);
    for (e, c) in pairs {
        let same = channel_diff(e, c) <= TOLERANCE;
        let is_content = channel_diff(e, &canvas) > 0 || channel_diff(c, &canvas) > 0;
        matched += u64::from(same);
        if is_content {
            content += 1;
            content_matched += u64::from(same);
        }
        // Red where they disagree, a faint grey copy of Chrome elsewhere.
        if same {
            let brightness = (u16::from(c[0]) + u16::from(c[1]) + u16::from(c[2])) / 3;
            let grey = 200 + (brightness / 5) as u8;
            diff.extend_from_slice(&[grey, grey, grey, 255]);
        } else {
            diff.extend_from_slice(&[230, 30, 30, 255]);
        }
    }
    let total = erk.pixels.len() as f64 / 4.0;
    Comparison {
        content_score: if content == 0 {
            100.0
        } else {
            100.0 * content_matched as f64 / content as f64
        },
        overall_score: 100.0 * matched as f64 / total,
        diff: Image {
            width: erk.width,
            height: erk.height,
            pixels: diff,
        },
    }
}

/// Scores are compared at the two decimals they are recorded with.
fn hundredths(score: f64) -> i64 {
    (score * 100.0).round() as i64
}

/// FNV-1a of a page, with line endings normalised: enough to notice a page
/// that changed after its Chrome reference was captured.
fn page_hash(html: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in html.bytes().filter(|&byte| byte != b'\r') {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// `chrome/pages.txt`: `name hash` of each page as it was when its Chrome
/// reference was captured.
fn captured_hashes() -> Vec<(String, String)> {
    std::fs::read_to_string(reference_dir().join("chrome/pages.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, hash)| (name.to_owned(), hash.trim().to_owned()))
        .collect()
}

/// `name score` per line; `#` starts a comment, also at the end of a line.
fn expectations() -> Vec<(String, f64)> {
    std::fs::read_to_string(reference_dir().join("expectations.txt"))
        .unwrap_or_default()
        .lines()
        .map(|line| line.split('#').next().unwrap().trim())
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (name, score) = line.split_once(char::is_whitespace).expect("`name score`");
            (
                name.to_owned(),
                score.trim().parse().expect("numeric score"),
            )
        })
        .collect()
}

#[test]
fn erk_matches_its_recorded_distance_from_chrome() {
    let pages = pages();
    let expected = expectations();
    let out = manifest().join("../../target/reference-diff");
    std::fs::create_dir_all(&out).unwrap();
    let mut failures = Vec::new();

    // Nothing may be checked by name without a page behind it: a removed or
    // renamed page must not leave an expectation that is silently skipped.
    let has_page = |name: &str| pages.iter().any(|(page, _)| page == name);
    for (i, (name, _)) in expected.iter().enumerate() {
        if !has_page(name) {
            failures.push(format!("expectation for `{name}`, which has no page"));
        }
        // Only the first would count, so a second, lower line would hide a
        // regression.
        if expected[..i].iter().any(|(earlier, _)| earlier == name) {
            failures.push(format!("more than one expectation for `{name}`"));
        }
    }
    let hashes = captured_hashes();
    for (name, _) in &hashes {
        if !has_page(name) {
            failures.push(format!(
                "chrome/pages.txt lists `{name}`, which has no page"
            ));
        }
    }
    for entry in std::fs::read_dir(reference_dir().join("chrome")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "png") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            if !has_page(&name) {
                failures.push(format!("Chrome reference {name}.png has no page"));
            }
        }
    }

    let mut report = String::from("page            content  overall  expected\n");
    for (name, path) in &pages {
        let chrome_path = reference_dir().join("chrome").join(format!("{name}.png"));
        let Ok(chrome_png) = std::fs::read(&chrome_path) else {
            failures.push(format!(
                "{name}: no Chrome reference; capture it (see module docs)"
            ));
            continue;
        };
        let html = std::fs::read_to_string(path).unwrap();
        // A score against a picture of another page means nothing.
        let captured = hashes.iter().find(|(n, _)| n == name).map(|(_, h)| h);
        if captured != Some(&page_hash(&html)) {
            failures.push(format!(
                "{name}: the page changed after its Chrome reference was captured; \
                 delete chrome/{name}.png and capture it again (see module docs)"
            ));
        }
        let scale = page_scale(&html);
        let (width, height) = device_size(scale);
        let erk_png = erk_renderer::render_html_at_scale(&html, width, height, scale, &mut provide)
            .to_png()
            .expect("non-empty frame");
        let result = compare(&decode(&erk_png), &decode(&chrome_png));

        std::fs::write(out.join(format!("{name}.erk.png")), &erk_png).unwrap();
        std::fs::write(out.join(format!("{name}.chrome.png")), &chrome_png).unwrap();
        std::fs::write(out.join(format!("{name}.diff.png")), encode(&result.diff)).unwrap();

        let expectation = expected.iter().find(|(n, _)| n == name).map(|&(_, s)| s);
        report.push_str(&format!(
            "{name:<15} {:>6.2}%  {:>6.2}%  {}\n",
            result.content_score,
            result.overall_score,
            expectation.map_or("-".to_owned(), |s| format!("{s:.2}%"))
        ));
        let score = hundredths(result.content_score);
        match expectation.map(hundredths) {
            None => failures.push(format!(
                "{name}: no expectation; add `{name} {:.2}` to expectations.txt",
                result.content_score
            )),
            Some(expected) if score < expected => failures.push(format!(
                "{name}: content score {:.2}% fell below the expected {:.2}%",
                result.content_score,
                expected as f64 / 100.0
            )),
            Some(expected) if score > expected => failures.push(format!(
                "{name}: content score rose to {:.2}% (expected {:.2}%); raise the expectation",
                result.content_score,
                expected as f64 / 100.0
            )),
            Some(_) => {}
        }
    }
    std::fs::write(out.join("report.txt"), &report).unwrap();
    println!("\n{report}");
    assert!(failures.is_empty(), "\n{report}\n{}", failures.join("\n"));
}

/// Two flat colours closer than the tolerance would let one stand in for
/// the other: a box that is not drawn at all would still score as a match.
#[test]
fn the_tolerance_cannot_hide_a_missing_background() {
    let mut failures = Vec::new();
    for (name, path) in pages() {
        let Ok(png) = std::fs::read(reference_dir().join("chrome").join(format!("{name}.png")))
        else {
            continue; // reported by the main test
        };
        let scale = page_scale(&std::fs::read_to_string(&path).unwrap());
        let flat_pixels = (FLAT_PIXELS as f32 * scale * scale) as u32;
        let image = decode(&png);
        let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
        for p in image.pixels.as_chunks::<4>().0 {
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
        let mut flat: Vec<[u8; 3]> = counts
            .into_iter()
            .filter(|&(_, count)| count >= flat_pixels)
            .map(|(colour, _)| colour)
            .collect();
        flat.sort();
        for (i, a) in flat.iter().enumerate() {
            for b in &flat[i + 1..] {
                let difference = channel_diff(a, b);
                if difference <= TOLERANCE {
                    failures.push(format!(
                        "{name}: flat colours {a:?} and {b:?} differ by {difference}, \
                         within the tolerance of {TOLERANCE}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn find_chrome() -> PathBuf {
    if let Some(path) = std::env::var_os("ERK_CHROME") {
        return path.into();
    }
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"]
    } else {
        &[
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
        ]
    };
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
        .expect("Chrome not found; set ERK_CHROME")
}

/// The version of the Chrome binary that takes the screenshots.
///
/// On Windows, `chrome.exe --version` prints nothing: it hands the command
/// line to an already running Chrome, which opens a window in the user's
/// own browser. There the version is read from the executable's version
/// resource instead, which does not start Chrome.
fn chrome_version(chrome: &Path) -> String {
    let output = if cfg!(windows) {
        // `-Command` does not pass trailing arguments to `$args`, so the path
        // goes into the script as a single-quoted literal ('' escapes ').
        let literal = chrome.to_string_lossy().replace('\'', "''");
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Item -LiteralPath '{literal}').VersionInfo.ProductVersion"),
            ])
            .output()
    } else {
        Command::new(chrome).arg("--version").output()
    };
    let version = output
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    assert!(
        !version.is_empty(),
        "could not read the version of {}",
        chrome.display()
    );
    if cfg!(windows) {
        format!("Google Chrome {version}")
    } else {
        version
    }
}

/// A `file://` URL, percent-encoded, for a path on disk.
fn file_url(path: &Path) -> String {
    let path = path.canonicalize().unwrap();
    // canonicalize returns `\\?\C:\...` on Windows, which url rejects.
    let path = PathBuf::from(path.to_string_lossy().trim_start_matches(r"\\?\"));
    url::Url::from_file_path(&path)
        .unwrap_or_else(|()| panic!("{} is not an absolute path", path.display()))
        .to_string()
}

/// Capture Chrome's rendering of every reference page that has no
/// reference yet, or of every page with `ERK_RECAPTURE_ALL=1`. Not part of
/// the normal run: it needs Chrome.
///
/// Capturing only missing pages keeps a new page from silently replacing
/// the others with whatever Chrome the machine has updated to. A partial
/// capture is refused when Chrome's version differs from the one recorded
/// in VERSION.txt, since the references would then mix versions.
#[test]
#[ignore]
fn capture_chrome_references() {
    let chrome = find_chrome();
    let version = chrome_version(&chrome);
    let chrome_dir = reference_dir().join("chrome");
    std::fs::create_dir_all(&chrome_dir).unwrap();
    let version_file = chrome_dir.join("VERSION.txt");
    let recorded = std::fs::read_to_string(&version_file).unwrap_or_default();
    let all = std::env::var_os("ERK_RECAPTURE_ALL").is_some();

    let todo: Vec<_> = pages()
        .into_iter()
        .filter(|(name, _)| all || !chrome_dir.join(format!("{name}.png")).exists())
        .collect();
    // Pages captured before text was measured: only their text is added,
    // with the Chrome that took their screenshots.
    let text_todo: Vec<_> = pages()
        .into_iter()
        .filter(|(name, _)| {
            !todo.iter().any(|(n, _)| n == name)
                && !chrome_dir.join(format!("{name}.text.txt")).exists()
        })
        .collect();
    if todo.is_empty() && text_todo.is_empty() {
        println!("every page has a Chrome reference; set ERK_RECAPTURE_ALL=1 to recapture");
        return;
    }
    if !all && !recorded.is_empty() {
        assert!(
            recorded.contains(&format!("{version} ")),
            "Chrome is now {version}, but the references were captured with:\n{recorded}\n\
             Recapture every page with ERK_RECAPTURE_ALL=1 instead of mixing versions."
        );
    }

    let work = manifest().join("../../target/chrome-capture");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("images")).unwrap();
    // The pages' images, beside them as the pages expect.
    for entry in std::fs::read_dir(reference_dir().join("images")).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, work.join("images").join(path.file_name().unwrap())).unwrap();
    }

    // Chrome must draw with the same fonts Erk embeds.
    let fonts = manifest().join("assets/fonts");
    let font_face = format!(
        "<style>@font-face {{ font-family: \"Noto Sans\"; font-weight: 400; src: url(\"{}\"); }}\n\
         @font-face {{ font-family: \"Noto Sans\"; font-weight: 700; src: url(\"{}\"); }}</style>\n",
        file_url(&fonts.join("NotoSans-Regular.ttf")),
        file_url(&fonts.join("NotoSans-Bold.ttf")),
    );

    let mut hashes = captured_hashes();
    let captured_screenshots = !todo.is_empty();
    for (name, path) in todo {
        let html = std::fs::read_to_string(&path).unwrap();
        hashes.retain(|(n, _)| *n != name);
        hashes.push((name.clone(), page_hash(&html)));
        let page = work.join(format!("{name}.html"));
        // Inside <head>, after the doctype: anything before `<!DOCTYPE html>`
        // puts Chrome into quirks mode, where the body's first child loses
        // its top margin and every page shifts.
        let at = html
            .find("<head>")
            .map(|i| i + "<head>".len())
            .expect("reference pages have a <head>");
        std::fs::write(&page, format!("{}{font_face}{}", &html[..at], &html[at..])).unwrap();
        let shot = chrome_dir.join(format!("{name}.png"));
        let scale = page_scale(&html);
        let status = Command::new(&chrome)
            .args([
                "--headless",
                "--disable-gpu",
                "--hide-scrollbars",
                &format!("--force-device-scale-factor={scale}"),
                "--disable-lcd-text",
                "--allow-file-access-from-files",
                "--no-first-run",
                "--no-default-browser-check",
                &format!("--window-size={WIDTH},{HEIGHT}"),
                &format!("--user-data-dir={}", work.join("profile").display()),
                &format!("--screenshot={}", shot.display()),
                &file_url(&page),
            ])
            .status()
            .expect("Chrome runs");
        assert!(status.success(), "Chrome failed on {name}");

        // Geometry: the same page with the measuring script before </body>.
        let with_fonts = format!("{}{font_face}{}", &html[..at], &html[at..]);
        let end = with_fonts
            .rfind("</body>")
            .expect("reference pages have a </body>");
        let measured = work.join(format!("{name}.geometry.html"));
        std::fs::write(
            &measured,
            format!(
                "{}{GEOMETRY_SCRIPT}{}",
                &with_fonts[..end],
                &with_fonts[end..]
            ),
        )
        .unwrap();
        let geometry = measure_geometry(&chrome, &work, &measured, &name, scale, "erk-geometry");
        std::fs::write(chrome_dir.join(format!("{name}.geometry.txt")), geometry).unwrap();
        measure_text(&chrome, &work, &chrome_dir, &name, &html, &font_face, scale);
        println!("captured {name}");
    }
    for (name, path) in &text_todo {
        let html = std::fs::read_to_string(path).unwrap();
        measure_text(
            &chrome,
            &work,
            &chrome_dir,
            name,
            &html,
            &font_face,
            page_scale(&html),
        );
        println!("captured the text of {name}");
    }
    if !captured_screenshots {
        return;
    }
    hashes.sort();
    let listing: String = hashes
        .iter()
        .map(|(name, hash)| format!("{name} {hash}\n"))
        .collect();
    std::fs::write(chrome_dir.join("pages.txt"), listing).unwrap();

    std::fs::write(
        version_file,
        format!(
            "Captured with: {version} ({})\nWindow {WIDTH}x{HEIGHT}, device scale 1 unless a page sets erk-device-scale, LCD text off, embedded Noto Sans.\n",
            std::env::consts::OS
        ),
    )
    .unwrap();
}

/// Chrome's boxes for a page carrying the measuring script, measured in a
/// viewport of exactly WIDTH x HEIGHT: the window is enlarged by whatever its
/// frame takes, which depends on the operating system.
fn measure_geometry(
    chrome: &Path,
    work: &Path,
    page: &Path,
    name: &str,
    scale: f32,
    marker: &str,
) -> String {
    let (mut window_width, mut window_height) = (i32::from(WIDTH), i32::from(HEIGHT));
    for _ in 0..2 {
        let output = Command::new(chrome)
            .args([
                "--headless",
                "--disable-gpu",
                "--hide-scrollbars",
                &format!("--force-device-scale-factor={scale}"),
                "--allow-file-access-from-files",
                "--no-first-run",
                "--no-default-browser-check",
                "--virtual-time-budget=5000",
                &format!("--window-size={window_width},{window_height}"),
                &format!("--user-data-dir={}", work.join("profile").display()),
                "--dump-dom",
                &file_url(page),
            ])
            .output()
            .expect("Chrome runs");
        assert!(output.status.success(), "Chrome failed measuring {name}");
        let dom = String::from_utf8_lossy(&output.stdout);
        let open = format!(r#"<pre id="{marker}">"#);
        let start = dom.find(&open).expect("the measuring script ran") + open.len();
        let len = dom[start..].find("</pre>").expect("the <pre> is closed");
        let text = dom[start..start + len].replace("\r\n", "\n");
        let text = text.trim();
        let (viewport, boxes) = text.split_once('\n').unwrap_or((text, ""));
        let size: Vec<i32> = viewport
            .strip_prefix("viewport ")
            .unwrap_or_else(|| panic!("{name}: the first line is not the viewport: {viewport:?}"))
            .split_whitespace()
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|_| panic!("{name}: viewport {viewport:?}"))
            })
            .collect();
        if size == [i32::from(WIDTH), i32::from(HEIGHT)] {
            return format!("{boxes}\n");
        }
        window_width += i32::from(WIDTH) - size[0];
        window_height += i32::from(HEIGHT) - size[1];
    }
    panic!("{name}: could not get a {WIDTH}x{HEIGHT} viewport in Chrome");
}

/// Write `{name}.text.txt`: where Chrome puts each text node of the page.
fn measure_text(
    chrome: &Path,
    work: &Path,
    chrome_dir: &Path,
    name: &str,
    html: &str,
    font_face: &str,
    scale: f32,
) {
    let measured = work.join(format!("{name}.text.html"));
    std::fs::write(&measured, measuring_page(html, font_face, TEXT_SCRIPT)).unwrap();
    let text = measure_geometry(chrome, work, &measured, name, scale, "erk-text");
    std::fs::write(chrome_dir.join(format!("{name}.text.txt")), text).unwrap();
}

/// Where each text node lies, line by line, in Erk and in Chrome. The
/// pixel score of a text-heavy page is held down by glyph antialiasing and
/// says little about layout; the box test skips inline content. This test
/// measures the text itself: every line of every text node, from its first
/// to its last character, within a pixel of Chrome's.
#[test]
fn erk_text_matches_chrome() {
    let chrome_dir = reference_dir().join("chrome");
    let mut failures = Vec::new();
    let mut report = String::from("page            nodes  lines  matched\n");
    for (name, path) in pages() {
        let Ok(chrome) = std::fs::read_to_string(chrome_dir.join(format!("{name}.text.txt")))
        else {
            failures.push(format!(
                "{name}: no Chrome text; capture it (see module docs)"
            ));
            continue;
        };
        let html = std::fs::read_to_string(&path).unwrap();
        let erk = erk_renderer::text_boxes(&html, WIDTH, HEIGHT, &mut provide);
        let chrome: Vec<[f32; 5]> = chrome
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let fields: Vec<f32> = line
                    .split_whitespace()
                    .map(|v| v.parse().expect("number"))
                    .collect();
                [fields[0], fields[1], fields[2], fields[3], fields[4]]
            })
            .collect();
        let nodes = chrome
            .iter()
            .map(|line| line[0] as usize)
            .max()
            .map_or(0, |n| n + 1);
        let (mut lines, mut matched) = (0, 0);
        for node in 0..nodes {
            let known = KNOWN_TEXT_DIFFERENCES
                .iter()
                .any(|(page, n, _)| *page == name && *n == node);
            let failures_before = failures.len();
            let theirs: Vec<&[f32; 5]> = chrome.iter().filter(|l| l[0] as usize == node).collect();
            let ours: Vec<&erk_renderer::TextBox> =
                erk.iter().filter(|b| b.index == node).collect();
            lines += theirs.len();
            if theirs.len() != ours.len() {
                failures.push(format!(
                    "{name}: text node {node} has {} line(s) in Chrome, {} in Erk: Chrome {:?}, Erk {:?}",
                    theirs.len(),
                    ours.len(),
                    theirs,
                    ours.iter().map(|b| (b.x, b.y, b.width, b.height)).collect::<Vec<_>>()
                ));
                continue;
            }
            for (line, (c, e)) in theirs.iter().zip(&ours).enumerate() {
                let edges = [
                    e.x - c[1],
                    e.y - c[2],
                    (e.x + e.width) - (c[1] + c[3]),
                    (e.y + e.height) - (c[2] + c[4]),
                ];
                if edges.iter().all(|d| d.abs() <= TEXT_TOLERANCE) {
                    matched += 1;
                } else {
                    failures.push(format!(
                        "{name}: text node {node}, line {line}: Erk {} {} {}x{}, Chrome {} {} {}x{}",
                        e.x, e.y, e.width, e.height, c[1], c[2], c[3], c[4]
                    ));
                }
            }
            match (known, failures.len() > failures_before) {
                // A known difference: expected, and not a failure.
                (true, true) => failures.truncate(failures_before),
                (true, false) => failures.push(format!(
                    "{name}: text node {node} now matches Chrome; remove it from KNOWN_TEXT_DIFFERENCES"
                )),
                _ => {}
            }
        }
        report.push_str(&format!(
            "{name:<15} {nodes:>5}  {lines:>5}  {matched:>7}\n"
        ));
    }
    println!("\n{report}");
    assert!(failures.is_empty(), "\n{report}\n{}", failures.join("\n"));
}

/// Every element's box as Erk lays it out, against Chrome's. Unlike the pixel
/// score this does not depend on antialiasing: it measures the layout itself,
/// line heights included. Inline elements (no box of their own in Erk yet)
/// and elements without a box are skipped, and counted.
#[test]
fn erk_boxes_match_chrome() {
    let chrome_dir = reference_dir().join("chrome");
    let pages = pages();
    let mut failures = Vec::new();

    for entry in std::fs::read_dir(&chrome_dir).unwrap() {
        let file = entry.unwrap().file_name().to_string_lossy().into_owned();
        if let Some(name) = file.strip_suffix(".geometry.txt")
            && !pages.iter().any(|(page, _)| page == name)
        {
            failures.push(format!("Chrome geometry {file} has no page"));
        }
    }

    let mut report = String::from("page            boxes  matched  skipped\n");
    for (name, path) in &pages {
        let Ok(chrome) = std::fs::read_to_string(chrome_dir.join(format!("{name}.geometry.txt")))
        else {
            failures.push(format!(
                "{name}: no Chrome geometry; capture it (see module docs)"
            ));
            continue;
        };
        let html = std::fs::read_to_string(path).unwrap();
        let erk = erk_renderer::element_boxes(&html, WIDTH, HEIGHT, &mut provide);
        let (mut compared, mut matched, mut skipped) = (0, 0, 0);
        for line in chrome.lines().filter(|line| !line.trim().is_empty()) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            let [index, tag, display, x, y, width, height] = fields[..] else {
                panic!("{name}: malformed geometry line {line:?}");
            };
            let index: usize = index.parse().expect("index");
            let [x, y, width, height] =
                [x, y, width, height].map(|v| v.parse::<f32>().expect("number"));
            if matches!(display, "inline" | "none" | "contents") {
                skipped += 1;
                continue;
            }
            compared += 1;
            let Some(erk_box) = erk.iter().find(|b| b.index == index) else {
                failures.push(format!(
                    "{name}: <{tag}> #{index} has a box in Chrome ({display}) but not in Erk"
                ));
                continue;
            };
            assert_eq!(
                erk_box.tag, tag,
                "{name}: element #{index} differs between Erk and Chrome; the DOMs do not line up"
            );
            let close = [
                erk_box.x - x,
                erk_box.y - y,
                erk_box.width - width,
                erk_box.height - height,
            ]
            .iter()
            .all(|d| d.abs() <= GEOMETRY_TOLERANCE);
            if close {
                matched += 1;
            } else {
                failures.push(format!(
                    "{name}: <{tag}> #{index}: Erk {} {} {}x{}, Chrome {x} {y} {width}x{height}",
                    erk_box.x, erk_box.y, erk_box.width, erk_box.height
                ));
            }
        }
        report.push_str(&format!(
            "{name:<15} {compared:>5}  {matched:>7}  {skipped:>7}\n"
        ));
    }
    println!("\n{report}");
    assert!(failures.is_empty(), "\n{report}\n{}", failures.join("\n"));
}
