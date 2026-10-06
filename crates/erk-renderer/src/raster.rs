//! The raster (p1-contract §1.1): it paints prepared frames and nothing
//! else. It holds no document, only its font and image tables (tables.rs),
//! filled from the frames' table updates.
//!
//! It paints with vello_cpu into frames for the host, or with vello_hybrid
//! into the host's window (M2.5). Starting the GPU takes seconds on a cold
//! start, so a raster on a window paints on the CPU until the GPU is ready,
//! then paints its last frame again there. When the GPU path is lost it
//! goes back to the CPU, that frame too.
//!
//! [`RasterThread`] runs a raster on a thread of its own, as the embedding
//! layer does.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::list::Prepared;
#[cfg(feature = "gpu")]
use crate::messages::Raster;
use crate::messages::{Frame, Painted};
use crate::paint;
use crate::tables::Tables;

/// How often a raster whose GPU is starting looks whether it is ready.
pub(crate) const GPU_POLL: Duration = Duration::from_millis(50);

/// How a raster starts: on the CPU, or on the CPU until the GPU path it is
/// starting is ready.
pub(crate) enum Start {
    Cpu,
    #[cfg(feature = "gpu")]
    Gpu(Receiver<Result<crate::gpu::Gpu, String>>),
}

/// What draws the frames.
#[cfg(feature = "gpu")]
enum Painter {
    /// vello_cpu: frames with their pixels go to the host.
    Cpu,
    /// vello_hybrid: frames go to the window.
    Gpu(Box<crate::gpu::Gpu>),
}

pub(crate) struct Rasterizer {
    tables: Tables,
    #[cfg(feature = "gpu")]
    painter: Painter,
    #[cfg(feature = "gpu")]
    starting: Option<Receiver<Result<crate::gpu::Gpu, String>>>,
    /// The last frame painted, its table updates applied: painted again
    /// when the GPU path takes over.
    last: Option<Prepared>,
}

impl Rasterizer {
    pub(crate) fn new(start: Start) -> Self {
        Self {
            tables: Tables::default(),
            #[cfg(feature = "gpu")]
            painter: Painter::Cpu,
            #[cfg(feature = "gpu")]
            starting: match start {
                Start::Gpu(starting) => Some(starting),
                Start::Cpu => None,
            },
            #[cfg(not(feature = "gpu"))]
            last: {
                let Start::Cpu = start;
                None
            },
            #[cfg(feature = "gpu")]
            last: None,
        }
    }

    /// Start on `window`'s GPU, painting on the CPU until it is ready. Call
    /// it on the thread that runs the window's event loop: some platforms
    /// give a window's handle only there. Also returns what to tell the
    /// host at once: why it paints on the CPU, when the GPU path cannot
    /// start at all.
    #[cfg(feature = "gpu")]
    pub(crate) fn on_window(
        window: std::sync::Arc<dyn crate::gpu::Window>,
        backends: wgpu::Backends,
    ) -> (Self, Vec<Painted>) {
        // The surface is made here, on the window's thread. Finding an
        // adapter and making a device takes seconds on a cold start (2-3 s
        // measured with a desktop GPU): that runs on a thread of its own.
        let start = match crate::gpu::Gpu::surface(window, backends) {
            Ok(surface) => {
                let (done, starting) = channel();
                let started = std::thread::Builder::new()
                    .name("erk-gpu-start".to_owned())
                    .spawn(move || {
                        let _ = done.send(crate::gpu::Gpu::for_surface(surface));
                    });
                match started {
                    Ok(_) => Ok(Start::Gpu(starting)),
                    Err(error) => Err(format!("no thread to start the GPU path: {error}")),
                }
            }
            Err(reason) => Err(reason),
        };
        match start {
            Ok(start) => (Self::new(start), Vec::new()),
            Err(reason) => (
                Self::new(Start::Cpu),
                vec![Painted::Raster(Raster::Cpu { reason })],
            ),
        }
    }

    /// Whether the GPU path is still starting: the caller should call
    /// [`Rasterizer::poll`] now and then.
    pub(crate) fn starting(&self) -> bool {
        #[cfg(feature = "gpu")]
        return self.starting.is_some();
        #[cfg(not(feature = "gpu"))]
        false
    }

