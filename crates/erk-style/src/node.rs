//! Stylo's DOM traits, implemented for Erk's arena DOM.
//!
//! Adapted from blitz-dom 0.3.0-beta.2, src/stylo.rs (MIT OR Apache-2.0,
//! https://github.com/DioxusLabs/blitz). The structure follows Blitz closely
//! on purpose; the differences are the handle type (an id plus a reference to
//! the tree, since Erk nodes do not know their document) and the style data,
//! which lives in the side table rather than on the node.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;

use erk_dom::{LocalName, Namespace, Node, NodeData, NodeId, local_name, ns};
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::{BLOOM_HASH_MASK, BloomFilter};
use selectors::matching::{ElementSelectorFlags, MatchingContext, VisitedHandlingMode};
use selectors::sink::Push;
use selectors::{Element, OpaqueElement};
use style::Atom;
use style::CaseSensitivityExt;
use style::animation::AnimationSetKey;
use style::applicable_declarations::ApplicableDeclarationBlock;
use style::bloom::each_relevant_element_hash;
use style::color::AbsoluteColor;
use style::context::{
    QuirksMode, RegisteredSpeculativePainter, RegisteredSpeculativePainters, SharedStyleContext,
    StyleContext,
};
use style::data::{ElementDataMut, ElementDataRef};
use style::dom::{LayoutIterator, NodeInfo, OpaqueNode, TDocument, TElement, TNode, TShadowRoot};
use style::properties::{Importance, PropertyDeclaration, PropertyDeclarationBlock};
use style::rule_tree::{CascadeLevel, CascadeOrigin};
use style::selector_parser::{NonTSPseudoClass, PseudoElement, SelectorImpl};
use style::servo_arc::{Arc, ArcBorrow};
use style::shared_lock::{Locked, SharedRwLock};
use style::stylesheets::layer_rule::LayerOrder;
use style::stylesheets::scope_rule::ImplicitScopeRoot;
use style::traversal::{DomTraversal, PerLevelTraversalData, recalc_style_at};
use style::values::{AtomIdent, AtomString, GenericAtomIdent};
use style_dom::ElementState;

use crate::side::{StyledNode, StyledTree};

/// A node as Stylo sees it. Stylo requires this to be exactly one pointer
/// wide, so it is a reference to the node's style state, which knows its id
/// and its tree.
#[derive(Clone, Copy)]
pub(crate) struct ErkNode<'a>(&'a StyledNode<'a>);

// Stylo's style sharing cache transmutes between a typed and an untyped cache
// and only checks the sizes at runtime. Catch a wider handle at compile time.
const _: () = assert!(size_of::<ErkNode<'static>>() == size_of::<usize>());

impl<'a> ErkNode<'a> {
    pub(crate) fn new(tree: &'a StyledTree<'a>, id: NodeId) -> Self {
        Self(tree.node(id))
    }

    fn node_id(&self) -> NodeId {
        self.0
            .id
            .expect("style traversal reached a slot outside the document")
    }

    fn with(&self, id: NodeId) -> Self {
        Self(self.0.tree.node(id))
    }

    fn node(&self) -> &'a Node {
        self.0
            .tree
            .doc
            .node(self.node_id())
            .expect("style traversal saw a stale NodeId")
    }

    fn slot(&self) -> &'a StyledNode<'a> {
        self.0
    }

    fn guard(&self) -> &'a SharedRwLock {
        self.0.tree.guard
    }

    fn element(&self) -> Option<&'a erk_dom::ElementData> {
        self.node().as_element()
    }

    fn attr(&self, name: &LocalName) -> Option<&'a str> {
        self.element()?.attr(name)
    }

    fn is_element_named(&self, name: &LocalName) -> bool {
        self.element()
            .is_some_and(|element| element.name.local == *name)
    }

    fn element_children(&self) -> impl Iterator<Item = ErkNode<'a>> + use<'a> {
        Children {
            tree: self.0.tree,
            next: self.node().first_child(),
        }
        .filter(|node| node.is_element())
    }

    fn sibling_element(&self, step: fn(&Node) -> Option<NodeId>) -> Option<Self> {
        let mut next = step(self.node());
        while let Some(id) = next {
            let node = self.with(id);
            if node.is_element() {
                return Some(node);
            }
            next = step(node.node());
        }
        None
    }

    fn mark_ancestors_dirty(&self) {
        let mut parent = self.node().parent();
        while let Some(id) = parent {
            let node = self.with(id);
            if node.slot().dirty_descendants() {
                break;
            }
            node.slot().set_dirty_descendants(true);
            parent = node.node().parent();
        }
    }
}

