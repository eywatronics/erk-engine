//! Per-node style state, kept beside the DOM rather than inside it.
//!
//! erk-dom is a leaf crate and does not know Stylo exists, so the data Stylo
//! wants attached to each element lives here, one [`StyledNode`] per arena
//! slot. This is a deliberate departure from Blitz, which stores it on the
//! node itself.
//!
//! Stylo requires its element handle to be exactly one pointer wide (the
//! style sharing cache erases the element type and asserts on its size), so
//! the handle is a reference to a `StyledNode`, and each `StyledNode` carries
//! its own id and a reference back to the tree. The tree and its nodes point
//! at each other; the nodes are installed through a `OnceLock` after the tree
//! exists, which keeps the cycle in safe code.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use erk_dom::{Document, NodeData, NodeId, local_name};
use selectors::matching::ElementSelectorFlags;
use style::Atom;
use style::context::QuirksMode;
use style::data::{ElementDataMut, ElementDataRef, ElementDataWrapper};
use style::properties::{PropertyDeclarationBlock, parse_style_attribute};
use style::servo_arc::Arc;
use style::shared_lock::{Locked, SharedRwLock};
use style::stylesheets::{CssRuleType, UrlExtraData};
use style_dom::ElementState;

/// A document plus the style state of each of its nodes.
pub(crate) struct StyledTree<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) guard: &'a SharedRwLock,
    nodes: OnceLock<Vec<StyledNode<'a>>>,
}

impl<'a> StyledTree<'a> {
    pub(crate) fn new(doc: &'a Document, guard: &'a SharedRwLock) -> Self {
        Self {
            doc,
            guard,
            nodes: OnceLock::new(),
        }
    }

    /// Create one node per arena slot, with the parts that come from
    /// attributes (`id`, `style`) parsed up front.
    pub(crate) fn populate(&'a self, url: &UrlExtraData) {
        let mut nodes: Vec<_> = (0..self.doc.capacity_hint())
            .map(|_| StyledNode::new(self))
            .collect();

        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(self.doc.children(id));
            let node = &mut nodes[id.index() as usize];
            node.id = Some(id);
            let Some(NodeData::Element(element)) = self.doc.node(id).map(|node| &node.data) else {
                continue;
            };
            node.id_attr = element.attr(&local_name!("id")).map(Atom::from);
            node.style_attribute = element.attr(&local_name!("style")).map(|css| {
                Arc::new(self.guard.wrap(parse_style_attribute(
                    css,
                    url,
                    None,
                    QuirksMode::NoQuirks,
                    CssRuleType::Style,
                )))
            });
            if element.name.local == local_name!("a")
                && element.attr(&local_name!("href")).is_some()
            {
                node.state = ElementState::UNVISITED;
            }
        }

        if self.nodes.set(nodes).is_err() {
            panic!("StyledTree::populate called twice");
        }
    }

    pub(crate) fn node(&'a self, id: NodeId) -> &'a StyledNode<'a> {
        &self.nodes.get().expect("StyledTree used before populate")[id.index() as usize]
    }

    pub(crate) fn nodes(&self) -> &[StyledNode<'a>] {
        self.nodes.get().map(Vec::as_slice).unwrap_or_default()
    }
}

pub(crate) struct StyledNode<'a> {
    pub(crate) tree: &'a StyledTree<'a>,
    /// `None` for arena slots that are not part of the document.
    pub(crate) id: Option<NodeId>,
    data: ElementDataWrapper,
    has_data: AtomicBool,
    dirty_descendants: AtomicBool,
    handled_snapshot: AtomicBool,
    selector_flags: AtomicUsize,
    pub(crate) state: ElementState,
    pub(crate) id_attr: Option<Atom>,
    pub(crate) style_attribute: Option<Arc<Locked<PropertyDeclarationBlock>>>,
}

impl<'a> StyledNode<'a> {
    fn new(tree: &'a StyledTree<'a>) -> Self {
        Self {
            tree,
            id: None,
            data: ElementDataWrapper::default(),
            has_data: AtomicBool::new(false),
            dirty_descendants: AtomicBool::new(false),
            handled_snapshot: AtomicBool::new(false),
            selector_flags: AtomicUsize::new(0),
            state: ElementState::empty(),
            id_attr: None,
            style_attribute: None,
        }
    }

    pub(crate) fn ensure_data(&self) -> ElementDataMut<'_> {
        self.has_data.store(true, Ordering::Relaxed);
        self.data.borrow_mut()
    }

    pub(crate) fn clear_data(&self) {
        *self.data.borrow_mut() = Default::default();
        self.has_data.store(false, Ordering::Relaxed);
    }

    pub(crate) fn has_data(&self) -> bool {
        self.has_data.load(Ordering::Relaxed)
    }

    pub(crate) fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.has_data().then(|| self.data.borrow())
    }

    pub(crate) fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        self.has_data().then(|| self.data.borrow_mut())
    }

    pub(crate) fn dirty_descendants(&self) -> bool {
        self.dirty_descendants.load(Ordering::Relaxed)
    }

    pub(crate) fn set_dirty_descendants(&self, dirty: bool) {
        self.dirty_descendants.store(dirty, Ordering::Relaxed);
    }

    pub(crate) fn handled_snapshot(&self) -> bool {
        self.handled_snapshot.load(Ordering::Relaxed)
    }

    pub(crate) fn set_handled_snapshot(&self) {
        self.handled_snapshot.store(true, Ordering::Relaxed);
    }

    pub(crate) fn selector_flags(&self) -> ElementSelectorFlags {
        ElementSelectorFlags::from_bits_retain(self.selector_flags.load(Ordering::Relaxed))
    }

    pub(crate) fn insert_selector_flags(&self, flags: ElementSelectorFlags) {
        self.selector_flags
            .fetch_or(flags.bits(), Ordering::Relaxed);
    }
}
