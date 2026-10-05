//! The renderer as a thread that talks only through messages.
//!
//! The shell and the renderer share no mutable state: the shell sends
//! [`ToRenderer`] and receives [`FromRenderer`], both plain owned data (see
//! messages.rs). When the renderer moves into its own process (M3), the
//! channel becomes IPC and the messages gain a serialization derive; see
//! docs/design/p0-architecture.md §2.2.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use erk_dom::NodeId;

use crate::messages::{Cursor, FromRenderer, Status, ToRenderer};
use crate::page::Page;
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
    // Parsed once when loaded; every frame paints it again.
    let mut page: Option<Page> = None;
    let mut size: Option<(u16, u16)> = None;
    let mut scale = 1.0;
    // The resources of the current document: asked for once, kept across
    // resizes, dropped with the document; the host's fonts outlive it.
    let mut resources = Resources::default();
    // The cursor the shell was last told to show.
    let mut cursor = Cursor::Default;
    while let Ok(first) = inbox.recv() {
        // Apply everything already queued before painting: during a window
        // drag dozens of resizes arrive, and only the last one matters, and
        // the answers to a batch of resource requests arrive together.
        // Input and questions are answered at once; a frame follows only a
        // change that shows.
        let mut changed = false;
        let mut message = Some(first);
        while let Some(current) = message {
            // Input that changes what the user is doing (hover, press,
            // focus) in a way some selector depends on is painted again.
            let before = page.as_ref().map(Page::interaction);
            let reply = match current {
                ToRenderer::Load { html } => {
                    // The same arena: the old document's ids go stale.
                    match page.as_mut() {
                        Some(page) => page.load(&html),
                        None => page = Some(Page::parse(&html)),
                    }
                    resources.new_document();
                    changed = true;
                    Vec::new()
                }
                ToRenderer::Resize { width, height } => {
                    size = Some((width, height));
                    changed = true;
                    Vec::new()
                }
                ToRenderer::Scale { factor } => {
                    scale = factor;
                    changed = true;
                    Vec::new()
                }
                ToRenderer::Fonts(catalogue) => {
                    resources.set_fonts(catalogue);
                    changed = true;
                    Vec::new()
                }
                ToRenderer::Resource(response) => {
                    resources.complete(&response);
                    changed = true;
                    Vec::new()
                }
                ToRenderer::ResourceMissing { id } => {
                    resources.missing(id);
                    changed = true;
                    Vec::new()
                }
                ToRenderer::Pointer(input) => page.as_mut().map_or_else(Vec::new, |page| {
                    page.pointer(&input)
                        .into_iter()
                        .map(FromRenderer::Event)
                        .collect()
                }),
                ToRenderer::Wheel { dx, dy, x, y } => {
                    if let Some(page) = page.as_mut() {
                        changed |= page.wheel((dx, dy), (x, y));
                    }
                    Vec::new()
                }
                ToRenderer::Key(input) => page.as_mut().map_or_else(Vec::new, |page| {
                    page.key(&input)
                        .into_iter()
                        .map(FromRenderer::Event)
                        .collect()
                }),
                ToRenderer::InspectAt { request, x, y } => vec![FromRenderer::Inspected {
                    request,
                    node: page
                        .as_ref()
                        .and_then(|page| page.hit_test(x, y))
                        .map(NodeId::to_bits),
                }],
                ToRenderer::Highlight { node } => {
                    if let Some(page) = page.as_mut() {
                        page.set_highlight(node);
                        changed = true;
                    }
                    Vec::new()
                }
                ToRenderer::Query {
                    request,
                    scope,
                    selector,
                } => vec![FromRenderer::QueryResult {
                    request,
                    result: page.as_ref().map_or(Ok(None), |page| {
                        page.query(scope, &selector)
                            .map(|node| node.map(NodeId::to_bits))
                    }),
                }],
                ToRenderer::SetText {
                    request,
                    node,
                    text,
                } => {
                    let result = page
                        .as_mut()
                        .map_or(Err(Status::StaleNode), |page| page.set_text(node, &text));
                    changed |= result.is_ok();
                    vec![FromRenderer::Done { request, result }]
                }
                ToRenderer::Shutdown => return,
            };
            if let (Some(page), Some(before)) = (page.as_ref(), before.as_ref()) {
                changed |= page.shows_change_from(before);
            }
            for answer in reply {
                if outbox.send(answer).is_err() {
                    return;
                }
            }
            message = inbox.try_recv().ok();
        }
        if changed && let (Some(page), Some((width, height))) = (page.as_mut(), size) {
            let (frame, requests) = page.render(width, height, scale, &mut resources);
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
        // After the frame: a restyle (`:hover`) may change the cursor.
        let now = page.as_ref().map_or(Cursor::Default, Page::cursor);
        if now != cursor {
            cursor = now;
            if outbox.send(FromRenderer::Cursor(now)).is_err() {
                return;
            }
        }
    }
}
