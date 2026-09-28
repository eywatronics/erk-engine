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
//! To capture Chrome references for pages that have none yet:
//! `cargo test -p erk-renderer --test chrome_reference -- --ignored capture_chrome_references`
//! After a Chrome upgrade, set `ERK_RECAPTURE_ALL=1` to recapture every page.
//! Chrome is found through `ERK_CHROME` or its default install path.

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
const TOLERANCE: u8 = 12;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn reference_dir() -> PathBuf {
    manifest().join("tests/reference")
}

/// Every reference page: `(name, path)`. The pages directory may hold only
/// `.html` files, so a misnamed page cannot silently drop out of the test.
fn pages() -> Vec<(String, PathBuf)> {
    let mut pages = vec![(
        "merhaba".to_owned(),
        manifest().join("../../examples/merhaba.html"),
    )];
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
    for (name, _) in &expected {
        if !has_page(name) {
            failures.push(format!("expectation for `{name}`, which has no page"));
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
        let erk_png = erk_renderer::render_html(&html, WIDTH, HEIGHT)
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
    if todo.is_empty() {
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
    std::fs::create_dir_all(&work).unwrap();

    // Chrome must draw with the same fonts Erk embeds.
    let fonts = manifest().join("assets/fonts");
    let font_face = format!(
        "<style>@font-face {{ font-family: \"Noto Sans\"; font-weight: 400; src: url(\"{}\"); }}\n\
         @font-face {{ font-family: \"Noto Sans\"; font-weight: 700; src: url(\"{}\"); }}</style>\n",
        file_url(&fonts.join("NotoSans-Regular.ttf")),
        file_url(&fonts.join("NotoSans-Bold.ttf")),
    );

    for (name, path) in todo {
        let html = std::fs::read_to_string(&path).unwrap();
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
        let status = Command::new(&chrome)
            .args([
                "--headless",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
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
        println!("captured {name}");
    }

    std::fs::write(
        version_file,
        format!(
            "Captured with: {version} ({})\nWindow {WIDTH}x{HEIGHT}, device scale 1, LCD text off, embedded Noto Sans.\n",
            std::env::consts::OS
        ),
    )
    .unwrap();
}
