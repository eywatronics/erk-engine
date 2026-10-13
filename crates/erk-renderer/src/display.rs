//! The display list: what to paint, in paint order, in absolute coordinates.
//!
//! Items are flat and own their data, so a list can later cross a process
//! boundary to a compositor. M0 has backgrounds and glyph runs; borders,
//! images, clips and the spatial tree come with the features that need them.
//!
//! Paint order follows CSS 2 Appendix E. Within a stacking context: the
//! positioned descendants with a negative `z-index`, then every in-flow
//! block background in tree order, then all inline content (each
//! paragraph's inline backgrounds, then its text), then the positioned
//! descendants with `z-index: auto` or 0 in tree order, then those with a
//! positive `z-index`. Painting a paragraph's text right after its own
//! background would let a later sibling's background cover text that
//! overflows into it.
//!
//! Each positioned element is painted as a unit, its own context. For
//! `z-index: auto` CSS lets positioned descendants inside it join the outer
//! context's order; Erk keeps them inside, which differs only when such a
//! descendant has a `z-index` meant to reach past its ancestor. An atomic
//! inline's own background is painted with the block backgrounds, before
//! the text of its line; Appendix E paints it in line order, which differs
//! only where they overlap.
//!
//! Clipping (scroll.rs) does not follow these phases: one clipping box's
//! content lands in several of them, and an absolutely positioned box may
//! escape a clip its parent is in. So every item is built tagged with the
//! scope it is painted in, and a last pass brackets runs of items in the
//! clips of their scopes.

use std::fmt::Write as _;

use erk_dom::{Document, NodeId, local_name};
use erk_style::style::computed_values::visibility::T as Visibility;
use erk_style::{ComputedValues, Styles};
use parley::PositionedLayoutItem;

use crate::color::srgb_bytes;
use crate::layout::{Layouts, ShapedText};
pub(crate) use crate::list::{
    DisplayItem, DisplayList, Frame, GlyphRun, Gradient, GradientShape, PositionedGlyph, Radii,
    Rgba,
};
use crate::resources::{Resources, image_url};
use crate::scroll::{Scrolling, VIEWPORT};
use crate::text::{DecorationRect, InlineLayout};

/// The part of a text node on one line, in CSS pixels relative to the
/// viewport: from its first to its last placed cluster (white space at the
/// end of the line left out), as high as its font's box. Chrome reports the
/// same rectangles for a text node with `Range.getClientRects()`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextFragment {
    pub(crate) node: NodeId,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

/// The colour of the highlight overlay, as browsers' developer tools tint a
/// selected element's box.
pub(crate) const HIGHLIGHT: Rgba = [111, 168, 220, 166];

/// An overlay scroll bar's thumb, shown while the pointer is over its
/// scroll container: its thickness, its gap to the edge, its shortest length.
const THUMB: (f32, f32, f32) = (6.0, 2.0, 20.0);
const THUMB_COLOR: Rgba = [0, 0, 0, 102];

/// An item and the scope it is painted in (`None`: unclipped).
type Tagged = (Option<usize>, DisplayItem);

const WHITE: Rgba = [255, 255, 255, 255];

/// What stays the same for every box during one display-list build.
struct Walk<'a> {
    doc: &'a Document,
    styles: &'a Styles,
    layouts: &'a Layouts,
    resources: &'a Resources,
    scrolling: &'a Scrolling,
    /// The scroll containers whose scroll bars are shown.
    bars: &'a [NodeId],
    /// The element whose background became the canvas colour.
    canvas_source: Option<NodeId>,
    /// The text fragments met so far.
    text: std::cell::RefCell<Vec<TextFragment>>,
    /// What is left out of the list: what lies wholly outside the band
    /// (in the document's coordinates) for what moves with the document,
    /// outside the viewport for what does not. `None`: nothing is.
    band: Option<Frame>,
    viewport: Option<Frame>,
    /// Whether the items being made now move with the document.
    moving: std::cell::Cell<bool>,
    /// Where each box and what it holds may paint: a box whose extent
    /// misses the view is not walked. `None`: every box is.
    extents: Option<&'a Extents>,
    /// How many transforms the walk is inside: their items are in the
    /// coordinates before the transform, and none is left out.
    transformed: std::cell::Cell<usize>,
}

impl Walk<'_> {
    /// Where items show, in the coordinates of the items being made now;
    /// `None` if anything may.
    fn view(&self) -> Option<Frame> {
        if self.transformed.get() > 0 {
            None
        } else if self.moving.get() {
            self.band
        } else {
            self.viewport
        }
    }

    /// The items made from now on are painted in `scope`.
    fn painting_in(&self, scope: Option<usize>) {
        self.moving.set(self.scrolling.moves(scope));
    }

    /// Whether an item with `ink` may show.
    fn shows(&self, ink: Frame) -> bool {
        self.view().is_none_or(|view| meets(view, ink))
    }
}

/// Whether `a` and `b` overlap, edges included.
pub(crate) fn meets(a: Frame, b: Frame) -> bool {
    a.x <= b.x + b.width && b.x <= a.x + a.width && a.y <= b.y + b.height && b.y <= a.y + a.height
}

/// Where a box's or an inline background's item may paint, or take
/// pointer input: its shape, which antialiasing does not paint past, and
/// a shadow's blur. `None` for an item that is always kept.
pub(crate) fn ink(item: &DisplayItem) -> Option<Frame> {
    match item {
        DisplayItem::Rect {
            x,
            y,
            width,
            height,
            ..
        } => Some(Frame {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        }),
        DisplayItem::RoundedRect { frame, .. }
        | DisplayItem::Border { frame, .. }
        | DisplayItem::Hit { frame, .. } => Some(*frame),
        // A blur of radius r spreads a Gaussian of deviation r / 2 (CSS
        // Backgrounds 3 §7.1), which fades out by 3 deviations.
        DisplayItem::Shadow { frame, blur, .. } => Some(around(*frame, 1.5 * blur)),
        DisplayItem::Image { area, .. } | DisplayItem::Gradient { area, .. } => Some(*area),
        _ => None,
    }
}

/// How far past its border box a box's outer shadows may paint: moved,
/// spread and blurred as `box_decoration` and `ink` make them.
fn shadow_reach(style: &ComputedValues) -> f32 {
    style
        .get_effects()
        .box_shadow
        .0
        .iter()
        .filter(|shadow| !shadow.inset)
        .map(|shadow| {
            shadow
                .base
                .horizontal
                .px()
                .abs()
                .max(shadow.base.vertical.px().abs())
                + shadow.spread.px().max(0.0)
                + 1.5 * shadow.base.blur.0.px()
        })
        .fold(0.0, f32::max)
}

/// `frame` grown by `by` on every side.
fn around(frame: Frame, by: f32) -> Frame {
    Frame {
        x: frame.x - by,
        y: frame.y - by,
        width: frame.width + 2.0 * by,
        height: frame.height + 2.0 * by,
    }
}

/// Where a paragraph's text may paint or take the pointer, from the top
/// of its content box down: its glyph runs as `glyph_runs` keeps them,
/// which holds their font boxes and so the fragments `text_fragments`
/// measures (no font rises 3 em or falls 2), and its inline backgrounds;
/// each moved by the relatively positioned element it is in. Made once
/// with the paragraph's layout, so that a paragraph out of view costs one
/// comparison: walking a line's runs adds up their clusters.
pub(crate) fn text_band(
    layout: &InlineLayout,
    relative: &[(f32, f32)],
    decorations: &[DecorationRect],
) -> (f32, f32) {
    let moved = |relative_index: u16| {
        usize::from(relative_index)
            .checked_sub(1)
            .and_then(|at| relative.get(at))
            .map_or(0.0, |(_, dy)| *dy)
    };
    let mut band = (f32::INFINITY, f32::NEG_INFINITY);
    let mut cover = |top: f32, bottom: f32| band = (band.0.min(top), band.1.max(bottom));
    for (index, line) in layout.layout.lines().enumerate() {
        let shift = layout.shifts.get(index).copied().unwrap_or(0.0);
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let size = run.run().font_size();
            let baseline = shift + run.baseline() - run.style().brush.raise
                + moved(run.style().brush.relative);
            cover(baseline - 3.0 * size, baseline + 2.0 * size);
        }
    }
    for rect in decorations {
        let top = rect.y + moved(rect.relative);
        cover(top, top + rect.height);
    }
    band
}

