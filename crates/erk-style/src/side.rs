//! Per-node style state, kept beside the DOM rather than inside it.
//!
//! erk-dom is a leaf crate and does not know Stylo exists, so the data Stylo
//! wants attached to each element lives here, one [`Slot`] per arena slot.
//! This is a deliberate departure from Blitz, which stores it on the node
//! itself.
//!
//! A slot outlives a frame (M5.3): Stylo's element data, the selector flags
//! matching left behind and the parsed `style` attribute are what the next
//! frame restyles from. A slot belongs to one node, by id with its
//! generation; when the arena gives the slot to another node it starts over.
//!
//! Stylo requires its element handle to be exactly one pointer wide (the
//! style sharing cache erases the element type and asserts on its size), so
//! the handle is a reference to a per-frame [`StyledNode`], which carries
//! its id, its slot and a reference back to the tree. The tree and its nodes
//! point at each other; the nodes are installed through a `OnceLock` after
//! the tree exists, which keeps the cycle in safe code.

use std::ops::Deref;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use erk_dom::{Document, ElementData, NodeId, local_name};
use selectors::matching::ElementSelectorFlags;
use style::Atom;
use style::context::QuirksMode;
use style::data::{ElementDataMut, ElementDataRef, ElementDataWrapper};
use style::properties::{PropertyDeclarationBlock, parse_style_attribute};
use style::servo_arc::Arc;
use style::shared_lock::{Locked, SharedRwLock};
use style::stylesheets::{CssRuleType, UrlExtraData};
use style_dom::ElementState;

/// What styling keeps of one arena slot, from frame to frame.
pub(crate) struct Slot {
    /// The node this slot belongs to; `None` while it belongs to none.
    pub(crate) owner: Option<NodeId>,
    /// Its parent at the last frame: a node that moved is styled again.
    pub(crate) parent: Option<NodeId>,
    /// Whether the node was outside the root element at the last frame,
    /// where styling does not reach.
    pub(crate) outside: bool,
    data: ElementDataWrapper,
    has_data: AtomicBool,
    dirty_descendants: AtomicBool,
    /// Whether the element has a snapshot this frame (set before the
    /// traversal), and whether the traversal used it.
    pub(crate) has_snapshot: bool,
    handled_snapshot: AtomicBool,
    selector_flags: AtomicUsize,
    pub(crate) state: ElementState,
    pub(crate) id_attr: Option<Atom>,
    pub(crate) style_attribute: Option<Arc<Locked<PropertyDeclarationBlock>>>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            owner: None,
            parent: None,
            outside: false,
            data: ElementDataWrapper::default(),
            has_data: AtomicBool::new(false),
            dirty_descendants: AtomicBool::new(false),
            has_snapshot: false,
            handled_snapshot: AtomicBool::new(false),
            selector_flags: AtomicUsize::new(0),
            state: ElementState::empty(),
            id_attr: None,
            style_attribute: None,
        }
    }
}

impl Slot {
    /// A slot for `id`, with nothing styled yet.
    pub(crate) fn new(id: NodeId) -> Self {
        Self {
            owner: Some(id),
            ..Self::default()
        }
    }

    /// Read what styling takes from `element`'s attributes: its `id` and
    /// its `style`.
    pub(crate) fn read_attributes(
        &mut self,
        element: &ElementData,
        url: &UrlExtraData,
        guard: &SharedRwLock,
    ) {
        self.id_attr = element.attr(&local_name!("id")).map(Atom::from);
        self.style_attribute = element.attr(&local_name!("style")).map(|css| {
            Arc::new(guard.wrap(parse_style_attribute(
                css,
                url,
                None,
                QuirksMode::NoQuirks,
                CssRuleType::Style,
            )))
        });
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

    /// Ready for a new frame: no snapshot yet, none handled.
    pub(crate) fn start_frame(&mut self) {
        self.has_snapshot = false;
        *self.handled_snapshot.get_mut() = false;
    }

    pub(crate) fn selector_flags(&self) -> ElementSelectorFlags {
        ElementSelectorFlags::from_bits_retain(self.selector_flags.load(Ordering::Relaxed))
    }

    pub(crate) fn insert_selector_flags(&self, flags: ElementSelectorFlags) {
        self.selector_flags
            .fetch_or(flags.bits(), Ordering::Relaxed);
    }
}

/// A document plus the style state of each of its nodes, for one frame.
pub(crate) struct StyledTree<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) guard: &'a SharedRwLock,
    slots: &'a [Slot],
    nodes: OnceLock<Vec<StyledNode<'a>>>,
}

impl<'a> StyledTree<'a> {
    /// `slots` must hold one slot per arena slot of `doc`, each node's own.
    pub(crate) fn new(doc: &'a Document, guard: &'a SharedRwLock, slots: &'a [Slot]) -> Self {
        Self {
            doc,
            guard,
            slots,
            nodes: OnceLock::new(),
        }
    }

    /// Create the handles: one per arena slot, with an id for the nodes
    /// that are in the document.
    pub(crate) fn link(&'a self) {
        let mut nodes: Vec<_> = self
            .slots
            .iter()
            .map(|slot| StyledNode {
                tree: self,
                id: None,
                slot,
            })
            .collect();
        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(self.doc.children(id));
            nodes[id.index() as usize].id = Some(id);
        }
        if self.nodes.set(nodes).is_err() {
            panic!("StyledTree::link called twice");
        }
    }

    pub(crate) fn node(&'a self, id: NodeId) -> &'a StyledNode<'a> {
        &self.nodes.get().expect("StyledTree used before link")[id.index() as usize]
    }
}

/// One node's handle for a frame: its id, its slot and its tree.
pub(crate) struct StyledNode<'a> {
    pub(crate) tree: &'a StyledTree<'a>,
    /// `None` for arena slots that are not part of the document.
    pub(crate) id: Option<NodeId>,
    slot: &'a Slot,
}

impl Deref for StyledNode<'_> {
    type Target = Slot;

    fn deref(&self) -> &Slot {
        self.slot
    }
}

/// One slot per arena slot of `doc`, each read fresh from its node: what a
/// full style starts from. `interaction` gives the user's states.
pub(crate) fn fresh_slots(
    doc: &Document,
    url: &UrlExtraData,
    guard: &SharedRwLock,
    states: &[ElementState],
) -> Vec<Slot> {
    let mut slots: Vec<Slot> = (0..doc.capacity_hint()).map(|_| Slot::default()).collect();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        stack.extend(doc.children(id));
        let slot = &mut slots[id.index() as usize];
        *slot = Slot::new(id);
        if let Some(element) = doc.node(id).and_then(|node| node.as_element()) {
            slot.read_attributes(element, url, guard);
            slot.state = states[id.index() as usize];
        }
    }
    slots
}
