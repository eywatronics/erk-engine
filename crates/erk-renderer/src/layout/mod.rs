//! Box layout with Taffy's low-level API.
//!
//! The DOM is the layout tree: Taffy walks Erk's nodes through the traits
//! below instead of a copy in a `TaffyTree`. Per-node layout state lives in a
//! side table indexed by `NodeId::index()`, like the style state in
//! erk-style. The trait implementations follow blitz-dom 0.3.0-beta.2,
//! src/layout/mod.rs (MIT OR Apache-2.0).
//!
//! There is no inline formatting context yet (M1.3). A block whose children
//! are only text and inline elements becomes a paragraph leaf: Parley shapes
//! its whole text with the block's style and Taffy sees only the resulting
//! width and height. In a block that mixes block children with text, each run
//! of inline content becomes an anonymous paragraph box (CSS 2 §9.2.1.1).
//! Anonymous boxes live only in this side table, at indices past the arena's
//! slots: the DOM, and the NodeIds a host sees, never contain them.

mod calc;

#[cfg(test)]
mod tests;

use erk_dom::{Document, NodeData, NodeId};
use erk_style::style::Atom;
use erk_style::style::values::specified::box_::DisplayOutside;
use erk_style::{ComputedValues, Styles};
use taffy::{
    AvailableSpace, BlockContext, Cache, CacheTree, Display, Layout, LayoutBlockContainer,
    LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    RoundTree, Size, Style, TraversePartialTree, TraverseTree, compute_block_layout,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_leaf_layout,
    compute_root_layout, round_layout,
};

use self::calc::CalcTable;
use crate::text::{Paragraph, TextBrush, TextEngine};

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

/// A paragraph's text and its shaped, line-broken layout.
pub(crate) struct ShapedText {
    pub(crate) text: String,
    pub(crate) layout: parley::Layout<TextBrush>,
}

impl Layouts {
    /// The final (pixel-rounded) layout of a box, relative to its parent box.
    /// `None` for nodes that generate no box.
    pub(crate) fn get(&self, id: NodeId) -> Option<&Layout> {
        self.nodes.get(id.index() as usize)?.as_ref()
    }

    /// The shaped, line-broken text of a paragraph leaf, positioned relative
    /// to the leaf's content box.
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
    let mut tree = LayoutTree::build(doc, styles, text);
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

    // Shape each paragraph once more at its final width, for painting.
    let LayoutTree { nodes, text, .. } = tree;
    let text_layouts = nodes
        .iter()
        .map(|node| {
            let paragraph = node.paragraph.as_ref().filter(|_| node.in_tree)?;
            let layout = &node.layout;
            let content_width = layout.size.width
                - layout.padding.left
                - layout.padding.right
                - layout.border.left
                - layout.border.right;
            Some(ShapedText {
                text: paragraph.text.clone(),
                layout: text.shape(paragraph, Some(content_width)),
            })
        })
        .collect();
    let mut text: Vec<Option<ShapedText>> = text_layouts;
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

#[derive(Default)]
struct LayoutNode {
    /// Whether this arena slot generates a box in the layout tree.
    in_tree: bool,
    children: Vec<taffy::NodeId>,
    style: Style<Atom>,
    /// Set for paragraph leaves: blocks laid out as one run of text.
    paragraph: Option<Paragraph>,
    /// For an anonymous paragraph box, the arena index of its block.
    anonymous_parent: Option<usize>,
    cache: Cache,
    unrounded: Layout,
    layout: Layout,
}

struct LayoutTree<'t> {
    nodes: Vec<LayoutNode>,
    calcs: CalcTable,
    text: &'t mut TextEngine,
}