/// The part of the document a list is made for, in the document's
/// coordinates, when the viewport is `viewport` and the document is
/// scrolled by `offset`: the cell of a grid of viewports the offset is in,
/// and the next one. The viewport, a viewport's size from an offset in the
/// first cell, always lies in it, and it is the same for every offset in
/// the cell: a scroll inside it changes nothing in the list but where it
/// is painted (p2-incremental §3.7.1).
pub(crate) fn band(viewport: Frame, offset: (f32, f32)) -> Frame {
    let cell = |offset: f32, size: f32| {
        if size > 0.0 {
            (offset / size).floor() * size
        } else {
            offset
        }
    };
    Frame {
        x: viewport.x + cell(offset.0, viewport.width),
        y: viewport.y + cell(offset.1, viewport.height),
        width: 2.0 * viewport.width,
        height: 2.0 * viewport.height,
    }
}

/// Whether the band from `top` to `bottom` meets `view` across: text is
/// left out by lines, which run across the page.
fn crosses(view: Frame, top: f32, bottom: f32) -> bool {
    top <= view.y + view.height && view.y <= bottom
}

/// What a list leaves out: what lies outside `view` (the viewport; the band
/// around it for what moves with the document), and with `extents`, every
/// box whose extent misses it without walking it.
pub(crate) struct Cull<'a> {
    pub(crate) view: Frame,
    pub(crate) extents: Option<&'a Extents>,
}

/// Where each node and everything it holds may paint or take the pointer,
/// top to bottom, in the coordinates its items are made in: its box and
/// its outer shadows, its text and its anonymous paragraphs by their bands,
/// and its descendants'. `None` where a box's items are in other
/// coordinates: a fixed box's (the viewport's) or a transformed one's (before
/// the transform), and so every box that holds one. Made with every new
/// layout or scroll of a container, in one walk of the document, so that a
/// list for another band walks only the boxes that may meet it.
pub(crate) struct Extents(Vec<Option<(f32, f32)>>);

impl Extents {
    pub(crate) fn new(
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
        scrolling: &Scrolling,
    ) -> Self {
        // Children after their parents: walked backwards, a node comes
        // after its children.
        let mut order = Vec::new();
        let mut stack = vec![doc.root()];
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(doc.children(id));
        }
        let mut extents: Vec<Option<(f32, f32)>> = vec![None; doc.capacity_hint()];
        let empty = (f32::INFINITY, f32::NEG_INFINITY);
        let union = |a: (f32, f32), b: (f32, f32)| (a.0.min(b.0), a.1.max(b.1));
        for &id in order.iter().rev() {
            let style = styles.computed(id);
            let elsewhere = style.as_ref().is_some_and(|style| {
                style.clone_position() == erk_style::style::computed_values::position::T::Fixed
                    || crate::transform::transforms(style)
            });
            let mut extent = Some(empty);
            if let Some(layout) = layouts.get(id)
                && let Some((_, y)) = scrolling.position(id)
            {
                let reach = style.as_ref().map_or(0.0, |style| shadow_reach(style));
                let own = (y - reach, y + layout.size.height + reach);
                extent = extent.map(|extent| union(extent, own));
                // Text is measured from the box unscrolled: a box that
                // scrolls its text clips it, and its box holds what shows.
                if let Some(shaped) = layouts.text(id) {
                    let top = y + layout.border.top + layout.padding.top;
                    let band = (top + shaped.band.0, top + shaped.band.1);
                    extent = extent.map(|extent| union(extent, band));
                }
                for anonymous in layouts.anonymous(id) {
                    let top = y + anonymous.layout.location.y;
                    let band = (top + anonymous.text.band.0, top + anonymous.text.band.1);
                    extent = extent.map(|extent| union(extent, band));
                }
            }
            for child in doc.children(id) {
                let child = extents[child.index() as usize];
                extent = extent
                    .zip(child)
                    .map(|(extent, child)| union(extent, child));
            }
            extents[id.index() as usize] = if elsewhere { None } else { extent };
        }
        Self(extents)
    }

    /// `id`'s extent; `None` if it cannot be bounded.
    fn of(&self, id: NodeId) -> Option<(f32, f32)> {
        self.0.get(id.index() as usize).copied().flatten()
    }
}

/// A display list before the document's scroll: what moves with the
/// document in the document's coordinates (`moves`), the rest in the
/// viewport's, and without the document's scroll bars. Nothing in it
/// depends on the document's offset but the band it was made for, so a
/// scroll inside the band needs only [`Content::scrolled`] (p2-incremental
/// §3.7.1).
pub(crate) struct Content {
    canvas: Rgba,
    items: Vec<DisplayItem>,
    moves: Vec<bool>,
    /// The band it was made for; `None`: the whole document.
    pub(crate) band: Option<Frame>,
}

impl Content {
    /// How many items it has.
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    /// What [`DisplayList::build`] lists, before the document's scroll.
    pub(crate) fn build(
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
        resources: &Resources,
        scrolling: &Scrolling,
        bars: &[NodeId],
        cull: Option<Cull<'_>>,
    ) -> (Self, Vec<TextFragment>) {
        let view = cull.as_ref().map(|cull| cull.view);
        let mut list = DisplayList {
            canvas: WHITE,
            items: Vec::new(),
        };
        let band = view.map(|view| band(view, scrolling.document_offset()));
        let walk = Walk {
            doc,
            styles,
            layouts,
            resources,
            scrolling,
            bars,
            canvas_source: list.propagate_canvas_background(doc, styles, layouts),
            text: std::cell::RefCell::new(Vec::new()),
            band,
            viewport: view,
            moving: std::cell::Cell::new(true),
            extents: cull.and_then(|cull| cull.extents),
            transformed: std::cell::Cell::new(0),
        };
        let items = stacking_context(&walk, doc.root());
        let (items, moves) = clipped(items, scrolling);
        let content = Self {
            canvas: list.canvas,
            items,
            moves,
            band,
        };
        (content, walk.text.into_inner())
    }

    /// The list, with what moves with the document moved by its offset
    /// and the document's scroll bars over everything.
    pub(crate) fn scrolled(&self, scrolling: &Scrolling, bars: &[NodeId]) -> DisplayList {
        let (x, y) = scrolling.document_offset();
        let mut items: Vec<DisplayItem> = self
            .items
            .iter()
            .zip(&self.moves)
            .map(|(item, moves)| {
                let mut item = item.clone();
                if *moves {
                    shift(&mut item, (-x, -y));
                }
                item
            })
            .collect();
        if bars.contains(&scrolling.scopes[VIEWPORT].node) {
            items.extend(scroll_bars(scrolling, VIEWPORT));
        }
        DisplayList {
            canvas: self.canvas,
            items,
        }
    }
}

