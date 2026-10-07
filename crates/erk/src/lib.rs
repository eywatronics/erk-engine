//! Erk: an embedded HTML/CSS UI engine, and its Rust API.
//!
//! An [`App`] is one document and one window (p1-contract §1.2). The thread
//! that makes it is its UI thread: the document, its styles and its layout
//! live there, so a change applies at once and a question is answered at
//! once. Only painting has a thread of its own, the raster, which gets each
//! frame as plain data (p1-contract §1.1). `App` is not `Send`: a call from
//! another thread does not compile. [`AppHandle`] is the way in from other
//! threads.
//!
//! The host subscribes to events on nodes; the callbacks run on the UI
//! thread between frames, with a [`Context`]: the document API, without the
//! loop. An `App` dereferences to its context.
//!
//! [`App::new`] makes an app with a window, which [`App::run`] opens;
//! [`App::headless`] one without, which the host ticks and gives input.
//!
//! ```
//! use std::cell::Cell;
//! use std::rc::Rc;
//!
//! let mut app = erk::App::headless(erk::Config::default()).unwrap();
//! // Buttons get their native look in M5; until then a style sizes them.
//! app.load_html(r#"<button id="b" style="display: inline-block; width: 80px; height: 30px">0</button>"#);
//! let button = app.query(None, "#b").unwrap().unwrap();
//! let count = Rc::new(Cell::new(0));
//! let counted = count.clone();
//! app.on(button, erk::EventKind::Click, move |cx, event| {
//!     counted.set(counted.get() + 1);
//!     cx.set_text(event.target, &counted.get().to_string()).unwrap();
//! })
//! .unwrap();
//! app.tick(0);
//! app.click(20.0, 15.0);
//! assert_eq!(count.get(), 1);
//! assert_eq!(app.text(button).unwrap(), "1");
//! ```
//!
//! A windowed app, as the README shows it: errors are `std::error::Error`s.
//!
//! ```no_run
//! use erk::{App, Config, EventKind};
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let mut app = App::new(Config { title: "Hello".to_owned(), ..Config::default() })?;
//!     app.load_html(r#"<button id="b">Click</button><p id="label">Not yet.</p>"#);
//!     let button = app.query(None, "#b")?.expect("the page has a button");
//!     let label = app.query(None, "#label")?.expect("the page has a label");
//!     app.on(button, EventKind::Click, move |cx, _event| {
//!         cx.set_text(label, "Clicked.").unwrap();
//!     })?;
//!     app.run()?;
//!     Ok(())
//! }
//! ```
//!
//! ```compile_fail
//! fn on_another_thread<T: Send>(_: T) {}
//! on_another_thread(erk::App::headless(erk::Config::default()).unwrap());
//! ```

mod context;
mod events;
mod fonts;
mod handle;
mod ids;
mod window;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use erk_renderer::{Engine, Painted, Prepared, RasterThread, Stage};

pub use context::Context;
pub use erk_renderer::{
    BoxModel, Frame, Key, KeyInput, KeyState, NodeKind, PointerButton, PointerInput, PointerKind,
    ResourceKind, ResourceRequest,
};
pub use events::{Event, EventKind, Modifiers, Phase, Subscription};
pub use handle::{AppHandle, Responder};
pub use ids::Node;

use crate::fonts::SystemFonts;

/// How long a windowless app waits for its raster to paint a frame before
/// it takes the raster to be gone.
const PAINT_PATIENCE: Duration = Duration::from_secs(60);

/// How many times a tick prepares a frame again when answers to its
/// resource requests arrive at once: an image, then a font its alt text
/// would use.
const ROUNDS: usize = 4;

/// What an app starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The viewport in logical (CSS) pixels: the window's inner size, or a
    /// windowless app's frame at `scale`.
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS pixel of a windowless app; a window has its
    /// screen's.
    pub scale: f32,
    /// The window's title.
    pub title: String,
    /// The least severe messages the log callback gets.
    pub log_level: LogLevel,
    /// Draw text with the system's fonts (p1-contract §6.2). Without them
    /// every family is the embedded Noto Sans, the same on every machine,
    /// as tests want.
    pub system_fonts: bool,
    /// Draw the window on the GPU when the machine can; it falls back to the
    /// CPU when it cannot.
    pub gpu: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.0,
            title: "Erk".to_owned(),
            log_level: LogLevel::Warning,
            system_fonts: true,
            gpu: true,
        }
    }
}

