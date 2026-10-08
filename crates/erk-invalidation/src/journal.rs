//! What changed in the document since the last frame, coalesced (M5.1,
//! p2-incremental §3.2).
//!
//! The host's changes reach the DOM at once, so that what it reads back is
//! what it wrote (M4's API reads its own writes; TodoMVC does). What waits
//! for the frame is the costly part, invalidation, and the journal is what
//! it starts from: for each node touched this frame, its state before the
//! first touch. At the frame boundary [`Journal::take`] compares that with
//! the node now, so the last value wins, a class added and removed again
//! cancels out, and a node made and removed in the same frame leaves no
//! trace. The state before the first touch is also what Stylo's element
//! snapshots ask for (M5.3).
//!
//! Every path that changes the document records before it changes it. One
//! that forgets would leave a change invisible to invalidation; while
//! verifying, the engine compares the journal with the document's actual
//! difference from the last frame ([`snapshot`], [`difference`]).

use std::collections::HashMap;

use erk_dom::{Document, NodeData, NodeId};

/// A node's state as the journal compares it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// A text or comment node's data.
    text: Option<String>,
    /// An element's attributes, by name.
    attrs: Vec<(String, String)>,
    children: Vec<NodeId>,
}

impl State {
    fn of(doc: &Document, id: NodeId) -> Self {
        let Some(node) = doc.node(id) else {
            return Self::default();
        };
        let (text, mut attrs) = match &node.data {
            NodeData::Text(text) | NodeData::Comment(text) => (Some(text.clone()), Vec::new()),
            NodeData::Element(element) => (
                None,
                element
                    .attrs
                    .iter()
                    .map(|attr| (attr.name.local.to_string(), attr.value.clone()))
                    .collect(),
            ),
            _ => (None, Vec::new()),
        };
        attrs.sort();
        Self {
            text,
            attrs,
            children: doc.children(id).collect(),
        }
    }
}

/// A frame's changes, coalesced: only what differs from the last frame, in
/// nodes that are in the document now and were then.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    /// The whole document was replaced: everything is new.
    pub everything: bool,
    /// Text and comment nodes whose data changed.
    pub text: Vec<NodeId>,
    /// Elements and the names of the attributes that changed.
    pub attrs: Vec<(NodeId, Vec<String>)>,
    /// Nodes whose children changed: added, removed or moved.
    pub children: Vec<NodeId>,
}

impl Changes {
    /// How many changes there are.
    pub fn count(&self) -> usize {
        self.text.len() + self.attrs.len() + self.children.len()
    }

    /// The changes between `before` and `after`, for one node.
    fn add(&mut self, id: NodeId, before: &State, after: &State) {
        if before.text != after.text {
            self.text.push(id);
        }
        if before.attrs != after.attrs {
            let mut names: Vec<String> = before
                .attrs
                .iter()
                .chain(&after.attrs)
                .filter(|pair| !before.attrs.contains(pair) || !after.attrs.contains(pair))
                .map(|(name, _)| name.clone())
                .collect();
            names.sort();
            names.dedup();
            self.attrs.push((id, names));
        }
        if before.children != after.children {
            self.children.push(id);
        }
    }

    fn sort(&mut self) {
        self.text.sort_by_key(|id| id.index());
        self.attrs.sort_by_key(|(id, _)| id.index());
        self.children.sort_by_key(|id| id.index());
    }
}

/// A node touched this frame.
struct Slot {
    id: NodeId,
    /// Made this frame: all of it is new, nothing to compare.
    created: bool,
    /// Its state before the first touch.
    before: State,
}

#[derive(Default)]
pub struct Journal {
    /// The touched nodes, by `NodeId::index()` (a side table: p2-incremental
    /// §3.12), and which entries are in use.
    slots: Vec<Option<Slot>>,
    touched: Vec<usize>,
    /// How many changes were recorded this frame, before coalescing.
    recorded: usize,
    everything: bool,
}

impl Journal {
    /// A journal for a document that is all new.
    pub fn new_document() -> Self {
        Self {
            everything: true,
            ..Self::default()
        }
    }

    /// `id` is about to change: its text, its attributes or its children.
    /// The first touch this frame keeps its state before.
    pub fn touch(&mut self, doc: &Document, id: NodeId) {
        self.recorded += 1;
        let at = id.index() as usize;
        if self.slots.len() <= at {
            self.slots.resize_with(at + 1, || None);
        }
        match &self.slots[at] {
            // Touched before this frame: the first state stands.
            Some(slot) if slot.id == id => {}
            // A slot of a node removed this frame, reused by a new one.
            _ => {
                if self.slots[at].is_none() {
                    self.touched.push(at);
                }
                self.slots[at] = Some(Slot {
                    id,
                    created: false,
                    before: State::of(doc, id),
                });
            }
        }
    }

    /// `id` was made this frame.
    pub fn created(&mut self, id: NodeId) {
        self.recorded += 1;
        let at = id.index() as usize;
        if self.slots.len() <= at {
            self.slots.resize_with(at + 1, || None);
        }
        if self.slots[at].is_none() {
            self.touched.push(at);
        }
        self.slots[at] = Some(Slot {
            id,
            created: true,
            before: State::default(),
        });
    }