/// `item` moved by `by`, all of it: the boxes, glyphs and gradients in
/// it, and a transform so that what it holds, moved alike, lands where it
/// did.
fn shift(item: &mut DisplayItem, (dx, dy): (f32, f32)) {
    let frame = |frame: &mut Frame| {
        frame.x += dx;
        frame.y += dy;
    };
    match item {
        DisplayItem::Rect { x, y, .. } => {
            *x += dx;
            *y += dy;
        }
        DisplayItem::RoundedRect { frame: f, .. }
        | DisplayItem::Border { frame: f, .. }
        | DisplayItem::PushClip { frame: f, .. }
        | DisplayItem::Hit { frame: f, .. }
        | DisplayItem::Highlight(f) => frame(f),
        DisplayItem::Shadow { frame: f, clip, .. } => {
            frame(f);
            frame(clip);
        }
        DisplayItem::Image {
            tile, area, clip, ..
        } => {
            frame(tile);
            frame(area);
            frame(clip);
        }
        DisplayItem::Gradient {
            gradient,
            tile,
            area,
            clip,
            ..
        } => {
            let point = |point: &mut (f32, f32)| {
                point.0 += dx;
                point.1 += dy;
            };
            match &mut gradient.shape {
                GradientShape::Linear { start, end } => {
                    point(start);
                    point(end);
                }
                GradientShape::Radial { center, .. } => point(center),
            }
            frame(tile);
            frame(area);
            frame(clip);
        }
        DisplayItem::Glyphs(run) => {
            for glyph in &mut run.glyphs {
                glyph.x += dx;
                glyph.y += dy;
            }
        }
        // A point p of what it holds is now p + d: the matrix M becomes
        // p -> M(p - d) + d.
        DisplayItem::PushTransform([a, b, c, d, e, f]) => {
            *e += dx - (*a * dx + *c * dy);
            *f += dy - (*b * dx + *d * dy);
        }
        DisplayItem::PushOpacity(_)
        | DisplayItem::PopOpacity
        | DisplayItem::PopTransform
        | DisplayItem::PopClip => {}
    }
}

impl DisplayList {
    /// The list of a laid-out document, and where each text node's text
    /// lies, line by line: not painted, kept for the renderer's tests and
    /// the inspection queries.
    ///
    /// With a `view`, the viewport's own rectangle, what lies wholly
    /// outside it is left out: its text fragments too. What moves with
    /// the document is left out by the band around the viewport instead.
    pub(crate) fn build(
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
        resources: &Resources,
        scrolling: &Scrolling,
        bars: &[NodeId],
        view: Option<Frame>,
    ) -> (Self, Vec<TextFragment>) {
        let cull = view.map(|view| Cull {
            view,
            extents: None,
        });
        let (content, text) =
            Content::build(doc, styles, layouts, resources, scrolling, bars, cull);
        (content.scrolled(scrolling, bars), text)
    }

    /// The root element's background paints the whole canvas; if it has
    /// none, the body's does (CSS 2 §14.2). Only elements that generate a
    /// box take part: a `display: none` root or body propagates nothing.
    /// Returns the element whose background was used, so it is not painted a
    /// second time.
    fn propagate_canvas_background(
        &mut self,
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
    ) -> Option<NodeId> {
        let html = child_element(doc, doc.root(), &local_name!("html"))?;
        let body = child_element(doc, html, &local_name!("body"));
        for id in std::iter::once(html).chain(body) {
            layouts.get(id)?;
            let style = styles.computed(id)?;
            let color = background(&style);
            if color[3] != 0 {
                self.canvas = color;
                return Some(id);
            }
        }
        None
    }

    /// A readable, stable text form of the list, one item per line. Useful
    /// when a golden image changes and the question is what moved.
    pub(crate) fn dump(&self) -> String {
        let mut out = format!("canvas {}\n", hex(self.canvas));
        for item in &self.items {
            match item {
                DisplayItem::Rect {
                    x,
                    y,
                    width,
                    height,
                    color,
                } => {
                    let _ = writeln!(out, "rect {x} {y} {width}x{height} {}", hex(*color));
                }
                DisplayItem::RoundedRect {
                    frame,
                    radii,
                    color,
                } => {
                    let _ = writeln!(
                        out,
                        "rrect {} {} {}x{} {} {}",
                        frame.x,
                        frame.y,
                        frame.width,
                        frame.height,
                        radii_text(radii),
                        hex(*color)
                    );
                }
                DisplayItem::Border {
                    frame,
                    widths,
                    colors,
                    radii,
                } => {
                    let colors: Vec<String> = colors.iter().map(|color| hex(*color)).collect();
                    let _ = writeln!(
                        out,
                        "border {} {} {}x{} {:?} {} {}",
                        frame.x,
                        frame.y,
                        frame.width,
                        frame.height,
                        widths,
                        radii_text(radii),
                        colors.join(",")
                    );
                }
                DisplayItem::Shadow {
                    frame,
                    radius,
                    blur,
                    color,
                    ..
                } => {
                    let _ = writeln!(
                        out,
                        "shadow {} {} {}x{} radius {radius} blur {blur} {}",
                        frame.x,
                        frame.y,
                        frame.width,
                        frame.height,
                        hex(*color)
                    );
                }
                DisplayItem::Image {
                    size, tile, repeat, ..
                } => {
                    let _ = writeln!(
                        out,
                        "image {}x{} at {} {} {}x{} repeat {:?}",
                        size.0, size.1, tile.x, tile.y, tile.width, tile.height, repeat
                    );
                }
                DisplayItem::Gradient {
                    gradient,
                    tile,
                    repeat,
                    ..
                } => {
                    let shape = match gradient.shape {
                        GradientShape::Linear { start, end } => {
                            format!("linear {} {} to {} {}", start.0, start.1, end.0, end.1)
                        }
                        GradientShape::Radial {
                            center,
                            radii,
                            inner,
                        } => format!(
                            "radial {} {} radii {}x{} inner {inner}",
                            center.0, center.1, radii.0, radii.1
                        ),
                    };
                    let stops: Vec<String> = gradient
                        .stops
                        .iter()
                        .map(|stop| format!("{} {}", stop.offset, hex(stop.color)))
                        .collect();
                    let _ = writeln!(
                        out,
                        "gradient {shape}{} stops {} at {} {} {}x{} repeat {:?}",
                        if gradient.repeating { " repeating" } else { "" },
                        stops.join(", "),
                        tile.x,
                        tile.y,
                        tile.width,
                        tile.height,
                        repeat
                    );
                }
                DisplayItem::PushOpacity(opacity) => {
                    let _ = writeln!(out, "opacity {opacity}");
                }
                DisplayItem::PopOpacity => {
                    let _ = writeln!(out, "end opacity");
                }
                DisplayItem::PushTransform(matrix) => {
                    let _ = writeln!(out, "transform {matrix:?}");
                }
                DisplayItem::PopTransform => {
                    let _ = writeln!(out, "end transform");
                }
                DisplayItem::PushClip { frame, radii } => {
                    let _ = write!(
                        out,
                        "clip {} {} {}x{}",
                        frame.x, frame.y, frame.width, frame.height
                    );
                    if radii.iter().any(|(x, y)| *x > 0.0 || *y > 0.0) {
                        let _ = write!(out, " {}", radii_text(radii));
                    }
                    let _ = writeln!(out);
                }
                DisplayItem::PopClip => {
                    let _ = writeln!(out, "end clip");
                }
                DisplayItem::Hit { .. } => {}
                DisplayItem::Highlight(frame) => {
                    let _ = writeln!(
                        out,
                        "highlight {} {} {}x{}",
                        frame.x, frame.y, frame.width, frame.height
                    );
                }
                DisplayItem::Glyphs(run) => {
                    let (x, y) = run.glyphs.first().map_or((0.0, 0.0), |g| (g.x, g.y));
                    let _ = writeln!(
                        out,
                        "glyphs {x} {y} {}px {} {:?}",
                        run.size,
                        hex(run.color),
                        run.text
                    );
                }
            }
        }
        out
    }
}

/// What one stacking context paints, sorted into the phases of CSS 2
/// Appendix E.
#[derive(Default)]
struct Context {
    backgrounds: Vec<Tagged>,
    inline: Vec<Tagged>,
    /// Positioned descendants, each painted later as a unit.
    layers: Vec<Layer>,
}