/// How severe a log message is; the numbers are `erk.h`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum LogLevel {
    Error = 1,
    Warning = 2,
    Info = 3,
    Debug = 4,
}

/// Why a call failed (p1-contract §8). The numbers are `erk.h`'s
/// `ERK_ERR_*`; `ERK_OK` (0) is `Ok`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Status {
    /// No node, a value out of range, a selector that does not parse.
    InvalidArgument = 1,
    /// The node was removed, belongs to a document since replaced, or to
    /// another app.
    StaleNode = 2,
    /// Called from a thread other than the app's UI thread (C ABI only:
    /// in Rust such a call does not compile).
    WrongThread = 3,
    /// The caller's buffer is too small (C ABI only).
    BufferTooSmall = 4,
    /// No such subscription; the app behind a handle is gone.
    NotFound = 5,
    /// Not allowed inside a callback (C ABI only: a Rust callback gets no
    /// way to call it).
    Reentrant = 6,
    /// The call panicked; the app is poisoned.
    Panic = 7,
    /// An earlier call panicked; only destroying the app works.
    Poisoned = 8,
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidArgument => "invalid argument",
            Self::StaleNode => "stale node",
            Self::WrongThread => "called from a thread other than the app's",
            Self::BufferTooSmall => "buffer too small",
            Self::NotFound => "not found",
            Self::Reentrant => "not allowed inside a callback",
            Self::Panic => "the call panicked",
            Self::Poisoned => "the app is poisoned",
        })
    }
}

impl std::error::Error for Status {}

impl From<erk_renderer::Status> for Status {
    fn from(status: erk_renderer::Status) -> Self {
        match status {
            erk_renderer::Status::InvalidArgument => Self::InvalidArgument,
            erk_renderer::Status::StaleNode => Self::StaleNode,
        }
    }
}

/// Why [`App::run`] could not run: no window could be made, the platform's
/// event loop failed, or the app has no window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunError(pub String);

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RunError {}

/// Input a host gives an app that has no window of its own, or synthetic
/// input for a test. Positions are in CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Pointer(PointerInput),
    Key(KeyInput),
    /// The wheel scrolled by `dx`, `dy` CSS pixels at `x`, `y`; positive
    /// values scroll towards the end of the page.
    Wheel {
        dx: f32,
        dy: f32,
        x: f32,
        y: f32,
    },
}

/// How long the stages of a frame took (p1-contract §8.1), measured by
/// this crate around the engine's stages: the core reads no clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameTimings {
    /// The frame's number, from 1.
    pub frame: u64,
    pub style_ns: u64,
    pub layout_ns: u64,
    pub display_list_ns: u64,
    /// From handing the frame to the raster until it reports it painted.
    pub raster_ns: u64,
}

type ResourceProvider = Box<dyn FnMut(&mut Context, &ResourceRequest, Responder)>;
type Log = Box<dyn FnMut(LogLevel, &str)>;

/// Where an app's frames go.
enum Output {
    /// Frames the host reads; the raster paints on the CPU.
    Headless {
        raster: RasterThread,
        painted: Receiver<Painted>,
        frame: Option<Frame>,
    },
    /// The window, once it is open; its raster draws into it.
    Window(Option<RasterThread>),
}

/// One document and its window. See the crate's documentation.
pub struct App {
    cx: Context,
    output: Output,
    config: Config,
    fonts: Option<Arc<SystemFonts>>,
    resources: Option<ResourceProvider>,
    log: Option<Log>,
    /// The frame with the raster, and when it went there.
    painting: Option<(FrameTimings, Instant)>,
    /// The last frame painted.
    timings: Option<FrameTimings>,
    frames: u64,
    /// Not `Send`, not `Sync`: the UI thread's alone.
    _ui_thread: PhantomData<*const ()>,
}

impl App {
    /// An app with a window, which [`App::run`] opens. `InvalidArgument` for
    /// an empty viewport.
    pub fn new(config: Config) -> Result<Self, Status> {
        if config.width == 0 || config.height == 0 {
            return Err(Status::InvalidArgument);
        }
        Ok(Self::with_output(
            config,
            Engine::new(),
            Output::Window(None),
        ))
    }