impl PartialEq for ErkNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for ErkNode<'_> {}

impl Hash for ErkNode<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node_id().hash(state);
    }
}

impl fmt::Debug for ErkNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.element() {
            Some(element) => write!(f, "<{}> {:?}", element.name.local, self.node_id()),
            None => write!(f, "{:?}", self.node_id()),
        }
    }
}

impl<'a> TDocument for ErkNode<'a> {
    type ConcreteNode = ErkNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn is_html_document(&self) -> bool {
        true
    }

    fn quirks_mode(&self) -> QuirksMode {
        QuirksMode::NoQuirks
    }

    fn shared_lock(&self) -> &SharedRwLock {
        self.guard()
    }
}

impl NodeInfo for ErkNode<'_> {
    fn is_element(&self) -> bool {
        matches!(self.node().data, NodeData::Element(_))
    }

    fn is_text_node(&self) -> bool {
        matches!(self.node().data, NodeData::Text(_))
    }
}

impl<'a> TShadowRoot for ErkNode<'a> {
    type ConcreteNode = ErkNode<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn host(&self) -> <Self::ConcreteNode as TNode>::ConcreteElement {
        unimplemented!("shadow DOM is not supported yet")
    }

    fn style_data<'b>(&self) -> Option<&'b style::stylist::CascadeData>
    where
        Self: 'b,
    {
        unimplemented!("shadow DOM is not supported yet")
    }
}

impl<'a> TNode for ErkNode<'a> {
    type ConcreteElement = ErkNode<'a>;
    type ConcreteDocument = ErkNode<'a>;
    type ConcreteShadowRoot = ErkNode<'a>;

    fn parent_node(&self) -> Option<Self> {
        self.node().parent().map(|id| self.with(id))
    }

    fn first_child(&self) -> Option<Self> {
        self.node().first_child().map(|id| self.with(id))
    }

    fn last_child(&self) -> Option<Self> {
        self.node().last_child().map(|id| self.with(id))
    }

    fn prev_sibling(&self) -> Option<Self> {
        self.node().prev_sibling().map(|id| self.with(id))
    }

    fn next_sibling(&self) -> Option<Self> {
        self.node().next_sibling().map(|id| self.with(id))
    }

    fn owner_doc(&self) -> Self::ConcreteDocument {
        self.with(self.0.tree.doc.root())
    }

    fn is_in_document(&self) -> bool {
        true
    }

    fn traversal_parent(&self) -> Option<Self::ConcreteElement> {
        self.parent_node().and_then(|node| node.as_element())
    }

    fn opaque(&self) -> OpaqueNode {
        OpaqueNode(self.node_id().index() as usize)
    }

    fn debug_id(self) -> usize {
        self.node_id().index() as usize
    }

    fn as_element(&self) -> Option<Self::ConcreteElement> {
        self.is_element().then_some(*self)
    }

    fn as_document(&self) -> Option<Self::ConcreteDocument> {
        matches!(self.node().data, NodeData::Document).then_some(*self)
    }

    fn as_shadow_root(&self) -> Option<Self::ConcreteShadowRoot> {
        None
    }
}