    /// See whether the GPU path has started: if it has, say so and paint
    /// the last frame again there; if it failed, say why.
    pub(crate) fn poll(&mut self) -> Vec<Painted> {
        #[cfg(feature = "gpu")]
        if let Some(done) = &self.starting {
            let result = match done.try_recv() {
                Ok(result) => result,
                Err(std::sync::mpsc::TryRecvError::Empty) => return Vec::new(),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("the GPU start stopped".to_owned())
                }
            };
            self.starting = None;
            return match result {
                Ok(gpu) => {
                    let adapter = gpu.adapter.clone();
                    self.painter = Painter::Gpu(Box::new(gpu));
                    let mut out = vec![Painted::Raster(Raster::Gpu { adapter })];
                    if let Some(last) = self.last.take() {
                        out.extend(self.paint(last));
                    }
                    out
                }
                Err(reason) => vec![Painted::Raster(Raster::Cpu { reason })],
            };
        }
        Vec::new()
    }

    /// Bring the tables up to date for `prepared` and paint it: a frame
    /// with its pixels on the CPU, a presented one on the GPU.
    pub(crate) fn paint(&mut self, mut prepared: Prepared) -> Vec<Painted> {
        self.tables.apply(std::mem::take(&mut prepared.updates));
        let mut out = Vec::new();
        #[cfg(feature = "gpu")]
        if let Painter::Gpu(gpu) = &mut self.painter {
            let drawn = gpu.render(
                &prepared.list,
                &self.tables,
                prepared.width,
                prepared.height,
                prepared.scale,
            );
            match drawn {
                Ok(()) => {
                    out.push(Painted::Presented {
                        width: prepared.width,
                        height: prepared.height,
                    });
                    self.last = Some(prepared);
                    return out;
                }
                // The GPU path is lost: on with vello_cpu, this frame too.
                Err(reason) => {
                    self.painter = Painter::Cpu;
                    out.push(Painted::Raster(Raster::Cpu { reason }));
                }
            }
        }
        let pixmap = paint::paint(
            &prepared.list,
            &self.tables,
            prepared.width,
            prepared.height,
            prepared.scale,
        );
        let frame = Frame::new(
            prepared.width,
            prepared.height,
            pixmap.data_as_u8_slice().to_vec(),
            prepared.list.dump(),
        )
        .painted_with_resources_pending(prepared.pending);
        out.push(Painted::Frame(frame));
        self.last = Some(prepared);
        out
    }

    /// Skip `prepared` for a newer frame: its table updates still apply.
    fn skip(&mut self, prepared: Prepared) {
        self.tables.apply(prepared.updates);
    }
}

enum ToRaster {
    Paint(Prepared),
    Skip(Prepared),
    Shutdown,
}

/// A raster on a thread of its own: frames go in with
/// [`RasterThread::paint`], and what was painted goes to the sink it was
/// started with, on the raster's thread: a channel's sender, or a window
/// event loop's proxy.
pub struct RasterThread {
    to: Sender<ToRaster>,
    handle: Option<JoinHandle<()>>,
}

impl RasterThread {
    /// A raster that paints on the CPU, into frames for the host.
    pub fn cpu(sink: impl FnMut(Painted) + Send + 'static) -> Self {
        Self::start(|| (Rasterizer::new(Start::Cpu), Vec::new()), sink)
    }