/// A positioned element, painted as a unit at its `z-index`.
struct Layer {
    z: i32,
    id: NodeId,
}

/// The items of the stacking context rooted at `id`, in paint order.
fn stacking_context(walk: &Walk<'_>, id: NodeId) -> Vec<Tagged> {
    // An element below 1 opacity is composited as one group.
    let style = walk.styles.computed(id);
    let opacity = style
        .as_ref()
        .map_or(1.0, |style| style.get_effects().opacity);
    // A transform applies to the border box and everything in it; one that
    // cannot be undone hides them all (CSS Transforms 1 §6). An element
    // without a box of its own (an inline one) is not transformable.
    let matrix = style
        .as_ref()
        .zip(walk.layouts.get(id))
        .and_then(|(style, layout)| {
            let (x, y) = walk.scrolling.position(id).unwrap_or_default();
            let frame = Frame {
                x,
                y,
                width: layout.size.width,
                height: layout.size.height,
            };
            crate::transform::matrix(style, frame)
        });
    if matrix.is_some_and(|matrix| !crate::transform::invertible(&matrix)) {
        return Vec::new();
    }
    let transformed = walk.transformed.get();
    if matrix.is_some() {
        walk.transformed.set(transformed + 1);
    }
    let mut context = Context::default();
    add_box(walk, id, &mut context, true);
    // Stable: equal `z-index` keeps tree order.
    context.layers.sort_by_key(|layer| layer.z);
    let (below, above): (Vec<Layer>, Vec<Layer>) =
        context.layers.into_iter().partition(|layer| layer.z < 0);
    let scope = walk.scrolling.scope(id);
    let mut items = Vec::new();
    if opacity < 1.0 {
        items.push((scope, DisplayItem::PushOpacity(opacity.max(0.0))));
    }
    if let Some(matrix) = matrix {
        items.push((scope, DisplayItem::PushTransform(matrix)));
    }
    for layer in below {
        items.extend(stacking_context(walk, layer.id));
    }
    items.append(&mut context.backgrounds);
    items.append(&mut context.inline);
    for layer in above {
        items.extend(stacking_context(walk, layer.id));
    }
    if matrix.is_some() {
        items.push((scope, DisplayItem::PopTransform));
    }
    walk.transformed.set(transformed);
    if opacity < 1.0 {
        items.push((scope, DisplayItem::PopOpacity));
    }
    items
}

/// The items with each run in the clips of its scope: a clip is pushed
/// where a scope starts and popped where it ends, nested as the scopes
/// are. An opacity group keeps the clips it starts in until it ends; an
/// item inside it that escapes them (an absolutely positioned box whose
/// containing block is outside a clip, inside a translucent box) stays
/// clipped.
fn clipped(items: Vec<Tagged>, scrolling: &Scrolling) -> (Vec<DisplayItem>, Vec<bool>) {
    let chain = |scope: Option<usize>| {
        let mut chain = Vec::new();
        let mut current = scope;
        while let Some(index) = current {
            if let Some(clip) = scrolling.scopes[index].clip {
                chain.push((index, clip, scrolling.scopes[index].radii));
            }
            current = scrolling.scopes[index].parent;
        }
        chain.reverse();
        chain
    };
    let mut out = Vec::with_capacity(items.len());
    // Whether each item moves with the document.
    let mut moves = Vec::with_capacity(items.len());
    let mut open: Vec<(usize, Frame, Radii)> = Vec::new();
    // How many clips each open opacity group started in.
    let mut floors: Vec<usize> = Vec::new();
    let enter = |open: &mut Vec<(usize, Frame, Radii)>,
                 out: &mut Vec<DisplayItem>,
                 moves: &mut Vec<bool>,
                 wanted: &[(usize, Frame, Radii)]| {
        let common = open
            .iter()
            .zip(wanted)
            .take_while(|(a, b)| a.0 == b.0)
            .count();
        while open.len() > common {
            open.pop();
            out.push(DisplayItem::PopClip);
            moves.push(false);
        }
        for clip in &wanted[common..] {
            open.push(*clip);
            out.push(DisplayItem::PushClip {
                frame: clip.1,
                radii: clip.2,
            });
            moves.push(scrolling.moves(Some(clip.0)));
        }
    };
    for (scope, item) in items {
        let floor = floors.last().copied().unwrap_or(0);
        let mut wanted = chain(scope);
        if matches!(item, DisplayItem::PopOpacity) || !wanted.starts_with(&open[..floor]) {
            wanted = open[..floor].to_vec();
        }
        enter(&mut open, &mut out, &mut moves, &wanted);
        moves.push(scrolling.moves(scope));
        match item {
            DisplayItem::PushOpacity(_) => {
                out.push(item);
                floors.push(open.len());
            }
            DisplayItem::PopOpacity => {
                out.push(item);
                floors.pop();
            }
            _ => out.push(item),
        }
    }
    enter(&mut open, &mut out, &mut moves, &[]);
    (out, moves)
}

/// The thumbs of scope `index`'s overlay scroll bars, along its padding
/// box's right and bottom edges, where it can scroll.
fn scroll_bars(scrolling: &Scrolling, index: usize) -> impl Iterator<Item = DisplayItem> {
    let scope = &scrolling.scopes[index];
    let area = scope.clip.unwrap_or_else(|| scrolling.viewport());
    let (thickness, gap, shortest) = THUMB;
    let radius = thickness / 2.0;
    let thumb = |length: f32, max: f32, offset: f32| {
        let size = (length * length / (length + max)).max(shortest).min(length);
        let start = if max > 0.0 {
            offset / max * (length - size)
        } else {
            0.0
        };
        (start, size)
    };
    let mut items = Vec::new();
    if scope.user && scope.max.1 > 0.0 {
        let (start, size) = thumb(area.height - 2.0 * gap, scope.max.1, scope.offset.1);
        items.push(DisplayItem::RoundedRect {
            frame: Frame {
                x: area.x + area.width - gap - thickness,
                y: area.y + gap + start,
                width: thickness,
                height: size,
            },
            radii: [(radius, radius); 4],
            color: THUMB_COLOR,
        });
    }
    if scope.user && scope.max.0 > 0.0 {
        let (start, size) = thumb(area.width - 2.0 * gap, scope.max.0, scope.offset.0);
        items.push(DisplayItem::RoundedRect {
            frame: Frame {
                x: area.x + gap + start,
                y: area.y + area.height - gap - thickness,
                width: size,
                height: thickness,
            },
            radii: [(radius, radius); 4],
            color: THUMB_COLOR,
        });
    }
    items.into_iter()
}