    /// `id` and everything in it are joining the document: new to it,
    /// whenever they were made. A node made in an earlier frame and kept
    /// outside the document was never in a frame to compare with.
    pub fn arrived(&mut self, doc: &Document, id: NodeId) {
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            stack.extend(doc.children(node));
            let at = node.index() as usize;
            if self.slots.len() <= at {
                self.slots.resize_with(at + 1, || None);
            }
            if self.slots[at].is_none() {
                self.touched.push(at);
            }
            self.slots[at] = Some(Slot {
                id: node,
                created: true,
                before: State::default(),
            });
        }
    }

    /// The frame's changes, coalesced, and how many were recorded; the
    /// journal starts the next frame empty.
    pub fn take(&mut self, doc: &Document) -> (Changes, usize) {
        let mut changes = Changes {
            everything: std::mem::take(&mut self.everything),
            ..Changes::default()
        };
        for at in self.touched.drain(..) {
            let Some(slot) = self.slots[at].take() else {
                continue;
            };
            // Made this frame, or no longer in the document: nothing there
            // to compare with the last frame.
            if slot.created || !connected(doc, slot.id) {
                continue;
            }
            changes.add(slot.id, &slot.before, &State::of(doc, slot.id));
        }
        changes.sort();
        (changes, std::mem::take(&mut self.recorded))
    }
}

/// Whether `id` is alive and in the document.
pub fn connected(doc: &Document, mut id: NodeId) -> bool {
    loop {
        if id == doc.root() {
            return true;
        }
        match doc.node(id).and_then(|node| node.parent()) {
            Some(parent) => id = parent,
            None => return false,
        }
    }
}

/// The state of every node in the document, for checking the journal.
pub type Snapshot = HashMap<NodeId, State>;

pub fn snapshot(doc: &Document) -> Snapshot {
    let mut states = HashMap::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        let state = State::of(doc, id);
        stack.extend(state.children.iter().copied());
        states.insert(id, state);
    }
    states
}

/// What really changed between `before` and the document now, in the
/// nodes in both: what the journal must have found.
pub fn difference(before: &Snapshot, doc: &Document) -> Changes {
    let mut changes = Changes::default();
    for (id, after) in snapshot(doc) {
        if let Some(before) = before.get(&id) {
            changes.add(id, before, &after);
        }
    }
    changes.sort();
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> Document {
        Document::parse_html(html)
    }

    fn find(doc: &Document, id_attr: &str) -> NodeId {
        snapshot(doc)
            .into_keys()
            .find(|id| {
                doc.node(*id)
                    .and_then(|node| node.as_element())
                    .is_some_and(|element| {
                        element.attr(&erk_dom::local_name!("id")) == Some(id_attr)
                    })
            })
            .unwrap()
    }

    #[test]
    fn the_last_value_wins_and_a_return_to_the_first_is_no_change() {
        let mut doc = doc("<p id=p>bir</p>");
        let p = find(&doc, "p");
        let mut journal = Journal::default();
        for value in ["a", "b", "c"] {
            journal.touch(&doc, p);
            doc.set_attr(p, "title", value).unwrap();
        }
        let (changes, recorded) = journal.take(&doc);
        assert_eq!(recorded, 3);
        assert_eq!(changes.attrs, [(p, vec!["title".to_owned()])]);
        // Added and removed again in one frame: nothing.
        journal.touch(&doc, p);
        doc.set_attr(p, "class", "on").unwrap();
        journal.touch(&doc, p);
        doc.remove_attr(p, "class").unwrap();
        assert_eq!(journal.take(&doc).0, Changes::default());
    }

    #[test]
    fn a_node_made_and_removed_in_one_frame_leaves_no_trace() {
        let mut doc = doc("<ul id=list></ul>");
        let list = find(&doc, "list");
        let mut journal = Journal::default();
        let item = doc.create_element("li").unwrap();
        journal.created(item);
        journal.touch(&doc, item);
        doc.set_attr(item, "class", "yeni").unwrap();
        journal.touch(&doc, list);
        doc.insert(list, item, None).unwrap();
        journal.touch(&doc, list);
        doc.remove(item);
        assert_eq!(journal.take(&doc).0, Changes::default());
        // Made and kept: its parent's children changed, the node itself
        // is all new.
        let item = doc.create_element("li").unwrap();
        journal.created(item);
        journal.touch(&doc, item);
        doc.set_attr(item, "class", "yeni").unwrap();
        journal.touch(&doc, list);
        doc.insert(list, item, None).unwrap();
        let (changes, _) = journal.take(&doc);
        assert_eq!(changes.children, [list]);
        assert!(changes.attrs.is_empty());
    }

    #[test]
    fn the_journal_finds_what_the_document_really_changed() {
        let mut doc = doc("<p id=p class=a>bir</p><p id=q>iki</p>");
        let (p, q) = (find(&doc, "p"), find(&doc, "q"));
        let before = snapshot(&doc);
        let mut journal = Journal::default();
        journal.touch(&doc, p);
        doc.set_attr(p, "class", "b").unwrap();
        journal.touch(&doc, q);
        doc.set_text(q, "üç").unwrap();
        // The text node `set_text` made is new, as the page records it.
        let text = doc.children(q).next().unwrap();
        journal.created(text);
        journal.touch(&doc, text);
        doc.set_text(text, "dört").unwrap();
        let (changes, _) = journal.take(&doc);
        assert_eq!(changes, difference(&before, &doc));
        assert_eq!(changes.attrs, [(p, vec!["class".to_owned()])]);
        assert_eq!(changes.children, [q]);
    }
}