impl Element for ErkNode<'_> {
    type Impl = SelectorImpl;

    fn opaque(&self) -> OpaqueElement {
        // The slot index plus one: unique among live nodes and never zero.
        let non_null = NonNull::new((self.node_id().index() as usize + 1) as *mut ()).unwrap();
        OpaqueElement::from_non_null_ptr(non_null)
    }

    fn parent_element(&self) -> Option<Self> {
        TElement::traversal_parent(self)
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        self.sibling_element(Node::prev_sibling)
    }

    fn next_sibling_element(&self) -> Option<Self> {
        self.sibling_element(Node::next_sibling)
    }

    fn first_element_child(&self) -> Option<Self> {
        self.element_children().next()
    }

    fn is_html_element_in_html_document(&self) -> bool {
        self.element()
            .is_some_and(|element| element.name.ns == ns!(html))
    }

    fn has_local_name(&self, local_name: &LocalName) -> bool {
        self.is_element_named(local_name)
    }

    fn has_namespace(&self, ns: &Namespace) -> bool {
        self.element().is_some_and(|element| element.name.ns == *ns)
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.local_name() == other.local_name() && self.namespace() == other.namespace()
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&style::Namespace>,
        local_name: &style::LocalName,
        operation: &AttrSelectorOperation<&AtomString>,
    ) -> bool {
        self.attr(&local_name.0)
            .is_some_and(|value| operation.eval_str(value))
    }

    fn match_non_ts_pseudo_class(
        &self,
        pseudo_class: &NonTSPseudoClass,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        let state = self.slot().state;
        match *pseudo_class {
            NonTSPseudoClass::Active => state.contains(ElementState::ACTIVE),
            NonTSPseudoClass::AnyLink => state.intersects(ElementState::VISITED_OR_UNVISITED),
            NonTSPseudoClass::Checked => state.contains(ElementState::CHECKED),
            NonTSPseudoClass::Disabled => state.contains(ElementState::DISABLED),
            NonTSPseudoClass::Enabled => state.contains(ElementState::ENABLED),
            NonTSPseudoClass::Focus => state.contains(ElementState::FOCUS),
            NonTSPseudoClass::Hover => state.contains(ElementState::HOVER),
            NonTSPseudoClass::Link => state.contains(ElementState::UNVISITED),
            NonTSPseudoClass::Valid
            | NonTSPseudoClass::Invalid
            | NonTSPseudoClass::Defined
            | NonTSPseudoClass::FocusWithin
            | NonTSPseudoClass::FocusVisible
            | NonTSPseudoClass::Fullscreen
            | NonTSPseudoClass::Indeterminate
            | NonTSPseudoClass::Lang(_)
            | NonTSPseudoClass::CustomState(_)
            | NonTSPseudoClass::PlaceholderShown
            | NonTSPseudoClass::ReadWrite
            | NonTSPseudoClass::ReadOnly
            | NonTSPseudoClass::ServoNonZeroBorder
            | NonTSPseudoClass::Target
            | NonTSPseudoClass::Visited
            | NonTSPseudoClass::Autofill
            | NonTSPseudoClass::Default
            | NonTSPseudoClass::InRange
            | NonTSPseudoClass::Modal
            | NonTSPseudoClass::Open
            | NonTSPseudoClass::Optional
            | NonTSPseudoClass::OutOfRange
            | NonTSPseudoClass::PopoverOpen
            | NonTSPseudoClass::Required
            | NonTSPseudoClass::UserInvalid
            | NonTSPseudoClass::UserValid
            | NonTSPseudoClass::MozMeterOptimum
            | NonTSPseudoClass::MozMeterSubOptimum
            | NonTSPseudoClass::MozMeterSubSubOptimum => false,
        }
    }

    fn match_pseudo_element(
        &self,
        pseudo: &PseudoElement,
        _context: &mut MatchingContext<Self::Impl>,
    ) -> bool {
        self.slot()
            .borrow_data()
            .and_then(|data| data.styles.get_primary().and_then(|s| s.pseudo()))
            .is_some_and(|own| own == *pseudo)
    }

    fn apply_selector_flags(&self, flags: ElementSelectorFlags) {
        let self_flags = flags.for_self();
        if !self_flags.is_empty() {
            self.slot().insert_selector_flags(self_flags);
        }
        let parent_flags = flags.for_parent();
        if !parent_flags.is_empty()
            && let Some(parent) = self.parent_node()
        {
            parent.slot().insert_selector_flags(parent_flags);
        }
    }

    fn is_link(&self) -> bool {
        self.is_element_named(&local_name!("a"))
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.slot()
            .id_attr
            .as_ref()
            .is_some_and(|own| case_sensitivity.eq_atom(own, &id.0))
    }

    fn has_class(&self, name: &AtomIdent, case_sensitivity: CaseSensitivity) -> bool {
        self.attr(&local_name!("class")).is_some_and(|classes| {
            classes
                .split_ascii_whitespace()
                .any(|class| case_sensitivity.eq_atom(&Atom::from(class), &name.0))
        })
    }

    fn imported_part(&self, _name: &AtomIdent) -> Option<AtomIdent> {
        None
    }

    fn is_part(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.node().first_child().is_none()
    }

    fn is_root(&self) -> bool {
        self.parent_node()
            .is_some_and(|parent| parent.as_document().is_some())
    }

    fn has_custom_state(&self, _name: &AtomIdent) -> bool {
        false
    }

    fn add_element_unique_hashes(&self, filter: &mut BloomFilter) -> bool {
        each_relevant_element_hash(*self, |hash| filter.insert_hash(hash & BLOOM_HASH_MASK));
        true
    }
}

