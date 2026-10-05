//! Box layout with Taffy's low-level API.
//!
//! The DOM is the layout tree: Taffy walks Erk's nodes through the traits
//! below instead of a copy in a `TaffyTree`. Per-node layout state lives in a
//! side table indexed by `NodeId::index()`, like the style state in
//! erk-style. The trait implementations follow blitz-dom 0.3.0-beta.2,
//! src/layout/mod.rs (MIT OR Apache-2.0).
//!
//! A block whose children are only text and inline elements becomes a
//! paragraph: Parley shapes its text as one layout, each inline element's
//! text in its own style. An inline element's horizontal margin, border and
//! padding become inline boxes of that width at its two ends, and its
//! background is painted per line it spans. An atomic inline
//! (`inline-block`, `inline-flex`) is a Taffy child of the paragraph: it is
//! laid out first, then placed in the line as an inline box of its size,
//! its baseline on the line's (the approach of blitz-dom 0.3.0-beta.2,
//! src/layout/inline.rs, MIT OR Apache-2.0). In a block that mixes block
//! children with text, each run of inline content becomes an anonymous
//! paragraph box (CSS 2 §9.2.1.1). Anonymous boxes live only in this side
//! table, at indices past the arena's slots: the DOM, and the NodeIds a host
//! sees, never contain them.
//!
//! An absolutely positioned element leaves the flow: it is not part of its
//! parent's inline content and does not split it. It becomes a Taffy child
//! of its containing block, the nearest positioned ancestor with a box, or
//! the viewport (CSS 2 §10.1); a fixed element's is always the viewport.
//! Where its insets are `auto` it sits at its static position, where it
//! would have been in the flow (CSS 2 §10.3.7): a zero-size placeholder
//! between blocks, or a zero-width anchor in a line, marks that place, and
//! after layout the box is moved there. In a flex or grid container the
//! static position is the container's content edge.
//! Floats are laid out as if `float: none` (css-support.md): Parley's lines
//! do not flow around them.
//!
//! The layout tree is therefore not the DOM: after layout every box's
//! position is made relative to its nearest DOM ancestor with a box, which
//! is what painting and the host walk.

mod calc;

#[cfg(test)]
mod tests;

use erk_dom::{Document, NodeData, NodeId};
use erk_style::style::Atom;
use erk_style::style::values::specified::box_::{DisplayInside, DisplayOutside};
use erk_style::{ComputedValues, Styles};
use icu_locale_core::LanguageIdentifier;
use taffy::{
    AvailableSpace, Baselines, BlockContext, Cache, CacheTree, Dimension, Display, Layout,
    LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput,
    LayoutPartialTree, LengthPercentageAuto, Line, Overflow, Point, RequestedAxis, ResolveOrZero,
    RoundTree, RunMode, Size, SizingMode, Style, TraversePartialTree, TraverseTree,
    compute_block_layout, compute_cached_layout, compute_flexbox_layout, compute_grid_layout,
    compute_leaf_layout, compute_root_layout, round_layout,
};

use std::sync::Arc;

use self::calc::CalcTable;
use crate::resources::{Image, Resources};
use crate::text::{AtomBox, DecorationRect, InlineLayout, InlineToken, Paragraph, TextEngine};

/// The result of laying out one document.
pub(crate) struct Layouts {
    nodes: Vec<Option<Layout>>,
    text: Vec<Option<ShapedText>>,
    /// Anonymous paragraph boxes, by the index of the block they belong to.
    anonymous: Vec<Vec<AnonymousText>>,
    /// The decoded image of each `<img>` whose image has arrived.
    images: Vec<Option<Arc<Image>>>,
}

/// An anonymous paragraph box: a run of inline content between the block
/// children of a block.
pub(crate) struct AnonymousText {
    /// Relative to the block's box, like a child's layout.
    pub(crate) layout: Layout,
    pub(crate) text: ShapedText,
}

/// A paragraph's text, its shaped and line-broken layout, and the
/// backgrounds of its inline elements, all relative to the paragraph's
/// content box.
pub(crate) struct ShapedText {
    pub(crate) text: String,
    /// Each text node and its range of `text`.
    pub(crate) sources: Vec<(NodeId, std::ops::Range<usize>)>,
    /// How far each relatively positioned inline element moves what it
    /// holds, as [`crate::text::TextBrush::relative`] counts them.
    pub(crate) relative: Vec<(f32, f32)>,
    /// The spaces `white-space` keeps.
    pub(crate) preserved: Vec<std::ops::Range<usize>>,
    /// The ids of the inline boxes that are atomic inlines, not edges.
    pub(crate) atom_boxes: Vec<u64>,
    pub(crate) layout: InlineLayout,
    pub(crate) decorations: Vec<DecorationRect>,
}

impl Layouts {
    /// The final (pixel-rounded) layout of a box, relative to its parent box.
    /// `None` for nodes that generate no box. An atomic inline's parent box
    /// is the block it sits in, whichever inline elements are between.
    pub(crate) fn get(&self, id: NodeId) -> Option<&Layout> {
        self.nodes.get(id.index() as usize)?.as_ref()
    }

    /// The shaped, line-broken text of a paragraph, positioned relative to
    /// its content box.
    pub(crate) fn text(&self, id: NodeId) -> Option<&ShapedText> {
        self.text.get(id.index() as usize)?.as_ref()
    }

    /// The anonymous paragraph boxes among `id`'s children, in tree order.
    pub(crate) fn anonymous(&self, id: NodeId) -> &[AnonymousText] {
        self.anonymous
            .get(id.index() as usize)
            .map_or(&[], Vec::as_slice)
    }

    /// The image an `<img>` shows, if it has arrived.
    pub(crate) fn image(&self, id: NodeId) -> Option<&Arc<Image>> {
        self.images.get(id.index() as usize)?.as_ref()
    }
}

