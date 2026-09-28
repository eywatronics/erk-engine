//! The shell's `--screenshot` path, run as the real `erk` binary.
//!
//! It must produce exactly the renderer's golden image: the golden test
//! calls the renderer directly, so without this a bug in the shell's path
//! (swapped channels, premultiplied pixels, the wrong size) would go
//! unnoticed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn decode(path: &Path) -> (u32, u32, Vec<u8>) {
    let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().expect("valid PNG");
    let mut pixels = vec![0; reader.output_buffer_size().expect("sized PNG")];
    let info = reader.next_frame(&mut pixels).expect("PNG frame");
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn erk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_erk"))
}

#[test]
fn screenshot_matches_the_renderers_golden_image() {
    let out = repo_root().join("target/shell-screenshot");
    std::fs::create_dir_all(&out).unwrap();
    let png = out.join("merhaba.png");
    let status = erk()
        .arg("--screenshot")
        .arg(&png)
        .arg(repo_root().join("examples/merhaba.html"))
        .status()
        .expect("erk runs");
    assert!(status.success());

    let golden = repo_root().join("crates/erk-renderer/tests/golden/merhaba.png");
    assert!(
        decode(&png) == decode(&golden),
        "{} differs from {}",
        png.display(),
        golden.display()
    );
}

#[test]
fn a_missing_page_is_an_error_not_a_panic() {
    let output = erk()
        .arg("--screenshot")
        .arg(repo_root().join("target/never.png"))
        .arg(repo_root().join("examples/does-not-exist.html"))
        .output()
        .expect("erk runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot read"));
}

#[test]
fn bad_arguments_print_usage() {
    let output = erk().arg("--screenshot").output().expect("erk runs");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage:"));
}