impl<'a> TElement for ErkNode<'a> {
    type ConcreteNode = ErkNode<'a>;
    type TraversalChildrenIterator = Children<'a>;

    fn as_node(&self) -> Self::ConcreteNode {
        *self
    }

    fn implicit_scope_for_sheet_in_shadow_root(
        _opaque_host: OpaqueElement,
        _sheet_index: usize,
    ) -> Option<ImplicitScopeRoot> {
        unimplemented!("shadow DOM is not supported yet")
    }

    fn traversal_children(&self) -> LayoutIterator<Children<'a>> {
        LayoutIterator(Children {
            tree: self.0.tree,
            next: self.node().first_child(),
        })
    }

    fn is_html_element(&self) -> bool {
        self.has_namespace(&ns!(html))
    }

    fn is_mathml_element(&self) -> bool {
        self.has_namespace(&ns!(mathml))
    }

    fn is_svg_element(&self) -> bool {
        self.has_namespace(&ns!(svg))
    }

    fn style_attribute(&self) -> Option<ArcBorrow<'_, Locked<PropertyDeclarationBlock>>> {
        self.slot()
            .style_attribute
            .as_ref()
            .map(|block| block.borrow_arc())
    }

    fn state(&self) -> ElementState {
        self.slot().state
    }

    fn has_part_attr(&self) -> bool {
        false
    }

    fn exports_any_part(&self) -> bool {
        false
    }

    fn id(&self) -> Option<&Atom> {
        self.slot().id_attr.as_ref()
    }

    fn each_class<F>(&self, mut callback: F)
    where
        F: FnMut(&AtomIdent),
    {
        if let Some(classes) = self.attr(&local_name!("class")) {
            for class in classes.split_ascii_whitespace() {
                callback(AtomIdent::cast(&Atom::from(class)));
            }
        }
    }

    fn each_attr_name<F>(&self, mut callback: F)
    where
        F: FnMut(&style::LocalName),
    {
        if let Some(element) = self.element() {
            for attr in &element.attrs {
                callback(&GenericAtomIdent(attr.name.local.clone()));
            }
        }
    }

    fn has_dirty_descendants(&self) -> bool {
        self.slot().dirty_descendants()
    }

    fn has_snapshot(&self) -> bool {
        false
    }

    fn handled_snapshot(&self) -> bool {
        self.slot().handled_snapshot()
    }

    // The five `unsafe fn`s below are unsafe only because Stylo's trait says
    // so; their bodies are safe calls into the side table. They are the whole
    // of this crate's unsafe surface, and CI checks that the count stays five.

    #[allow(unsafe_code)] // signature required by TElement
    unsafe fn set_handled_snapshot(&self) {
        self.slot().set_handled_snapshot();
    }

    #[allow(unsafe_code)] // signature required by TElement
    unsafe fn set_dirty_descendants(&self) {
        self.slot().set_dirty_descendants(true);
        self.mark_ancestors_dirty();
    }

    #[allow(unsafe_code)] // signature required by TElement
    unsafe fn unset_dirty_descendants(&self) {
        self.slot().set_dirty_descendants(false);
    }

    fn store_children_to_process(&self, _n: isize) {
        unimplemented!("only used by postorder traversals, which Erk does not run")
    }

    fn did_process_child(&self) -> isize {
        unimplemented!("only used by postorder traversals, which Erk does not run")
    }

    #[allow(unsafe_code)] // signature required by TElement
    unsafe fn ensure_data(&self) -> ElementDataMut<'_> {
        self.slot().ensure_data()
    }

    #[allow(unsafe_code)] // signature required by TElement
    unsafe fn clear_data(&self) {
        self.slot().clear_data();
    }

    fn has_data(&self) -> bool {
        self.slot().has_data()
    }

    fn borrow_data(&self) -> Option<ElementDataRef<'_>> {
        self.slot().borrow_data()
    }

    fn mutate_data(&self) -> Option<ElementDataMut<'_>> {
        self.slot().mutate_data()
    }

    fn skip_item_display_fixup(&self) -> bool {
        false
    }

    fn may_have_animations(&self) -> bool {
        false
    }

    fn has_animations(&self, context: &SharedStyleContext) -> bool {
        self.has_css_animations(context, None) || self.has_css_transitions(context, None)
    }

    fn has_css_animations(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(self), pseudo_element);
        context.animations.has_active_animations(&key)
    }

    fn has_css_transitions(
        &self,
        context: &SharedStyleContext,
        pseudo_element: Option<PseudoElement>,
    ) -> bool {
        let key = AnimationSetKey::new(TNode::opaque(self), pseudo_element);
        context.animations.has_active_transitions(&key)
    }

    fn animation_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        context.animations.get_animation_declarations(
            &AnimationSetKey::new_for_non_pseudo(TNode::opaque(self)),
            context.current_time_for_animations,
            self.guard(),
        )
    }

    fn transition_rule(
        &self,
        context: &SharedStyleContext,
    ) -> Option<Arc<Locked<PropertyDeclarationBlock>>> {
        context.animations.get_transition_declarations(
            &AnimationSetKey::new_for_non_pseudo(TNode::opaque(self)),
            context.current_time_for_animations,
            self.guard(),
        )
    }

    fn shadow_root(&self) -> Option<Self> {
        None
    }

    fn containing_shadow(&self) -> Option<Self> {
        None
    }

    fn get_attr(&self, attr: &style::LocalName, _ns: &style::Namespace) -> Option<String> {
        self.attr(&attr.0).map(str::to_owned)
    }

    fn lang_attr(&self) -> Option<style::selector_parser::AttrValue> {
        None
    }

    fn match_element_lang(
        &self,
        _override_lang: Option<Option<style::selector_parser::AttrValue>>,
        _value: &style::selector_parser::Lang,
    ) -> bool {
        false
    }

    fn is_html_document_body_element(&self) -> bool {
        self.is_element_named(&local_name!("body"))
            && self.parent_node().is_some_and(|parent| parent.is_root())
    }

    fn synthesize_presentational_hints_for_legacy_attributes<V>(
        &self,
        _visited_handling: VisitedHandlingMode,
        hints: &mut V,
    ) where
        V: Push<ApplicableDeclarationBlock>,
    {
        let Some(element) = self.element() else {
            return;
        };
        let mut push = |declaration: PropertyDeclaration| {
            hints.push(ApplicableDeclarationBlock::from_declarations(
                Arc::new(self.guard().wrap(PropertyDeclarationBlock::with_one(
                    declaration,
                    Importance::Normal,
                ))),
                CascadeLevel::new(CascadeOrigin::PresHints),
                LayerOrder::root(),
            ));
        };
        presentational_hints(element, &mut push);
    }

    fn local_name(&self) -> &LocalName {
        &self
            .element()
            .expect("local_name on a non-element")
            .name
            .local
    }

    fn namespace(&self) -> &Namespace {
        &self.element().expect("namespace on a non-element").name.ns
    }

    fn query_container_size(
        &self,
        _display: &style::values::specified::Display,
    ) -> euclid::default::Size2D<Option<app_units::Au>> {
        // Container queries are not implemented; an unknown size disables
        // them without panicking.
        Default::default()
    }

    fn each_custom_state<F>(&self, _callback: F)
    where
        F: FnMut(&AtomIdent),
    {
    }

    fn has_selector_flags(&self, flags: ElementSelectorFlags) -> bool {
        self.slot().selector_flags().contains(flags)
    }

    fn relative_selector_search_direction(&self) -> ElementSelectorFlags {
        let flags = self.slot().selector_flags();
        [
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR_SIBLING,
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_ANCESTOR,
            ElementSelectorFlags::RELATIVE_SELECTOR_SEARCH_DIRECTION_SIBLING,
        ]
        .into_iter()
        .find(|direction| flags.contains(*direction))
        .unwrap_or_else(ElementSelectorFlags::empty)
    }
}