/// Lay out `doc` in a viewport of `width` × `height` CSS pixels.
pub(crate) fn layout(
    doc: &Document,
    styles: &Styles,
    resources: &Resources,
    text: &mut TextEngine,
    width: f32,
    height: f32,
) -> Layouts {
    let slots = doc.capacity_hint();
    let (mut nodes, calcs) = build(doc, styles, resources);
    // The initial containing block is the viewport (CSS 2 §10.1).
    nodes[doc.root().index() as usize].style.size = Size {
        width: Dimension::length(width),
        height: Dimension::length(height),
    };
    let mut tree = LayoutTree {
        nodes,
        calcs: &calcs,
        text,
    };
    let root = taffy_id(doc.root());
    compute_root_layout(
        &mut tree,
        root,
        Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        },
    );
    round_layout(&mut tree, root);

    let LayoutTree { mut nodes, .. } = tree;
    snap_locations(&mut nodes, usize::from(root));
    place_at_static_positions(doc, &mut nodes);
    move_relative_atoms(&mut nodes);
    relative_to_dom(doc, &mut nodes);
    let mut text: Vec<Option<ShapedText>> = nodes
        .iter_mut()
        .map(|node| {
            let paragraph = node.paragraph.as_ref().filter(|_| node.in_tree)?;
            let layout = node.shaped.take()?;
            Some(ShapedText {
                relative: relative_offsets(paragraph, &node.layout),
                text: paragraph.text.clone(),
                sources: paragraph.sources.clone(),
                preserved: paragraph.preserved.clone(),
                atom_boxes: paragraph
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| matches!(item.kind, crate::text::InlineItemKind::Atom(..)))
                    .map(|(id, _)| id as u64)
                    .collect(),
                decorations: layout.decorations(paragraph),
                layout,
            })
        })
        .collect();
    let mut anonymous: Vec<Vec<AnonymousText>> = (0..slots).map(|_| Vec::new()).collect();
    for (index, node) in nodes.iter().enumerate().skip(slots) {
        if let (Some(parent), Some(shaped)) = (node.anonymous_parent, text[index].take()) {
            anonymous[parent].push(AnonymousText {
                layout: node.layout,
                text: shaped,
            });
        }
    }
    text.truncate(slots);
    let images = nodes
        .iter()
        .take(slots)
        .map(|node| {
            node.replaced
                .as_ref()
                .and_then(|replaced| replaced.image.clone())
        })
        .collect();
    Layouts {
        nodes: nodes
            .into_iter()
            .take(slots)
            .map(|node| node.in_tree.then_some(node.layout))
            .collect(),
        text,
        anonymous,
        images,
    }
}

/// Taffy rounds each box's position relative to its parent's, so a
/// parent's fraction of a pixel is lost and a box can land a pixel away from
/// where Chrome draws it. Chrome snaps absolute positions: each box's
/// rounded offset here is its rounded absolute position minus its parent's
/// (found by the Chrome reference test: a `<sup>` line's fraction moved a
/// `vertical-align: middle` box further down the page). Sizes, borders and
/// padding are already rounded that way by Taffy.
fn snap_locations(nodes: &mut [LayoutNode], root: usize) {
    let round = |value: f32| (value + 0.5).floor();
    let mut stack = vec![(root, 0.0_f32, 0.0_f32)];
    while let Some((index, parent_x, parent_y)) = stack.pop() {
        let unrounded = nodes[index].unrounded.location;
        let (x, y) = (parent_x + unrounded.x, parent_y + unrounded.y);
        let location = &mut nodes[index].layout.location;
        location.x = round(x) - round(parent_x);
        location.y = round(y) - round(parent_y);
        for child in &nodes[index].children {
            stack.push((usize::from(*child), x, y));
        }
    }
}

/// Each box's absolute position, down the layout tree.
fn absolute_positions(doc: &Document, nodes: &[LayoutNode]) -> Vec<Point<f32>> {
    let mut absolute = vec![Point::ZERO; nodes.len()];
    let mut stack = vec![(doc.root().index() as usize, Point::ZERO)];
    while let Some((index, parent)) = stack.pop() {
        let location = nodes[index].layout.location;
        let here = Point {
            x: parent.x + location.x,
            y: parent.y + location.y,
        };
        absolute[index] = here;
        for child in &nodes[index].children {
            stack.push((usize::from(*child), here));
        }
    }
    absolute
}

/// Move each absolutely positioned element whose insets are `auto` along an
/// axis to its static position on that axis. Taffy has placed it in its
/// containing block already, at that block's content edge; it has no
/// boxes in flow after it, so moving it changes nothing else. An element
/// whose static position is inside another such element sees that one's
/// position before the move (rare; not handled).
fn place_at_static_positions(doc: &Document, nodes: &mut [LayoutNode]) {
    let absolute = absolute_positions(doc, nodes);
    let round = |value: f32| (value + 0.5).floor();
    for index in 0..nodes.len() {
        let Some(source) = nodes[index].static_position else {
            continue;
        };
        let inset = nodes[index].style.inset;
        let (auto_x, auto_y) = (
            inset.left.is_auto() && inset.right.is_auto(),
            inset.top.is_auto() && inset.bottom.is_auto(),
        );
        if !auto_x && !auto_y {
            continue;
        }
        let content = |box_index: usize| {
            let layout = &nodes[box_index].layout;
            Point {
                x: absolute[box_index].x + layout.border.left + layout.padding.left,
                y: absolute[box_index].y + layout.border.top + layout.padding.top,
            }
        };
        let static_point = match source {
            StaticPosition::Placeholder(placeholder) => absolute[placeholder],
            StaticPosition::ContentStart(container) => {
                let mut point = content(container);
                let (dx, dy) = sole_flex_item_offset(&nodes[container], &nodes[index]);
                point.x += round(dx);
                point.y += round(dy);
                point
            }
            StaticPosition::Line(paragraph) => {
                let origin = content(paragraph);
                let anchor = nodes[paragraph].shaped.as_ref().and_then(|shaped| {
                    shaped
                        .anchor_positions()
                        .iter()
                        .find(|(element, ..)| *element == index)
                        .copied()
                });
                let Some((_, x, top, bottom)) = anchor else {
                    continue;
                };
                if nodes[index].block_level {
                    Point {
                        x: origin.x,
                        y: origin.y + round(bottom),
                    }
                } else {
                    Point {
                        x: origin.x + round(x),
                        y: origin.y + round(top),
                    }
                }
            }
        };
        let layout = nodes[index].layout;
        let parent = Point {
            x: absolute[index].x - layout.location.x,
            y: absolute[index].y - layout.location.y,
        };
        let location = &mut nodes[index].layout.location;
        if auto_x {
            location.x = static_point.x + layout.margin.left - parent.x;
        }
        if auto_y {
            location.y = static_point.y + layout.margin.top - parent.y;
        }
    }
}

/// The offsets of `paragraph`'s relatively positioned inline elements, in
/// the content box of the block laid out as `layout`.
fn relative_offsets(paragraph: &Paragraph, layout: &Layout) -> Vec<(f32, f32)> {
    let width = layout.size.width
        - layout.border.left
        - layout.border.right
        - layout.padding.left
        - layout.padding.right;
    let height = layout.size.height
        - layout.border.top
        - layout.border.bottom
        - layout.padding.top
        - layout.padding.bottom;
    paragraph
        .relative
        .iter()
        .map(|offset| offset.resolve(width.max(0.0), height.max(0.0)))
        .collect()
}

