//! Golden image tests: Erk's output for a page must not change unless the
//! change is intended.
//!
//! A mismatch writes the actual image and display list to
//! `target/golden-actual/`. To accept an intended change, rerun with
//! `ERK_BLESS=1` (exactly `1`; `ERK_BLESS=0` does not bless) and commit the new golden image together with the change
//! that caused it, saying in the message why the image changed. (Committing
//! it separately would leave the causing commit red.)

use std::path::{Path, PathBuf};

const WIDTH: u16 = 800;
const HEIGHT: u16 = 600;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn decode(png_bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut pixels = vec![0; reader.output_buffer_size().expect("sized PNG")];
    let info = reader.next_frame(&mut pixels).expect("PNG frame");
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn check_golden(name: &str, html_path: &Path) {
    let html = std::fs::read_to_string(html_path).expect("test page exists");
    let frame = erk_renderer::render_html(&html, WIDTH, HEIGHT);
    let actual = frame.to_png().expect("non-empty frame");
    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.png"));

    if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
        std::fs::write(&golden_path, &actual).unwrap();
        return;
    }

    let golden = std::fs::read(&golden_path).unwrap_or_else(|_| {
        panic!(
            "no golden image at {}; run with ERK_BLESS=1",
            golden_path.display()
        )
    });
    if decode(&golden) != decode(&actual) {
        let out = repo_root().join("target/golden-actual");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join(format!("{name}.png")), &actual).unwrap();
        std::fs::write(
            out.join(format!("{name}.display-list.txt")),
            frame.display_list(),
        )
        .unwrap();
        panic!(
            "{name}: rendering differs from {}; actual output in {}",
            golden_path.display(),
            out.display()
        );
    }
}

#[test]
fn merhaba() {
    check_golden("merhaba", &repo_root().join("examples/merhaba.html"));
}

#[test]
fn rendering_is_deterministic() {
    let html = std::fs::read_to_string(repo_root().join("examples/merhaba.html")).unwrap();
    let first = erk_renderer::render_html(&html, WIDTH, HEIGHT);
    let second = erk_renderer::render_html(&html, WIDTH, HEIGHT);
    assert_eq!(first.rgba(), second.rgba());
    assert_eq!(first.display_list(), second.display_list());
}
