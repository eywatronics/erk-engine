//! What the engine and the raster say to their host, and what it gives
//! them: input, events, resources, fonts, frames.
//!
//! The same values cross the C ABI (M3.5) and, should the raster ever move
//! into a process of its own, a process boundary, so they hold only plain
//! owned data: strings, integers and byte vectors, nothing shared, nothing
//! borrowed, no engine types. CI keeps this file to prelude types only
//! (.github/scripts/check-renderer-surface.sh).

/// What a raster painted ([`crate::RasterThread`]).
pub enum Painted {
    /// A frame painted on the CPU, with its pixels.
    Frame(Frame),
    /// A frame went to the window on the GPU, `width` × `height` device
    /// pixels: the window already shows it.
    Presented { width: u16, height: u16 },
    /// How the raster draws from now on: first thing on a window, and
    /// again if the GPU path is lost.
    Raster(Raster),
}

/// How the renderer draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Raster {
    /// With vello_hybrid on this GPU adapter, into the window.
    Gpu { adapter: String },
    /// With vello_cpu, into frames the host shows, and why not on the GPU.
    Cpu { reason: String },
}

/// Why a request failed: p1-contract §9's status codes, numbered the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// No node (0), a selector that does not parse.
    InvalidArgument = 1,
    /// The node was removed, or belonged to a document since replaced.
    StaleNode = 2,
}

/// The CSS `cursor` keywords (CSS UI 4 §5.1), and `None` for a hidden
/// pointer. Cursor images are not loaded: their fallback keyword is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Default,
    None,
    ContextMenu,
    Help,
    Pointer,
    Progress,
    Wait,
    /// `cell`.
    CellSelect,
    Crosshair,
    Text,
    VerticalText,
    Alias,
    Copy,
    Move,
    NoDrop,
    NotAllowed,
    Grab,
    Grabbing,
    EResize,
    NResize,
    NeResize,
    NwResize,
    SResize,
    SeResize,
    SwResize,
    WResize,
    EwResize,
    NsResize,
    NeswResize,
    NwseResize,
    ColResize,
    RowResize,
    AllScroll,
    ZoomIn,
    ZoomOut,
}

/// One pointer action at `x`, `y`, in CSS pixels of the viewport (a window
/// sends its physical position divided by its scale).
#[derive(Clone, Debug, PartialEq)]
pub struct PointerInput {
    pub kind: PointerKind,
    pub x: f32,
    pub y: f32,
    /// The button pressed or released; `None` for moves.
    pub button: PointerButton,
    pub modifiers: Modifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Move,
    Down,
    Up,
    /// The pointer left the page.
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    None,
    Primary,
    Secondary,
    Middle,
}

/// The modifier keys held during an input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// One key action. Only the keys the engine acts on are told apart; the
/// rest arrive as `Other` and are ignored until text input (M5).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyInput {
    pub key: Key,
    pub state: KeyState,
    pub modifiers: Modifiers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Tab,
    Enter,
    Space,
    Escape,
    /// The key that deletes backwards: what a host entering text itself
    /// needs (M4 plan, decision 4).
    Backspace,
    /// A key that types `text`.
    Character(String),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Down,
    Up,
}

/// What the last frame did, counted (M5.0): how many elements were styled,
/// boxes laid out, paragraphs shaped and display list items made. M5's
/// incremental steps are measured by how far these fall: styling counts
/// only the elements it styled again (M5.3); layout and the display list
/// still do everything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub styled: usize,
    pub laid_out: usize,
    pub shaped: usize,
    pub items: usize,
    /// The document changes recorded since the frame before (M5.1).
    pub recorded: usize,
    /// What is left of them coalesced: the changes invalidation starts
    /// from.
    pub changes: usize,
}

/// An event on the page, as p1-contract's `ErkEvent` describes it: its
/// kind, the node it happened to, and the path from that node up to the
/// root element, from which the host dispatches the capture, target and
/// bubble phases. Nodes are `NodeId` bits; coordinates are CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub target: u64,
    /// `target` first, then each ancestor element up to the root element.
    pub path: Vec<u64>,
    pub x: f32,
    pub y: f32,
    pub modifiers: Modifiers,
    /// The key of a key event.
    pub key: Option<Key>,
}

