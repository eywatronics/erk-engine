//! Any bytes, read as HTML, render without a panic. Pages are painted on one
//! long-lived thread with the renderer thread's stack, as the shell paints
//! them: deep documents need the stack, and a thread per input would leak
//! the thread-local data some dependencies never free when a thread ends
//! (about 14 KB a thread, which LeakSanitizer reports; the shell has one
//! renderer thread). A finding goes into `crates/erk-renderer/tests/robustness/`,
//! where the corpus test keeps it.

#![no_main]

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;

/// The renderer thread's stack (erk-renderer, thread.rs).
const RENDERER_STACK: usize = 16 * 1024 * 1024;

/// The render thread: pages go in, a token comes back when one is painted.
struct Renderer {
    pages: Sender<String>,
    painted: Receiver<()>,
}

fn renderer() -> &'static Mutex<Renderer> {
    static RENDERER: OnceLock<Mutex<Renderer>> = OnceLock::new();
    RENDERER.get_or_init(|| {
        let (pages, inbox) = channel::<String>();
        let (outbox, painted) = channel();
        std::thread::Builder::new()
            .name("erk-renderer".to_owned())
            .stack_size(RENDERER_STACK)
            .spawn(move || {
                for html in inbox {
                    // CANARY, removed in the next commit.
                    assert!(html.len() % 7 != 3, "fuzz canary: length {}", html.len());
                    erk_renderer::render_html(&html, 320, 240);
                    if outbox.send(()).is_err() {
                        return;
                    }
                }
            })
            .expect("the render thread starts");
        Mutex::new(Renderer { pages, painted })
    })
}

fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data).into_owned();
    let renderer = renderer().lock().expect("one input at a time");
    renderer
        .pages
        .send(html)
        .expect("the render thread is running");
    // A panic on the render thread aborts the process through the panic
    // hook libfuzzer-sys installs: libFuzzer reports it and keeps the input.
    renderer
        .painted
        .recv()
        .expect("the render thread is running");
});
