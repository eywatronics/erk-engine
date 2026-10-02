//! Any bytes, read as HTML, render without a panic. The page is painted on
//! a thread with the renderer thread's stack, as the shell paints it: deep
//! documents need it. A finding goes into
//! `crates/erk-renderer/tests/robustness/`, where the corpus test keeps it.

#![no_main]

use libfuzzer_sys::fuzz_target;

/// The renderer thread's stack (erk-renderer, thread.rs).
const RENDERER_STACK: usize = 16 * 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data).into_owned();
    let painted = std::thread::Builder::new()
        .stack_size(RENDERER_STACK)
        .spawn(move || {
            erk_renderer::render_html(&html, 320, 240);
        })
        .expect("a render thread starts")
        .join();
    // A panic on the render thread is a finding: let libFuzzer see it.
    if let Err(panic) = painted {
        std::panic::resume_unwind(panic);
    }
});