    /// A raster that paints into `window` on the GPU (p1-contract §7: the
    /// host's window), on the CPU while the GPU starts or when it cannot.
    /// Call it on the thread that runs the window's event loop: some
    /// platforms give a window's handle only there.
    #[cfg(feature = "gpu")]
    pub fn on_window(
        window: impl crate::gpu::Window,
        sink: impl FnMut(Painted) + Send + 'static,
    ) -> Self {
        Self::on_window_with(
            std::sync::Arc::new(window),
            crate::gpu::WINDOW_BACKENDS,
            sink,
        )
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn on_window_with(
        window: std::sync::Arc<dyn crate::gpu::Window>,
        backends: wgpu::Backends,
        sink: impl FnMut(Painted) + Send + 'static,
    ) -> Self {
        let (rasterizer, told) = Rasterizer::on_window(window, backends);
        Self::start(move || (rasterizer, told), sink)
    }

    fn start(
        make: impl FnOnce() -> (Rasterizer, Vec<Painted>) + Send + 'static,
        mut sink: impl FnMut(Painted) + Send + 'static,
    ) -> Self {
        let (to, inbox) = channel();
        let handle = std::thread::Builder::new()
            .name("erk-raster".to_owned())
            .spawn(move || {
                let (rasterizer, told) = make();
                told.into_iter().for_each(&mut sink);
                run(rasterizer, &inbox, &mut sink);
            })
            .expect("the raster thread starts");
        Self {
            to,
            handle: Some(handle),
        }
    }

    /// Paint `prepared`. A frame still queued when a newer one arrives is
    /// skipped.
    pub fn paint(&self, prepared: Prepared) {
        // A raster that has stopped paints nothing.
        let _ = self.to.send(ToRaster::Paint(prepared));
    }

    /// Do not paint `prepared`, a newer frame will be; its table updates
    /// still reach the raster, which the next frames rely on.
    pub fn skip(&self, prepared: Prepared) {
        let _ = self.to.send(ToRaster::Skip(prepared));
    }
}

impl Drop for RasterThread {
    fn drop(&mut self) {
        let _ = self.to.send(ToRaster::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(mut rasterizer: Rasterizer, inbox: &Receiver<ToRaster>, sink: &mut impl FnMut(Painted)) {
    loop {
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
        let mut out = rasterizer.poll();
        // Only the newest of the queued frames is painted.
        let mut newest = None;
        let mut message = first;
        while let Some(current) = message {
            match current {
                ToRaster::Paint(prepared) => {
                    if let Some(older) = newest.replace(prepared) {
                        rasterizer.skip(older);
                    }
                }
                ToRaster::Skip(prepared) => {
                    // Its updates come before any newer frame's.
                    if let Some(older) = newest.take() {
                        rasterizer.skip(older);
                    }
                    rasterizer.skip(prepared);
                }
                ToRaster::Shutdown => return,
            }
            message = inbox.try_recv().ok();
        }
        if let Some(prepared) = newest {
            out.extend(rasterizer.paint(prepared));
        }
        out.into_iter().for_each(&mut *sink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;

    fn frame(painted: Vec<Painted>) -> Frame {
        painted
            .into_iter()
            .find_map(|painted| match painted {
                Painted::Frame(frame) => Some(frame),
                _ => None,
            })
            .expect("a CPU raster paints frames")
    }

    #[test]
    fn a_skipped_frames_table_updates_still_apply() {
        // The first frame brings the face; the second, painted in its
        // place, names it without sending it again.
        let mut engine = Engine::new();
        engine.load_html(r#"<p style="font-size: 30px">Erk</p>"#);
        engine.resize(120, 80);
        let (first, _) = engine.prepare();
        engine.resize(121, 80);
        let (second, _) = engine.prepare();
        let (first, second) = (first.unwrap(), second.unwrap());
        assert!(!first.updates.is_empty() && second.updates.is_empty());
        let mut skipping = Rasterizer::new(Start::Cpu);
        skipping.skip(first);
        let painted = frame(skipping.paint(second));
        assert!(
            painted.rgba().chunks(4).any(|pixel| pixel[0] < 128),
            "the text is painted"
        );
    }
}

#[cfg(all(test, feature = "gpu"))]
mod gpu_tests {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use wgpu::rwh::{DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle};

    use super::*;
    use crate::engine::Engine;

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
    fn without_the_gpu_path_the_raster_still_paints_frames() {
        for backends in [wgpu::Backends::all(), wgpu::Backends::empty()] {
            let (sink, painted) = channel();
            let raster = RasterThread::on_window_with(Arc::new(NoWindow), backends, move |p| {
                let _ = sink.send(p);
            });
            let patience = Duration::from_secs(60);
            match painted.recv_timeout(patience) {
                Ok(Painted::Raster(Raster::Cpu { reason })) => {
                    assert!(!reason.is_empty(), "{backends:?}");
                }
                Ok(_) => panic!("{backends:?}: the raster was not told first"),
                Err(error) => panic!("{backends:?}: {error:?}"),
            }
            let mut engine = Engine::new();
            engine.load_html(r#"<html style="background: #123456"></html>"#);
            engine.resize(8, 4);
            raster.paint(engine.prepare().0.expect("a frame"));
            let frame = loop {
                match painted.recv_timeout(patience) {
                    Ok(Painted::Frame(frame)) => break frame,
                    Ok(Painted::Presented { .. }) => panic!("presented without a GPU"),
                    Ok(_) => {}
                    Err(error) => panic!("{backends:?}: no frame: {error:?}"),
                }
            };
            assert_eq!(&frame.rgba()[..4], &[0x12, 0x34, 0x56, 255]);
        }
    }
}