/// The legacy presentational attributes M0 needs. Blitz maps many more
/// (dimensions, spacing, borders); they come with the elements that use them.
fn presentational_hints(
    element: &erk_dom::ElementData,
    push: &mut impl FnMut(PropertyDeclaration),
) {
    for attr in &element.attrs {
        let value = attr.value.as_str();
        if attr.name.local == local_name!("bgcolor")
            && let Some((r, g, b)) = parse_hex_color(value)
        {
            use style::values::specified::Color;
            push(PropertyDeclaration::BackgroundColor(
                Color::from_absolute_color(AbsoluteColor::srgb_legacy(r, g, b, 1.0)),
            ));
        }
        if attr.name.local == local_name!("align") {
            use style::values::computed::text::TextAlign as Keyword;
            use style::values::specified::TextAlign;
            let keyword = match value {
                "left" => Some(Keyword::MozLeft),
                "right" => Some(Keyword::MozRight),
                "center" => Some(Keyword::MozCenter),
                _ => None,
            };
            if let Some(keyword) = keyword {
                push(PropertyDeclaration::TextAlign(TextAlign::Keyword(keyword)));
            }
        }
    }
}

fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    let channel = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    match hex.len() {
        3 => Some((
            channel(0..1)? * 17,
            channel(1..2)? * 17,
            channel(2..3)? * 17,
        )),
        6 => Some((channel(0..2)?, channel(2..4)?, channel(4..6)?)),
        _ => None,
    }
}

