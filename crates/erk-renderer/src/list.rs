//! What crosses from the engine to the raster: the display list, and the
//! font and image tables its items name by number (p1-contract §1.1).
//!
//! The document, its styles and its layout stay on the UI thread; the raster
//! gets only these. So they hold plain owned data: numbers, strings and
//! byte vectors, nothing shared, nothing borrowed, no engine or third-party
//! type. A glyph run names its face by a `FontId` and an image item its
//! pixels by an `ImageId`; the bytes travel once, as a `TableUpdate`, before
//! the first list that uses them. CI keeps this file to prelude types and
//! the types defined in it (.github/scripts/check-renderer-surface.sh).

/// Straight (non-premultiplied) sRGB bytes, `[r, g, b, a]`.
pub(crate) type Rgba = [u8; 4];

pub(crate) struct DisplayList {
    /// The canvas colour behind everything (CSS 2 §14.2).
    pub(crate) canvas: Rgba,
    pub(crate) items: Vec<DisplayItem>,
}

pub(crate) enum DisplayItem {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: Rgba,
    },
    /// A background clipped to rounded corners.
    RoundedRect {
        frame: Frame,
        radii: Radii,
        color: Rgba,
    },
    /// A solid border: the ring between the border box and the padding box,
    /// each side in its own colour.
    Border {
        frame: Frame,
        /// Top, right, bottom, left.
        widths: [f32; 4],
        colors: [Rgba; 4],
        radii: Radii,
    },
    /// An outer box shadow: a blurred rounded rectangle, never painted
    /// inside the box that casts it (`clip`).
    Shadow {
        frame: Frame,
        radius: f32,
        blur: f32,
        color: Rgba,
        clip: Frame,
        clip_radii: Radii,
    },
    /// An image of `size` pixels: one copy fills `tile`, repeated along an
    /// axis where `repeat` says so; `area` is the region painted, clipped
    /// to `clip`.
    Image {
        image: ImageId,
        size: (u16, u16),
        tile: Frame,
        repeat: (bool, bool),
        area: Frame,
        clip: Frame,
        clip_radii: Radii,
    },
    /// A gradient image (CSS Images 3 §3), placed as `Image` is: one copy
    /// fills `tile`, repeated along an axis where `repeat` says so; `area`
    /// is the region painted, clipped to `clip`. The gradient's geometry is
    /// that of the copy at `tile`.
    Gradient {
        gradient: Gradient,
        tile: Frame,
        repeat: (bool, bool),
        area: Frame,
        clip: Frame,
        clip_radii: Radii,
    },
    /// Everything until the matching `PopOpacity` is composited at this
    /// opacity, as one group.
    PushOpacity(f32),
    PopOpacity,
    /// Everything until the matching `PopClip` is clipped to this box,
    /// with these corner radii.
    PushClip {
        frame: Frame,
        radii: Radii,
    },
    PopClip,
    Glyphs(GlyphRun),
    /// Where `node` (a node id's bits) takes pointer input: an element's
    /// border box, or a line of text standing for its element (`text`).
    /// Not painted and not in the dump; sitting in paint order, the last
    /// one under a point is the topmost.
    Hit {
        node: u64,
        frame: Frame,
        text: bool,
    },
    /// The developer tools' highlight of a selected node's boxes: drawn
    /// over the page, not part of the document (p1-contract §8.1).
    Highlight(Frame),
}

/// A box in absolute coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// Corner radii as (horizontal, vertical): top-left, top-right,
/// bottom-right, bottom-left.
pub(crate) type Radii = [(f32, f32); 4];

/// A gradient ready to paint: colours along a line or out from a centre,
/// its stops already fitted so that the first is at offset 0 and the last
/// at 1, which both rasterizers take as they are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Gradient {
    pub(crate) shape: GradientShape,
    /// In order, from offset 0 to offset 1.
    pub(crate) stops: Vec<GradientStop>,
    /// Past the ends the stops repeat (a `repeating-` gradient); otherwise
    /// the end colours go on.
    pub(crate) repeating: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum GradientShape {
    /// Offset 0 at `start`, offset 1 at `end`; the colour is constant
    /// across the line.
    Linear { start: (f32, f32), end: (f32, f32) },
    /// Ellipses around `center`: offset 1 on the one with `radii`, offset 0
    /// on the one `inner` times as large.
    Radial {
        center: (f32, f32),
        radii: (f32, f32),
        inner: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GradientStop {
    pub(crate) offset: f32,
    pub(crate) color: Rgba,
}

pub(crate) struct GlyphRun {
    pub(crate) font: FontId,
    pub(crate) size: f32,
    pub(crate) color: Rgba,
    pub(crate) glyphs: Vec<PositionedGlyph>,
    /// The source text, for dumps and debugging.
    pub(crate) text: String,
}

#[derive(Clone, Copy)]
pub(crate) struct PositionedGlyph {
    pub(crate) id: u32,
    pub(crate) x: f32,
    pub(crate) y: f32,
}

/// A font face, numbered by the engine the first time a list uses it. The
/// numbers are never reused: fonts outlive documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct FontId(pub(crate) u32);

/// A decoded image, numbered by the engine when it arrives. The numbers
/// are never reused, so a stale one names nothing rather than another
/// image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct ImageId(pub(crate) u32);

/// A change to the raster's tables, sent before the list that needs it.
pub(crate) enum TableUpdate {
    /// Face `index` of the font file `data`.
    Font {
        id: FontId,
        data: Vec<u8>,
        index: u32,
    },
    /// An image's pixels: `width` × `height`, premultiplied RGBA rows.
    Image {
        id: ImageId,
        width: u16,
        height: u16,
        rgba: Vec<u8>,
    },
    /// No list will paint this image again (its document is gone).
    ForgetImage(ImageId),
}

/// One frame's work for the raster: the table updates it needs first, then
/// the list, for a `width` × `height` target of device pixels at `scale`
/// device pixels per CSS pixel. Made by the engine on the UI thread, moved
/// to the raster whole.
pub struct Prepared {
    pub(crate) updates: Vec<TableUpdate>,
    pub(crate) list: DisplayList,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) scale: f32,
    /// Whether some of the document's resources were still unanswered:
    /// a later frame will show them.
    pub(crate) pending: bool,
}
