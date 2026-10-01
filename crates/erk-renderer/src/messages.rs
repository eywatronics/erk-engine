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
    /// engine core does no I/O (.github/scripts/check-core-io.sh).
    Load { html: String },
    /// The viewport is now `width` × `height` device pixels.
    Resize { width: u16, height: u16 },
    /// The screen now has `factor` device pixels per CSS pixel (1 until
    /// told otherwise; a window moved to a HiDPI screen sends 2).
    Scale { factor: f32 },
    /// The host's answer to a resource request.
    Resource(ResourceResponse),
    /// The host has no resource for request `id`; the page renders
    /// without it.
    ResourceMissing { id: u64 },
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
