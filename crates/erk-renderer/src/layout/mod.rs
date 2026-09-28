//! Box layout with Taffy's low-level API.
//!
//! The DOM is the layout tree: Taffy walks Erk's nodes through the traits
//! below instead of a copy in a `TaffyTree`. Per-node layout state lives in a
//! side table indexed by `NodeId::index()`, like the style state in
//! erk-style. The trait implementations follow blitz-dom 0.3.0-beta.2,
//! src/layout/mod.rs (MIT OR Apache-2.0).
//!
//! M0 has no inline formatting context. A block whose children are only text
//! and inline elements becomes a paragraph leaf: Parley shapes its whole text
//! with the block's style and Taffy sees only the resulting width and height.
//! A block that mixes block children with text keeps the blocks and drops
//! the text; anonymous block boxes and the real inline layout come in M1.

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
}

/// Lay out `doc` in a viewport of `width` × `height` CSS pixels.
pub(crate) fn layout(
    doc: &Document,
    styles: &Styles,
    text: &mut TextEngine,
    width: f32,
    height: f32,
) -> Layouts {
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
    Layouts {
        nodes: nodes
            .into_iter()
            .map(|node| node.in_tree.then_some(node.layout))
            .collect(),
        text: text_layouts,
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
            let mut blocks = Vec::new();
            let mut has_inline_content = false;
            for child in doc.children(parent) {
                match doc.node(child).map(|node| &node.data) {
                    Some(NodeData::Text(text)) => {
                        has_inline_content |= !text.trim_ascii().is_empty();
                    }
                    Some(NodeData::Element(_)) => {
                        // Elements inside display:none are not styled, so a
                        // missing style means no box.
                        let Some(computed) = styles.computed(child) else {
                            continue;
                        };
                        if is_inline_level(&computed) {
                            has_inline_content = true;
                        } else {
                            blocks.push((child, computed));
                        }
                    }
                    _ => {}
                }
            }

            // A paragraph leaf: only inline content, laid out as one run.
            if blocks.is_empty() && has_inline_content && parent != doc.root() {
                if let Some(computed) = styles.computed(parent) {
                    let content = inline_text(doc, styles, parent);
                    nodes[parent.index() as usize].paragraph =
                        Some(Paragraph::new(&content, &computed));
                }
                continue;
            }

            let mut children = Vec::new();
            for (child, computed) in blocks {
                let style = stylo_taffy::to_taffy_style(&computed);
                if style.display == Display::None {
                    continue;
                }
                calcs.record(&computed);
                let node = &mut nodes[child.index() as usize];
                node.in_tree = true;
                node.style = style;
                children.push(taffy_id(child));
                stack.push(child);
            }
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
