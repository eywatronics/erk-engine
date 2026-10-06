//! The renderer as a thread that talks only through messages: the engine
//! (engine.rs) and a raster (raster.rs) on one thread, behind
//! [`ToRenderer`] and [`FromRenderer`], both plain owned data (see
//! messages.rs).
//!
//! This is M0's demo path, kept until the shell moves onto `erk` (M3.3): the
//! embedding layer keeps the engine on its UI thread and gives the raster a
//! thread of its own (p1-contract §1.1).

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;

use crate::engine::Engine;
use crate::messages::{Cursor, FromRenderer, Painted, ToRenderer};
use crate::raster::{GPU_POLL, Rasterizer, Start};

/// Start the renderer on its own thread. It paints with vello_cpu and sends
/// each frame's pixels.
pub fn spawn() -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    start(|| (Rasterizer::new(Start::Cpu), Vec::new()))
}

/// Start the renderer on its own thread, drawing into `window` on the GPU
/// (M2.5; the host's window, p1-contract §7). Call it on the thread that
/// runs the window's event loop: some platforms give a window's handle only
/// there. Its first message says how it draws: when there is no GPU adapter
/// or the window gives no surface, it falls back to vello_cpu and sends
/// frames as [`spawn`]'s renderer does.
#[cfg(feature = "gpu")]
pub fn spawn_on_window(
    window: impl crate::gpu::Window,
) -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    spawn_on_window_with(std::sync::Arc::new(window), crate::gpu::WINDOW_BACKENDS)
}

#[cfg(feature = "gpu")]
pub(crate) fn spawn_on_window_with(
    window: std::sync::Arc<dyn crate::gpu::Window>,
    backends: wgpu::Backends,
) -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    // The surface is made here, on the window's thread.
    let (rasterizer, told) = Rasterizer::on_window(window, backends);
    start(move || (rasterizer, told))
}

fn start(
    make: impl FnOnce() -> (Rasterizer, Vec<Painted>) + Send + 'static,
) -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    let (to_renderer, inbox) = channel();
    let (outbox, from_renderer) = channel();
    let handle = std::thread::Builder::new()
        .name("erk-renderer".to_owned())
        .spawn(move || {
            let (rasterizer, told) = make();
            if send(&outbox, told.into_iter().map(painted)) {
                run(&inbox, &outbox, rasterizer);
            }
        })
        .expect("the renderer thread starts");
    (to_renderer, from_renderer, handle)
}

/// Send every message; false when the shell has gone away.
fn send(outbox: &Sender<FromRenderer>, messages: impl IntoIterator<Item = FromRenderer>) -> bool {
    messages
        .into_iter()
        .all(|message| outbox.send(message).is_ok())
}

fn painted(painted: Painted) -> FromRenderer {
    match painted {
        Painted::Frame(frame) => FromRenderer::Frame(frame),
        Painted::Presented { width, height } => FromRenderer::Presented { width, height },
        Painted::Raster(raster) => FromRenderer::Raster(raster),
    }
}

fn run(inbox: &Receiver<ToRenderer>, outbox: &Sender<FromRenderer>, mut rasterizer: Rasterizer) {
    let mut engine = Engine::new();
    // No frame until a document is loaded.
    let mut loaded = false;
    // The cursor the shell was last told to show.
    let mut cursor = Cursor::Default;
    loop {
        // While the GPU path starts, the renderer wakes now and then to see
        // whether it is ready; the raster then paints its last frame there.
        let first = if rasterizer.starting() {
            match inbox.recv_timeout(GPU_POLL) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match inbox.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        };
        if !send(outbox, rasterizer.poll().into_iter().map(painted)) {
            return;
        }
        // Apply everything already queued before painting: during a window
        // drag dozens of resizes arrive, and only the last one matters, and
        // the answers to a batch of resource requests arrive together.
        // Input and questions are answered at once; a frame follows only a
        // change that shows.
        let mut message = first;
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
            if !send(outbox, reply) {
                return;
            }
            message = inbox.try_recv().ok();
        }
        if loaded && engine.needs_frame() {
            let (prepared, requests) = engine.prepare();
            // The requests first: a host that answers at once has its
            // answers queued before it sees the frame painted without them.
            if !requests.is_empty() && outbox.send(FromRenderer::Resources(requests)).is_err() {
                return;
            }
            if let Some(prepared) = prepared
                && !send(outbox, rasterizer.paint(prepared).into_iter().map(painted))
            {
                return;
            }
        }
        // After the frame: a restyle (`:hover`) may change the cursor.
        let now = if loaded {
            engine.cursor()
        } else {
            Cursor::Default
        };
        if now != cursor {
            cursor = now;
            if outbox.send(FromRenderer::Cursor(now)).is_err() {
                return;
            }
        }
    }
}

#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use wgpu::rwh::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    use super::*;
    use crate::messages::Raster;

    /// A window that gives no handle, as one on a machine where the GPU
    /// path cannot start.
    #[derive(Debug)]
    struct NoWindow;

    impl HasWindowHandle for NoWindow {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    impl HasDisplayHandle for NoWindow {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Err(HandleError::Unavailable)
        }
    }

    #[test]
    fn without_the_gpu_path_the_renderer_still_sends_frames() {
        for backends in [wgpu::Backends::all(), wgpu::Backends::empty()] {
            let (to, from, renderer) = spawn_on_window_with(Arc::new(NoWindow), backends);
            let patience = Duration::from_secs(60);
            match from.recv_timeout(patience) {
                Ok(FromRenderer::Raster(Raster::Cpu { reason })) => {
                    assert!(!reason.is_empty(), "{backends:?}");
                }
                Ok(_) => panic!("{backends:?}: the raster was not told first"),
                Err(error) => panic!("{backends:?}: {error:?}"),
            }
            to.send(ToRenderer::Load {
                html: r#"<html style="background: #123456"></html>"#.to_owned(),
            })
            .unwrap();
            to.send(ToRenderer::Resize {
                width: 8,
                height: 4,
            })
            .unwrap();
            let frame = loop {
                match from.recv_timeout(patience) {
                    Ok(FromRenderer::Frame(frame)) => break frame,
                    Ok(FromRenderer::Presented { .. }) => panic!("presented without a GPU"),
                    Ok(_) => {}
                    Err(error) => panic!("{backends:?}: no frame: {error:?}"),
                }
            };
            assert_eq!(&frame.rgba()[..4], &[0x12, 0x34, 0x56, 255]);
            to.send(ToRenderer::Shutdown).unwrap();
            renderer.join().unwrap();
        }
    }
}
