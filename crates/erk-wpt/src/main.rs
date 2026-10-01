//! Runs the reftests of the Web Platform Tests through Erk and compares the
//! results with a recorded baseline (tests/wpt/expectations.txt).
//!
//! Erk runs no script, so only reftests count: a test file with
//! `<link rel=match>` or `<link rel=mismatch>` is rendered at 800 × 600, its
//! references too, and the frames are compared by WPT's rules (at least one
//! match reference must match, every mismatch reference must differ, a
//! `<meta name=fuzzy>` widens "identical"). Tests run in-process through
//! `render_html`: no browser driver, no process per test, the same pixels on
//! every machine. Resources are not loaded, as Erk's core loads none: a test
//! that needs an external stylesheet, an image or a web font fails, and the
//! baseline says so.
//!
//! ```text
//! erk-wpt check --wpt <checkout>    run, compare with the baseline, exit 1 on any change
//! erk-wpt write --wpt <checkout>    run and rewrite the baseline
//! ```
//!
//! The checkout must be at the commit in tests/wpt/WPT_COMMIT with the
//! directories in tests/wpt/dirs.txt; CI fetches exactly that.

mod reftest;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use erk_renderer::{ResourceKind, ResourceRequest, ResourceResponse, render_html_with_resources};

use crate::reftest::{Fuzzy, Reference, Relation};

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;

/// The renderer's own stack size: the parser allows 512 levels of nesting
/// and layout recurses once per level (erk-renderer, thread.rs).
const STACK: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Pass,
    Fail,
    /// The renderer panicked on the test or one of its references.
    Crash,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Crash => "CRASH",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "PASS" => Some(Self::Pass),
            "FAIL" => Some(Self::Fail),
            "CRASH" => Some(Self::Crash),
            _ => None,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str);
    let wpt = args
        .iter()
        .position(|arg| arg == "--wpt")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from);
    let (Some(mode @ ("check" | "write")), Some(wpt)) = (mode, wpt) else {
        eprintln!("usage: erk-wpt check|write --wpt <web-platform-tests checkout>");
        return ExitCode::from(2);
    };
    let config = repo_root().join("tests/wpt");
    let dirs: Vec<String> = match read_lines(&config.join("dirs.txt")) {
        // `support:` directories are checked out for their references, not run.
        Ok(lines) => lines
            .into_iter()
            .filter(|line| !line.starts_with("support:"))
            .collect(),
        Err(error) => {
            eprintln!("cannot read tests/wpt/dirs.txt: {error}");
            return ExitCode::from(2);
        }
    };
    let tests = match discover(&wpt, &dirs) {
        Ok(tests) => tests,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(2);
        }
    };
    let runs = run_all(&wpt, &tests);
    print_summary(&dirs, &runs);
    let results: BTreeMap<String, Status> = runs
        .into_iter()
        .map(|(name, (status, _))| (name, status))
        .collect();

    let expectations = config.join("expectations.txt");
    if mode == "write" {
        let old = read_expectations(&expectations).unwrap_or_default();
        if let Err(error) = write_expectations(&expectations, &results, &old) {
            eprintln!("cannot write {}: {error}", expectations.display());
            return ExitCode::from(2);
        }
        println!("wrote {}", expectations.display());
        return ExitCode::SUCCESS;
    }
    let baseline = match read_expectations(&expectations) {
        Ok(baseline) => baseline,
        Err(error) => {
            eprintln!("cannot read {}: {error}", expectations.display());
            return ExitCode::from(2);
        }
    };
    let changes = compare(&baseline, &results);
    if changes.is_empty() {
        println!("all {} results match the baseline", results.len());
        ExitCode::SUCCESS
    } else {
        for change in &changes {
            println!("{change}");
        }
        println!(
            "{} results differ from the baseline. A regression must be fixed; an improvement, \
             or a regression with a reason (`# lowered: ...`), goes into tests/wpt/expectations.txt \
             with `erk-wpt write`.",
            changes.len()
        );
        ExitCode::FAILURE
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}

/// A reftest: its path from the checkout root, with `/` separators, and its
/// references resolved to paths from the root.
struct Test {
    name: String,
    references: Vec<(Relation, String)>,
    fuzzy: Vec<Fuzzy>,
}

