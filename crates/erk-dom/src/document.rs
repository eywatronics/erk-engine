use html5ever::tree_builder::QuirksMode;

use crate::{Arena, Node, NodeData, NodeId};

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

    pub(crate) fn get(&self, id: NodeId) -> &Node {
        self.nodes.get(id).expect("stale NodeId")
    }

    pub(crate) fn get_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes.get_mut(id).expect("stale NodeId")
    }
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
