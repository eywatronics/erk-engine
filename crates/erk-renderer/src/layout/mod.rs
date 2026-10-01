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

mod calc;

#[cfg(test)]
mod tests;

use erk_dom::{Document, NodeData, NodeId};
use erk_style::style::Atom;
use erk_style::style::values::specified::box_::{DisplayInside, DisplayOutside};
use erk_style::{ComputedValues, Styles};
use taffy::{
    AvailableSpace, Baselines, BlockContext, Cache, CacheTree, Display, Layout,
    LayoutBlockContainer, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput,
    LayoutPartialTree, Line, Overflow, Point, RequestedAxis, ResolveOrZero, RoundTree, RunMode,
    Size, SizingMode, Style, TraversePartialTree, TraverseTree, compute_block_layout,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
    compute_root_layout, round_layout,
};

use self::calc::CalcTable;
use crate::text::{AtomBox, DecorationRect, InlineLayout, InlineToken, Paragraph, TextEngine};

/// The result of laying out one document.
pub(crate) struct Layouts {
    nodes: Vec<Option<Layout>>,
    text: Vec<Option<ShapedText>>,
    /// Anonymous paragraph boxes, by the index of the block they belong to.
    anonymous: Vec<Vec<AnonymousText>>,
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
}

/// Lay out `doc` in a viewport of `width` × `height` CSS pixels.
pub(crate) fn layout(
    doc: &Document,
    styles: &Styles,
    text: &mut TextEngine,
    width: f32,
    height: f32,
) -> Layouts {
    let slots = doc.capacity_hint();
    let (nodes, calcs) = build(doc, styles);
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
    // An atomic inline in an anonymous box is laid out relative to that
    // box, which the DOM does not have: make it relative to the block.
    for index in slots..nodes.len() {
        let origin = nodes[index].layout.location;
        for child in nodes[index].children.clone() {
            let location = &mut nodes[usize::from(child)].layout.location;
            location.x += origin.x;
            location.y += origin.y;
        }
    }
    let mut text: Vec<Option<ShapedText>> = nodes
        .iter_mut()
        .map(|node| {
            let paragraph = node.paragraph.as_ref().filter(|_| node.in_tree)?;
            let layout = node.shaped.take()?;
            Some(ShapedText {
                text: paragraph.text.clone(),
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
    Layouts {
        nodes: nodes
            .into_iter()
            .take(slots)
            .map(|node| node.in_tree.then_some(node.layout))
            .collect(),
        text,
        anonymous,
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

/// A child of a block, as layout sees it.
enum Entry {
    Block(NodeId, StyleArc),
    /// A text node, or an inline element with everything inside it.
    Inline(Vec<Token>, Atoms),
}

/// Build the layout tree: which slots generate boxes, their Taffy styles,
/// and the paragraphs.
fn build(doc: &Document, styles: &Styles) -> (Vec<LayoutNode>, CalcTable) {
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

    let mut stack = vec![doc.root()];
    while let Some(parent) = stack.pop() {
        // The children in tree order: block-level elements, and everything
        // inline (text nodes, inline elements, atomic inlines).
        let mut entries = Vec::new();
        let mut has_blocks = false;
        for child in doc.children(parent) {
            match doc.node(child).map(|node| &node.data) {
                Some(NodeData::Text(text)) => {
                    if let Some(style) = styles.computed(parent) {
                        entries.push(Entry::Inline(
                            vec![InlineToken::Text(text.clone(), style)],
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
                    if is_atomic_inline(&computed) {
                        entries.push(Entry::Inline(
                            vec![InlineToken::Atom(child.index() as usize, computed.clone())],
                            vec![(child, computed)],
                        ));
                    } else if is_inline_level(&computed) {
                        let (mut tokens, mut atoms) = (Vec::new(), Vec::new());
                        inline_tokens(doc, styles, child, &computed, &mut tokens, &mut atoms);
                        entries.push(Entry::Inline(tokens, atoms));
                    } else {
                        has_blocks = true;
                        entries.push(Entry::Block(child, computed));
                    }
                }
                _ => {}
            }
        }
        let parent_style = styles.computed(parent);

        // A paragraph: only inline content, laid out as one run.
        if !has_blocks && parent != doc.root() {
            let (mut tokens, mut atoms) = (Vec::new(), Vec::new());
            for entry in entries {
                if let Entry::Inline(more_tokens, more_atoms) = entry {
                    tokens.extend(more_tokens);
                    atoms.extend(more_atoms);
                }
            }
            if let Some(computed) = parent_style {
                let paragraph = Paragraph::new(&tokens, &computed);
                if !paragraph.is_empty() {
                    let children = add_atoms(&mut nodes, &mut calcs, &mut stack, atoms);
                    let node = &mut nodes[parent.index() as usize];
                    node.paragraph = Some(paragraph);
                    node.children = children;
                }
            }
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
                Entry::Block(child, computed) => {
                    let style = stylo_taffy::to_taffy_style(&computed);
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
                    );
                    calcs.record(&computed);
                    let node = &mut nodes[child.index() as usize];
                    node.in_tree = true;
                    node.style = style;
                    children.push(taffy_id(child));
                    stack.push(child);
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
        );
        nodes[parent.index() as usize].children = children;
    }

    (nodes, calcs)
}

/// Inline content between the block children of a block.
#[derive(Default)]
struct Run {
    tokens: Vec<Token>,
    atoms: Atoms,
}

impl Run {
    /// Close the run: unless it is only whitespace, it becomes an anonymous
    /// paragraph box after the children so far, styled like its block.
    fn close(
        &mut self,
        nodes: &mut Vec<LayoutNode>,
        calcs: &mut CalcTable,
        stack: &mut Vec<NodeId>,
        children: &mut Vec<taffy::NodeId>,
        parent: NodeId,
        parent_style: &Option<StyleArc>,
    ) {
        let Self { tokens, atoms } = std::mem::take(self);
        let Some(style) = parent_style else {
            return;
        };
        let paragraph = Paragraph::new(&tokens, style);
        if paragraph.is_empty() {
            return;
        }
        let atoms = add_atoms(nodes, calcs, stack, atoms);
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
    stack: &mut Vec<NodeId>,
    atoms: Atoms,
) -> Vec<taffy::NodeId> {
    atoms
        .into_iter()
        .map(|(id, computed)| {
            calcs.record(&computed);
            let node = &mut nodes[id.index() as usize];
            node.in_tree = true;
            node.style = stylo_taffy::to_taffy_style(&computed);
            stack.push(id);
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

/// The content of inline element `id`, in tree order: its start, the text
/// of each text node with the style of its element, nested inline elements,
/// atomic inlines, and its end.
fn inline_tokens(
    doc: &Document,
    styles: &Styles,
    id: NodeId,
    style: &StyleArc,
    tokens: &mut Vec<Token>,
    atoms: &mut Atoms,
) {
    tokens.push(InlineToken::Open(style.clone()));
    for child in doc.children(id) {
        match doc.node(child).map(|node| &node.data) {
            Some(NodeData::Text(content)) => {
                tokens.push(InlineToken::Text(content.clone(), style.clone()));
            }
            Some(NodeData::Element(_)) => {
                if let Some(child_style) = styles.computed(child) {
                    if is_atomic_inline(&child_style) {
                        tokens.push(InlineToken::Atom(
                            child.index() as usize,
                            child_style.clone(),
                        ));
                        atoms.push((child, child_style));
                    } else {
                        inline_tokens(doc, styles, child, &child_style, tokens, atoms);
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
