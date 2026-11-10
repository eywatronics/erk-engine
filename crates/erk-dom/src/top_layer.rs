//! The top layer (CSS Position 4 §3): the elements drawn above the whole
//! page, last added on top, each laid out against the viewport whatever
//! its ancestors are. An open popover is there (M5.9); a modal dialog
//! will be.
//!
//! The document keeps the list because styling (`:popover-open`), layout
//! and painting all read it, as they read attributes. An element leaves
//! it when it leaves the document, or is moved: HTML hides a popover
//! that is removed, whether or not it comes back.

use crate::{Document, MutationError, NodeId};

impl Document {
    /// The elements in the top layer, the one drawn on top last.
    pub fn top_layer(&self) -> &[NodeId] {
        &self.top_layer
    }

    /// Whether element `id` is in the top layer.
    pub fn in_top_layer(&self, id: NodeId) -> bool {
        self.top_layer.contains(&id)
    }

    /// Put element `id` on top of the top layer; one already in it moves
    /// to the top. `Err` for a node that is not an element in the
    /// document.
    pub fn add_to_top_layer(&mut self, id: NodeId) -> Result<(), MutationError> {
        let node = self.node(id).ok_or(MutationError::Stale)?;
        if node.as_element().is_none() || !self.is_connected(id) {
            return Err(MutationError::Hierarchy);
        }
        self.top_layer.retain(|entry| *entry != id);
        self.top_layer.push(id);
        Ok(())
    }

    /// Take element `id` out of the top layer; whether it was in it.
    pub fn remove_from_top_layer(&mut self, id: NodeId) -> bool {
        let before = self.top_layer.len();
        self.top_layer.retain(|entry| *entry != id);
        self.top_layer.len() != before
    }

    /// `id` is leaving its parent: it and what it holds leave the top
    /// layer.
    pub(crate) fn leave_top_layer(&mut self, id: NodeId) {
        if self.top_layer.is_empty() {
            return;
        }
        let inside = |doc: &Document, mut at: NodeId| loop {
            if at == id {
                return true;
            }
            match doc.node(at).and_then(|node| node.parent()) {
                Some(parent) => at = parent,
                None => return false,
            }
        };
        let kept: Vec<NodeId> = self
            .top_layer
            .iter()
            .copied()
            .filter(|entry| !inside(self, *entry))
            .collect();
        self.top_layer = kept;
    }

    /// Whether `id` is in the document, under its document node.
    fn is_connected(&self, mut id: NodeId) -> bool {
        loop {
            if id == self.root() {
                return true;
            }
            match self.node(id).and_then(|node| node.parent()) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Document, MutationError, NodeId, local_name};

    fn find(doc: &Document, id: &str) -> NodeId {
        let mut stack = vec![doc.root()];
        while let Some(at) = stack.pop() {
            stack.extend(doc.children(at));
            if doc
                .node(at)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.attr(&local_name!("id")) == Some(id))
            {
                return at;
            }
        }
        panic!("no #{id}");
    }

    #[test]
    fn the_last_added_is_on_top_and_one_added_again_moves_there() {
        let mut doc = Document::parse_html("<div id=a></div><div id=b></div>");
        let (a, b) = (find(&doc, "a"), find(&doc, "b"));
        doc.add_to_top_layer(a).unwrap();
        doc.add_to_top_layer(b).unwrap();
        assert_eq!(doc.top_layer(), [a, b]);
        doc.add_to_top_layer(a).unwrap();
        assert_eq!(doc.top_layer(), [b, a]);
        assert!(doc.remove_from_top_layer(b));
        assert!(!doc.remove_from_top_layer(b));
        assert_eq!(doc.top_layer(), [a]);
        assert!(doc.in_top_layer(a) && !doc.in_top_layer(b));
    }

    #[test]
    fn only_an_element_in_the_document_joins_it() {
        let mut doc = Document::parse_html("<div id=a>text</div>");
        let a = find(&doc, "a");
        let text = doc.children(a).next().unwrap();
        assert_eq!(doc.add_to_top_layer(text), Err(MutationError::Hierarchy));
        let loose = doc.create_element("div").unwrap();
        assert_eq!(doc.add_to_top_layer(loose), Err(MutationError::Hierarchy));
        doc.remove(a);
        assert_eq!(doc.add_to_top_layer(a), Err(MutationError::Stale));
        assert!(doc.top_layer().is_empty());
    }

    #[test]
    fn an_element_leaves_it_when_it_or_an_ancestor_is_moved_or_removed() {
        let mut doc = Document::parse_html(
            "<div id=outer><div id=inner></div></div><div id=other></div><div id=gone></div>",
        );
        let (outer, inner, other, gone) = (
            find(&doc, "outer"),
            find(&doc, "inner"),
            find(&doc, "other"),
            find(&doc, "gone"),
        );
        for id in [inner, other, gone] {
            doc.add_to_top_layer(id).unwrap();
        }
        // Moved, even back into the document.
        let body = doc.node(outer).unwrap().parent().unwrap();
        doc.insert(body, outer, None).unwrap();
        assert_eq!(doc.top_layer(), [other, gone]);
        doc.remove(gone);
        assert_eq!(doc.top_layer(), [other]);
        doc.detach(other);
        assert!(doc.top_layer().is_empty());
    }
}
