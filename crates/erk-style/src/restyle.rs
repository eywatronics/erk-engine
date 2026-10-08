//! Styling one document frame after frame, restyling only what changed
//! (M5.3, p2-incremental §3.4).
//!
//! Erk writes no selector invalidation of its own. The element data Stylo
//! left in each slot stays there between frames, and each frame tells Stylo
//! what changed the way a browser does: an element snapshot (its attributes
//! as they were at the last frame, from the change journal, and its old
//! state) for every element whose attributes or state changed, from which
//! Stylo works out which elements, siblings and descendants the change
//! restyles. What snapshots cannot say, Erk says with restyle hints:
//!
//! - children added, removed or moved, and text changed: the selector flags
//!   matching left on the parent say whether its children's styles depend
//!   on their position (`:nth-child`, `:first-child`, sibling combinators)
//!   or the parent's on being empty;
//! - a node that moved to another parent restyles with all it holds.
//!
//! `:has()` needs nothing yet: Stylo 0.20 does not parse it in Servo mode,
//! so no rule depends on it (`has_is_not_parsed_yet` notices the day it
//! does). Its anchors will then restyle when something their selector can
//! see changes: in their subtree or their later siblings', so they are
//! found on the way up from a change, each ancestor and the siblings
//! before it.
//!
//! A `<style>` element whose text changed has its sheet replaced, and
//! Stylo's stylesheet invalidation restyles what the rules it lost and
//! gained can match. Sheets added or removed, another viewport or a new
//! document style everything from nothing. The style each element ends up with must equal
//! a full style's; the engine checks the frames against one (decision 9).

use erk_dom::{Document, LocalName, NodeId, ns};
use erk_invalidation::Invalidation;
use erk_invalidation::journal::Changes;
use selectors::matching::ElementSelectorFlags;
use style::dom::OpaqueNode;
use style::invalidation::element::restyle_hints::RestyleHint;
use style::properties::ComputedValues;
use style::selector_parser::{RestyleDamage, Snapshot, SnapshotMap};
use style::servo::attr::{AttrIdentifier, AttrValue};
use style::servo_arc::Arc;
use style::stylesheets::DocumentStyleSheet;
use style::stylist::Stylist;
use style::values::GenericAtomIdent;

use crate::node::ErkNode;
use crate::side::Slot;
use crate::{Interaction, StyleEngine, Styles, author_styles, primary, reacts, states};

/// Styles one document frame after frame, keeping what Stylo computed and
/// restyling only what the frame's changes reach.
pub struct Restyler {
    engine: StyleEngine,
    stylist: Option<Stylist>,
    /// The author sheets the stylist holds, and their text.
    sheets: Vec<DocumentStyleSheet>,
    author: Vec<String>,
    slots: Vec<Slot>,
    /// Each slot's style at the last frame.
    computed: Vec<Option<Arc<ComputedValues>>>,
}

/// The selector flags that make a parent's children depend on where they
/// are among their siblings.
const POSITIONAL: ElementSelectorFlags = ElementSelectorFlags::HAS_SLOW_SELECTOR
    .union(ElementSelectorFlags::HAS_SLOW_SELECTOR_LATER_SIBLINGS)
    .union(ElementSelectorFlags::HAS_SLOW_SELECTOR_NTH)
    .union(ElementSelectorFlags::HAS_SLOW_SELECTOR_NTH_OF)
    .union(ElementSelectorFlags::HAS_EDGE_CHILD_SELECTOR);

impl Restyler {
    /// A restyler styling with `engine`, whose viewport it keeps: a new
    /// viewport needs a new restyler.
    pub fn new(engine: StyleEngine) -> Self {
        Self {
            engine,
            stylist: None,
            sheets: Vec::new(),
            author: Vec::new(),
            slots: Vec::new(),
            computed: Vec::new(),
        }
    }

