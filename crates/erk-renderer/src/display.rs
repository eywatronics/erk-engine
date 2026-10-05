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
use parley::{FontData, PositionedLayoutItem};

use std::sync::Arc;

use vello_cpu::Pixmap;

use crate::color::{Rgba, srgb_bytes};
use crate::layout::{Layouts, ShapedText};
use crate::resources::{Resources, image_url};
use crate::scroll::{Scrolling, VIEWPORT};
use crate::text::InlineLayout;

pub(crate) struct DisplayList {
    /// The canvas colour behind everything (CSS 2 §14.2).
    pub(crate) canvas: Rgba,
    pub(crate) items: Vec<DisplayItem>,
    /// Where each text node's text lies, line by line: not painted, kept
    /// for the renderer's tests and the inspection queries.
    pub(crate) text: Vec<TextFragment>,
}

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
    /// An image: one copy fills `tile`, repeated along an axis where
    /// `repeat` says so; `area` is the region painted, clipped to `clip`.
    Image {
        image: Arc<Pixmap>,
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
    /// Everything until the matching `PopClip` is clipped to this box.
    PushClip(Frame),
    PopClip,
    Glyphs(GlyphRun),
    /// Where `node` takes pointer input: an element's border box, or a line
    /// of text standing for its element (`text`). Not painted and not in
    /// the dump; sitting in paint order, the last one under a point is the
    /// topmost.
    Hit {
        node: NodeId,
        frame: Frame,
        text: bool,
    },
    /// The developer tools' highlight of a selected node's boxes: drawn
    /// over the page, not part of the document (p1-contract §8.1).
    Highlight(Frame),
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

pub(crate) struct GlyphRun {
    pub(crate) font: FontData,
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
}

impl DisplayList {
    pub(crate) fn build(
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
        resources: &Resources,
        scrolling: &Scrolling,
        bars: &[NodeId],
    ) -> Self {
        let mut list = Self {
            canvas: WHITE,
            items: Vec::new(),
            text: Vec::new(),
        };
        let walk = Walk {
            doc,
            styles,
            layouts,
            resources,
            scrolling,
            bars,
            canvas_source: list.propagate_canvas_background(doc, styles, layouts),
            text: std::cell::RefCell::new(Vec::new()),
        };
        let mut items = stacking_context(&walk, doc.root());
        // The document's scroll bars are over everything.
        if bars.contains(&doc.root()) {
            items.extend(scroll_bars(scrolling, VIEWPORT).map(|item| (None, item)));
        }
        list.items = clipped(items, scrolling);
        list.text = walk.text.into_inner();
        list
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
                    image,
                    tile,
                    repeat,
                    ..
                } => {
                    let _ = writeln!(
                        out,
                        "image {}x{} at {} {} {}x{} repeat {:?}",
                        image.width(),
                        image.height(),
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
                DisplayItem::PushClip(frame) => {
                    let _ = writeln!(
                        out,
                        "clip {} {} {}x{}",
                        frame.x, frame.y, frame.width, frame.height
                    );
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
    let mut context = Context::default();
    add_box(walk, id, &mut context, true);
    // Stable: equal `z-index` keeps tree order.
    context.layers.sort_by_key(|layer| layer.z);
    let (below, above): (Vec<Layer>, Vec<Layer>) =
        context.layers.into_iter().partition(|layer| layer.z < 0);
    // An element below 1 opacity is composited as one group.
    let opacity = walk
        .styles
        .computed(id)
        .map_or(1.0, |style| style.get_effects().opacity);
    let scope = walk.scrolling.scope(id);
    let mut items = Vec::new();
    if opacity < 1.0 {
        items.push((scope, DisplayItem::PushOpacity(opacity.max(0.0))));
    }
    for layer in below {
        items.extend(stacking_context(walk, layer.id));
    }
    items.append(&mut context.backgrounds);
    items.append(&mut context.inline);
    for layer in above {
        items.extend(stacking_context(walk, layer.id));
    }
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
fn clipped(items: Vec<Tagged>, scrolling: &Scrolling) -> Vec<DisplayItem> {
    let chain = |scope: Option<usize>| {
        let mut chain = Vec::new();
        let mut current = scope;
        while let Some(index) = current {
            if let Some(clip) = scrolling.scopes[index].clip {
                chain.push((index, clip));
            }
            current = scrolling.scopes[index].parent;
        }
        chain.reverse();
        chain
    };
    let mut out = Vec::with_capacity(items.len());
    let mut open: Vec<(usize, Frame)> = Vec::new();
    // How many clips each open opacity group started in.
    let mut floors: Vec<usize> = Vec::new();
    let enter =
        |open: &mut Vec<(usize, Frame)>, out: &mut Vec<DisplayItem>, wanted: &[(usize, Frame)]| {
            let common = open
                .iter()
                .zip(wanted)
                .take_while(|(a, b)| a.0 == b.0)
                .count();
            while open.len() > common {
                open.pop();
                out.push(DisplayItem::PopClip);
            }
            for clip in &wanted[common..] {
                open.push(*clip);
                out.push(DisplayItem::PushClip(clip.1));
            }
        };
    for (scope, item) in items {
        let floor = floors.last().copied().unwrap_or(0);
        let mut wanted = chain(scope);
        if matches!(item, DisplayItem::PopOpacity) || !wanted.starts_with(&open[..floor]) {
            wanted = open[..floor].to_vec();
        }
        enter(&mut open, &mut out, &wanted);
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
    enter(&mut open, &mut out, &[]);
    out
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
    // A positioned element, or one below 1 opacity, paints as a unit; an
    // unpositioned one with opacity as if positioned with z-index 0 (CSS
    // Color 4 §9).
    if !context_root
        && let Some(style) = &style
        && (crate::layout::is_positioned(style) || style.get_effects().opacity < 1.0)
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
    let tag = |items: Vec<DisplayItem>, scope| items.into_iter().map(move |item| (scope, item));
    // `visibility: hidden` keeps the box but paints nothing of it; its
    // descendants may still be visible, so the walk continues.
    let visible = style
        .as_ref()
        .is_none_or(|style| style.clone_visibility() == Visibility::Visible);

    if let Some(style) = &style
        && visible
    {
        let frame = Frame {
            x,
            y,
            width: layout.size.width,
            height: layout.size.height,
        };
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
                node: id,
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
                    image: image.pixmap.clone(),
                    tile: content,
                    repeat: (false, false),
                    area: content,
                    clip: content,
                    clip_radii: [(0.0, 0.0); 4],
                });
            }
        }
        context.backgrounds.extend(tag(decoration, scope));
    }

    // Hidden text is laid out all the same: its fragments are kept, only
    // its painting is skipped.
    if let Some(shaped) = walk.layouts.text(id) {
        let content_x = cx + layout.border.left + layout.padding.left;
        let content_y = cy + layout.border.top + layout.padding.top;
        let fragments = text_fragments(shaped, (content_x, content_y));
        let mut items = Vec::new();
        if visible {
            items.extend(inline_content(shaped, (content_x, content_y)));
        }
        text_hits(walk, &fragments, &mut items);
        context.inline.extend(tag(items, inner));
        walk.text.borrow_mut().extend(fragments);
    }

    // Anonymous boxes inherit their block's visibility and have no
    // border or padding of their own.
    for anonymous in walk.layouts.anonymous(id) {
        let origin = (
            cx + anonymous.layout.location.x,
            cy + anonymous.layout.location.y,
        );
        let fragments = text_fragments(&anonymous.text, origin);
        let mut items = Vec::new();
        if visible {
            items.extend(inline_content(&anonymous.text, origin));
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
fn inline_content(shaped: &ShapedText, origin: (f32, f32)) -> Vec<DisplayItem> {
    let mut items: Vec<DisplayItem> = shaped
        .decorations
        .iter()
        .flat_map(|rect| {
            let (left, top) = ((origin.0 + rect.x).round(), (origin.1 + rect.y).round());
            let right = (origin.0 + rect.x + rect.width).round();
            let bottom = (origin.1 + rect.y + rect.height).round();
            let mut items = Vec::new();
            if rect.color[3] != 0 {
                items.push(DisplayItem::Rect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                    color: rect.color,
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
                    colors: rect.border_colors,
                    radii: [(0.0, 0.0); 4],
                });
            }
            items
        })
        .collect();
    items.extend(glyph_runs(&shaped.text, &shaped.layout, origin));
    items
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
        if walk
            .styles
            .computed(element)
            .is_some_and(|style| takes_pointer(&style))
        {
            out.push(DisplayItem::Hit {
                node: element,
                frame: Frame {
                    x: fragment.x,
                    y: fragment.y,
                    width: fragment.width,
                    height: fragment.height,
                },
                text: true,
            });
        }
    }
}

/// Where each text node of a shaped paragraph whose content box starts at
/// `origin` lies, line by line.
fn text_fragments(shaped: &ShapedText, origin: (f32, f32)) -> Vec<TextFragment> {
    let mut fragments = Vec::new();
    for (index, line) in shaped.layout.layout.lines().enumerate() {
        let shift = shaped.layout.shifts.get(index).copied().unwrap_or(0.0);
        let (mut clusters, then_a_box) = crate::text::placed_clusters(&line);
        // White space at the end of a line hangs past it in CSS; before an
        // inline box (an inline-block, an image) it is not at the end.
        if !then_a_box {
            while clusters.last().is_some_and(|cluster| cluster.space) {
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
                fragments.push(TextFragment {
                    node: *node,
                    x: origin.0 + left,
                    y: origin.1 + shift + top,
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
/// above.
fn glyph_runs(text: &str, shaped: &InlineLayout, origin: (f32, f32)) -> Vec<DisplayItem> {
    let mut runs = Vec::new();
    for (index, line) in shaped.layout.lines().enumerate() {
        let shift = shaped.shifts.get(index).copied().unwrap_or(0.0);
        let mut ranges = crate::text::glyph_run_ranges(&line).into_iter();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let range = ranges.next().unwrap_or_default();
            // `vertical-align` moves the run off the line's baseline.
            let raise = run.style().brush.raise;
            let glyphs = run
                .positioned_glyphs()
                .map(|glyph| PositionedGlyph {
                    id: glyph.id,
                    x: origin.0 + glyph.x,
                    y: origin.1 + shift + glyph.y - raise,
                })
                .collect();
            runs.push(DisplayItem::Glyphs(GlyphRun {
                font: run.run().font().clone(),
                size: run.run().font_size(),
                color: run.style().brush.color,
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

/// The `background-image` layers that have arrived, last layer first (the
/// first is on top). Each is placed in the padding box by
/// `background-size`, `background-position` and `background-repeat`, and
/// clipped to the border box (CSS Backgrounds 3 §3). `space` and `round`
/// repeat plainly.
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
    for (index, layer) in layers.iter().enumerate().rev() {
        let Some(image) = image_url(layer).and_then(|url| resources.image(&url).cloned()) else {
            continue;
        };
        let (natural_w, natural_h) = (image.width(), image.height());
        let sizes = &background.background_size.0;
        let (width, height) = match sizes.get(nth(sizes.len(), index)) {
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
        out.push(DisplayItem::Image {
            image: image.pixmap.clone(),
            tile,
            repeat: (repeat_x, repeat_y),
            area: painted,
            clip: frame,
            clip_radii: radii,
        });
    }
}

/// The corner radii of a box `width` × `height`: percentages of the box's
/// size, then all scaled down together if adjacent radii would overlap
/// along a side (CSS Backgrounds 3 §5.5).
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