/// Add `id`'s background and inline content to `context`, then its
/// children's. A positioned element other than the context's own root
/// becomes a layer instead.
fn add_box(walk: &Walk<'_>, id: NodeId, context: &mut Context, context_root: bool) {
    let Some(layout) = walk.layouts.get(id) else {
        // An element without a box (an inline element) can contain one
        // that has a box (an atomic inline), positioned relative to the
        // block around them.
        for child in walk.doc.children(id) {
            add_box(walk, child, context, false);
        }
        return;
    };
    let style = walk.styles.computed(id);
    // A positioned element, or one below 1 opacity or transformed, paints
    // as a unit; an unpositioned one with opacity or a transform as if
    // positioned with z-index 0 (CSS Color 4 §9, CSS Transforms 1 §3).
    if !context_root
        && let Some(style) = &style
        && (crate::layout::is_positioned(style)
            || style.get_effects().opacity < 1.0
            || crate::transform::transforms(style))
    {
        let z = if crate::layout::is_positioned(style) {
            style.clone_z_index().integer_or(0)
        } else {
            0
        };
        context.layers.push(Layer { z, id });
        return;
    }
    let (x, y) = walk.scrolling.position(id).unwrap_or_default();
    // Where the box's own content is: moved by its scroll offset, in its
    // own clip, if it scrolls.
    let (cx, cy) = walk.scrolling.content_origin(id).unwrap_or((x, y));
    let (scope, inner) = (walk.scrolling.scope(id), walk.scrolling.inner_scope(id));
    // Nothing it holds may show: not walked at all.
    walk.painting_in(scope);
    if let Some(view) = walk.view()
        && let Some((top, bottom)) = walk.extents.and_then(|extents| extents.of(id))
        && !crosses(view, top, bottom)
    {
        return;
    }
    let tag = |items: Vec<DisplayItem>, scope| items.into_iter().map(move |item| (scope, item));
    // `visibility: hidden` keeps the box but paints nothing of it; its
    // descendants may still be visible, so the walk continues.
    let visible = style
        .as_ref()
        .is_none_or(|style| style.clone_visibility() == Visibility::Visible);

    let frame = Frame {
        x,
        y,
        width: layout.size.width,
        height: layout.size.height,
    };
    if let Some(style) = &style
        && visible
        && walk.shows(around(frame, shadow_reach(style)))
    {
        let widths = [
            layout.border.top,
            layout.border.right,
            layout.border.bottom,
            layout.border.left,
        ];
        // The canvas already shows the root's (or body's) background.
        let paint_background = Some(id) != walk.canvas_source;
        let mut decoration = Vec::new();
        box_decoration(
            style,
            frame,
            widths,
            paint_background,
            walk.resources,
            &mut decoration,
        );
        if takes_pointer(style) {
            decoration.push(DisplayItem::Hit {
                node: id.to_bits(),
                frame,
                text: false,
            });
        }
        // A replaced element's image fills its content box.
        if let Some(image) = walk.layouts.image(id) {
            let content = Frame {
                x: x + layout.border.left + layout.padding.left,
                y: y + layout.border.top + layout.padding.top,
                width: layout.size.width
                    - layout.border.left
                    - layout.border.right
                    - layout.padding.left
                    - layout.padding.right,
                height: layout.size.height
                    - layout.border.top
                    - layout.border.bottom
                    - layout.padding.top
                    - layout.padding.bottom,
            };
            if content.width > 0.0 && content.height > 0.0 {
                decoration.push(DisplayItem::Image {
                    image: image.id,
                    size: image.size(),
                    tile: content,
                    repeat: (false, false),
                    area: content,
                    clip: content,
                    clip_radii: [(0.0, 0.0); 4],
                });
            }
        }
        decoration.retain(|item| ink(item).is_none_or(|ink| walk.shows(ink)));
        context.backgrounds.extend(tag(decoration, scope));
    }

    // Hidden text is laid out all the same: its fragments are kept, only
    // its painting is skipped.
    // Its content is painted in its own scope.
    walk.painting_in(inner);
    // A paragraph out of view is not walked at all.
    let shown = |shaped: &ShapedText, top: f32| {
        walk.view()
            .is_none_or(|view| crosses(view, top + shaped.band.0, top + shaped.band.1))
    };
    if let Some(shaped) = walk
        .layouts
        .text(id)
        .filter(|shaped| shown(shaped, cy + layout.border.top + layout.padding.top))
    {
        let content_x = cx + layout.border.left + layout.padding.left;
        let content_y = cy + layout.border.top + layout.padding.top;
        let fragments = text_fragments(shaped, (content_x, content_y), walk.view());
        let mut items = Vec::new();
        if visible {
            items.extend(inline_content(shaped, (content_x, content_y), walk, id));
        }
        text_hits(walk, &fragments, &mut items);
        context.inline.extend(tag(items, inner));
        walk.text.borrow_mut().extend(fragments);
    }

    // Anonymous boxes inherit their block's visibility and have no
    // border or padding of their own.
    for anonymous in walk
        .layouts
        .anonymous(id)
        .iter()
        .filter(|anonymous| shown(&anonymous.text, cy + anonymous.layout.location.y))
    {
        let origin = (
            cx + anonymous.layout.location.x,
            cy + anonymous.layout.location.y,
        );
        let fragments = text_fragments(&anonymous.text, origin, walk.view());
        let mut items = Vec::new();
        if visible {
            items.extend(inline_content(&anonymous.text, origin, walk, id));
        }
        text_hits(walk, &fragments, &mut items);
        context.inline.extend(tag(items, inner));
        walk.text.borrow_mut().extend(fragments);
    }

    for child in walk.doc.children(id) {
        add_box(walk, child, context, false);
    }

    // A shown scroll bar is over the content it scrolls, in its clip.
    if inner != scope
        && walk.bars.contains(&id)
        && let Some(index) = inner
    {
        context
            .inline
            .extend(scroll_bars(walk.scrolling, index).map(|item| (inner, item)));
    }
}

/// A paragraph whose content box starts at `origin`: the backgrounds of its
/// inline elements, then its glyph runs. A background's edges follow the
/// text and fall between pixels; they are snapped to whole pixels, as
/// Chrome snaps them (found by the Chrome reference test: unsnapped, every
/// edge is a column of blended pixels).
///
/// The colours are the frame's, not the layout's (M5.4): a glyph run is
/// painted in the colour of the element its text node is in, text outside
/// any text node (a collapsed space) in that of `block`, whose paragraph
/// it is.
fn inline_content(
    shaped: &ShapedText,
    origin: (f32, f32),
    walk: &Walk<'_>,
    block: NodeId,
) -> Vec<DisplayItem> {
    let (doc, styles, resources) = (walk.doc, walk.styles, walk.resources);
    let color_of = |element: NodeId| {
        styles
            .computed(element)
            .map(|style| srgb_bytes(style.clone_color()))
    };
    let block_color = color_of(block).unwrap_or([0, 0, 0, 255]);
    // A glyph run's text had one colour when it was laid out, its brush's:
    // the run is painted in the frame's colour of the element whose text
    // had it. Shaping can give a run the glyphs of a neighbour's joiner, so
    // that its text range holds none of that text: the nearest text node
    // that had the colour is taken then.
    let element_colour = |node: NodeId| {
        doc.node(node)
            .and_then(|node| node.parent())
            .and_then(color_of)
            .unwrap_or(block_color)
    };
    let colors = |range: &std::ops::Range<usize>, laid_out: Rgba| -> Rgba {
        // Most often the text node the run starts in, found by its start:
        // the sources are in text order.
        let sources = &shaped.sources;
        let at = sources.partition_point(|(_, source)| source.start <= range.start);
        if let Some(at) = at.checked_sub(1)
            && sources[at].1.contains(&range.start)
            && shaped.colours.get(at) == Some(&laid_out)
        {
            return element_colour(sources[at].0);
        }
        let distance = |source: &std::ops::Range<usize>| {
            if source.start < range.end.max(range.start + 1) && range.start < source.end {
                0
            } else {
                source
                    .start
                    .abs_diff(range.start)
                    .min(source.end.abs_diff(range.start))
            }
        };
        shaped
            .sources
            .iter()
            .zip(&shaped.colours)
            .filter(|(_, colour)| **colour == laid_out)
            .min_by_key(|((_, source), _)| distance(source))
            .map_or(block_color, |((node, _), _)| element_colour(*node))
    };
    let mut items: Vec<DisplayItem> = shaped
        .decorations
        .iter()
        .flat_map(|rect| {
            let (dx, dy) = moved(shaped, rect.relative);
            let origin = (origin.0 + dx, origin.1 + dy);
            let (left, top) = ((origin.0 + rect.x).round(), (origin.1 + rect.y).round());
            let right = (origin.0 + rect.x + rect.width).round();
            let bottom = (origin.1 + rect.y + rect.height).round();
            let mut items = Vec::new();
            // The colours are the frame's, not the layout's (M5.4).
            let Some((color, border_colors, true)) =
                style_of(styles, rect.element).map(|style| crate::text::decoration_colors(&style))
            else {
                return items;
            };
            if color[3] != 0 {
                items.push(DisplayItem::Rect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                    color,
                });
            }
            if rect.border.iter().any(|width| *width > 0.0) {
                items.push(DisplayItem::Border {
                    frame: Frame {
                        x: left,
                        y: top,
                        width: right - left,
                        height: bottom - top,
                    },
                    widths: rect.border,
                    colors: border_colors,
                    radii: [(0.0, 0.0); 4],
                });
            }
            items
        })
        .filter(|item| ink(item).is_none_or(|ink| walk.shows(ink)))
        .collect();
    items.extend(glyph_runs(shaped, origin, resources, &colors, walk.view()));
    items
}