    /// Style `doc` as it is now, `changes` being what changed in it since
    /// the last call and `interaction` what the user is doing.
    pub fn restyle(
        &mut self,
        doc: &Document,
        interaction: &Interaction,
        changes: &Changes,
    ) -> Styles {
        let author = author_styles(doc);
        match &mut self.stylist {
            Some(stylist) if !changes.everything && author.len() == self.author.len() => {
                // A sheet whose text changed is replaced in the stylist,
                // and the stylist's next flush restyles the elements the
                // rules it lost and gained can match (Stylo's stylesheet
                // invalidation; a selector it cannot narrow restyles
                // everything).
                let read = self.engine.guard.read();
                for (at, css) in author.iter().enumerate() {
                    if *css == self.author[at] {
                        continue;
                    }
                    let sheet = self.engine.sheet(css);
                    match self.sheets.get(at + 1) {
                        Some(next) => {
                            stylist.insert_stylesheet_before(sheet.clone(), next.clone(), &read)
                        }
                        None => stylist.append_stylesheet(sheet.clone(), &read),
                    }
                    stylist
                        .remove_stylesheet(std::mem::replace(&mut self.sheets[at], sheet), &read);
                }
                self.author = author;
            }
            _ => {
                // The first frame, a new document, or sheets added or
                // removed: nothing kept is worth anything.
                self.sheets = author.iter().map(|css| self.engine.sheet(css)).collect();
                self.stylist = Some(self.engine.stylist(&self.sheets));
                self.author = author;
                self.slots.clear();
                self.computed.clear();
            }
        }
        let size = doc.capacity_hint();
        if self.slots.len() < size {
            self.slots.resize_with(size, Slot::default);
            self.computed.resize(size, None);
        }

        let mut frame = Frame::default();
        self.walk(doc, interaction, &mut frame);
        self.attributes(doc, changes, &mut frame);
        // A snapshot of the state alone has the attributes as they are:
        // they did not change. Stylo's stylesheet invalidation reads them
        // from every snapshot.
        for (opaque, snapshot) in frame.snapshots.iter_mut() {
            if snapshot.attrs.is_none() {
                let element = self.slots[opaque.0]
                    .owner
                    .and_then(|id| doc.node(id)?.as_element());
                snapshot.attrs = Some(
                    element
                        .map(|element| {
                            element
                                .attrs
                                .iter()
                                .map(|attr| attribute(&attr.name.local, &attr.value))
                                .collect()
                        })
                        .unwrap_or_default(),
                );
            }
        }
        let mut parents: Vec<NodeId> = changes.children.clone();
        parents.extend(
            changes
                .text
                .iter()
                .filter_map(|text| doc.node(*text)?.parent()),
        );
        self.positions(doc, &parents, &mut frame);

        for (id, hint) in frame.hints {
            let slot = &self.slots[id.index() as usize];
            if let Some(mut data) = slot.mutate_data() {
                data.hint.insert(hint);
                frame.reach.push(id);
            }
        }
        for opaque in frame.snapshots.keys() {
            self.slots[opaque.0].has_snapshot = true;
        }
        for id in frame.reach {
            mark_ancestors(doc, &self.slots, id);
        }

        let stylist = self.stylist.as_mut().expect("made above");
        self.engine
            .traverse(stylist, doc, &self.slots, &frame.snapshots);
        // Rule nodes no style uses any more wait in the rule tree until it
        // collects them; a stylist that lives for the document's whole life
        // has to, every frame, or it grows with every change.
        stylist.rule_tree().gc();

        let mut styled = 0;
        let mut damage = Vec::new();
        for (at, slot) in self.slots.iter().enumerate() {
            let new = primary(slot);
            let old = std::mem::replace(&mut self.computed[at], new.clone());
            let same = match (&old, &new) {
                (Some(old), Some(new)) => Arc::ptr_eq(old, new),
                (None, None) => true,
                _ => false,
            };
            if same {
                continue;
            }
            styled += usize::from(new.is_some());
            let bits = invalidation(old.as_deref(), new.as_deref());
            if let Some(id) = slot.owner
                && !bits.is_empty()
            {
                damage.push((id, bits));
            }
        }
        Styles {
            computed: self.computed.clone(),
            reacts: reacts(stylist),
            styled,
            damage,
        }
    }