    /// An app without a window: it paints on the CPU into frames the host
    /// reads with [`App::frame`], and takes its input from [`App::input`].
    /// For tests, offscreen rendering and hosts that draw the frames
    /// themselves. `InvalidArgument` for a viewport that is empty, larger
    /// than 65535 device pixels a side, or a scale that is not a positive
    /// number.
    pub fn headless(config: Config) -> Result<Self, Status> {
        let scale = config.scale;
        // A scale that is not a positive number gives no side in range.
        let side = |logical: u32| {
            let device = (logical as f32 * scale).round();
            (1.0..=f32::from(u16::MAX))
                .contains(&device)
                .then_some(device as u16)
                .ok_or(Status::InvalidArgument)
        };
        let (width, height) = (side(config.width)?, side(config.height)?);
        let mut engine = Engine::new();
        engine.set_scale(scale);
        engine.resize(width, height);
        let (sink, painted) = std::sync::mpsc::channel();
        let raster = RasterThread::cpu(move |frame| {
            let _ = sink.send(frame);
        });
        let output = Output::Headless {
            raster,
            painted,
            frame: None,
        };
        Ok(Self::with_output(config, engine, output))
    }

    fn with_output(config: Config, mut engine: Engine, output: Output) -> Self {
        let fonts = config.system_fonts.then(|| Arc::new(SystemFonts::scan()));
        if let Some(fonts) = &fonts {
            engine.set_fonts(fonts.catalogue().clone());
        }
        Self {
            cx: Context::new(engine, ids::new_app_key()),
            output,
            config,
            fonts,
            resources: None,
            log: None,
            painting: None,
            timings: None,
            frames: 0,
            _ui_thread: PhantomData,
        }
    }

    /// Open the window and run until it closes (p1-contract §7: Erk's own
    /// event loop). A panic in a callback closes the window and comes out of
    /// here. One window at a time: the platform gives a process one event
    /// loop.
    pub fn run(&mut self) -> Result<(), RunError> {
        if !matches!(self.output, Output::Window(None)) {
            return Err(RunError(
                "only an app made with App::new runs a window".to_owned(),
            ));
        }
        window::run(self)
    }

    /// Answer the document's resource requests (p1-contract §6): `provide`
    /// is called on the UI thread, between frames, once per URL, and may use
    /// the document as any callback may. Without a provider no resource
    /// loads. Font faces come from the system's fonts and never reach it.
    pub fn set_resource_provider(
        &mut self,
        provide: impl FnMut(&mut Context, &ResourceRequest, Responder) + 'static,
    ) {
        self.resources = Some(Box::new(provide));
    }

    /// Where Erk's own messages go (a refused resource, how the window
    /// draws), as severe as the config's `log_level` or more.
    pub fn set_log(&mut self, log: impl FnMut(LogLevel, &str) + 'static) {
        self.log = Some(Box::new(log));
    }

    /// Give the page input, as a window would. Events it causes reach their
    /// subscriptions before this returns; a panic in one of them comes out
    /// of here.
    pub fn input(&mut self, input: Input) {
        let events = match input {
            Input::Pointer(pointer) => self.cx.engine.pointer(&pointer),
            Input::Key(key) => self.cx.engine.key(&key),
            Input::Wheel { dx, dy, x, y } => {
                self.cx.engine.wheel((dx, dy), (x, y));
                Vec::new()
            }
        };
        for event in events {
            self.cx.dispatch(event);
        }
    }

    /// A primary-button click at `x`, `y`: down, then up.
    pub fn click(&mut self, x: f32, y: f32) {
        for kind in [PointerKind::Down, PointerKind::Up] {
            self.input(Input::Pointer(PointerInput {
                kind,
                x,
                y,
                button: PointerButton::Primary,
                modifiers: Modifiers::default(),
            }));
        }
    }

    /// Run a turn of the app at `now_ns`, the host's monotonic clock in
    /// nanoseconds (p1-contract §7: the core reads no clock). Work posted
    /// from other threads and answers to resource requests are handled
    /// first; then, if something that shows has changed, the frame is
    /// prepared and painted, and a windowless app waits for its pixels.
    /// Nothing in the engine depends on time yet; the caret (M5) and
    /// transitions (M9) will, through this.
    pub fn tick(&mut self, now_ns: u64) {
        let _ = now_ns;
        // A window not yet open has no raster to paint with.
        if matches!(self.output, Output::Window(None)) {
            self.cx.drain();
            self.flush_log();
            return;
        }
        let prepared = self.turn();
        self.flush_log();
        let Some((prepared, timings)) = prepared else {
            return;
        };
        self.painting = Some((timings, Instant::now()));
        match &mut self.output {
            Output::Headless {
                raster,
                painted,
                frame,
            } => {
                raster.paint(prepared);
                while let Ok(done) = painted.recv_timeout(PAINT_PATIENCE) {
                    if let Painted::Frame(done) = done {
                        *frame = Some(done);
                        self.painted();
                        return;
                    }
                }
                panic!("the raster thread stopped");
            }
            Output::Window(Some(raster)) => raster.paint(prepared),
            Output::Window(None) => unreachable!("checked above"),
        }
    }