/// Move each atomic inline that a relatively positioned inline element
/// holds by the element's offset: after line breaking, which it changes
/// nothing of.
fn move_relative_atoms(nodes: &mut [LayoutNode]) {
    for index in 0..nodes.len() {
        let Some(paragraph) = nodes[index]
            .paragraph
            .as_ref()
            .filter(|_| nodes[index].in_tree)
        else {
            continue;
        };
        if paragraph.moved_atoms.is_empty() {
            continue;
        }
        let offsets = relative_offsets(paragraph, &nodes[index].layout);
        let moved = paragraph.moved_atoms.clone();
        for (atom, relative) in moved {
            let Some(&(dx, dy)) = usize::from(relative)
                .checked_sub(1)
                .and_then(|at| offsets.get(at))
            else {
                continue;
            };
            let location = &mut nodes[atom].layout.location;
            location.x += dx;
            location.y += dy;
        }
    }
}

/// Where an absolutely positioned child's static position lies in its flex
/// container, from the content edge: where it would be as the container's
/// only flex item (CSS Flexbox §4.1), by `justify-content` along the main
/// axis and `align-self` (or the container's `align-items`) across it.
/// Nothing for a grid container.
fn sole_flex_item_offset(container: &LayoutNode, item: &LayoutNode) -> (f32, f32) {
    use taffy::style::{AlignContentKeyword as Main, AlignItemsKeyword as Cross};
    let style = &container.style;
    if style.display != Display::Flex {
        return (0.0, 0.0);
    }
    use taffy::FlexDirection;
    let (row, reverse) = match style.flex_direction {
        FlexDirection::Row => (true, false),
        FlexDirection::RowReverse => (true, true),
        FlexDirection::Column => (false, false),
        FlexDirection::ColumnReverse => (false, true),
    };
    // The flex-relative start and end swap with a reversed direction.
    let main = match style.justify_content.map(|justify| justify.keyword) {
        Some(Main::Center | Main::SpaceAround | Main::SpaceEvenly) => 0.5,
        Some(Main::End) => 1.0,
        Some(Main::Start) => 0.0,
        Some(Main::FlexEnd) => {
            if reverse {
                0.0
            } else {
                1.0
            }
        }
        None | Some(Main::FlexStart | Main::Stretch | Main::SpaceBetween) => {
            if reverse {
                1.0
            } else {
                0.0
            }
        }
    };
    let cross = match item
        .style
        .align_self
        .or(style.align_items)
        .map(|align| align.keyword)
    {
        Some(Cross::Center) => 0.5,
        Some(Cross::End | Cross::FlexEnd | Cross::SelfEnd) => 1.0,
        _ => 0.0,
    };
    let (outer, inner) = (&container.layout, &item.layout);
    let free_x = outer.size.width
        - outer.border.left
        - outer.border.right
        - outer.padding.left
        - outer.padding.right
        - (inner.size.width + inner.margin.left + inner.margin.right);
    let free_y = outer.size.height
        - outer.border.top
        - outer.border.bottom
        - outer.padding.top
        - outer.padding.bottom
        - (inner.size.height + inner.margin.top + inner.margin.bottom);
    if row {
        (free_x * main, free_y * cross)
    } else {
        (free_x * cross, free_y * main)
    }
}

/// Make every element's position relative to its nearest DOM ancestor with
/// a box. Layout places a box relative to its parent in the layout tree,
/// which differs from the DOM for atomic inlines inside inline elements or
/// anonymous boxes, and for absolutely positioned elements, whose parent is
/// their containing block. Anonymous boxes stay relative to their block.
fn relative_to_dom(doc: &Document, nodes: &mut [LayoutNode]) {
    let absolute = absolute_positions(doc, nodes);
    // Down the DOM, with the absolute position of the nearest box above.
    let mut stack = vec![(doc.root(), Point::ZERO)];
    while let Some((id, above)) = stack.pop() {
        let index = id.index() as usize;
        let mut origin = above;
        if nodes[index].in_tree {
            let here = absolute[index];
            nodes[index].layout.location = Point {
                x: here.x - above.x,
                y: here.y - above.y,
            };
            origin = here;
        }
        for child in doc.children(id) {
            stack.push((child, origin));
        }
    }
}

#[derive(Default)]
struct LayoutNode {
    /// Whether this arena slot generates a box in the layout tree.
    in_tree: bool,
    /// For a paragraph, its atomic inlines.
    children: Vec<taffy::NodeId>,
    style: Style<Atom>,
    /// Set for paragraphs: blocks laid out as one run of inline content.
    paragraph: Option<Paragraph>,
    /// A paragraph's lines as its final layout broke them.
    shaped: Option<InlineLayout>,
    /// For a replaced element (`<img>`): its image, once it has arrived.
    replaced: Option<Replaced>,
    /// For an absolutely positioned element: where it would have been, and
    /// whether it was block-level before it was taken out of the flow.
    static_position: Option<StaticPosition>,
    block_level: bool,
    /// The CSS `order` of a flex or grid item.
    order: i32,
    /// For an anonymous paragraph box, the arena index of its block.
    anonymous_parent: Option<usize>,
    cache: Cache,
    unrounded: Layout,
    layout: Layout,
}

struct LayoutTree<'t> {
    nodes: Vec<LayoutNode>,
    calcs: &'t CalcTable,
    text: &'t mut TextEngine,
}

type StyleArc = erk_style::style::servo_arc::Arc<ComputedValues>;

type Token = InlineToken<StyleArc>;

/// The atomic inlines met while reading inline content.
type Atoms = Vec<(NodeId, StyleArc)>;

/// Absolutely positioned elements met while reading a block's children,
/// with their containing block.
type OutOfFlow = Vec<(NodeId, StyleArc, NodeId)>;

/// A child of a block, as layout sees it.
enum Entry {
    Block(NodeId, StyleArc),
    /// A text node, or an inline element with everything inside it.
    Inline(Vec<Token>, Atoms),
    /// An absolutely positioned child: where it would have been.
    OutOfFlow(NodeId),
}

