//! The renderer's M0 message protocol, kept for the tests: a thread that
//! runs the engine and a raster behind two channels, as `spawn` did until
//! the shell moved onto `erk` (M3.3). The tests drive the engine through it
//! as a host drives an app, and wait for what it says with a timeout.

#![allow(dead_code)]

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

pub use erk_renderer::{Cursor, Event};
use erk_renderer::{
    Engine, FontCatalog, Frame, KeyInput, Painted, PointerInput, Raster, RasterThread,
    ResourceRequest, ResourceResponse, Status,
};

/// Messages from the test, as host, to the renderer.
#[derive(Debug)]
pub enum ToRenderer {
    /// Show this document. The host reads files, not the renderer: the
    /// engine core does no I/O (.github/scripts/check-core-io.sh). Every
    /// node of the document before it is removed: their ids go stale.
    Load { html: String },
    /// The viewport is now `width` × `height` device pixels.
    Resize { width: u16, height: u16 },
    /// The screen now has `factor` device pixels per CSS pixel (1 until
    /// told otherwise; a window moved to a HiDPI screen sends 2).
    Scale { factor: f32 },
    /// The fonts the host can provide; until it is sent, every family is
    /// the embedded Noto Sans. Kept across documents.
    Fonts(FontCatalog),
    /// The host's answer to a resource request.
    Resource(ResourceResponse),
    /// The host has no resource for request `id`; the page renders
    /// without it.
    ResourceMissing { id: u64 },
    /// Pointer input over the page.
    Pointer(PointerInput),
    /// A key went down or up while the page has the keyboard.
    Key(KeyInput),
    /// The wheel (or a touchpad) scrolled by `dx`, `dy` CSS pixels at `x`,
    /// `y`; positive values scroll towards the end of the page.
    Wheel { dx: f32, dy: f32, x: f32, y: f32 },
    /// Which node is under the point `x`, `y` (CSS pixels)? Answered with
    /// `FromRenderer::Inspected` (p1-contract §8.1, `erk_inspect_at`).
    InspectAt { request: u64, x: f32, y: f32 },
    /// Draw the developer tools' highlight over `node`'s boxes, or remove it
    /// (`None`). It is drawn over the page, never added to the document.
    Highlight { node: Option<u64> },
    /// The first element inside `scope` (the whole document for `None`)
    /// matching the CSS selector list `selector`, answered with
    /// `FromRenderer::QueryResult` (`erk_query`).
    Query {
        request: u64,
        scope: Option<u64>,
        selector: String,
    },
    /// Set `node`'s text as the DOM's `textContent` does (`erk_node_set_text`,
    /// the first of M4's mutations), answered with `FromRenderer::Done`.
    SetText {
        request: u64,
        node: u64,
        text: String,
    },
    /// Stop the renderer thread.
    Shutdown,
}

/// Messages from the renderer to the test.
pub enum FromRenderer {
    /// A newly painted frame of the current document at the current size.
    Frame(Frame),
    /// The document names resources the host has not been asked for yet.
    /// Sent before the frame that is painted without them.
    Resources(Vec<ResourceRequest>),
    /// Something happened on the page the host may answer (a click, a
    /// change of focus).
    Event(Event),
    /// The answer to `ToRenderer::InspectAt`: the topmost node there, if any.
    Inspected { request: u64, node: Option<u64> },
    /// The answer to `ToRenderer::Query`: the node, if one matches.
    QueryResult {
        request: u64,
        result: Result<Option<u64>, Status>,
    },
    /// The answer to a change of the document (`ToRenderer::SetText`).
    Done {
        request: u64,
        result: Result<(), Status>,
    },
    /// The pointer should now look like this (the `cursor` property of
    /// what it is over). Sent when it changes.
    Cursor(Cursor),
    /// How a renderer started on a window draws: first thing, and again if
    /// the GPU path is lost and it falls back.
    Raster(Raster),
    /// A frame went to the window on the GPU, `width` × `height` device
    /// pixels: there is no `Frame` to show, the window already shows it.
    Presented { width: u16, height: u16 },
}

