//! The renderer thread: messages in, frames out.

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use erk_renderer::{FromRenderer, ToRenderer, render_html, spawn};

const PAGE: &str = "<h1>Merhaba</h1><p>İş parçacığından.</p>";

/// Long enough for the renderer to paint a small page in a debug build; a
/// wait that ends here is a failure, never a hang.
const PATIENCE: Duration = Duration::from_secs(10);

#[test]
fn a_loaded_document_is_painted_at_the_requested_size() {
    let (to, from, handle) = spawn();
    to.send(ToRenderer::Load {
        html: PAGE.to_owned(),
    })
    .unwrap();
    to.send(ToRenderer::Resize {
        width: 320,
        height: 200,
    })
    .unwrap();

    let FromRenderer::Frame(frame) = from.recv_timeout(PATIENCE).expect("a frame");
    assert_eq!((frame.width(), frame.height()), (320, 200));
    // The same pixels the direct path paints.
    assert_eq!(frame.rgba(), render_html(PAGE, 320, 200).rgba());

    to.send(ToRenderer::Shutdown).unwrap();
    handle.join().unwrap();
}

#[test]
fn nothing_is_painted_before_the_size_is_known() {
    let (to, from, handle) = spawn();
    to.send(ToRenderer::Load {
        html: PAGE.to_owned(),
    })
    .unwrap();
    // Load alone, and time for it to be handled: sending Shutdown right
    // behind it would let the renderer drain both and return before
    // painting, whether or not it knew a size.
    assert_eq!(
        from.recv_timeout(Duration::from_secs(2)).err(),
        Some(RecvTimeoutError::Timeout),
        "a frame was painted with no size"
    );
    to.send(ToRenderer::Shutdown).unwrap();
    handle.join().unwrap();
}

#[test]
fn the_last_resize_wins() {
    let (to, from, handle) = spawn();
    to.send(ToRenderer::Load {
        html: PAGE.to_owned(),
    })
    .unwrap();
    for width in [100, 150, 200, 250] {
        to.send(ToRenderer::Resize { width, height: 80 }).unwrap();
    }
    // Queued resizes may be coalesced, so only the final state is certain.
    let last = loop {
        let FromRenderer::Frame(frame) = from
            .recv_timeout(PATIENCE)
            .expect("the last resize was never painted");
        if frame.width() == 250 {
            break frame;
        }
    };
    assert_eq!(last.height(), 80);
    to.send(ToRenderer::Shutdown).unwrap();
    handle.join().unwrap();
}

#[test]
fn the_thread_stops_when_the_shell_goes_away() {
    let (to, from, handle) = spawn();
    drop(to);
    drop(from);
    handle.join().unwrap();
}