/// Where an absolutely positioned element would have been in the flow.
#[derive(Clone, Copy, Debug)]
enum StaticPosition {
    /// A zero-size placeholder box between blocks, by index.
    Placeholder(usize),
    /// An anchor in the lines of a paragraph box, by index; a block-level
    /// element starts below the anchor's line, an inline one at the anchor.
    Line(usize),
    /// The content edge of a flex or grid container, by index.
    ContentStart(usize),
}

/// Build the layout tree: which slots generate boxes, their Taffy styles,
/// and the paragraphs.
/// A replaced element: its content is an image, sized by the image's
/// natural size and ratio rather than by children (CSS 2 §10.3.2). Until
/// the image arrives, or if it never does, it has no natural size.
#[derive(Clone, Default)]
struct Replaced {
    image: Option<Arc<Image>>,
}

/// The border-box size of a replaced element with a natural size and ratio,
/// by CSS 2 §10.3.2, §10.6.2 and the constraint table of §10.4: an auto
/// dimension follows the other through the ratio, and when `min-*` or
/// `max-*` changes one, an auto other follows the changed one. Taffy
/// applies the ratio before the limits, so a `max-width` image kept its
/// unlimited height (found by WPT, inline-replaced-height-010). A size the
/// parent imposes (`known_dimensions`) wins.
fn replaced_size(
    style: &Style<Atom>,
    natural: Size<f32>,
    inputs: LayoutInput,
    calcs: &CalcTable,
) -> Size<Option<f32>> {
    use taffy::MaybeResolve;
    let resolve = |ptr, basis| calcs.resolve(ptr, basis);
    let parent = inputs.parent_size;
    let padding = style.padding.resolve_or_zero(parent.width, resolve);
    let border = style.border.resolve_or_zero(parent.width, resolve);
    let inset = Size {
        width: padding.left + padding.right + border.left + border.right,
        height: padding.top + padding.bottom + border.top + border.bottom,
    };
    // Content-box sizes from the style.
    let content = |value: Option<f32>, axis_inset: f32| {
        value.map(|v| match style.box_sizing {
            taffy::BoxSizing::BorderBox => (v - axis_inset).max(0.0),
            taffy::BoxSizing::ContentBox => v,
        })
    };
    let size = style.size.maybe_resolve(parent, resolve);
    let min = style.min_size.maybe_resolve(parent, resolve);
    let max = style.max_size.maybe_resolve(parent, resolve);
    let (width, height) = (
        content(size.width, inset.width),
        content(size.height, inset.height),
    );
    let (min_w, min_h) = (
        content(min.width, inset.width).unwrap_or(0.0),
        content(min.height, inset.height).unwrap_or(0.0),
    );
    let (max_w, max_h) = (
        content(max.width, inset.width).unwrap_or(f32::INFINITY),
        content(max.height, inset.height).unwrap_or(f32::INFINITY),
    );
    let ratio = style
        .aspect_ratio
        .unwrap_or(natural.width / natural.height.max(f32::EPSILON));
    let (mut w, mut h) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w / ratio),
        (None, Some(h)) => (h * ratio, h),
        (None, None) => (natural.width, natural.height),
    };
    let width_auto = width.is_none();
    let height_auto = height.is_none();
    // The limits; an auto dimension follows a limited one through the ratio.
    let clamped_w = w.clamp(min_w, max_w.max(min_w));
    if clamped_w != w {
        w = clamped_w;
        if height_auto {
            h = w / ratio;
        }
    }
    let clamped_h = h.clamp(min_h, max_h.max(min_h));
    if clamped_h != h {
        h = clamped_h;
        if width_auto {
            w = (h * ratio).clamp(min_w, max_w.max(min_w));
        }
    }
    let known = inputs.known_dimensions;
    Size {
        width: known.width.or(Some(w + inset.width)),
        height: known.height.or(Some(h + inset.height)),
    }
}

/// Whether element `id` is a replaced element Erk lays out: `<img>`.
fn is_replaced(doc: &Document, id: NodeId) -> bool {
    doc.node(id)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.name.local == erk_dom::local_name!("img"))
}