/// Every reftest under `dirs`, sorted. References and support files are
/// not tests; a reference that does not exist in the checkout is a
/// configuration error (a directory missing from dirs.txt), not a failure.
fn discover(root: &Path, dirs: &[String]) -> Result<Vec<Test>, String> {
    let mut files = Vec::new();
    for dir in dirs {
        collect_files(&root.join(dir), &mut files)
            .map_err(|error| format!("cannot read {dir} in the checkout: {error}"))?;
    }
    let mut tests = Vec::new();
    for path in files {
        let name = relative_name(root, &path);
        if is_support(&name) {
            continue;
        }
        let markup = read_markup(&path).map_err(|error| format!("{name}: {error}"))?;
        let references: Vec<Reference> = reftest::references(&markup);
        if references.is_empty() {
            continue;
        }
        let mut resolved = Vec::new();
        for reference in references {
            let target = resolve(root, &name, &reference.href);
            if !root.join(&target).is_file() {
                return Err(format!(
                    "{name}: reference {} is not in the checkout (add its directory to tests/wpt/dirs.txt as `support:`)",
                    reference.href
                ));
            }
            resolved.push((reference.relation, target));
        }
        tests.push(Test {
            name,
            references: resolved,
            fuzzy: reftest::fuzzy(&markup),
        });
    }
    tests.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tests)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "html" | "htm" | "xht" | "xhtml"))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn relative_name(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// References and support files, by WPT's naming conventions.
fn is_support(name: &str) -> bool {
    let file = name.rsplit('/').next().unwrap_or(name);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    name.split('/')
        .any(|part| matches!(part, "reference" | "references" | "support" | "resources"))
        || stem.ends_with("-ref")
        || stem.ends_with("-notref")
        || stem.starts_with("ref-")
}

/// `href` relative to the test's directory, or to the root when it starts
/// with `/`; query and fragment dropped.
fn resolve(_root: &Path, test: &str, href: &str) -> String {
    let href = href.split(['?', '#']).next().unwrap_or(href);
    let mut parts: Vec<&str> = if href.starts_with('/') {
        Vec::new()
    } else {
        let mut dir: Vec<&str> = test.split('/').collect();
        dir.pop();
        dir
    };
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn read_markup(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let markup = String::from_utf8_lossy(&bytes).into_owned();
    let xhtml = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "xht" | "xhtml"));
    Ok(if xhtml {
        reftest::xhtml_as_html(&markup)
    } else {
        markup
    })
}

/// Run every test on a pool of threads with the renderer's stack size.
fn run_all(root: &Path, tests: &[Test]) -> BTreeMap<String, (Status, bool)> {
    let next = AtomicUsize::new(0);
    let results = Mutex::new(BTreeMap::new());
    let workers = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(scope, || {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(test) = tests.get(index) else {
                            break;
                        };
                        let status = run(root, test);
                        results
                            .lock()
                            .expect("no worker panics while holding the lock")
                            .insert(test.name.clone(), status);
                    }
                })
                .expect("a worker thread starts");
        }
    });
    results.into_inner().expect("the workers are done")
}

/// The pixels of a page, or `None` if rendering it panicked. Its images
/// come from the checkout, as a browser would load them.
fn render(root: &Path, name: &str) -> Option<Vec<u8>> {
    let markup = read_markup(&root.join(name)).ok()?;
    std::panic::catch_unwind(|| {
        let mut provide = |request: &ResourceRequest| image(root, name, request);
        render_html_with_resources(&markup, WIDTH, HEIGHT, &mut provide)
            .rgba()
            .to_vec()
    })
    .ok()
}

/// An image a test names, from the checkout: a relative URL from the test's
/// directory, a `/` path from the checkout's root. Other schemes, and
/// stylesheets and fonts (not loaded yet), are not served.
fn image(root: &Path, page: &str, request: &ResourceRequest) -> Option<ResourceResponse> {
    if request.kind != ResourceKind::Image
        || request.url.contains("://")
        || request.url.starts_with("data:")
    {
        return None;
    }
    let path = resolve(root, page, &request.url);
    let data = std::fs::read(root.join(&path)).ok()?;
    let mime = match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "",
    };
    Some(ResourceResponse {
        id: request.id,
        mime: mime.to_owned(),
        data,
    })
}

