//! Erk: an embedded HTML/CSS UI engine, and its Rust API.
//!
//! An [`App`] is one document and one window (p1-contract §1.2). The thread
//! that makes it is its UI thread: the document, its styles and its layout
//! live there, so a change applies at once and a question is answered at
//! once. Only painting has a thread of its own, the raster, which gets each
//! frame as plain data (p1-contract §1.1). `App` is not `Send`: a call from
//! another thread does not compile.
//!
//! ```
//! let mut app = erk::App::headless(erk::Config::default()).unwrap();
//! app.load_html("<p>Merhaba</p>");
//! let p = app.query(None, "p").unwrap().unwrap();
//! app.set_text(p, "Merhaba, Erk").unwrap();
//! app.tick(0);
//! assert_eq!(app.text(p).unwrap(), "Merhaba, Erk");
//! assert!(app.frame().is_some());
//! ```
//!
//! ```compile_fail
//! fn on_another_thread<T: Send>(_: T) {}
//! on_another_thread(erk::App::headless(erk::Config::default()).unwrap());
//! ```

mod ids;

use std::marker::PhantomData;
use std::time::Duration;

use erk_renderer::{Engine, Painted, RasterThread};

pub use erk_renderer::{
    Frame, Key, KeyInput, KeyState, Modifiers, PointerButton, PointerInput, PointerKind,
};
pub use ids::Node;

/// How long a windowless app waits for its raster to paint a frame before
/// it takes the raster to be gone.
const PAINT_PATIENCE: Duration = Duration::from_secs(60);

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.0,
            title: "Erk".to_owned(),
        }
    }
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
    NotFound = 5,
    /// Not allowed inside a callback.
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

/// One document and its window. See the crate's documentation.
pub struct App {
    engine: Engine,
    raster: RasterThread,
    /// The key this app's node ids are mixed with (p1-contract §2).
    key: u64,
    /// The last frame painted, for a windowless app.
    frame: Option<Frame>,
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
            engine,
            raster: RasterThread::cpu(),
            key: ids::new_app_key(),
            frame: None,
            _ui_thread: PhantomData,
        })
    }

    /// Show `html` instead of the document. Every node id of the document
    /// before goes stale (p1-contract §2).
    pub fn load_html(&mut self, html: &str) {
        self.engine.load_html(html);
    }

    /// The document node.
    pub fn root(&self) -> Node {
        self.outward(self.engine.root())
            .expect("the document node has an id")
    }

    /// The first element inside `scope` (the whole document for `None`)
    /// matching the CSS selector list `selector`, in document order.
    pub fn query(&self, scope: Option<Node>, selector: &str) -> Result<Option<Node>, Status> {
        let scope = scope.map(|node| self.inward(node));
        let found = self.engine.query(scope, selector)?;
        Ok(found.and_then(|id| self.outward(id)))
    }

    /// Set `node`'s text as the DOM's `textContent` does: an element's
    /// children are replaced by one text node. The next frame shows it.
    pub fn set_text(&mut self, node: Node, text: &str) -> Result<(), Status> {
        let id = self.inward(node);
        Ok(self.engine.set_text(id, text)?)
    }

    /// `node`'s text as the DOM's `textContent` reads it.
    pub fn text(&self, node: Node) -> Result<String, Status> {
        Ok(self.engine.text(self.inward(node))?)
    }

    /// Give the page input, as a window would.
    pub fn input(&mut self, input: Input) {
        match input {
            Input::Pointer(pointer) => {
                self.engine.pointer(&pointer);
            }
            Input::Key(key) => {
                self.engine.key(&key);
            }
            Input::Wheel { dx, dy, x, y } => self.engine.wheel((dx, dy), (x, y)),
        }
    }

    /// Run a turn of the app at `now_ns`, the host's monotonic clock in
    /// nanoseconds (p1-contract §7: the core reads no clock). If something
    /// that shows has changed, the frame is prepared and painted; a
    /// windowless app waits for its pixels. Nothing in the engine depends on
    /// time yet; the caret (M5) and transitions (M9) will, through this.
    pub fn tick(&mut self, now_ns: u64) {
        let _ = now_ns;
        let (prepared, _requests) = self.engine.prepare();
        let Some(prepared) = prepared else {
            return;
        };
        self.raster.paint(prepared);
        while let Some(painted) = self.raster.recv_timeout(PAINT_PATIENCE) {
            if let Painted::Frame(frame) = painted {
                self.frame = Some(frame);
                return;
            }
        }
        panic!("the raster thread stopped");
    }

    /// The last frame a windowless app painted.
    pub fn frame(&self) -> Option<&Frame> {
        self.frame.as_ref()
    }

    fn inward(&self, node: Node) -> u64 {
        ids::inward(node, self.key)
    }

    fn outward(&self, id: u64) -> Option<Node> {
        ids::outward(id, self.key)
    }
}
