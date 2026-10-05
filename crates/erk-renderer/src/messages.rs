//! Everything that crosses between the shell and the renderer.
//!
//! These types become IPC messages when the renderer moves into its own
//! process (M3), so they hold only plain owned data: strings, integers and
//! byte vectors, nothing shared, nothing borrowed, no engine types. Making
//! them serializable should then be one derive line. CI keeps this file to
//! prelude types only (.github/scripts/check-renderer-surface.sh).

/// Messages from the shell to the renderer.
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

/// Messages from the renderer to the shell.
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
    /// A key that types `text`.
    Character(String),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyState {
    Down,
    Up,
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
}

/// The kinds of p1-contract §10, numbered the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Click = 1,
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
