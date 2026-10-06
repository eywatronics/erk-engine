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
//! ```compile_fail
//! fn on_another_thread<T: Send>(_: T) {}
//! on_another_thread(erk::App::headless(erk::Config::default()).unwrap());
//! ```

mod context;
mod events;
mod handle;
mod ids;

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

use erk_renderer::{Engine, Painted, RasterThread};

pub use context::Context;
pub use erk_renderer::{
    Frame, Key, KeyInput, KeyState, PointerButton, PointerInput, PointerKind, ResourceKind,
    ResourceRequest,
};
pub use events::{Event, EventKind, Modifiers, Phase, Subscription};
pub use handle::{AppHandle, Responder};
pub use ids::Node;

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
    /// The viewport in logical (CSS) pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels per CSS pixel.
    pub scale: f32,
    /// The window's title.
    pub title: String,
    /// The least severe messages the log callback gets.
    pub log_level: LogLevel,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.0,
            title: "Erk".to_owned(),
            log_level: LogLevel::Warning,
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

impl From<erk_renderer::Status> for Status {
    fn from(status: erk_renderer::Status) -> Self {
        match status {
            erk_renderer::Status::InvalidArgument => Self::InvalidArgument,
            erk_renderer::Status::StaleNode => Self::StaleNode,
        }
    }
}

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

type ResourceProvider = Box<dyn FnMut(&ResourceRequest, Responder)>;
type Log = Box<dyn FnMut(LogLevel, &str)>;

/// One document and its window. See the crate's documentation.
pub struct App {
    cx: Context,
    raster: RasterThread,
    /// The last frame painted, for a windowless app.
    frame: Option<Frame>,
    resources: Option<ResourceProvider>,
    log: Option<Log>,
    log_level: LogLevel,
    /// Not `Send`, not `Sync`: the UI thread's alone.
    _ui_thread: PhantomData<*const ()>,
}

impl App {
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
        Ok(Self {
            cx: Context::new(engine, ids::new_app_key()),
            raster: RasterThread::cpu(),
            frame: None,
            resources: None,
            log: None,
            log_level: config.log_level,
            _ui_thread: PhantomData,
        })
    }

    /// Answer the document's resource requests (p1-contract §6): `provide`
    /// is called on the UI thread, between frames, once per URL. Without a
    /// provider no resource loads.
    pub fn set_resource_provider(
        &mut self,
        provide: impl FnMut(&ResourceRequest, Responder) + 'static,
    ) {
        self.resources = Some(Box::new(provide));
    }

    /// Where Erk's own messages go (a refused resource, say), as severe as
    /// the config's `log_level` or more.
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
        for round in 1..=ROUNDS {
            self.cx.drain();
            self.flush_log();
            let (prepared, requests) = self.cx.engine.prepare();
            let mut answered = false;
            for request in requests {
                let responder = Responder {
                    id: request.id,
                    to: Some(self.cx.sender()),
                };
                match &mut self.resources {
                    Some(provide) => provide(&request, responder),
                    // No provider, no resource: answered missing at once.
                    None => {
                        responder.missing();
                        answered = true;
                    }
                }
            }
            answered |= self.cx.collect();
            let last = round == ROUNDS || !answered;
            match prepared {
                // Answered at once: prepare again with the answers rather
                // than paint a frame without them. Its table updates still
                // go to the raster.
                Some(prepared) if !last => self.raster.skip(prepared),
                Some(prepared) => {
                    self.paint(prepared);
                    break;
                }
                None if last => break,
                None => {}
            }
        }
        self.flush_log();
    }

    fn paint(&mut self, prepared: erk_renderer::Prepared) {
        self.raster.paint(prepared);
        while let Some(painted) = self.raster.recv_timeout(PAINT_PATIENCE) {
            if let Painted::Frame(frame) = painted {
                self.frame = Some(frame);
                return;
            }
        }
        panic!("the raster thread stopped");
    }

    /// Pass the engine's warnings to the log.
    fn flush_log(&mut self) {
        let warnings = self.cx.engine.take_warnings();
        if let Some(log) = &mut self.log
            && LogLevel::Warning <= self.log_level
        {
            for warning in warnings {
                log(LogLevel::Warning, &warning);
            }
        }
    }

    /// The last frame a windowless app painted.
    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
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