/// Start the engine and a raster that paints on the CPU behind the
/// protocol's channels.
pub fn spawn() -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    let (to_renderer, inbox) = channel();
    let (outbox, from_renderer) = channel();
    let handle = std::thread::Builder::new()
        .name("erk-test-renderer".to_owned())
        .spawn(move || run(&inbox, &outbox))
        .expect("the test renderer thread starts");
    (to_renderer, from_renderer, handle)
}

fn run(inbox: &Receiver<ToRenderer>, outbox: &Sender<FromRenderer>) {
    let frames = outbox.clone();
    let raster = RasterThread::cpu(move |painted| {
        let _ = frames.send(match painted {
            Painted::Frame(frame) => FromRenderer::Frame(frame),
            Painted::Presented { width, height } => FromRenderer::Presented { width, height },
            Painted::Raster(raster) => FromRenderer::Raster(raster),
        });
    });
    let mut engine = Engine::new();
    // No frame until a document is loaded.
    let mut loaded = false;
    let mut cursor = Cursor::Default;
    let send = |messages: Vec<FromRenderer>| {
        messages
            .into_iter()
            .all(|message| outbox.send(message).is_ok())
    };
    while let Ok(first) = inbox.recv() {
        // Everything queued is applied before a frame, as the renderer
        // thread did.
        let mut message = Some(first);
        while let Some(current) = message {
            let reply = match current {
                ToRenderer::Load { html } => {
                    engine.load_html(&html);
                    loaded = true;
                    Vec::new()
                }
                ToRenderer::Resize { width, height } => {
                    engine.resize(width, height);
                    Vec::new()
                }
                ToRenderer::Scale { factor } => {
                    engine.set_scale(factor);
                    Vec::new()
                }
                ToRenderer::Fonts(catalogue) => {
                    engine.set_fonts(catalogue);
                    Vec::new()
                }
                ToRenderer::Resource(response) => {
                    engine.complete_resource(&response);
                    Vec::new()
                }
                ToRenderer::ResourceMissing { id } => {
                    engine.resource_missing(id);
                    Vec::new()
                }
                ToRenderer::Pointer(input) => engine
                    .pointer(&input)
                    .into_iter()
                    .map(FromRenderer::Event)
                    .collect(),
                ToRenderer::Wheel { dx, dy, x, y } => {
                    engine.wheel((dx, dy), (x, y));
                    Vec::new()
                }
                ToRenderer::Key(input) => engine
                    .key(&input)
                    .into_iter()
                    .map(FromRenderer::Event)
                    .collect(),
                ToRenderer::InspectAt { request, x, y } => vec![FromRenderer::Inspected {
                    request,
                    node: engine.inspect_at(x, y),
                }],
                ToRenderer::Highlight { node } => {
                    engine.highlight(node);
                    Vec::new()
                }
                ToRenderer::Query {
                    request,
                    scope,
                    selector,
                } => vec![FromRenderer::QueryResult {
                    request,
                    result: engine.query(scope, &selector),
                }],
                ToRenderer::SetText {
                    request,
                    node,
                    text,
                } => vec![FromRenderer::Done {
                    request,
                    result: engine.set_text(node, &text),
                }],
                ToRenderer::Shutdown => return,
            };
            if !send(reply) {
                return;
            }
            message = inbox.try_recv().ok();
        }
        if loaded && engine.needs_frame() {
            let (prepared, requests) = engine.prepare();
            // The requests first: a host that answers at once has its
            // answers queued before it sees the frame painted without them.
            if !requests.is_empty() && !send(vec![FromRenderer::Resources(requests)]) {
                return;
            }
            if let Some(prepared) = prepared {
                raster.paint(prepared);
            }
        }
        let now = if loaded {
            engine.cursor()
        } else {
            Cursor::Default
        };
        if now != cursor {
            cursor = now;
            if !send(vec![FromRenderer::Cursor(now)]) {
                return;
            }
        }
    }
}
