//! include/erk.h is what cbindgen generates from this crate: a change to the
//! ABI without its header, or an edited header, fails here. Run with
//! `ERK_BLESS=1` to write the header after changing the ABI.

use std::path::Path;

#[test]
fn the_committed_header_is_the_generated_one() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config =
        cbindgen::Config::from_file(crate_dir.join("cbindgen.toml")).expect("cbindgen.toml reads");
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(config)
        .generate()
        .expect("the header generates")
        .write(&mut generated);
    let generated = String::from_utf8(generated).expect("the header is UTF-8");
    let committed_path = crate_dir.join("../../include/erk.h");
    if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
        std::fs::write(&committed_path, &generated).expect("include/erk.h writes");
        return;
    }
    let committed = std::fs::read_to_string(&committed_path)
        .expect("include/erk.h exists; run with ERK_BLESS=1 to write it")
        .replace("\r\n", "\n");
    assert!(
        committed == generated,
        "include/erk.h is not what cbindgen generates; run with ERK_BLESS=1"
    );
}