/// The style of the element whose `NodeId` bits are `element`, in this
/// frame's styles.
fn style_of(
    styles: &Styles,
    element: u64,
) -> Option<erk_style::style::servo_arc::Arc<ComputedValues>> {
    styles.computed(NodeId::from_bits(element)?)
}

/// How far the relatively positioned inline element `relative` (as
/// [`crate::text::TextBrush::relative`] counts them) moves what it holds.
fn moved(shaped: &ShapedText, relative: u16) -> (f32, f32) {
    usize::from(relative)
        .checked_sub(1)
        .and_then(|at| shaped.relative.get(at))
        .copied()
        .unwrap_or_default()
}

/// Whether an element with `style` is a target for pointer input: shown,
/// and not `pointer-events: none`.
fn takes_pointer(style: &ComputedValues) -> bool {
    use erk_style::style::computed_values::pointer_events::T as PointerEvents;
    style.clone_visibility() == Visibility::Visible
        && style.clone_pointer_events() != PointerEvents::None
}

/// A line of text takes pointer input for the element it is in.
fn text_hits(walk: &Walk<'_>, fragments: &[TextFragment], out: &mut Vec<DisplayItem>) {
    for fragment in fragments {
        let Some(element) = walk.doc.node(fragment.node).and_then(|node| node.parent()) else {
            continue;
        };
        let frame = Frame {
            x: fragment.x,
            y: fragment.y,
            width: fragment.width,
            height: fragment.height,
        };
        if walk.shows(frame)
            && walk
                .styles
                .computed(element)
                .is_some_and(|style| takes_pointer(&style))
        {
            out.push(DisplayItem::Hit {
                node: element.to_bits(),
                frame,
                text: true,
            });
        }
    }
}

/// How far a relatively positioned inline element moves text in the
/// paragraph, at most, along either axis.
fn reach(shaped: &ShapedText) -> f32 {
    shaped.relative.iter().fold(0.0, |reach: f32, (dx, dy)| {
        reach.max(dx.abs()).max(dy.abs())
    })
}

/// Where each text node of a shaped paragraph whose content box starts at
/// `origin` lies, line by line; with a `view`, only on the lines that may
/// meet it.
fn text_fragments(
    shaped: &ShapedText,
    origin: (f32, f32),
    view: Option<Frame>,
) -> Vec<TextFragment> {
    let mut fragments = Vec::new();
    let reach = reach(shaped);
    let lines: Vec<_> = shaped.layout.layout.lines().collect();
    // Whether a line starts with an atomic inline: Chrome measures the
    // space before one at the end of the line above, as if no break came
    // between them.
    let starts_with_atom = |index: usize| {
        lines.get(index).is_some_and(|line| {
            matches!(
                line.items().next(),
                Some(PositionedLayoutItem::InlineBox(inline_box))
                    if shaped.atom_boxes.contains(&inline_box.id)
            )
        })
    };
    for (index, line) in shaped.layout.layout.lines().enumerate() {
        let shift = shaped.layout.shifts.get(index).copied().unwrap_or(0.0);
        if let Some(view) = view {
            // Its fragments span its runs' font boxes, as `placed_clusters`
            // makes them, moved as far as a relative span moves text.
            let (top, bottom) = line
                .items()
                .filter_map(|item| match item {
                    PositionedLayoutItem::GlyphRun(run) => {
                        let baseline = run.baseline() - run.style().brush.raise;
                        let metrics = run.run().metrics();
                        Some((baseline - metrics.ascent, baseline + metrics.descent))
                    }
                    PositionedLayoutItem::InlineBox(_) => None,
                })
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(top, bottom), run| {
                    (top.min(run.0), bottom.max(run.1))
                });
            let at = origin.1 + shift;
            if !crosses(view, at + top - reach, at + bottom + reach) {
                continue;
            }
        }
        let (mut clusters, then_a_box) = crate::text::placed_clusters(&line);
        let then_a_box = then_a_box || starts_with_atom(index + 1);
        // Zero-width spaces Erk adds (see `Paragraph::flush`, `<br>`) are
        // nobody's text.
        clusters.retain(|cluster| shaped.text.get(cluster.text.clone()) != Some("\u{200B}"));
        // White space at the end of a line hangs past it in CSS; before an
        // inline box (an inline-block, an image) it is not at the end. A
        // space `white-space` keeps hangs too, but is text all the same.
        let kept = |at: usize| shaped.preserved.iter().any(|range| range.contains(&at));
        if !then_a_box {
            while clusters
                .last()
                .is_some_and(|cluster| cluster.space && !kept(cluster.text.start))
            {
                clusters.pop();
            }
        }
        for (node, range) in &shaped.sources {
            let mine = clusters
                .iter()
                .filter(|cluster| cluster.text.start < range.end && range.start < cluster.text.end);
            let mut bounds: Option<(f32, f32, f32, f32)> = None;
            for cluster in mine {
                bounds = Some(match bounds {
                    None => (cluster.left, cluster.right, cluster.top, cluster.bottom),
                    Some((left, right, top, bottom)) => (
                        left.min(cluster.left),
                        right.max(cluster.right),
                        top.min(cluster.top),
                        bottom.max(cluster.bottom),
                    ),
                });
            }
            if let Some((left, right, top, bottom)) = bounds {
                // A text node lies in one inline element: its clusters are
                // all moved alike.
                let relative = clusters
                    .iter()
                    .find(|cluster| {
                        cluster.text.start < range.end && range.start < cluster.text.end
                    })
                    .map_or(0, |cluster| cluster.relative);
                let (dx, dy) = moved(shaped, relative);
                fragments.push(TextFragment {
                    node: *node,
                    x: origin.0 + dx + left,
                    y: origin.1 + dy + shift + top,
                    width: right - left,
                    height: bottom - top,
                });
            }
        }
    }
    fragments
}

/// The glyph runs of a shaped paragraph whose content box starts at
/// `origin`. Parley's positioned glyphs already include each line's offset
/// and baseline; the line's shift moves them down past taller inline boxes
/// above. With a `view`, a run that cannot meet it is left out.
fn glyph_runs(
    paragraph: &ShapedText,
    origin: (f32, f32),
    resources: &Resources,
    colors: &dyn Fn(&std::ops::Range<usize>, Rgba) -> Rgba,
    view: Option<Frame>,
) -> Vec<DisplayItem> {
    let (text, shaped) = (&paragraph.text, &paragraph.layout);
    let mut runs = Vec::new();
    for (index, line) in shaped.layout.lines().enumerate() {
        let shift = shaped.shifts.get(index).copied().unwrap_or(0.0);
        // Glyphs reach about an em above the baseline and less below it;
        // three and two times that leave room for tall scripts and stacked
        // marks.
        let shows = |run: &parley::GlyphRun<'_, crate::text::TextBrush>| {
            let size = run.run().font_size();
            let (_, dy) = moved(paragraph, run.style().brush.relative);
            let baseline = origin.1 + dy + shift + run.baseline() - run.style().brush.raise;
            view.is_none_or(|view| crosses(view, baseline - 3.0 * size, baseline + 2.0 * size))
        };
        // The runs' text, found by walking the line's clusters: only once
        // a run shows.
        let mut ranges = None;
        let runs_of_line = line.items().filter_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => Some(run),
            PositionedLayoutItem::InlineBox(_) => None,
        });
        for (at, run) in runs_of_line.enumerate() {
            if !shows(&run) {
                continue;
            }
            let range = ranges
                .get_or_insert_with(|| crate::text::glyph_run_ranges(&line))
                .get(at)
                .cloned()
                .unwrap_or_default();
            // `vertical-align` moves the run off the line's baseline, and a
            // relatively positioned element around it moves it further.
            let raise = run.style().brush.raise;
            let (dx, dy) = moved(paragraph, run.style().brush.relative);
            let glyphs = run
                .positioned_glyphs()
                .map(|glyph| PositionedGlyph {
                    id: glyph.id,
                    x: origin.0 + dx + glyph.x,
                    y: origin.1 + dy + shift + glyph.y - raise,
                })
                .collect();
            runs.push(DisplayItem::Glyphs(GlyphRun {
                font: resources.font_id(run.run().font()),
                size: run.run().font_size(),
                // All of a run's text had one colour when it was laid out,
                // and still has (`Layouts::colours_split_alike`).
                color: colors(&range, run.style().brush.color),
                glyphs,
                text: text.get(range).unwrap_or_default().to_owned(),
            }));
        }
    }
    runs
}