    /// Handle what arrived, then prepare the frame if something that shows
    /// has changed. Resource requests go to the system's fonts and the
    /// host's provider; when answers arrive at once the frame is prepared
    /// again with them, and the one prepared without them only brings the
    /// raster's tables up to date.
    fn turn(&mut self) -> Option<(Prepared, FrameTimings)> {
        for round in 1..=ROUNDS {
            self.cx.drain();
            self.flush_log();
            let start = Instant::now();
            let mut ends = [Duration::ZERO; 3];
            let (prepared, requests) = self.cx.engine.prepare_marked(&mut |stage| {
                let at = match stage {
                    Stage::Style => 0,
                    Stage::Layout => 1,
                    Stage::DisplayList => 2,
                };
                ends[at] = start.elapsed();
            });
            let nanos = |d: Duration| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX);
            let timings = FrameTimings {
                frame: 0,
                style_ns: nanos(ends[0]),
                layout_ns: nanos(ends[1].saturating_sub(ends[0])),
                display_list_ns: nanos(ends[2].saturating_sub(ends[1])),
                raster_ns: 0,
            };
            for request in requests {
                self.request(&request);
            }
            let answered = self.cx.collect();
            let last = round == ROUNDS || !answered;
            match prepared {
                Some(prepared) if !last => self.skip(prepared),
                Some(prepared) => return Some((prepared, timings)),
                None if last => return None,
                None => {}
            }
        }
        None
    }

    /// Ask for `request`: a font face of the system's, or the host's
    /// resource. With no one to answer, it is missing at once.
    fn request(&mut self, request: &ResourceRequest) {
        let responder = self.cx.responder(request.id);
        match (request.kind, &self.fonts, &mut self.resources) {
            (ResourceKind::Font, Some(fonts), _) => match fonts.data(&request.url) {
                // The bytes say what kind of font file it is.
                Some(data) => responder.respond("", data),
                None => responder.missing(),
            },
            (ResourceKind::Font, None, _) | (_, _, None) => responder.missing(),
            (_, _, Some(provide)) => provide(&mut self.cx, request, responder),
        }
    }

    fn skip(&self, prepared: Prepared) {
        match &self.output {
            Output::Headless { raster, .. } | Output::Window(Some(raster)) => raster.skip(prepared),
            Output::Window(None) => {}
        }
    }

    /// Pass the engine's warnings to the log.
    fn flush_log(&mut self) {
        for warning in self.cx.engine.take_warnings() {
            self.log(LogLevel::Warning, &warning);
        }
    }

    pub(crate) fn log(&mut self, level: LogLevel, message: &str) {
        if let Some(log) = &mut self.log
            && level <= self.config.log_level
        {
            log(level, message);
        }
    }

    /// The raster has painted the frame it was given: its timings are
    /// complete.
    pub(crate) fn painted(&mut self) {
        if let Some((mut timings, sent)) = self.painting.take() {
            self.frames += 1;
            timings.frame = self.frames;
            timings.raster_ns = u64::try_from(sent.elapsed().as_nanos()).unwrap_or(u64::MAX);
            self.timings = Some(timings);
        }
    }

    /// How long the last painted frame's stages took (p1-contract §8.1,
    /// `erk_last_frame_timings`); `None` before the first.
    pub fn last_frame_timings(&self) -> Option<FrameTimings> {
        self.timings
    }

    /// The last frame a windowless app painted.
    pub fn frame(&self) -> Option<&Frame> {
        match &self.output {
            Output::Headless { frame, .. } => frame.as_ref(),
            Output::Window(_) => None,
        }
    }
}

impl Deref for App {
    type Target = Context;

    fn deref(&self) -> &Context {
        &self.cx
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut Context {
        &mut self.cx
    }
}