fn build(doc: &Document, styles: &Styles, resources: &Resources) -> (Vec<LayoutNode>, CalcTable) {
    let mut nodes: Vec<LayoutNode> = (0..doc.capacity_hint())
        .map(|_| LayoutNode::default())
        .collect();
    let mut calcs = CalcTable::default();

    // The document node is the initial containing block's box.
    let root = &mut nodes[doc.root().index() as usize];
    root.in_tree = true;
    root.style = Style {
        display: Display::Block,
        ..Style::DEFAULT
    };

    // Each block to build, with the containing block of the absolutely
    // positioned elements below it.
    let mut stack = vec![(doc.root(), doc.root())];
    while let Some((parent, outer_container)) = stack.pop() {
        let parent_style = styles.computed(parent);
        let positioned = parent_style
            .as_ref()
            .is_some_and(|style| is_positioned(style));
        let container = if positioned { parent } else { outer_container };
        // Only a block container lays its inline content out as lines; in a
        // flex or grid container each run of text is an anonymous item.
        let block_container = parent_style.as_ref().is_none_or(|style| {
            matches!(
                style.get_box().clone_display().inside(),
                DisplayInside::Flow | DisplayInside::FlowRoot
            )
        });
        // The children in tree order: block-level elements, and everything
        // inline (text nodes, inline elements, atomic inlines). Absolutely
        // positioned elements are set aside for their containing block.
        let mut entries = Vec::new();
        let mut out_of_flow: OutOfFlow = Vec::new();
        let mut has_blocks = false;
        for child in doc.children(parent) {
            match doc.node(child).map(|node| &node.data) {
                Some(NodeData::Text(text)) => {
                    if let Some(style) = styles.computed(parent) {
                        entries.push(Entry::Inline(
                            vec![InlineToken::Text(
                                text.clone(),
                                style.clone(),
                                text_language(doc, parent),
                                child,
                            )],
                            Vec::new(),
                        ));
                    }
                }
                Some(NodeData::Element(_)) => {
                    // Elements inside display:none are not styled, so a
                    // missing style means no box.
                    let Some(computed) = styles.computed(child) else {
                        continue;
                    };
                    if is_out_of_flow(&computed) {
                        out_of_flow.push((child, computed, container));
                        entries.push(Entry::OutOfFlow(child));
                    } else if is_atomic_inline(&computed)
                        || (is_replaced(doc, child) && is_inline_level(&computed))
                    {
                        entries.push(Entry::Inline(
                            vec![InlineToken::Atom(child.index() as usize, computed.clone())],
                            vec![(child, computed)],
                        ));
                    } else if is_inline_level(&computed) {
                        let (mut tokens, mut atoms) = (Vec::new(), Vec::new());
                        inline_tokens(
                            doc,
                            styles,
                            child,
                            &computed,
                            container,
                            &mut tokens,
                            &mut atoms,
                            &mut out_of_flow,
                        );
                        entries.push(Entry::Inline(tokens, atoms));
                    } else {
                        has_blocks = true;
                        entries.push(Entry::Block(child, computed));
                    }
                }
                _ => {}
            }
        }
        // A paragraph: only inline content, laid out as one run. A positioned
        // block keeps a block box with an anonymous paragraph inside, so the
        // absolutely positioned elements it contains are laid out by Taffy.
        if !has_blocks && !positioned && block_container && parent != doc.root() {
            let (mut tokens, mut atoms) = (Vec::new(), Vec::new());
            for entry in entries {
                match entry {
                    Entry::Inline(more_tokens, more_atoms) => {
                        tokens.extend(more_tokens);
                        atoms.extend(more_atoms);
                    }
                    Entry::OutOfFlow(child) => {
                        tokens.push(InlineToken::Anchor(child.index() as usize));
                    }
                    Entry::Block(..) => {}
                }
            }
            if let Some(computed) = parent_style {
                let paragraph = Paragraph::new(&tokens, &computed);
                if !paragraph.is_empty() {
                    let children = add_atoms(&mut nodes, &mut calcs, &mut stack, atoms, container);
                    let node = &mut nodes[parent.index() as usize];
                    node.paragraph = Some(paragraph);
                    node.children = children;
                }
            }
            add_out_of_flow(&mut nodes, &mut calcs, &mut stack, out_of_flow, doc.root());
            continue;
        }

        let mut children = Vec::new();
        let mut run = Run::default();
        for entry in entries {
            match entry {
                Entry::Inline(tokens, atoms) => {
                    run.tokens.extend(tokens);
                    run.atoms.extend(atoms);
                }
                Entry::OutOfFlow(child) => {
                    let index = child.index() as usize;
                    if !block_container {
                        // It ends the text run before it: the text on each
                        // side is an anonymous item of its own (CSS Flexbox
                        // §4: "contiguous" runs).
                        run.close(
                            &mut nodes,
                            &mut calcs,
                            &mut stack,
                            &mut children,
                            parent,
                            &parent_style,
                            container,
                        );
                        // A placeholder would be a flex or grid item of its
                        // own. When the container is also the containing
                        // block, Taffy places the element as if it were the
                        // sole item (CSS Flexbox §4.1); otherwise it starts at
                        // the container's content edge.
                        if container != parent {
                            nodes[index].static_position =
                                Some(StaticPosition::ContentStart(parent.index() as usize));
                        }
                    } else if run.has_content() {
                        // Inside a line: an anchor in the anonymous paragraph.
                        run.tokens.push(InlineToken::Anchor(index));
                    } else {
                        run.close(
                            &mut nodes,
                            &mut calcs,
                            &mut stack,
                            &mut children,
                            parent,
                            &parent_style,
                            container,
                        );
                        nodes.push(LayoutNode {
                            in_tree: true,
                            style: Style {
                                display: Display::Block,
                                ..Style::DEFAULT
                            },
                            ..LayoutNode::default()
                        });
                        let placeholder = nodes.len() - 1;
                        nodes[index].static_position =
                            Some(StaticPosition::Placeholder(placeholder));
                        children.push(taffy::NodeId::from(placeholder));
                    }
                }
                Entry::Block(child, computed) => {
                    let style = taffy_style(&computed);
                    if style.display == Display::None {
                        continue;
                    }
                    run.close(
                        &mut nodes,
                        &mut calcs,
                        &mut stack,
                        &mut children,
                        parent,
                        &parent_style,
                        container,
                    );
                    calcs.record(&computed);
                    let node = &mut nodes[child.index() as usize];
                    node.in_tree = true;
                    node.style = style;
                    node.order = computed.clone_order();
                    children.push(taffy_id(child));
                    stack.push((child, container));
                }
            }
        }
        run.close(
            &mut nodes,
            &mut calcs,
            &mut stack,
            &mut children,
            parent,
            &parent_style,
            container,
        );
        // Taffy lays out flex and grid items in child order and has no
        // `order` property: sort the items by it, keeping tree order between
        // equal values (CSS Flexbox §5.4). Anonymous items have order 0.
        if !block_container {
            children.sort_by_key(|child| nodes[usize::from(*child)].order);
        }
        nodes[parent.index() as usize].children = children;
        add_out_of_flow(&mut nodes, &mut calcs, &mut stack, out_of_flow, doc.root());
    }

    // Replaced elements: an `<img>` with a box shows its image, if it has
    // arrived; its natural ratio, unless the style sets one, keeps a
    // single given dimension in proportion.
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        let index = id.index() as usize;
        if nodes[index].in_tree && is_replaced(doc, id) {
            let image = doc
                .node(id)
                .and_then(|node| node.as_element())
                .and_then(|element| element.attr(&erk_dom::local_name!("src")))
                .and_then(|src| resources.image(src.trim()))
                .cloned();
            // Laid out by Taffy's block layout: a block-level image whose
            // parent is a block container (a flex or grid item sizes itself
            // from its content, and an inline one is measured in its line).
            let in_block_flow = styles
                .computed(id)
                .is_some_and(|style| !is_inline_level(&style))
                && doc
                    .node(id)
                    .and_then(|node| node.parent())
                    .and_then(|parent| styles.computed(parent))
                    .is_some_and(|parent| {
                        matches!(
                            parent.get_box().clone_display().inside(),
                            DisplayInside::Flow | DisplayInside::FlowRoot
                        )
                    });
            if let Some(image) = &image {
                let style = &mut nodes[index].style;
                if style.aspect_ratio.is_none() {
                    style.aspect_ratio = Some(image.width() / image.height());
                }
                // An auto width is the natural width, or what the ratio makes
                // of a fixed height (CSS 2 §10.3.2, §10.3.4); Taffy's block
                // layout would stretch it to the container like a block.
                if in_block_flow && style.size.width.is_auto() {
                    let ratio = style.aspect_ratio.unwrap_or(1.0);
                    let height = style.size.height.into_raw();
                    let width = if height.tag() == taffy::CompactLength::length(0.0).tag() {
                        height.value() * ratio
                    } else if style.size.height.is_auto() {
                        image.width()
                    } else {
                        // A percentage height: left to Taffy.
                        f32::NAN
                    };
                    if width.is_finite() {
                        style.size.width = Dimension::length(width);
                    }
                }
            }
            nodes[index].replaced = Some(Replaced { image });
        }
        stack.extend(doc.children(id));
    }

    // Anchors in lines: their paragraph is the static position.
    for index in 0..nodes.len() {
        let anchors: Vec<usize> = nodes[index]
            .paragraph
            .as_ref()
            .map(Paragraph::anchors)
            .unwrap_or_default();
        for element in anchors {
            nodes[element].static_position = Some(StaticPosition::Line(index));
        }
    }

    (nodes, calcs)
}

