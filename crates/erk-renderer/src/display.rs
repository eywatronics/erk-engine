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

use crate::color::{Rgba, srgb_bytes};
use crate::layout::{Layouts, ShapedText};
use crate::text::InlineLayout;

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
    /// Everything until the matching `PopOpacity` is composited at this
    /// opacity, as one group.
    PushOpacity(f32),
    PopOpacity,
    Glyphs(GlyphRun),
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
    /// The element whose background became the canvas colour.
    canvas_source: Option<NodeId>,
}

impl DisplayList {
    pub(crate) fn build(doc: &Document, styles: &Styles, layouts: &Layouts) -> Self {
        let mut list = Self {
            canvas: WHITE,
            items: Vec::new(),
        };
        let walk = Walk {
            doc,
            styles,
            layouts,
            canvas_source: list.propagate_canvas_background(doc, styles, layouts),
        };
        list.items = stacking_context(&walk, doc.root(), (0.0, 0.0));
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
                DisplayItem::PushOpacity(opacity) => {
                    let _ = writeln!(out, "opacity {opacity}");
                }
                DisplayItem::PopOpacity => {
                    let _ = writeln!(out, "end opacity");
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
            &mut context.backgrounds,
        );
    }

    if let Some(shaped) = walk.layouts.text(id)
        && visible
    {
        let content_x = x + layout.border.left + layout.padding.left;
        let content_y = y + layout.border.top + layout.padding.top;
        context
            .inline
            .extend(inline_content(shaped, (content_x, content_y)));
    }

    // Anonymous boxes inherit their block's visibility and have no
    // border or padding of their own.
    if visible {
        for anonymous in walk.layouts.anonymous(id) {
            let origin = (
                x + anonymous.layout.location.x,
                y + anonymous.layout.location.y,
            );
            context
                .inline
                .extend(inline_content(&anonymous.text, origin));
        }
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
