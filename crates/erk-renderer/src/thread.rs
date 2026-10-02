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
use crate::render_document;
use crate::resources::Resources;

/// Layout recurses once per level of nesting, and the parser allows 512
/// levels (as Chrome's does). A debug build needs more than the default 2 MiB
/// for that; this is address space, committed only as it is used.
pub(crate) const STACK_SIZE: usize = 16 * 1024 * 1024;

/// Start the renderer on its own thread.
pub fn spawn() -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    let (to_renderer, inbox) = channel();
    let (outbox, from_renderer) = channel();
    let handle = std::thread::Builder::new()
        .name("erk-renderer".to_owned())
        .stack_size(STACK_SIZE)
        .spawn(move || run(&inbox, &outbox))
        .expect("the renderer thread starts");
    (to_renderer, from_renderer, handle)
}

fn run(inbox: &Receiver<ToRenderer>, outbox: &Sender<FromRenderer>) {
    let mut html: Option<String> = None;
    let mut size: Option<(u16, u16)> = None;
    let mut scale = 1.0;
    // The resources of the current document: asked for once, kept across
    // resizes, dropped with the document; the host's fonts outlive it.
    let mut resources = Resources::default();
    while let Ok(first) = inbox.recv() {
        // Apply everything already queued before painting: during a window
        // drag dozens of resizes arrive, and only the last one matters, and
        // the answers to a batch of resource requests arrive together.
        let mut message = Some(first);
        while let Some(current) = message {
            match current {
                ToRenderer::Load { html: document } => {
                    html = Some(document);
                    resources.new_document();
                }
                ToRenderer::Resize { width, height } => size = Some((width, height)),
                ToRenderer::Scale { factor } => scale = factor,
                ToRenderer::Fonts(catalogue) => resources.set_fonts(catalogue),
                ToRenderer::Resource(response) => resources.complete(&response),
                ToRenderer::ResourceMissing { id } => resources.missing(id),
                ToRenderer::Shutdown => return,
            }
            message = inbox.try_recv().ok();
        }
        if let (Some(document), Some((width, height))) = (&html, size) {
            let (frame, requests) = render_document(document, width, height, scale, &mut resources);
            // The requests first: a host that answers at once has its
            // answers queued before it sees the frame painted without them.
            if !requests.is_empty() && outbox.send(FromRenderer::Resources(requests)).is_err() {
                return;
            }
            let frame = frame.painted_with_resources_pending(resources.pending());
            if outbox.send(FromRenderer::Frame(frame)).is_err() {
                // The shell has gone away.
                return;
            }
        }
    }
}
