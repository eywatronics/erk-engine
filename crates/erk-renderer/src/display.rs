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
    Glyphs(GlyphRun),
}

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
    let mut items = Vec::new();
    for layer in below {
        items.extend(stacking_context(walk, layer.id, layer.parent_origin));
    }
    items.append(&mut context.backgrounds);
    items.append(&mut context.inline);
    for layer in above {
        items.extend(stacking_context(walk, layer.id, layer.parent_origin));
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
    if !context_root
        && let Some(style) = &style
        && crate::layout::is_positioned(style)
    {
        context.layers.push(Layer {
            z: style.clone_z_index().integer_or(0),
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
        && Some(id) != walk.canvas_source
    {
        let color = background(style);
        if color[3] != 0 {
            context.backgrounds.push(DisplayItem::Rect {
                x,
                y,
                width: layout.size.width,
                height: layout.size.height,
                color,
            });
        }
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
        .map(|rect| {
            let (left, top) = ((origin.0 + rect.x).round(), (origin.1 + rect.y).round());
            let right = (origin.0 + rect.x + rect.width).round();
            let bottom = (origin.1 + rect.y + rect.height).round();
            DisplayItem::Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
                color: rect.color,
            }
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