/// Give each absolutely positioned element a box as the last child of its
/// containing block. The containing block was built before it (it is an
/// ancestor), and is never a paragraph: positioned blocks keep a block box.
/// A fixed element goes to the viewport, the document node's box.
fn add_out_of_flow(
    nodes: &mut [LayoutNode],
    calcs: &mut CalcTable,
    stack: &mut Vec<(NodeId, NodeId)>,
    out_of_flow: OutOfFlow,
    viewport: NodeId,
) {
    use erk_style::style::computed_values::position::T as CssPosition;
    for (id, computed, container) in out_of_flow {
        // A fixed element's containing block is always the viewport.
        let container = if computed.clone_position() == CssPosition::Fixed {
            viewport
        } else {
            container
        };
        calcs.record(&computed);
        let node = &mut nodes[id.index() as usize];
        node.in_tree = true;
        node.style = taffy_style(&computed);
        node.block_level = computed.get_box().original_display.outside() == DisplayOutside::Block;
        nodes[container.index() as usize]
            .children
            .push(taffy_id(id));
        // A positioned element is the containing block of what it holds.
        stack.push((id, id));
    }
}

/// Stylo's style as Taffy's, with what Erk lays out differently: floats as
/// `float: none`, and insets only where `position` applies them
/// (stylo_taffy maps `static` and `sticky` to Taffy's relative position,
/// which would apply `top` and `left` as offsets).
fn taffy_style(computed: &ComputedValues) -> Style<Atom> {
    use erk_style::style::computed_values::position::T as CssPosition;
    let mut style = stylo_taffy::to_taffy_style(computed);
    style.float = taffy::Float::None;
    if matches!(
        computed.clone_position(),
        CssPosition::Static | CssPosition::Sticky
    ) {
        style.inset = taffy::Rect {
            left: LengthPercentageAuto::auto(),
            right: LengthPercentageAuto::auto(),
            top: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::auto(),
        };
    }
    style
}

/// `position` other than `static`: a containing block for absolutely
/// positioned descendants, and painted after the flow (CSS 2 Appendix E).
pub(crate) fn is_positioned(style: &ComputedValues) -> bool {
    use erk_style::style::computed_values::position::T as CssPosition;
    style.clone_position() != CssPosition::Static
}

/// `position: absolute` or `fixed`: out of the flow.
fn is_out_of_flow(style: &ComputedValues) -> bool {
    use erk_style::style::computed_values::position::T as CssPosition;
    matches!(
        style.clone_position(),
        CssPosition::Absolute | CssPosition::Fixed
    )
}

/// Inline content between the block children of a block.
#[derive(Default)]
struct Run {
    tokens: Vec<Token>,
    atoms: Atoms,
}

impl Run {
    /// Whether the run holds anything a line would be made of: visible
    /// text, an atomic inline or an anchor.
    fn has_content(&self) -> bool {
        self.tokens.iter().any(|token| match token {
            InlineToken::Text(text, ..) => text.chars().any(|c| !c.is_ascii_whitespace()),
            InlineToken::Atom(..) | InlineToken::Anchor(_) | InlineToken::Break => true,
            InlineToken::Open(_) | InlineToken::Close => false,
        })
    }

    /// Close the run: unless it is only whitespace, it becomes an anonymous
    /// paragraph box after the children so far, styled like its block.
    #[allow(clippy::too_many_arguments)]
    fn close(
        &mut self,
        nodes: &mut Vec<LayoutNode>,
        calcs: &mut CalcTable,
        stack: &mut Vec<(NodeId, NodeId)>,
        children: &mut Vec<taffy::NodeId>,
        parent: NodeId,
        parent_style: &Option<StyleArc>,
        container: NodeId,
    ) {
        let Self { tokens, atoms } = std::mem::take(self);
        let Some(style) = parent_style else {
            return;
        };
        let paragraph = Paragraph::new(&tokens, style);
        if paragraph.is_empty() {
            return;
        }
        let container = if is_positioned(style) {
            parent
        } else {
            container
        };
        let atoms = add_atoms(nodes, calcs, stack, atoms, container);
        nodes.push(LayoutNode {
            in_tree: true,
            children: atoms,
            style: Style {
                display: Display::Block,
                ..Style::DEFAULT
            },
            paragraph: Some(paragraph),
            anonymous_parent: Some(parent.index() as usize),
            ..LayoutNode::default()
        });
        children.push(taffy::NodeId::from(nodes.len() - 1));
    }
}

/// Give each atomic inline a box of its own, laid out like a block of its
/// display, and return them as the paragraph's children.
fn add_atoms(
    nodes: &mut [LayoutNode],
    calcs: &mut CalcTable,
    stack: &mut Vec<(NodeId, NodeId)>,
    atoms: Atoms,
    container: NodeId,
) -> Vec<taffy::NodeId> {
    atoms
        .into_iter()
        .map(|(id, computed)| {
            calcs.record(&computed);
            let node = &mut nodes[id.index() as usize];
            node.in_tree = true;
            node.style = taffy_style(&computed);
            stack.push((id, container));
            taffy_id(id)
        })
        .collect()
}

fn is_inline_level(style: &ComputedValues) -> bool {
    style.get_box().clone_display().outside() == DisplayOutside::Inline
}

/// An inline-level box laid out as a whole: `inline-block`, `inline-flex`,
/// `inline-grid`.
fn is_atomic_inline(style: &ComputedValues) -> bool {
    let display = style.get_box().clone_display();
    display.outside() == DisplayOutside::Inline
        && matches!(
            display.inside(),
            DisplayInside::FlowRoot | DisplayInside::Flex | DisplayInside::Grid
        )
}

/// The language of the text in element `id`, for `text-transform` and the
/// choice of fallback fonts: its nearest `lang` attribute (HTML §3.2.6.2).
fn text_language(doc: &Document, id: NodeId) -> LanguageIdentifier {
    crate::case::language(&crate::fonts::language_of(doc, id))
}

