//! The renderer as a thread that talks only through messages.
//!
//! The shell and the renderer share no mutable state: the shell sends
//! [`ToRenderer`] and receives [`FromRenderer`], both plain owned data (see
//! messages.rs). When the renderer moves into its own process (M3), the
//! channel becomes IPC and the messages gain a serialization derive; see
//! docs/design/p0-architecture.md §2.2.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use crate::messages::{FromRenderer, ToRenderer};
use crate::render_html;

/// Start the renderer on its own thread.
pub fn spawn() -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    let (to_renderer, inbox) = channel();
    let (outbox, from_renderer) = channel();
    let handle = std::thread::Builder::new()
        .name("erk-renderer".to_owned())
        .spawn(move || run(&inbox, &outbox))
        .expect("the renderer thread starts");
    (to_renderer, from_renderer, handle)
}

fn run(inbox: &Receiver<ToRenderer>, outbox: &Sender<FromRenderer>) {
    let mut html: Option<String> = None;
    let mut size: Option<(u16, u16)> = None;
    while let Ok(first) = inbox.recv() {
        // Apply everything already queued before painting: during a window
        // drag dozens of resizes arrive, and only the last one matters.
        let mut message = Some(first);
        while let Some(current) = message {
            match current {
                ToRenderer::Load { html: document } => html = Some(document),
                ToRenderer::Resize { width, height } => size = Some((width, height)),
                ToRenderer::Shutdown => return,
            }
            message = inbox.try_recv().ok();
        }
        if let (Some(document), Some((width, height))) = (&html, size) {
            let frame = render_html(document, width, height);
            if outbox.send(FromRenderer::Frame(frame)).is_err() {
                // The shell has gone away.
                return;
            }
        }
    }
}
