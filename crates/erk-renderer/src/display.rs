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
    Glyphs(GlyphRun),
    /// Where `node` takes pointer input: an element's border box, or a line
    /// of text standing for its element. Not painted and not in the dump;
    /// sitting in paint order, the last one under a point is the topmost.
    Hit {
        node: NodeId,
        frame: Frame,
    },
    /// The developer tools' highlight of a selected node's boxes: drawn
    /// over the page, not part of the document (p1-contract §8.1).
    Highlight(Frame),
}

/// The colour of the highlight overlay, as browsers' developer tools tint a
/// selected element's box.
pub(crate) const HIGHLIGHT: Rgba = [111, 168, 220, 166];

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
            canvas_source: list.propagate_canvas_background(doc, styles, layouts),
            text: std::cell::RefCell::new(Vec::new()),
        };
        list.items = stacking_context(&walk, doc.root(), (0.0, 0.0));
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
    backgrounds: Vec<DisplayItem>,
    inline: Vec<DisplayItem>,
    /// Positioned descendants, each painted later as a unit.
    layers: Vec<Layer>,
}

/// A positioned element, painted as a unit at its `z-index`.
struct Layer {
    z: i32,
    id: NodeId,
    parent_origin: (f32, f32),
}

/// The items of the stacking context rooted at `id`, in paint order.
fn stacking_context(walk: &Walk<'_>, id: NodeId, parent_origin: (f32, f32)) -> Vec<DisplayItem> {
    let mut context = Context::default();
    add_box(walk, id, parent_origin, &mut context, true);
    // Stable: equal `z-index` keeps tree order.
    context.layers.sort_by_key(|layer| layer.z);
    let (below, above): (Vec<Layer>, Vec<Layer>) =
        context.layers.into_iter().partition(|layer| layer.z < 0);
    // An element below 1 opacity is composited as one group.
    let opacity = walk
        .styles
        .computed(id)
        .map_or(1.0, |style| style.get_effects().opacity);
    let mut items = Vec::new();
    if opacity < 1.0 {
        items.push(DisplayItem::PushOpacity(opacity.max(0.0)));
    }
    for layer in below {
        items.extend(stacking_context(walk, layer.id, layer.parent_origin));
    }
    items.append(&mut context.backgrounds);
    items.append(&mut context.inline);
    for layer in above {
        items.extend(stacking_context(walk, layer.id, layer.parent_origin));
    }
    if opacity < 1.0 {
        items.push(DisplayItem::PopOpacity);
    }
    items
}

/// Add `id`'s background and inline content to `context`, then its
/// children's. A positioned element other than the context's own root
/// becomes a layer instead.
fn add_box(
    walk: &Walk<'_>,
    id: NodeId,
    parent_origin: (f32, f32),
    context: &mut Context,
    context_root: bool,
) {
    let Some(layout) = walk.layouts.get(id) else {
        // An element without a box (an inline element) can contain one
        // that has a box (an atomic inline), positioned relative to the
        // block around them.
        for child in walk.doc.children(id) {
            add_box(walk, child, parent_origin, context, false);
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
        context.layers.push(Layer {
            z,
            id,
            parent_origin,
        });
        return;
    }
    let x = parent_origin.0 + layout.location.x;
    let y = parent_origin.1 + layout.location.y;
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
        box_decoration(
            style,
            frame,
            widths,
            paint_background,
            walk.resources,
            &mut context.backgrounds,
        );
        if takes_pointer(style) {
            context
                .backgrounds
                .push(DisplayItem::Hit { node: id, frame });
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
                context.backgrounds.push(DisplayItem::Image {
                    image: image.pixmap.clone(),
                    tile: content,
                    repeat: (false, false),
                    area: content,
                    clip: content,
                    clip_radii: [(0.0, 0.0); 4],
                });
            }
        }
    }

    // Hidden text is laid out all the same: its fragments are kept, only
    // its painting is skipped.
    if let Some(shaped) = walk.layouts.text(id) {
        let content_x = x + layout.border.left + layout.padding.left;
        let content_y = y + layout.border.top + layout.padding.top;
        let fragments = text_fragments(shaped, (content_x, content_y));
        if visible {
            context
                .inline
                .extend(inline_content(shaped, (content_x, content_y)));
        }
        text_hits(walk, &fragments, &mut context.inline);
        walk.text.borrow_mut().extend(fragments);
    }

    // Anonymous boxes inherit their block's visibility and have no
    // border or padding of their own.
    for anonymous in walk.layouts.anonymous(id) {
        let origin = (
            x + anonymous.layout.location.x,
            y + anonymous.layout.location.y,
        );
        let fragments = text_fragments(&anonymous.text, origin);
        if visible {
            context
                .inline
                .extend(inline_content(&anonymous.text, origin));
        }
        text_hits(walk, &fragments, &mut context.inline);
        walk.text.borrow_mut().extend(fragments);
    }

    for child in walk.doc.children(id) {
        add_box(walk, child, (x, y), context, false);
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