/// The kinds of p1-contract §10, numbered the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Click = 1,
    /// A key went down, at the focused element or the body.
    KeyDown = 5,
    /// A key came up.
    KeyUp = 6,
    /// `target` received the focus.
    Focus = 7,
    /// `target` lost the focus.
    Blur = 8,
}

/// What a resource is for (p1-contract §6): the host may serve one URL in
/// several forms, and Erk refuses a response of the wrong kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Image,
    Stylesheet,
    Font,
}

/// A resource the content names: an id for the answer, the URL as the
/// content wrote it, and its kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRequest {
    pub id: u64,
    pub url: String,
    pub kind: ResourceKind,
}

/// The host's answer to request `id`: a MIME type (empty: let the bytes
/// decide) and the bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceResponse {
    pub id: u64,
    pub mime: String,
    pub data: Vec<u8>,
}

/// The fonts the host can provide (p1-contract §6.2). The renderer asks
/// for a face of a family it uses as a `ResourceKind::Font` request for
/// `font:<family>?weight=<weight>&style=<normal|italic>`, the family with
/// `%`, `?`, `&` and `#` percent-encoded; the host answers with the file of
/// the family's face nearest that weight and style.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontCatalog {
    /// Every family the host has, by the name its font files give.
    pub families: Vec<String>,
    /// The families each generic family stands for, in order of preference.
    pub generic: Vec<GenericFamilies>,
    /// The families that draw a writing system the chosen font lacks.
    pub fallback: Vec<ScriptFallback>,
}

/// What a generic family (`serif`, `sans-serif`, `monospace`, `cursive`,
/// `fantasy`, `system-ui`, `emoji`) stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericFamilies {
    pub generic: String,
    pub families: Vec<String>,
}

/// The families for text in `script` (an ISO 15924 code such as `Hani`),
/// in `language` (a BCP 47 tag such as `ja`), or in any language when it is
/// empty.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptFallback {
    pub script: String,
    pub language: String,
    pub families: Vec<String>,
}

/// A rendered page.
pub struct Frame {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
    display_list: String,
    resources_pending: bool,
}

impl Frame {
    /// `rgba` holds `width` × `height` pixels, four bytes each.
    pub(crate) fn new(width: u16, height: u16, rgba: Vec<u8>, display_list: String) -> Self {
        Self {
            width,
            height,
            rgba,
            display_list,
            resources_pending: false,
        }
    }

    /// The same frame, marked as painted while resource requests were still
    /// unanswered.
    pub(crate) fn painted_with_resources_pending(self, pending: bool) -> Self {
        Self {
            resources_pending: pending,
            ..self
        }
    }

    /// Whether the frame was painted while some of the document's resources
    /// were still unanswered: a later frame will show them.
    pub fn resources_pending(&self) -> bool {
        self.resources_pending
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    /// Pixels as premultiplied RGBA8, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The display list the frame was painted from, as text.
    pub fn display_list(&self) -> &str {
        &self.display_list
    }
}

/// Where a text node's text lies on one line, in CSS pixels relative to
/// the viewport, from its first to its last character on that line and as
/// high as its font's box: what Chrome's `Range.getClientRects()` reports
/// for the node.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    /// Position among the text nodes under the body that hold more than
    /// white space, in document order; those inside `<script>`, `<style>`
    /// and `<template>` are not counted.
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// The border box of one element, in CSS pixels relative to the viewport.
/// A forerunner of the inspection queries of M3 (p1-contract §8.1).
#[derive(Clone, Debug, PartialEq)]
pub struct ElementBox {
    /// Position among the body and its descendant elements, in document
    /// order, the body being 0. Elements without a box still count.
    pub index: usize,
    pub tag: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// An element's box in the last frame (p1-contract §8.1, `ErkBox`): its
/// border box in CSS pixels relative to the viewport, and the widths of its
/// margin, border and padding, each top, right, bottom, left.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxModel {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub margin: [f32; 4],
    pub border: [f32; 4],
    pub padding: [f32; 4],
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Document,
    Element,
    Text,
    Comment,
    /// A doctype, a processing instruction, a template's fragment.
    Other,
}

/// A stage of preparing a frame, reported as it ends: the embedding layer
/// times them, the core reads no clock (p1-contract §8.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Styles computed, and resource requests found.
    Style,
    /// Boxes laid out and text shaped.
    Layout,
    /// The display list built.
    DisplayList,
}