/// A test's status, and whether its own frame was a single colour: a pass
/// on a blank frame proves little (both sides may have rendered nothing),
/// so the summary counts them.
fn run(root: &Path, test: &Test) -> (Status, bool) {
    let Some(pixels) = render(root, &test.name) else {
        return (Status::Crash, false);
    };
    let (rgba, _) = pixels.as_chunks::<4>();
    let uniform = rgba.iter().all(|pixel| Some(pixel) == rgba.first());
    (judge(root, test, &pixels), uniform)
}

fn judge(root: &Path, test: &Test, pixels: &[u8]) -> Status {
    let mut any_match = None;
    let mut all_mismatch = true;
    for (relation, reference) in &test.references {
        let Some(expected) = render(root, reference) else {
            return Status::Crash;
        };
        let file = reference.rsplit('/').next().unwrap_or(reference);
        let fuzzy = test
            .fuzzy
            .iter()
            .find(|fuzzy| {
                fuzzy
                    .reference
                    .as_deref()
                    .is_some_and(|url| url.ends_with(file))
            })
            .or_else(|| test.fuzzy.iter().find(|fuzzy| fuzzy.reference.is_none()));
        let equal = same(pixels, &expected, fuzzy);
        match relation {
            Relation::Match => any_match = Some(any_match.unwrap_or(false) || equal),
            Relation::Mismatch => all_mismatch &= !equal,
        }
    }
    if any_match.unwrap_or(true) && all_mismatch {
        Status::Pass
    } else {
        Status::Fail
    }
}

/// Whether two frames count as identical: pixel for pixel, or within a
/// fuzzy allowance (both ranges inclusive).
fn same(a: &[u8], b: &[u8], fuzzy: Option<&Fuzzy>) -> bool {
    let mut largest = 0u8;
    let mut differing = 0u64;
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let difference = pa
            .iter()
            .zip(pb)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if difference > 0 {
            differing += 1;
            largest = largest.max(difference);
        }
    }
    if differing == 0 {
        return true;
    }
    fuzzy.is_some_and(|fuzzy| {
        (fuzzy.max_difference.0..=fuzzy.max_difference.1).contains(&largest)
            && (fuzzy.total_pixels.0..=fuzzy.total_pixels.1).contains(&differing)
    })
}

/// Tests, passes and pass rate per directory; `blank` counts the passes
/// whose frame was a single colour.
fn print_summary(dirs: &[String], results: &BTreeMap<String, (Status, bool)>) {
    println!(
        "{:<28} {:>6} {:>6} {:>6} {:>6}",
        "directory", "tests", "pass", "rate", "blank"
    );
    for dir in dirs {
        let prefix = format!("{dir}/");
        let in_dir: Vec<(Status, bool)> = results
            .iter()
            .filter(|(name, _)| name.starts_with(&prefix))
            .map(|(_, result)| *result)
            .collect();
        let pass = in_dir
            .iter()
            .filter(|(status, _)| *status == Status::Pass)
            .count();
        let blank = in_dir
            .iter()
            .filter(|(status, uniform)| *status == Status::Pass && *uniform)
            .count();
        let rate = if in_dir.is_empty() {
            0.0
        } else {
            100.0 * pass as f64 / in_dir.len() as f64
        };
        println!(
            "{dir:<28} {:>6} {pass:>6} {rate:>5.1}% {blank:>6}",
            in_dir.len()
        );
    }
}

/// A baseline line: the status and the comment after it, if any.
type Expectations = BTreeMap<String, (Status, String)>;

fn read_expectations(path: &Path) -> Result<Expectations, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let mut expectations = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (entry, comment) = line
            .split_once('#')
            .map_or((line, ""), |(entry, comment)| (entry, comment));
        let mut fields = entry.split_whitespace();
        let (Some(name), Some(status), None) = (fields.next(), fields.next(), fields.next()) else {
            return Err(format!("line {}: expected `test STATUS`", number + 1));
        };
        let status = Status::parse(status)
            .ok_or_else(|| format!("line {}: unknown status {status}", number + 1))?;
        if expectations
            .insert(name.to_owned(), (status, comment.trim().to_owned()))
            .is_some()
        {
            return Err(format!("line {}: {name} is listed twice", number + 1));
        }
    }
    Ok(expectations)
}