/// The content of inline element `id`, in tree order: its start, the text
/// of each text node with the style of its element, nested inline elements,
/// atomic inlines, and its end.
#[allow(clippy::too_many_arguments)]
fn inline_tokens(
    doc: &Document,
    styles: &Styles,
    id: NodeId,
    style: &StyleArc,
    container: NodeId,
    tokens: &mut Vec<Token>,
    atoms: &mut Atoms,
    out_of_flow: &mut OutOfFlow,
) {
    // `<br>` is a forced line break, not an element with content.
    if doc
        .node(id)
        .and_then(|node| node.as_element())
        .is_some_and(|element| element.name.local == erk_dom::local_name!("br"))
    {
        tokens.push(InlineToken::Break);
        return;
    }
    tokens.push(InlineToken::Open(style.clone()));
    for child in doc.children(id) {
        match doc.node(child).map(|node| &node.data) {
            Some(NodeData::Text(content)) => {
                tokens.push(InlineToken::Text(
                    content.clone(),
                    style.clone(),
                    text_language(doc, id),
                    child,
                ));
            }
            Some(NodeData::Element(_)) => {
                if let Some(child_style) = styles.computed(child) {
                    if is_out_of_flow(&child_style) {
                        tokens.push(InlineToken::Anchor(child.index() as usize));
                        out_of_flow.push((child, child_style, container));
                    } else if is_atomic_inline(&child_style) || is_replaced(doc, child) {
                        tokens.push(InlineToken::Atom(
                            child.index() as usize,
                            child_style.clone(),
                        ));
                        atoms.push((child, child_style));
                    } else {
                        inline_tokens(
                            doc,
                            styles,
                            child,
                            &child_style,
                            container,
                            tokens,
                            atoms,
                            out_of_flow,
                        );
                    }
                }
            }
            _ => {}
        }
    }
    tokens.push(InlineToken::Close);
}

fn taffy_id(id: NodeId) -> taffy::NodeId {
    taffy::NodeId::from(id.index() as usize)
}

/// How an atomic inline is laid out inside a paragraph whose content box
/// offers `space`: as an independent formatting context, at its own size.
fn atom_input(space: AvailableSpace) -> LayoutInput {
    LayoutInput {
        run_mode: RunMode::PerformLayout,
        sizing_mode: SizingMode::InherentSize,
        axis: RequestedAxis::Both,
        known_dimensions: Size::NONE,
        known_dimensions_are_definite: Size {
            width: true,
            height: true,
        },
        parent_size: Size {
            width: space.into_option(),
            height: None,
        },
        available_space: Size {
            width: space,
            height: AvailableSpace::MaxContent,
        },
        vertical_margins_are_collapsible: Line::FALSE,
    }
}

impl<'t> LayoutTree<'t> {
    fn node(&self, id: taffy::NodeId) -> &LayoutNode {
        &self.nodes[usize::from(id)]
    }

    fn node_mut(&mut self, id: taffy::NodeId) -> &mut LayoutNode {
        &mut self.nodes[usize::from(id)]
    }

    fn compute_child_layout_internal(
        &mut self,
        id: taffy::NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        if let Some(replaced) = &self.node(id).replaced {
            // Sized by its style, else by its image's natural size; an image
            // that has not arrived has none.
            let natural = replaced.image.as_ref().map_or(Size::ZERO, |image| Size {
                width: image.width(),
                height: image.height(),
            });
            let mut style = self.node(id).style.clone();
            let calcs = self.calcs;
            let inputs =
                if replaced.image.is_some() && inputs.sizing_mode == SizingMode::InherentSize {
                    let known = replaced_size(&style, natural, inputs, calcs);
                    // The ratio is applied: Taffy's leaf layout would apply
                    // it again, keeping the box at least width / ratio high
                    // whatever its height says.
                    style.aspect_ratio = None;
                    LayoutInput {
                        known_dimensions: known,
                        ..inputs
                    }
                } else {
                    inputs
                };
            return compute_leaf_layout(
                inputs,
                &style,
                |ptr, basis| calcs.resolve(ptr, basis),
                |known, _| Size {
                    width: known.width.unwrap_or(natural.width),
                    height: known.height.unwrap_or(natural.height),
                },
            );
        }
        if self.node(id).paragraph.is_some() {
            return self.compute_paragraph_layout(id, inputs);
        }
        match self.node(id).style.display {
            Display::Block => compute_block_layout(self, id, inputs, block_ctx),
            // A flow root establishes a new block formatting context: floats
            // and margins do not cross it.
            Display::FlowRoot => compute_block_layout(self, id, inputs, None),
            Display::Flex => compute_flexbox_layout(self, id, inputs),
            Display::Grid => compute_grid_layout(self, id, inputs),
            Display::None => LayoutOutput::HIDDEN,
        }
    }