    /// Take the document's nodes in: a slot for each new one, the old
    /// state of each element whose state changed, and the elements that
    /// moved. Slots of nodes that left the document are emptied, and so
    /// are those of elements beside the root element, which styling never
    /// reaches (a full style gives them none).
    fn walk(&mut self, doc: &Document, interaction: &Interaction, frame: &mut Frame) {
        let states = states(doc, interaction);
        let root = doc.children(doc.root()).find(|child| {
            doc.node(*child)
                .is_some_and(|node| node.as_element().is_some())
        });
        let mut present = vec![false; self.slots.len()];
        let mut stack = vec![(doc.root(), None, false)];
        while let Some((id, parent, inside)) = stack.pop() {
            stack.extend(
                doc.children(id)
                    .map(|child| (child, Some(id), inside || Some(child) == root)),
            );
            let at = id.index() as usize;
            present[at] = true;
            let slot = &mut self.slots[at];
            slot.start_frame();
            let element = doc.node(id).and_then(|node| node.as_element());
            let fresh = slot.owner != Some(id);
            if fresh || !inside || slot.outside {
                if fresh {
                    // What it holds is another node's.
                    self.computed[at] = None;
                }
                *slot = Slot::new(id);
                slot.parent = parent;
                slot.outside = !inside;
                if let Some(element) = element {
                    slot.read_attributes(element, &self.engine.url, &self.engine.guard);
                    slot.state = states[at];
                    if inside {
                        frame.reach.push(id);
                    }
                }
                continue;
            }
            if element.is_none() {
                slot.parent = parent;
                continue;
            }
            if slot.parent != parent {
                // Its new ancestors and siblings may style it otherwise.
                slot.parent = parent;
                frame.hints.push((id, RestyleHint::restyle_subtree()));
            }
            if slot.state != states[at] {
                frame.snapshot(id).state = Some(slot.state);
                slot.state = states[at];
            }
        }
        for (at, slot) in self.slots.iter_mut().enumerate() {
            // Its style goes from `computed` when the frame collects them.
            if !present[at] && slot.owner.is_some() {
                *slot = Slot::default();
            }
        }
    }

    /// Read the attributes that changed again, and give each styled
    /// element whose attributes changed a snapshot of the old ones.
    fn attributes(&mut self, doc: &Document, changes: &Changes, frame: &mut Frame) {
        for change in &changes.attrs {
            let id = change.node;
            let Some(element) = doc.node(id).and_then(|node| node.as_element()) else {
                continue;
            };
            let slot = &mut self.slots[id.index() as usize];
            if slot.owner != Some(id) {
                continue;
            }
            // Read even without a style (inside `display: none`): the
            // element is styled with them once it shows.
            slot.read_attributes(element, &self.engine.url, &self.engine.guard);
            if !slot.has_data() {
                continue;
            }
            let snapshot = frame.snapshot(id);
            snapshot.attrs = Some(
                change
                    .before
                    .iter()
                    .map(|(name, value)| attribute(name, value))
                    .collect(),
            );
            for name in &change.names {
                match name.as_str() {
                    "class" => snapshot.class_changed = true,
                    "id" => snapshot.id_changed = true,
                    _ => snapshot.other_attributes_changed = true,
                }
                snapshot
                    .changed_attrs
                    .push(GenericAtomIdent(LocalName::from(name.as_str())));
            }
            if change.names.iter().any(|name| name == "style") {
                frame.hints.push((id, RestyleHint::RESTYLE_STYLE_ATTRIBUTE));
            }
        }
    }

    /// Restyle what depends on where `parents`' children are: all their
    /// children when a positional selector looked at them, and when one
    /// asked whether a parent is empty, the parent and its siblings.
    fn positions(&self, doc: &Document, parents: &[NodeId], frame: &mut Frame) {
        for &parent in parents {
            let flags = self.slots[parent.index() as usize].selector_flags();
            let element = self.element_or_root(doc, parent);
            if flags.intersects(POSITIONAL) {
                frame.hints.push((element, RestyleHint::restyle_subtree()));
            }
            if flags.contains(ElementSelectorFlags::HAS_EMPTY_SELECTOR) {
                let around = doc
                    .node(parent)
                    .and_then(|node| node.parent())
                    .map_or(element, |grand| self.element_or_root(doc, grand));
                frame.hints.push((around, RestyleHint::restyle_subtree()));
            }
        }
    }