/// Iterator over a node's children as style handles.
pub(crate) struct Children<'a> {
    tree: &'a StyledTree<'a>,
    next: Option<NodeId>,
}

impl<'a> Iterator for Children<'a> {
    type Item = ErkNode<'a>;

    fn next(&mut self) -> Option<ErkNode<'a>> {
        let id = self.next?;
        let node = ErkNode::new(self.tree, id);
        self.next = node.node().next_sibling();
        Some(node)
    }
}

/// No CSS Painting API worklets.
pub(crate) struct NoPainters;

impl RegisteredSpeculativePainters for NoPainters {
    fn get(&self, _name: &Atom) -> Option<&dyn RegisteredSpeculativePainter> {
        None
    }
}

/// The style traversal: restyle each element on the way down.
pub(crate) struct RecalcStyle<'a> {
    context: SharedStyleContext<'a>,
}

impl<'a> RecalcStyle<'a> {
    pub(crate) fn new(context: SharedStyleContext<'a>) -> Self {
        Self { context }
    }
}

impl<'dom> DomTraversal<ErkNode<'dom>> for RecalcStyle<'_> {
    fn process_preorder<F: FnMut(ErkNode<'dom>)>(
        &self,
        traversal_data: &PerLevelTraversalData,
        context: &mut StyleContext<ErkNode<'dom>>,
        node: ErkNode<'dom>,
        note_child: F,
    ) {
        if let Some(element) = node.as_element() {
            // Straight to the side table instead of TElement::ensure_data, so
            // the traversal needs no unsafe block of its own.
            let mut data = element.slot().ensure_data();
            recalc_style_at(
                self,
                traversal_data,
                context,
                element,
                &mut data,
                note_child,
            );
            element.slot().set_dirty_descendants(false);
        }
    }

    fn needs_postorder_traversal() -> bool {
        false
    }

    fn process_postorder(&self, _context: &mut StyleContext<ErkNode<'dom>>, _node: ErkNode<'dom>) {
        unreachable!("needs_postorder_traversal is false")
    }

    fn shared_context(&self) -> &SharedStyleContext<'_> {
        &self.context
    }
}