    /// A paragraph is sized like a leaf whose content is its lines. Its
    /// atomic inlines are laid out first, at the width the lines are broken
    /// at, so their sizes can go into the lines; in the final layout they
    /// are then placed where the lines put them.
    fn compute_paragraph_layout(&mut self, id: taffy::NodeId, inputs: LayoutInput) -> LayoutOutput {
        let index = usize::from(id);
        let Some(paragraph) = self.nodes[index].paragraph.take() else {
            return LayoutOutput::HIDDEN;
        };
        let style = self.nodes[index].style.clone();
        let calcs = self.calcs;
        let mut measured: Option<InlineLayout> = None;
        let mut output = compute_leaf_layout(
            inputs,
            &style,
            |ptr, basis| calcs.resolve(ptr, basis),
            |known, available| {
                let max_advance = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    // Break at every opportunity: the result is as wide as
                    // the longest unbreakable run.
                    AvailableSpace::MinContent => Some(0.0),
                    AvailableSpace::MaxContent => None,
                });
                let space = known
                    .width
                    .map_or(available.width, AvailableSpace::Definite);
                let atoms = self.measure_atoms(&paragraph, space);
                let shaped = self.text.shape(&paragraph, max_advance, &atoms);
                let size = Size {
                    width: known.width.unwrap_or_else(|| shaped.width()),
                    height: known.height.unwrap_or(shaped.height),
                };
                measured = Some(shaped);
                size
            },
        );
        if inputs.run_mode == RunMode::PerformLayout {
            let resolve = |ptr, basis| calcs.resolve(ptr, basis);
            let padding = style
                .padding
                .resolve_or_zero(inputs.parent_size.width, resolve);
            let border = style
                .border
                .resolve_or_zero(inputs.parent_size.width, resolve);
            let content_width =
                output.size.width - padding.left - padding.right - border.left - border.right;
            let space = AvailableSpace::Definite(content_width);
            let atoms = self.measure_atoms(&paragraph, space);
            // A box sized from its own content (shrink-to-fit) is narrower
            // than the width it was measured at: break and align the lines
            // again at the final width.
            let shaped = match measured {
                Some(shaped) if shaped.broken_at(Some(content_width)) => shaped,
                _ => self.text.shape(&paragraph, Some(content_width), &atoms),
            };
            let inset = Point {
                x: padding.left + border.left,
                y: padding.top + border.top,
            };
            let last_line = shaped.line_count().checked_sub(1);
            output.baselines = Baselines {
                first: shaped.baseline(0).map(|baseline| inset.y + baseline),
                last: last_line
                    .and_then(|line| shaped.baseline(line))
                    .map(|baseline| inset.y + baseline),
            };
            self.place_atoms(&shaped, inset, space);
            self.nodes[index].shaped = Some(shaped);
        }
        self.nodes[index].paragraph = Some(paragraph);
        output
    }

    /// Lay out each atomic inline of `paragraph` in `space` and return the
    /// room it takes in a line.
    fn measure_atoms(&mut self, paragraph: &Paragraph, space: AvailableSpace) -> Vec<AtomBox> {
        let calcs = self.calcs;
        let mut sizes = Vec::new();
        for atom in paragraph.atoms() {
            let output = self.compute_child_layout(taffy::NodeId::from(atom), atom_input(space));
            let style = &self.nodes[atom].style;
            let margin = style
                .margin
                .resolve_or_zero(space.into_option(), |ptr, basis| calcs.resolve(ptr, basis));
            let outer_height = margin.top + output.size.height + margin.bottom;
            // An inline-block's baseline is its last line's; one without
            // lines, or that clips its content, sits on its bottom margin
            // edge (CSS 2 §10.8.1).
            let visible =
                style.overflow.x == Overflow::Visible && style.overflow.y == Overflow::Visible;
            let baseline = visible
                .then(|| output.baselines.last.or(output.baselines.first))
                .flatten();
            let above = baseline.map_or(outer_height, |baseline| margin.top + baseline);
            sizes.push(AtomBox {
                width: (margin.left + output.size.width + margin.right).max(0.0),
                above,
                below: outer_height - above,
            });
        }
        sizes
    }

    /// Give each atomic inline its final position: on its line's baseline,
    /// where the line put it.
    fn place_atoms(&mut self, shaped: &InlineLayout, inset: Point<f32>, space: AvailableSpace) {
        let calcs = self.calcs;
        let resolve = |ptr, basis| calcs.resolve(ptr, basis);
        for (order, &(atom, x, top)) in shaped.atom_positions().iter().enumerate() {
            let id = taffy::NodeId::from(atom);
            // The same input as when measuring: a cache hit.
            let output = self.compute_child_layout(id, atom_input(space));
            let style = &self.nodes[atom].style;
            let basis = space.into_option();
            let margin = style.margin.resolve_or_zero(basis, resolve);
            let layout = Layout {
                location: Point {
                    x: inset.x + x + margin.left,
                    y: inset.y + top + margin.top,
                },
                size: output.size,
                scrollable_overflow_rect: output.scrollable_overflow_rect,
                border: style.border.resolve_or_zero(basis, resolve),
                padding: style.padding.resolve_or_zero(basis, resolve),
                margin,
                ..Layout::with_order(order as u32)
            };
            self.set_unrounded_layout(id, &layout);
        }
    }
}

impl TraversePartialTree for LayoutTree<'_> {
    type ChildIter<'a>
        = std::iter::Copied<std::slice::Iter<'a, taffy::NodeId>>
    where
        Self: 'a;

    fn child_ids(&self, parent: taffy::NodeId) -> Self::ChildIter<'_> {
        self.node(parent).children.iter().copied()
    }

    fn child_count(&self, parent: taffy::NodeId) -> usize {
        self.node(parent).children.len()
    }

    fn get_child_id(&self, parent: taffy::NodeId, index: usize) -> taffy::NodeId {
        self.node(parent).children[index]
    }
}

impl TraverseTree for LayoutTree<'_> {}

impl LayoutPartialTree for LayoutTree<'_> {
    type CoreContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type CustomIdent = Atom;

    fn get_core_container_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }

    fn resolve_calc_value(&self, ptr: *const (), basis: f32) -> f32 {
        self.calcs.resolve(ptr, basis)
    }

    fn set_unrounded_layout(&mut self, id: taffy::NodeId, layout: &Layout) {
        self.node_mut(id).unrounded = *layout;
    }

    fn compute_child_layout(&mut self, id: taffy::NodeId, inputs: LayoutInput) -> LayoutOutput {
        compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            tree.compute_child_layout_internal(id, inputs, None)
        })
    }
}

impl CacheTree for LayoutTree<'_> {
    fn cache_get(&mut self, id: taffy::NodeId, inputs: &LayoutInput) -> Option<LayoutOutput> {
        self.node_mut(id).cache.get(inputs)
    }

    fn cache_store(&mut self, id: taffy::NodeId, inputs: &LayoutInput, output: LayoutOutput) {
        self.node_mut(id).cache.store(inputs, output);
    }

    fn cache_clear(&mut self, id: taffy::NodeId) {
        self.node_mut(id).cache.clear();
    }
}

impl LayoutBlockContainer for LayoutTree<'_> {
    type BlockContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type BlockItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_block_container_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }

    fn get_block_child_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }

    fn compute_block_child_layout(
        &mut self,
        id: taffy::NodeId,
        inputs: LayoutInput,
        block_ctx: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            tree.compute_child_layout_internal(id, inputs, block_ctx)
        })
    }
}

impl LayoutFlexboxContainer for LayoutTree<'_> {
    type FlexboxContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type FlexboxItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_flexbox_container_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }

    fn get_flexbox_child_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }
}

impl LayoutGridContainer for LayoutTree<'_> {
    type GridContainerStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = &'a Style<Atom>
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }

    fn get_grid_child_style(&self, id: taffy::NodeId) -> &Style<Atom> {
        &self.node(id).style
    }
}

impl RoundTree for LayoutTree<'_> {
    fn get_unrounded_layout(&self, id: taffy::NodeId) -> Layout {
        self.node(id).unrounded
    }

    fn set_final_layout(&mut self, id: taffy::NodeId, layout: &Layout) {
        self.node_mut(id).layout = *layout;
    }
}