    /// `id` if it is an element, otherwise the root element: the document
    /// node takes no hints, and restyling from the root covers it.
    fn element_or_root(&self, doc: &Document, id: NodeId) -> NodeId {
        let is_element = |id: NodeId| doc.node(id).is_some_and(|node| node.as_element().is_some());
        if is_element(id) {
            return id;
        }
        doc.children(doc.root())
            .find(|child| is_element(*child))
            .unwrap_or(id)
    }
}

/// What one frame tells Stylo.
struct Frame {
    snapshots: SnapshotMap,
    hints: Vec<(NodeId, RestyleHint)>,
    /// The elements styling must reach: new ones, and those with a
    /// snapshot or a hint.
    reach: Vec<NodeId>,
}

impl Default for Frame {
    // `SnapshotMap` has `new` but no `Default`.
    fn default() -> Self {
        Self {
            snapshots: SnapshotMap::new(),
            hints: Vec::new(),
            reach: Vec::new(),
        }
    }
}

impl Frame {
    /// `id`'s snapshot, made empty the first time.
    fn snapshot(&mut self, id: NodeId) -> &mut Snapshot {
        self.reach.push(id);
        self.snapshots
            .entry(OpaqueNode(id.index() as usize))
            .or_default()
    }
}

/// An attribute as a snapshot holds it.
fn attribute(name: &str, value: &str) -> (AttrIdentifier, AttrValue) {
    let local = GenericAtomIdent(LocalName::from(name));
    let value = match name {
        "class" => AttrValue::from_serialized_tokenlist(value.to_owned()),
        "id" => AttrValue::from_atomic(value.to_owned()),
        _ => AttrValue::String(value.to_owned()),
    };
    (
        AttrIdentifier {
            local_name: local.clone(),
            name: local,
            namespace: GenericAtomIdent(ns!()),
            prefix: None,
        },
        value,
    )
}

/// Set `id`'s ancestors' dirty-descendants bits, so that the traversal,
/// which goes down only where they are set, reaches it.
fn mark_ancestors(doc: &Document, slots: &[Slot], id: NodeId) {
    let mut current = doc.node(id).and_then(|node| node.parent());
    while let Some(parent) = current {
        let slot = &slots[parent.index() as usize];
        if slot.dirty_descendants() {
            break;
        }
        slot.set_dirty_descendants(true);
        current = doc.node(parent).and_then(|node| node.parent());
    }
}

/// What an element's style changing from `old` to `new` makes dirty (M5
/// plan, decision 10): a style that comes or goes rebuilds everything; a
/// change Stylo says only repaints is painting and hit testing; anything
/// else is layout, and shaping too when a font or text property changed.
fn invalidation(old: Option<&ComputedValues>, new: Option<&ComputedValues>) -> Invalidation {
    let everything = Invalidation::ALL - Invalidation::STYLE_SELF - Invalidation::STYLE_SUBTREE;
    let (Some(old), Some(new)) = (old, new) else {
        return everything;
    };
    let damage = RestyleDamage::compute_style_difference::<ErkNode<'_>>(old, new).damage;
    if damage.is_empty() {
        return Invalidation::NONE;
    }
    let paint = Invalidation::PAINT_SELF | Invalidation::HIT_TEST | Invalidation::A11Y_SELF;
    if damage == RestyleDamage::REPAINT {
        return paint;
    }
    if !damage.contains(RestyleDamage::RECALCULATE_OVERFLOW) {
        // Stacking contexts: this element and what it paints over.
        return paint | Invalidation::PAINT_SUBTREE;
    }
    let mut bits = paint | Invalidation::PAINT_SUBTREE | Invalidation::LAYOUT_SELF;
    if old.get_font() != new.get_font() || old.get_inherited_text() != new.get_inherited_text() {
        bits |= Invalidation::TEXT_SHAPE;
    }
    bits
}