/// Rewrite the baseline. A line keeps its comment while its status stays
/// the same; a status that changes loses it, so a new `# lowered:` reason is
/// written for the new state.
fn write_expectations(
    path: &Path,
    results: &BTreeMap<String, Status>,
    old: &Expectations,
) -> std::io::Result<()> {
    let mut text = String::from(
        "# WPT reftest results through Erk, one `test STATUS` per line, at the commit in\n\
         # WPT_COMMIT for the directories in dirs.txt. CI (`erk-wpt check`) fails if a\n\
         # result differs in either direction: a regression must be fixed, an improvement\n\
         # recorded. A test that goes from PASS to anything else needs a reason on its\n\
         # line, checked by CI:\n\
         #   css/... FAIL  # lowered: reason\n",
    );
    for (name, status) in results {
        text.push_str(name);
        text.push(' ');
        text.push_str(status.name());
        if let Some((old_status, comment)) = old.get(name)
            && old_status == status
            && !comment.is_empty()
        {
            text.push_str("  # ");
            text.push_str(comment);
        }
        text.push('\n');
    }
    std::fs::write(path, text)
}

/// Every difference between the baseline and the results, as a line.
fn compare(baseline: &Expectations, results: &BTreeMap<String, Status>) -> Vec<String> {
    let mut changes = Vec::new();
    for (name, status) in results {
        match baseline.get(name) {
            None => changes.push(format!("new test {name}: {}", status.name())),
            Some((expected, _)) if expected != status => changes.push(format!(
                "{} {name}: {} -> {}",
                if *status == Status::Pass {
                    "improved"
                } else {
                    "REGRESSED"
                },
                expected.name(),
                status.name()
            )),
            Some(_) => {}
        }
    }
    for name in baseline.keys() {
        if !results.contains_key(name) {
            changes.push(format!("missing test {name}"));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_against_the_test_or_the_root() {
        assert_eq!(
            resolve(
                Path::new("."),
                "css/CSS2/normal-flow/a.xht",
                "../reference/b.xht"
            ),
            "css/CSS2/reference/b.xht"
        );
        assert_eq!(
            resolve(
                Path::new("."),
                "css/css-position/a.html",
                "/css/reference/c.html#x"
            ),
            "css/reference/c.html"
        );
    }

    #[test]
    fn references_and_support_files_are_not_tests() {
        assert!(is_support("css/CSS2/normal-flow/support/a.html"));
        assert!(is_support("css/css-position/a-ref.html"));
        assert!(is_support(
            "css/reference/ref-filled-green-100px-square.xht"
        ));
        assert!(!is_support("css/css-position/position-relative-001.html"));
    }

    #[test]
    fn fuzzy_widens_identity_inclusively() {
        let a = [10, 10, 10, 255, 0, 0, 0, 255];
        let b = [15, 10, 10, 255, 0, 0, 0, 255];
        assert!(same(&a, &a, None));
        assert!(!same(&a, &b, None));
        let fuzzy = Fuzzy {
            reference: None,
            max_difference: (0, 5),
            total_pixels: (0, 1),
        };
        assert!(same(&a, &b, Some(&fuzzy)));
        let tight = Fuzzy {
            max_difference: (0, 4),
            ..fuzzy
        };
        assert!(!same(&a, &b, Some(&tight)));
    }

    #[test]
    fn a_change_in_either_direction_is_reported() {
        let baseline: Expectations = [
            ("a".to_owned(), (Status::Pass, String::new())),
            ("b".to_owned(), (Status::Fail, String::new())),
            ("c".to_owned(), (Status::Fail, String::new())),
        ]
        .into();
        let results: BTreeMap<String, Status> = [
            ("a".to_owned(), Status::Fail),
            ("b".to_owned(), Status::Pass),
            ("d".to_owned(), Status::Pass),
        ]
        .into();
        let changes = compare(&baseline, &results);
        assert_eq!(changes.len(), 4, "{changes:?}");
        assert!(
            changes
                .iter()
                .any(|change| change.starts_with("REGRESSED a"))
        );
        assert!(
            changes
                .iter()
                .any(|change| change.starts_with("improved b"))
        );
        assert!(changes.iter().any(|change| change == "missing test c"));
        assert!(
            changes
                .iter()
                .any(|change| change.starts_with("new test d"))
        );
    }
}