fn background(style: &ComputedValues) -> Rgba {
    srgb_bytes(style.resolve_color(&style.get_background().background_color))
}

/// A box's decoration in CSS's painting order (CSS Backgrounds 3): outer
/// shadows, the background clipped to the rounded border box, the border.
/// Border styles other than `none` and `hidden` are drawn solid.
fn box_decoration(
    style: &ComputedValues,
    frame: Frame,
    widths: [f32; 4],
    paint_background: bool,
    resources: &Resources,
    out: &mut Vec<DisplayItem>,
) {
    let radii = corner_radii(style, frame.width, frame.height);
    let rounded = radii.iter().any(|(x, y)| *x > 0.0 && *y > 0.0);
    // The first shadow is on top, so it is painted last.
    for shadow in style.get_effects().box_shadow.0.iter().rev() {
        if shadow.inset {
            continue;
        }
        let color = srgb_bytes(style.resolve_color(&shadow.base.color));
        if color[3] == 0 {
            continue;
        }
        let spread = shadow.spread.px();
        let blur = shadow.base.blur.0.px();
        let shadow_frame = Frame {
            x: frame.x + shadow.base.horizontal.px() - spread,
            y: frame.y + shadow.base.vertical.px() - spread,
            width: (frame.width + 2.0 * spread).max(0.0),
            height: (frame.height + 2.0 * spread).max(0.0),
        };
        // One radius for the blurred shape: the corners' mean, grown by the
        // spread like every radius.
        let mean = radii.iter().map(|(x, y)| (x + y) / 2.0).sum::<f32>() / 4.0;
        let radius = if rounded {
            (mean + spread).max(0.0)
        } else {
            0.0
        };
        out.push(DisplayItem::Shadow {
            frame: shadow_frame,
            radius,
            blur,
            color,
            clip: frame,
            clip_radii: radii,
        });
    }
    let color = background(style);
    if paint_background && color[3] != 0 {
        if rounded {
            out.push(DisplayItem::RoundedRect {
                frame,
                radii,
                color,
            });
        } else {
            out.push(DisplayItem::Rect {
                x: frame.x,
                y: frame.y,
                width: frame.width,
                height: frame.height,
                color,
            });
        }
    }
    if paint_background {
        background_images(style, frame, widths, radii, resources, out);
    }
    let border = style.get_border();
    let colors = [
        &border.border_top_color,
        &border.border_right_color,
        &border.border_bottom_color,
        &border.border_left_color,
    ]
    .map(|color| srgb_bytes(style.resolve_color(color)));
    let visible = widths
        .iter()
        .zip(&colors)
        .any(|(width, color)| *width > 0.0 && color[3] != 0);
    if visible {
        out.push(DisplayItem::Border {
            frame,
            widths,
            colors,
            radii,
        });
    }
}

/// The `background-image` layers, last layer first (the first is on top):
/// images that have arrived and gradients. Each is placed in the padding
/// box by `background-size`, `background-position` and
/// `background-repeat`, and clipped to the border box (CSS Backgrounds 3
/// §3). `space` and `round` repeat plainly. A gradient has no natural size:
/// `auto`, `cover` and `contain` give it the padding box's (CSS Images 3
/// §5.3).
fn background_images(
    style: &ComputedValues,
    frame: Frame,
    widths: [f32; 4],
    radii: Radii,
    resources: &Resources,
    out: &mut Vec<DisplayItem>,
) {
    use erk_style::style::values::computed::Length;
    use erk_style::style::values::generics::background::GenericBackgroundSize;
    use erk_style::style::values::generics::length::GenericLengthPercentageOrAuto;
    use erk_style::style::values::specified::background::BackgroundRepeatKeyword;

    let background = style.get_background();
    let layers = &background.background_image.0;
    let [top, right, bottom, left] = widths;
    let area = Frame {
        x: frame.x + left,
        y: frame.y + top,
        width: (frame.width - left - right).max(0.0),
        height: (frame.height - top - bottom).max(0.0),
    };
    let nth = |slice: usize, index: usize| index % slice.max(1);
    use erk_style::style::values::generics::image::GenericImage;

    for (index, layer) in layers.iter().enumerate().rev() {
        let image = image_url(layer).and_then(|url| resources.image(&url).cloned());
        let css_gradient = match layer {
            GenericImage::Gradient(gradient) => Some(gradient),
            _ => None,
        };
        if image.is_none() && css_gradient.is_none() {
            continue;
        }
        // A gradient's "natural size" is the area's: auto fills it.
        let (natural_w, natural_h) = image.as_ref().map_or((area.width, area.height), |image| {
            (image.width(), image.height())
        });
        let sizes = &background.background_size.0;
        let (width, height) = match sizes.get(nth(sizes.len(), index)) {
            Some(GenericBackgroundSize::Cover | GenericBackgroundSize::Contain)
                if image.is_none() =>
            {
                (area.width, area.height)
            }
            Some(GenericBackgroundSize::Cover) => {
                let scale = (area.width / natural_w).max(area.height / natural_h);
                (natural_w * scale, natural_h * scale)
            }
            Some(GenericBackgroundSize::Contain) => {
                let scale = (area.width / natural_w).min(area.height / natural_h);
                (natural_w * scale, natural_h * scale)
            }
            Some(GenericBackgroundSize::ExplicitSize { width, height }) => {
                let resolve = |value: &GenericLengthPercentageOrAuto<_>, basis: f32| match value {
                    GenericLengthPercentageOrAuto::LengthPercentage(length) => {
                        let length: &erk_style::style::values::computed::NonNegativeLengthPercentage =
                            length;
                        Some(length.0.resolve(Length::new(basis)).px())
                    }
                    GenericLengthPercentageOrAuto::Auto => None,
                };
                match (resolve(width, area.width), resolve(height, area.height)) {
                    (Some(w), Some(h)) => (w, h),
                    // No natural ratio: the auto side is the area's.
                    (Some(w), None) if image.is_none() => (w, area.height),
                    (None, Some(h)) if image.is_none() => (area.width, h),
                    (Some(w), None) => (w, w * natural_h / natural_w),
                    (None, Some(h)) => (h * natural_w / natural_h, h),
                    (None, None) => (natural_w, natural_h),
                }
            }
            None => (natural_w, natural_h),
        };
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let xs = &background.background_position_x.0;
        let ys = &background.background_position_y.0;
        let x = xs
            .get(nth(xs.len(), index))
            .map_or(0.0, |x| x.resolve(Length::new(area.width - width)).px());
        let y = ys
            .get(nth(ys.len(), index))
            .map_or(0.0, |y| y.resolve(Length::new(area.height - height)).px());
        let repeats = &background.background_repeat.0;
        let (repeat_x, repeat_y) =
            repeats
                .get(nth(repeats.len(), index))
                .map_or((true, true), |repeat| {
                    (
                        repeat.0 != BackgroundRepeatKeyword::NoRepeat,
                        repeat.1 != BackgroundRepeatKeyword::NoRepeat,
                    )
                });
        let tile = Frame {
            x: area.x + x,
            y: area.y + y,
            width,
            height,
        };
        // A repeating axis covers the whole border box; a single copy only
        // its own extent.
        let painted = Frame {
            x: if repeat_x { frame.x } else { tile.x },
            y: if repeat_y { frame.y } else { tile.y },
            width: if repeat_x { frame.width } else { tile.width },
            height: if repeat_y { frame.height } else { tile.height },
        };
        match (image, css_gradient) {
            (Some(image), _) => out.push(DisplayItem::Image {
                image: image.id,
                size: image.size(),
                tile,
                repeat: (repeat_x, repeat_y),
                area: painted,
                clip: frame,
                clip_radii: radii,
            }),
            (None, Some(css)) => {
                if let Some(gradient) = crate::gradient::gradient(style, css, tile) {
                    out.push(DisplayItem::Gradient {
                        gradient,
                        tile,
                        repeat: (repeat_x, repeat_y),
                        area: painted,
                        clip: frame,
                        clip_radii: radii,
                    });
                }
            }
            (None, None) => {}
        }
    }
}

