use html5ever::tree_builder::QuirksMode;
use html5ever::{LocalName, QualName, local_name, ns};

use crate::{Arena, Attribute, ElementData, Node, NodeData, NodeId};

/// A DOM tree: an arena of nodes plus the id of the document node.
pub struct Document {
    nodes: Arena<Node>,
    root: NodeId,
    pub(crate) quirks_mode: QuirksMode,
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document containing only the document node.
    pub fn new() -> Self {
        let mut nodes = Arena::new();
        let root = nodes.insert(Node::new(NodeData::Document));
        Self {
            nodes,
            root,
            quirks_mode: QuirksMode::NoQuirks,
        }
    }

    pub fn root(&self) -> NodeId {
        self.root
    }

    pub fn quirks_mode(&self) -> QuirksMode {
        self.quirks_mode
    }

    /// The node behind `id`, or `None` if it was removed.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Number of arena slots; see [`Arena::capacity_hint`].
    pub fn capacity_hint(&self) -> usize {
        self.nodes.capacity_hint()
    }

    pub fn children(&self, parent: NodeId) -> Children<'_> {
        Children {
            doc: self,
            next: self.get(parent).first_child,
        }
    }

    /// Create a detached node.
    pub fn create(&mut self, data: NodeData) -> NodeId {
        self.nodes.insert(Node::new(data))
    }

    /// Make `child` the last child of `parent`, detaching it first if it is
    /// attached elsewhere.
    pub fn append(&mut self, parent: NodeId, child: NodeId) {
        self.detach(child);
        let last = self.get(parent).last_child;
        {
            let child_node = self.get_mut(child);
            child_node.parent = Some(parent);
            child_node.prev_sibling = last;
        }
        match last {
            Some(last) => self.get_mut(last).next_sibling = Some(child),
            None => self.get_mut(parent).first_child = Some(child),
        }
        self.get_mut(parent).last_child = Some(child);
    }

    /// Insert `child` immediately before `sibling`, detaching it first if it
    /// is attached elsewhere.
    ///
    /// # Panics
    ///
    /// If `sibling` has no parent.
    pub fn insert_before(&mut self, sibling: NodeId, child: NodeId) {
        self.detach(child);
        let sibling_node = self.get(sibling);
        let parent = sibling_node
            .parent
            .expect("insert_before: sibling has no parent");
        let prev = sibling_node.prev_sibling;
        {
            let child_node = self.get_mut(child);
            child_node.parent = Some(parent);
            child_node.prev_sibling = prev;
            child_node.next_sibling = Some(sibling);
        }
        self.get_mut(sibling).prev_sibling = Some(child);
        match prev {
            Some(prev) => self.get_mut(prev).next_sibling = Some(child),
            None => self.get_mut(parent).first_child = Some(child),
        }
    }

    /// A new element named `tag`, not in the tree yet; HTML lowercases the
    /// name (DOM's `createElement`). A `<template>` gets its contents
    /// fragment.
    pub fn create_element(&mut self, tag: &str) -> Result<NodeId, MutationError> {
        if !is_name(tag) {
            return Err(MutationError::InvalidName);
        }
        let name = QualName::new(None, ns!(html), LocalName::from(tag.to_ascii_lowercase()));
        let template = name.local == local_name!("template");
        let mut element = ElementData::new(name, Vec::new());
        if template {
            element.template_contents = Some(self.create(NodeData::DocumentFragment));
        }
        Ok(self.create(NodeData::Element(element)))
    }

    /// A new text node, not in the tree yet.
    pub fn create_text(&mut self, text: &str) -> NodeId {
        self.create(NodeData::Text(text.to_owned()))
    }

    /// Insert `child` into `parent`, before `before` or last, moving it from
    /// wherever it was, as DOM's `insertBefore` does; refused where DOM's
    /// pre-insert validity refuses it.
    pub fn insert(
        &mut self,
        parent: NodeId,
        child: NodeId,
        before: Option<NodeId>,
    ) -> Result<(), MutationError> {
        let (Some(parent_node), Some(child_node)) = (self.nodes.get(parent), self.nodes.get(child))
        else {
            return Err(MutationError::Stale);
        };
        let parent_takes = match &parent_node.data {
            NodeData::Element(_) | NodeData::DocumentFragment => true,
            // A document holds elements, comments and processing
            // instructions, never text.
            NodeData::Document => !matches!(child_node.data, NodeData::Text(_)),
            _ => false,
        };
        let child_moves = !matches!(
            child_node.data,
            NodeData::Document | NodeData::DocumentFragment | NodeData::Doctype { .. }
        );
        if !parent_takes || !child_moves {
            return Err(MutationError::Hierarchy);
        }
        // Not into itself or its own descendant.
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            if id == child {
                return Err(MutationError::Hierarchy);
            }
            ancestor = self.nodes.get(id).and_then(|node| node.parent);
        }
        match before {
            None => self.append(parent, child),
            Some(before) => {
                let sibling = self.nodes.get(before).ok_or(MutationError::Stale)?;
                if sibling.parent != Some(parent) {
                    return Err(MutationError::Hierarchy);
                }
                // Before itself: where it already is.
                if before != child {
                    self.insert_before(before, child);
                }
            }
        }
        Ok(())
    }

    /// Set attribute `name` of element `id` to `value`, replacing its value
    /// if it has one; HTML lowercases the name.
    pub fn set_attr(&mut self, id: NodeId, name: &str, value: &str) -> Result<(), MutationError> {
        if !is_name(name) {
            return Err(MutationError::InvalidName);
        }
        let element = self.element_mut(id)?;
        let local = LocalName::from(name.to_ascii_lowercase());
        match element
            .attrs
            .iter_mut()
            .find(|attr| attr.name.ns == ns!() && attr.name.local == local)
        {
            Some(attr) => value.clone_into(&mut attr.value),
            None => element.attrs.push(Attribute {
                name: QualName::new(None, ns!(), local),
                value: value.to_owned(),
            }),
        }
        Ok(())
    }

    /// Remove attribute `name` of element `id`; whether it had one.
    pub fn remove_attr(&mut self, id: NodeId, name: &str) -> Result<bool, MutationError> {
        let element = self.element_mut(id)?;
        let local = LocalName::from(name.to_ascii_lowercase());
        let before = element.attrs.len();
        element
            .attrs
            .retain(|attr| !(attr.name.ns == ns!() && attr.name.local == local));
        Ok(element.attrs.len() != before)
    }

    fn element_mut(&mut self, id: NodeId) -> Result<&mut ElementData, MutationError> {
        match &mut self.nodes.get_mut(id).ok_or(MutationError::Stale)?.data {
            NodeData::Element(element) => Ok(element),
            _ => Err(MutationError::Hierarchy),
        }
    }

    /// Remove `id` from its parent. The node and its subtree stay in the
    /// arena and can be inserted again.
    pub fn detach(&mut self, id: NodeId) {
        let node = self.get(id);
        let Some(parent) = node.parent else {
            return;
        };
        let (prev, next) = (node.prev_sibling, node.next_sibling);
        match prev {
            Some(prev) => self.get_mut(prev).next_sibling = next,
            None => self.get_mut(parent).first_child = next,
        }
        match next {
            Some(next) => self.get_mut(next).prev_sibling = prev,
            None => self.get_mut(parent).last_child = prev,
        }
        let node = self.get_mut(id);
        node.parent = None;
        node.prev_sibling = None;
        node.next_sibling = None;
    }

    /// Remove `id` and everything in it, a `<template>`'s contents too: their
    /// ids go stale and never name another node. Whether `id` was a node
    /// that can be removed; the document node cannot.
    pub fn remove(&mut self, id: NodeId) -> bool {
        if id == self.root || self.nodes.get(id).is_none() {
            return false;
        }
        self.detach(id);
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            let Some(removed) = self.nodes.remove(node) else {
                continue;
            };
            let mut child = removed.first_child;
            while let Some(id) = child {
                stack.push(id);
                child = self.nodes.get(id).and_then(|node| node.next_sibling);
            }
            if let NodeData::Element(element) = removed.data {
                stack.extend(element.template_contents);
            }
        }
        true
    }

    /// Set `id`'s text as the DOM's `textContent` does: a text or comment
    /// node's data becomes `text`; an element's children are removed and
    /// replaced by one text node, or by none for an empty string. On other
    /// nodes it does nothing. `Err` if `id` is not a node (it was removed).
    pub fn set_text(&mut self, id: NodeId, text: &str) -> Result<(), StaleNode> {
        let node = self.nodes.get_mut(id).ok_or(StaleNode)?;
        match &mut node.data {
            NodeData::Text(data) | NodeData::Comment(data) => {
                text.clone_into(data);
            }
            NodeData::Element(_) => {
                while let Some(child) = self.get(id).first_child {
                    self.remove(child);
                }
                if !text.is_empty() {
                    let child = self.create(NodeData::Text(text.to_owned()));
                    self.append(id, child);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Remove every node but the document node, which is left empty.
    pub(crate) fn clear(&mut self) {
        self.nodes.remove_all_but(self.root);
        let root = self.get_mut(self.root);
        root.first_child = None;
        root.last_child = None;
        self.quirks_mode = QuirksMode::NoQuirks;
    }

    pub(crate) fn get(&self, id: NodeId) -> &Node {
        self.nodes.get(id).expect("stale NodeId")
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes.get_mut(id).expect("stale NodeId")
    }
}

/// The node an id named has been removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaleNode;

/// Why a change from outside the parser was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationError {
    /// A node it names was removed.
    Stale,
    /// The change would make a tree DOM does not allow: a node inside
    /// itself, the document node moved, a child under a node that cannot
    /// have it, `before` not a child of the parent, an attribute on a node
    /// that is not an element.
    Hierarchy,
    /// A tag or attribute name that is not a name.
    InvalidName,
}

/// Whether `name` is a name an element or attribute may have: XML's Name,
/// which is what the parser produces and selectors can match. A letter,
/// `_`, `:` or any non-ASCII character first; letters, digits, `-`, `.`,
/// `_`, `:` and non-ASCII after.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let starts = |c: char| c.is_ascii_alphabetic() || c == '_' || c == ':' || !c.is_ascii();
    starts(first) && chars.all(|c| starts(c) || c.is_ascii_digit() || c == '-' || c == '.')
}

/// Iterator over a node's children, first to last.
pub struct Children<'a> {
    doc: &'a Document,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;

    fn next(&mut self) -> Option<NodeId> {
        let id = self.next?;
        self.next = self.doc.get(id).next_sibling;
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &mut Document, s: &str) -> NodeId {
        doc.create(NodeData::Text(s.to_owned()))
    }

    fn texts(doc: &Document, parent: NodeId) -> Vec<&str> {
        doc.children(parent)
            .map(|id| doc.get(id).as_text().unwrap())
            .collect()
    }

    #[test]
    fn append_keeps_order() {
        let mut doc = Document::new();
        let root = doc.root();
        for s in ["a", "b", "c"] {
            let id = text(&mut doc, s);
            doc.append(root, id);
        }
        assert_eq!(texts(&doc, root), ["a", "b", "c"]);
    }

    #[test]
    fn insert_before_first_child_becomes_first() {
        let mut doc = Document::new();
        let root = doc.root();
        let b = text(&mut doc, "b");
        doc.append(root, b);
        let a = text(&mut doc, "a");
        doc.insert_before(b, a);

        assert_eq!(texts(&doc, root), ["a", "b"]);
        assert_eq!(doc.get(root).first_child, Some(a));
        assert_eq!(doc.get(b).prev_sibling, Some(a));
    }

    #[test]
    fn detach_repairs_sibling_links() {
        let mut doc = Document::new();
        let root = doc.root();
        let ids: Vec<_> = ["a", "b", "c"]
            .into_iter()
            .map(|s| {
                let id = text(&mut doc, s);
                doc.append(root, id);
                id
            })
            .collect();

        doc.detach(ids[1]);

        assert_eq!(texts(&doc, root), ["a", "c"]);
        assert_eq!(doc.get(ids[0]).next_sibling, Some(ids[2]));
        assert_eq!(doc.get(ids[2]).prev_sibling, Some(ids[0]));
        assert_eq!(doc.get(ids[1]).parent, None);
    }

    #[test]
    fn detaching_the_only_child_empties_the_parent() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = text(&mut doc, "a");
        doc.append(root, a);
        doc.detach(a);

        assert_eq!(doc.get(root).first_child, None);
        assert_eq!(doc.get(root).last_child, None);
    }

    #[test]
    fn appending_an_attached_node_moves_it() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = text(&mut doc, "a");
        let b = text(&mut doc, "b");
        doc.append(root, a);
        doc.append(root, b);
        doc.append(root, a);

        assert_eq!(texts(&doc, root), ["b", "a"]);
    }
}
