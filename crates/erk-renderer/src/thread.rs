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

#[cfg(feature = "gpu")]
use crate::messages::Raster;
use crate::messages::{Cursor, FromRenderer, Status, ToRenderer};
use crate::page::Page;
use crate::resources::Resources;

/// Layout recurses once per level of nesting, and the parser allows 512
/// levels (as Chrome's does). A debug build needs more than the default 2 MiB
/// for that; this is address space, committed only as it is used.
pub(crate) const STACK_SIZE: usize = 16 * 1024 * 1024;

/// Start the renderer on its own thread. It paints with vello_cpu and sends
/// each frame's pixels.
pub fn spawn() -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    start(|_| Start::Cpu)
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
    // The surface is made here, on the window's thread. Finding an adapter
    // and making a device takes seconds on a cold start (2-3 s measured with
    // a desktop GPU): that runs on a thread of its own while the renderer
    // paints on the CPU, and the renderer switches when it is done.
    let surface = crate::gpu::Gpu::surface(window, backends);
    start(move |outbox| match surface {
        Ok(surface) => {
            let (done, starting) = channel();
            let started = std::thread::Builder::new()
                .name("erk-gpu-start".to_owned())
                .spawn(move || {
                    let _ = done.send(crate::gpu::Gpu::for_surface(surface));
                });
            match started {
                Ok(_) => Start::Gpu(starting),
                Err(error) => {
                    let reason = format!("no thread to start the GPU path: {error}");
                    let _ = outbox.send(FromRenderer::Raster(Raster::Cpu { reason }));
                    Start::Cpu
                }
            }
        }
        Err(reason) => {
            let _ = outbox.send(FromRenderer::Raster(Raster::Cpu { reason }));
            Start::Cpu
        }
    })
}

/// How the renderer starts: on the CPU, or on the CPU until the GPU path
/// it is starting is ready.
enum Start {
    Cpu,
    #[cfg(feature = "gpu")]
    Gpu(Receiver<Result<crate::gpu::Gpu, String>>),
}

fn start(
    painter: impl FnOnce(&Sender<FromRenderer>) -> Start + Send + 'static,
) -> (Sender<ToRenderer>, Receiver<FromRenderer>, JoinHandle<()>) {
    let (to_renderer, inbox) = channel();
    let (outbox, from_renderer) = channel();
    let handle = std::thread::Builder::new()
        .name("erk-renderer".to_owned())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            let painter = painter(&outbox);
            run(&inbox, &outbox, painter);
        })
        .expect("the renderer thread starts");
    (to_renderer, from_renderer, handle)
}

/// What draws the frames.
#[cfg(feature = "gpu")]
enum Painter {
    /// vello_cpu: frames with their pixels go to the host.
    Cpu,
    /// vello_hybrid: frames go to the window.
    Gpu(Box<crate::gpu::Gpu>),
}

fn run(inbox: &Receiver<ToRenderer>, outbox: &Sender<FromRenderer>, start: Start) {
    // Every renderer starts painting on the CPU.
    #[cfg(feature = "gpu")]
    let mut painter = Painter::Cpu;
    #[cfg(feature = "gpu")]
    let mut starting = match start {
        Start::Gpu(starting) => Some(starting),
        Start::Cpu => None,
    };
    #[cfg(not(feature = "gpu"))]
    let Start::Cpu = start;
    // Parsed once when loaded; every frame paints it again.
    let mut page: Option<Page> = None;
    let mut size: Option<(u16, u16)> = None;
    let mut scale = 1.0;
    // The resources of the current document: asked for once, kept across
    // resizes, dropped with the document; the host's fonts outlive it.
    let mut resources = Resources::default();
    // The cursor the shell was last told to show.
    let mut cursor = Cursor::Default;
    loop {
        let mut changed = false;
        // While the GPU path starts, the renderer wakes now and then to see
        // whether it is ready, and paints the page again there.
        #[cfg(feature = "gpu")]
        let waiting = if let Some(done) = &starting {
            match done.try_recv() {
                Ok(result) => {
                    starting = None;
                    let raster = match result {
                        Ok(gpu) => {
                            let adapter = gpu.adapter.clone();
                            painter = Painter::Gpu(Box::new(gpu));
                            changed = true;
                            Raster::Gpu { adapter }
                        }
                        Err(reason) => Raster::Cpu { reason },
                    };
                    if outbox.send(FromRenderer::Raster(raster)).is_err() {
                        return;
                    }
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    starting = None;
                    let reason = "the GPU start stopped".to_owned();
                    if outbox
                        .send(FromRenderer::Raster(Raster::Cpu { reason }))
                        .is_err()
                    {
                        return;
                    }
                    false
                }
            }
        } else {
            false
        };
        #[cfg(not(feature = "gpu"))]
        let waiting = false;
        let first = if waiting {
            match inbox.recv_timeout(std::time::Duration::from_millis(50)) {
                Ok(message) => Some(message),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        } else if changed {
            inbox.try_recv().ok()
        } else {
            match inbox.recv() {
                Ok(message) => Some(message),
                Err(_) => return,
            }
        };
        // Apply everything already queued before painting: during a window
        // drag dozens of resizes arrive, and only the last one matters, and
        // the answers to a batch of resource requests arrive together.
        // Input and questions are answered at once; a frame follows only a
        // change that shows.
        let mut message = first;
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
            #[cfg(feature = "gpu")]
            if let Painter::Gpu(gpu) = &mut painter {
                let (list, requests) = page.prepare(width, height, scale, &mut resources);
                if !requests.is_empty() && outbox.send(FromRenderer::Resources(requests)).is_err() {
                    return;
                }
                let drawn = gpu.render(&list, width, height, crate::device_scale(scale));
                let message = match drawn {
                    Ok(()) => FromRenderer::Presented { width, height },
                    // The GPU path is lost: on with vello_cpu, this frame too.
                    Err(reason) => {
                        painter = Painter::Cpu;
                        FromRenderer::Raster(Raster::Cpu { reason })
                    }
                };
                let presented = matches!(message, FromRenderer::Presented { .. });
                if outbox.send(message).is_err() {
                    return;
                }
                if presented {
                    changed = false;
                }
            }
            if changed {
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

#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use wgpu::rwh::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    use super::*;

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