/// The corner radii of a box `width` × `height`: percentages of the box's
/// size, then all scaled down together if adjacent radii would overlap
/// along a side (CSS Backgrounds 3 §5.5).
/// The radii of a box's padding box: what of its corner radii is left
/// inside its border widths (top, right, bottom, left), as CSS rounds the
/// inner edge of a border (CSS Backgrounds 3 §5.2).
pub(crate) fn padding_radii(style: &ComputedValues, frame: Frame, widths: [f32; 4]) -> Radii {
    let [top, right, bottom, left] = widths;
    let [tl, tr, br, bl] = corner_radii(style, frame.width, frame.height);
    [
        ((tl.0 - left).max(0.0), (tl.1 - top).max(0.0)),
        ((tr.0 - right).max(0.0), (tr.1 - top).max(0.0)),
        ((br.0 - right).max(0.0), (br.1 - bottom).max(0.0)),
        ((bl.0 - left).max(0.0), (bl.1 - bottom).max(0.0)),
    ]
}

fn corner_radii(style: &ComputedValues, width: f32, height: f32) -> Radii {
    use erk_style::style::values::computed::Length;
    let border = style.get_border();
    let mut radii = [
        &border.border_top_left_radius,
        &border.border_top_right_radius,
        &border.border_bottom_right_radius,
        &border.border_bottom_left_radius,
    ]
    .map(|corner| {
        (
            corner.0.width.0.resolve(Length::new(width)).px().max(0.0),
            corner.0.height.0.resolve(Length::new(height)).px().max(0.0),
        )
    });
    let [tl, tr, br, bl] = radii;
    let scale = [
        width / (tl.0 + tr.0),
        width / (bl.0 + br.0),
        height / (tl.1 + bl.1),
        height / (tr.1 + br.1),
    ]
    .into_iter()
    .filter(|factor| factor.is_finite())
    .fold(1.0_f32, f32::min);
    if scale < 1.0 {
        for corner in &mut radii {
            corner.0 *= scale;
            corner.1 *= scale;
        }
    }
    radii
}

fn radii_text(radii: &Radii) -> String {
    radii
        .iter()
        .map(|(x, y)| {
            if x == y {
                format!("{x}")
            } else {
                format!("{x}/{y}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn child_element(doc: &Document, parent: NodeId, name: &erk_dom::LocalName) -> Option<NodeId> {
    doc.children(parent).find(|&child| {
        doc.node(child)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.name.local == *name)
    })
}

fn hex([r, g, b, a]: Rgba) -> String {
    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use erk_dom::Document;
    use erk_style::StyleEngine;

    use super::*;
    use crate::scroll::Offsets;
    use crate::text::{EmbeddedFontMetrics, TextEngine};

    #[test]
    fn a_paragraphs_band_holds_all_its_text_paints_and_measures() {
        // Text taller than its line, moved by a relatively positioned span
        // with a padded background and without one, raised, padded past
        // its line, and lines pushed down by a tall inline box.
        let html = "<body style='margin: 0; font: 14px Noto Sans'>\
             <p>satır <span style='background: #fd8'>vurgu</span></p>\
             <p style='font-size: 60px; line-height: 10px'>Büyük</p>\
             <p><span style='position: relative; top: 250px; padding: 40px 0; background: #9cf'>aşağı</span></p>\
             <p><span style='position: relative; top: 250px'>arka plansız</span></p>\
             <p>x <span style='vertical-align: 80px'>yukarı</span></p>\
             <p>a <span style='padding: 40px 0; background: #fc9'>dolgulu</span> b</p>\
             <p>önce <span style='display: inline-block; width: 10px; height: 300px'></span> sonra<br>alt</p>\
             <div>anonim <p>blok</p> sonra</div>";
        let doc = Document::parse_html(html);
        let styles =
            StyleEngine::with_font_metrics(400.0, 300.0, Arc::new(EmbeddedFontMetrics)).style(&doc);
        let resources = Resources::default();
        let layouts = crate::layout::layout(
            &doc,
            &styles,
            &resources,
            &mut TextEngine::with_fonts(resources.fonts()),
            (400.0, 300.0),
            None,
        )
        .0;
        let scrolling = Scrolling::new(&doc, &styles, &layouts, (400.0, 300.0), &Offsets::new());
        // Each paragraph with the top-left corner of its content box.
        let mut paragraphs: Vec<(&ShapedText, (f32, f32))> = Vec::new();
        let mut stack = vec![doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(doc.children(id));
            let (Some(layout), Some((cx, cy))) = (layouts.get(id), scrolling.content_origin(id))
            else {
                continue;
            };
            if let Some(shaped) = layouts.text(id) {
                let left = cx + layout.border.left + layout.padding.left;
                paragraphs.push((shaped, (left, cy + layout.border.top + layout.padding.top)));
            }
            for anonymous in layouts.anonymous(id) {
                let origin = (
                    cx + anonymous.layout.location.x,
                    cy + anonymous.layout.location.y,
                );
                paragraphs.push((&anonymous.text, origin));
            }
        }
        assert_eq!(paragraphs.len(), 10);
        for (shaped, origin) in paragraphs {
            let (top, bottom) = (origin.1 + shaped.band.0, origin.1 + shaped.band.1);
            let holds = |from: f32, to: f32| top <= from + 0.01 && to <= bottom + 0.01;
            for fragment in text_fragments(shaped, origin, None) {
                assert!(
                    holds(fragment.y, fragment.y + fragment.height),
                    "{fragment:?} out of {top}..{bottom}"
                );
            }
            let colors = |_: &std::ops::Range<usize>, colour: Rgba| colour;
            for run in glyph_runs(shaped, origin, &resources, &colors, None) {
                let DisplayItem::Glyphs(run) = run else {
                    continue;
                };
                // Above the baseline about an em, below it a little.
                for glyph in &run.glyphs {
                    assert!(
                        holds(glyph.y - run.size, glyph.y + run.size / 2.0),
                        "{:?} at {} out of {top}..{bottom}",
                        run.text,
                        glyph.y
                    );
                }
            }
            for rect in &shaped.decorations {
                let y = origin.1 + rect.y + moved(shaped, rect.relative).1;
                assert!(
                    holds(y, y + rect.height),
                    "a background at {y} out of {top}..{bottom}"
                );
            }
        }
    }
}