impl<'t> LayoutTree<'t> {
    fn build(doc: &Document, styles: &Styles, text: &'t mut TextEngine) -> Self {
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
            // The children in tree order: block-level elements, and the text
            // of everything inline (text nodes and inline elements).
            let mut entries = Vec::new();
            let mut has_blocks = false;
            for child in doc.children(parent) {
                match doc.node(child).map(|node| &node.data) {
                    Some(NodeData::Text(text)) => entries.push(Entry::Inline(text.clone())),
                    Some(NodeData::Element(_)) => {
                        // Elements inside display:none are not styled, so a
                        // missing style means no box.
                        let Some(computed) = styles.computed(child) else {
                            continue;
                        };
                        if is_inline_level(&computed) {
                            entries.push(Entry::Inline(inline_text(doc, styles, child)));
                        } else {
                            has_blocks = true;
                            entries.push(Entry::Block(child, computed));
                        }
                    }
                    _ => {}
                }
            }
            let parent_style = styles.computed(parent);

            // A paragraph leaf: only inline content, laid out as one run.
            if !has_blocks && parent != doc.root() {
                let content: String = entries
                    .into_iter()
                    .filter_map(|entry| match entry {
                        Entry::Inline(text) => Some(text),
                        Entry::Block(..) => None,
                    })
                    .collect();
                if let Some(computed) = parent_style
                    && !content.trim_ascii().is_empty()
                {
                    nodes[parent.index() as usize].paragraph =
                        Some(Paragraph::new(&content, &computed));
                }
                continue;
            }

            let mut children = Vec::new();
            let mut run = String::new();
            for entry in entries {
                match entry {
                    Entry::Inline(text) => run.push_str(&text),
                    Entry::Block(child, computed) => {
                        let style = stylo_taffy::to_taffy_style(&computed);
                        if style.display == Display::None {
                            continue;
                        }
                        push_anonymous(&mut nodes, &mut children, parent, &parent_style, &mut run);
                        calcs.record(&computed);
                        let node = &mut nodes[child.index() as usize];
                        node.in_tree = true;
                        node.style = style;
                        children.push(taffy_id(child));
                        stack.push(child);
                    }
                }
            }
            push_anonymous(&mut nodes, &mut children, parent, &parent_style, &mut run);
            nodes[parent.index() as usize].children = children;
        }

        Self { nodes, calcs, text }
    }

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
        let node = &self.nodes[usize::from(id)];
        if let Some(paragraph) = &node.paragraph {
            let text = &mut *self.text;
            let calcs = &self.calcs;
            return compute_leaf_layout(
                inputs,
                &node.style,
                |ptr, basis| calcs.resolve(ptr, basis),
                |known, available| text.measure(paragraph, known, available),
            );
        }
        match node.style.display {
            Display::Block => compute_block_layout(self, id, inputs, block_ctx),
            // A flow root establishes a new block formatting context: floats
            // and margins do not cross it.
            Display::FlowRoot => compute_block_layout(self, id, inputs, None),
            Display::Flex => compute_flexbox_layout(self, id, inputs),
            Display::Grid => compute_grid_layout(self, id, inputs),
            Display::None => LayoutOutput::HIDDEN,
        }
    }
}

/// A child of a block, as layout sees it.
enum Entry {
    Block(NodeId, erk_style::style::servo_arc::Arc<ComputedValues>),
    /// The text of a text node or of an inline element.
    Inline(String),
}

/// Close the current run of inline content: unless it is only whitespace,
/// it becomes an anonymous paragraph box after the children so far, styled
/// like its block.
fn push_anonymous(
    nodes: &mut Vec<LayoutNode>,
    children: &mut Vec<taffy::NodeId>,
    parent: NodeId,
    parent_style: &Option<erk_style::style::servo_arc::Arc<ComputedValues>>,
    run: &mut String,
) {
    let text = std::mem::take(run);
    let Some(style) = parent_style else {
        return;
    };
    if text.trim_ascii().is_empty() {
        return;
    }
    nodes.push(LayoutNode {
        in_tree: true,
        style: Style {
            display: Display::Block,
            ..Style::DEFAULT
        },
        paragraph: Some(Paragraph::new(&text, style)),
        anonymous_parent: Some(parent.index() as usize),
        ..LayoutNode::default()
    });
    children.push(taffy::NodeId::from(nodes.len() - 1));
}

fn is_inline_level(style: &ComputedValues) -> bool {
    style.get_box().clone_display().outside() == DisplayOutside::Inline
}

/// The text of `id`'s children, descending into inline elements, in tree
/// order.
fn inline_text(doc: &Document, styles: &Styles, id: NodeId) -> String {
    let mut text = String::new();
    for child in doc.children(id) {
        match doc.node(child).map(|node| &node.data) {
            Some(NodeData::Text(content)) => text.push_str(content),
            Some(NodeData::Element(_)) if styles.computed(child).is_some() => {
                text.push_str(&inline_text(doc, styles, child));
            }
            _ => {}
        }
    }
    text
}

fn taffy_id(id: NodeId) -> taffy::NodeId {
    taffy::NodeId::from(id.index() as usize)
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
