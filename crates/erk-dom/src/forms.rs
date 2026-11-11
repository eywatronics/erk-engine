//! The state form controls keep apart from their attributes, as HTML's
//! "dirty value" and "dirty checkedness" (HTML §4.10): what the user or
//! the host set, which an attribute no longer changes.
//!
//! The state is a side table of the document, so an element that is not
//! a control, or a control nobody set, costs nothing.

use html5ever::{LocalName, local_name, ns};

use crate::{Document, MutationError, NodeData, NodeId, StaleNode};

/// What a control holds apart from its attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    /// An `input`'s or a `textarea`'s value, set: the `value` attribute
    /// or the text no longer is the value.
    Value(String),
    /// A checkbox's or radio button's checkedness, set: the `checked`
    /// attribute no longer is.
    Checked(bool),
    /// An `option`'s selectedness, set: the `selected` attribute no
    /// longer is.
    Selected(bool),
}

/// What an input method is composing in a text field (M5.8): `text`, shown
/// in the field at byte `at` of its value but not yet part of it, with the
/// caret `caret` bytes into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Composition {
    pub at: usize,
    pub text: String,
    pub caret: usize,
}

/// What kind of control an element is, as far as its value goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// An `input` whose value is text.
    Text,
    /// An `input` whose value is text without surrounding whitespace
    /// (`url`, `email`).
    Trimmed,
    /// A checkbox or radio button: its value is its `value` attribute.
    Check,
    TextArea,
    Select,
    Option,
}

impl Document {
    /// Element `id`'s value, as DOM's `value` reads it: what was set, or
    /// else what the markup says. `None` for an element that has no
    /// value (not an `input`, `textarea`, `select` or `option`).
    pub fn value(&self, id: NodeId) -> Result<Option<String>, StaleNode> {
        let Some(kind) = self.kind(id)? else {
            return Ok(None);
        };
        let attr = |name: LocalName| self.attribute(id, &name);
        Ok(Some(match kind {
            Kind::Text | Kind::Trimmed => match self.controls.get(&id) {
                Some(Control::Value(value)) => value.clone(),
                _ => sanitize(kind, &attr(local_name!("value")).unwrap_or_default()),
            },
            Kind::Check => attr(local_name!("value")).unwrap_or_else(|| "on".to_owned()),
            Kind::TextArea => match self.controls.get(&id) {
                Some(Control::Value(value)) => value.clone(),
                _ => normalize_newlines(&self.child_text(id)),
            },
            Kind::Select => self
                .selected_options(id)
                .first()
                .map(|option| self.option_value(*option))
                .unwrap_or_default(),
            Kind::Option => self.option_value(id),
        }))
    }

    /// Set element `id`'s value, as DOM's `value` does: an `input`'s or a
    /// `textarea`'s value becomes `value` (an `input` drops line breaks),
    /// a `select` selects its first option with that value and no other,
    /// a checkbox's, a radio button's or an `option`'s `value` attribute
    /// is set. `Err` for an element that has no value.
    pub fn set_value(&mut self, id: NodeId, value: &str) -> Result<(), MutationError> {
        let kind = self
            .kind(id)
            .map_err(|_| MutationError::Stale)?
            .ok_or(MutationError::NotAControl)?;
        match kind {
            Kind::Text | Kind::Trimmed => {
                self.controls
                    .insert(id, Control::Value(sanitize(kind, value)));
            }
            Kind::TextArea => {
                self.controls
                    .insert(id, Control::Value(normalize_newlines(value)));
            }
            Kind::Check | Kind::Option => self.set_attr(id, "value", value)?,
            Kind::Select => {
                let options = self.options(id);
                let first = options
                    .iter()
                    .copied()
                    .find(|option| self.option_value(*option) == value);
                for option in options {
                    self.controls
                        .insert(option, Control::Selected(Some(option) == first));
                }
            }
        }
        Ok(())
    }

    /// Set an `input`'s or a `textarea`'s value to what the user typed:
    /// as it is, but for an `input`'s line breaks (HTML sanitizes a value
    /// the user is still typing no further: an `email` keeps the space
    /// typed last). `Err` for any other element.
    pub fn set_typed_value(&mut self, id: NodeId, value: &str) -> Result<(), MutationError> {
        let kind = self
            .kind(id)
            .map_err(|_| MutationError::Stale)?
            .ok_or(MutationError::NotAControl)?;
        let value = match kind {
            Kind::Text | Kind::Trimmed => sanitize(Kind::Text, value),
            Kind::TextArea => normalize_newlines(value),
            Kind::Check | Kind::Select | Kind::Option => return Err(MutationError::NotAControl),
        };
        self.controls.insert(id, Control::Value(value));
        Ok(())
    }

    /// Whether element `id` is checked, as DOM's `checked` reads it, or
    /// for an `option` whether it is selected (DOM's `selected`): what was
    /// set, or else what the markup says. `None` for an element that is
    /// not a checkbox, a radio button or an option.
    pub fn checked(&self, id: NodeId) -> Result<Option<bool>, StaleNode> {
        Ok(match self.kind(id)? {
            Some(Kind::Check) => Some(self.is_checked(id)),
            Some(Kind::Option) => Some(match self.select_of(id) {
                Some(select) => self.selected_options(select).contains(&id),
                None => self.is_marked(id),
            }),
            _ => None,
        })
    }

    /// Check or uncheck element `id`, a checkbox or radio button, or
    /// select or unselect it, an option. A radio button checked unchecks
    /// the others of its group: those with the same `name` in the same
    /// form, or outside any form, of the same tree (HTML §4.10.5.1.18). An
    /// option selected in a single-choice list unselects the others; one
    /// unselected leaves the list's first option that is not disabled
    /// selected, if none is. `Err` for any other element.
    pub fn set_checked(&mut self, id: NodeId, checked: bool) -> Result<(), MutationError> {
        match self.kind(id).map_err(|_| MutationError::Stale)? {
            Some(Kind::Check) => {
                if checked {
                    for other in self.radio_group(id) {
                        self.controls.insert(other, Control::Checked(false));
                    }
                }
                self.controls.insert(id, Control::Checked(checked));
            }
            Some(Kind::Option) => {
                let Some(select) = self.select_of(id) else {
                    self.controls.insert(id, Control::Selected(checked));
                    return Ok(());
                };
                let single = self.attribute(select, &local_name!("multiple")).is_none();
                let before = self.selected_options(select);
                let options = self.options(select);
                for option in &options {
                    let selected = if *option == id {
                        checked
                    } else {
                        before.contains(option) && !(single && checked)
                    };
                    self.controls.insert(*option, Control::Selected(selected));
                }
                if single && !checked && self.selected_options(select).is_empty() {
                    let first = options
                        .into_iter()
                        .find(|option| self.attribute(*option, &local_name!("disabled")).is_none());
                    if let Some(first) = first {
                        self.controls.insert(first, Control::Selected(true));
                    }
                }
            }
            _ => return Err(MutationError::NotAControl),
        }
        Ok(())
    }

    /// The value set on `id`, an `input` or a `textarea`, if one was: what
    /// replaced the markup's default. A change journal compares it; the
    /// value itself, which follows a `textarea`'s text until set, is
    /// [`Document::value`].
    pub fn value_set(&self, id: NodeId) -> Option<&str> {
        match self.controls.get(&id) {
            Some(Control::Value(value)) => Some(value),
            _ => None,
        }
    }

    /// Show `composition` in text field `id` (an `input` or a `textarea`),
    /// or with `None` take its composition away. One field composes at a
    /// time: a composition shown in one leaves the field that had one.
    /// `Err` for any other element.
    pub fn set_composition(
        &mut self,
        id: NodeId,
        composition: Option<Composition>,
    ) -> Result<(), MutationError> {
        match self.kind(id).map_err(|_| MutationError::Stale)? {
            Some(Kind::Text | Kind::Trimmed | Kind::TextArea) => {}
            _ => return Err(MutationError::NotAControl),
        }
        match composition {
            Some(composition) => self.composition = Some((id, composition)),
            None if self.composing() == Some(id) => self.composition = None,
            None => {}
        }
        Ok(())
    }

    /// What an input method is composing in text field `id`, if anything.
    pub fn composition(&self, id: NodeId) -> Option<&Composition> {
        self.composition
            .as_ref()
            .filter(|(field, _)| *field == id)
            .map(|(_, composition)| composition)
    }

    /// The text field an input method is composing in.
    pub fn composing(&self) -> Option<NodeId> {
        self.composition.as_ref().map(|(field, _)| *field)
    }

    /// Forget the state of `id`, a node that is gone.
    pub(crate) fn forget_control(&mut self, id: NodeId) {
        self.controls.remove(&id);
        if self.composing() == Some(id) {
            self.composition = None;
        }
    }

    fn kind(&self, id: NodeId) -> Result<Option<Kind>, StaleNode> {
        let node = self.node(id).ok_or(StaleNode)?;
        let Some(element) = node.as_element() else {
            return Ok(None);
        };
        if element.name.ns != ns!(html) {
            return Ok(None);
        }
        Ok(match element.name.local {
            local_name!("input") => Some(
                match element
                    .attr(&local_name!("type"))
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    Some("checkbox" | "radio") => Kind::Check,
                    Some("url" | "email") => Kind::Trimmed,
                    _ => Kind::Text,
                },
            ),
            local_name!("textarea") => Some(Kind::TextArea),
            local_name!("select") => Some(Kind::Select),
            local_name!("option") => Some(Kind::Option),
            _ => None,
        })
    }

    fn attribute(&self, id: NodeId, name: &LocalName) -> Option<String> {
        self.node(id)?.as_element()?.attr(name).map(str::to_owned)
    }

    fn is_checked(&self, id: NodeId) -> bool {
        match self.controls.get(&id) {
            Some(Control::Checked(checked)) => *checked,
            _ => self.attribute(id, &local_name!("checked")).is_some(),
        }
    }

    /// Whether option `id` is selected by itself: what was set, or else
    /// whether it has a `selected` attribute.
    fn is_marked(&self, id: NodeId) -> bool {
        match self.controls.get(&id) {
            Some(Control::Selected(selected)) => *selected,
            _ => self.attribute(id, &local_name!("selected")).is_some(),
        }
    }

    /// The `select` whose list option `id` is in: its parent, or its
    /// `optgroup` parent's.
    fn select_of(&self, id: NodeId) -> Option<NodeId> {
        let is = |id: NodeId, name: LocalName| {
            self.node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.name.ns == ns!(html) && element.name.local == name)
        };
        let parent = self.node(id)?.parent()?;
        if is(parent, local_name!("select")) {
            return Some(parent);
        }
        let grandparent = self.node(parent)?.parent()?;
        (is(parent, local_name!("optgroup")) && is(grandparent, local_name!("select")))
            .then_some(grandparent)
    }

    /// The text of `id`'s text children, as a `textarea`'s default value.
    fn child_text(&self, id: NodeId) -> String {
        self.children(id)
            .filter_map(|child| self.node(child)?.as_text())
            .collect()
    }

    /// An `option`'s value: its `value` attribute, or else its text with
    /// whitespace stripped and collapsed.
    fn option_value(&self, id: NodeId) -> String {
        if let Some(value) = self.attribute(id, &local_name!("value")) {
            return value;
        }
        let mut text = String::new();
        let mut stack = vec![id];
        while let Some(node) = stack.pop() {
            match self.node(node).map(|node| &node.data) {
                Some(NodeData::Text(own)) => text.push_str(own),
                Some(NodeData::Element(_)) => {
                    let mut children: Vec<_> = self.children(node).collect();
                    children.reverse();
                    stack.extend(children);
                }
                _ => {}
            }
        }
        text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// A `select`'s options: its `option` children and those of its
    /// `optgroup` children, in tree order.
    fn options(&self, select: NodeId) -> Vec<NodeId> {
        let named = |id: NodeId, name: LocalName| {
            self.node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.name.ns == ns!(html) && element.name.local == name)
        };
        let mut options = Vec::new();
        for child in self.children(select) {
            if named(child, local_name!("option")) {
                options.push(child);
            } else if named(child, local_name!("optgroup")) {
                options.extend(
                    self.children(child)
                        .filter(|grandchild| named(*grandchild, local_name!("option"))),
                );
            }
        }
        options
    }

    /// The options of `select` that are selected, in tree order. With
    /// nothing set and none marked `selected`, a single-choice list shows
    /// its first option that is not disabled; with more than one marked,
    /// the last (HTML's selectedness setting algorithm).
    fn selected_options(&self, select: NodeId) -> Vec<NodeId> {
        let options = self.options(select);
        let set = options
            .iter()
            .any(|option| matches!(self.controls.get(option), Some(Control::Selected(_))));
        let mut selected: Vec<NodeId> = options
            .iter()
            .copied()
            .filter(|option| self.is_marked(*option))
            .collect();
        let multiple = self.attribute(select, &local_name!("multiple")).is_some();
        if multiple {
            return selected;
        }
        if selected.is_empty() && !set {
            selected.extend(
                options
                    .iter()
                    .copied()
                    .find(|option| self.attribute(*option, &local_name!("disabled")).is_none()),
            );
        }
        selected.split_off(selected.len().saturating_sub(1))
    }

    /// Whether element `id` is disabled, as HTML's "actually disabled" and
    /// `:disabled` say (HTML §4.10.18.2): a `button`, `input`, `select`,
    /// `textarea` or `fieldset` with a `disabled` attribute or inside a
    /// `fieldset` with one, but not in that fieldset's first `legend`; an
    /// `optgroup` with the attribute; an `option` with it or in an
    /// `optgroup` with it. `None` for an element that cannot be disabled.
    pub fn disabled(&self, id: NodeId) -> Option<bool> {
        let element = self.node(id)?.as_element()?;
        if element.name.ns != ns!(html) {
            return None;
        }
        let own = element.attr(&local_name!("disabled")).is_some();
        match element.name.local {
            local_name!("optgroup") => return Some(own),
            local_name!("option") => {
                let in_disabled_group = self
                    .node(id)?
                    .parent()
                    .and_then(|parent| self.node(parent)?.as_element())
                    .is_some_and(|parent| {
                        parent.name.local == local_name!("optgroup")
                            && parent.attr(&local_name!("disabled")).is_some()
                    });
                return Some(own || in_disabled_group);
            }
            local_name!("button")
            | local_name!("input")
            | local_name!("select")
            | local_name!("textarea")
            | local_name!("fieldset") => {}
            _ => return None,
        }
        if own {
            return Some(true);
        }
        let mut child = id;
        while let Some(parent) = self.node(child).and_then(|node| node.parent()) {
            let disabled_fieldset = self
                .node(parent)
                .and_then(|node| node.as_element())
                .is_some_and(|parent| {
                    parent.name.ns == ns!(html)
                        && parent.name.local == local_name!("fieldset")
                        && parent.attr(&local_name!("disabled")).is_some()
                });
            if disabled_fieldset && Some(child) != self.first_legend(parent) {
                return Some(true);
            }
            child = parent;
        }
        Some(false)
    }

    /// `fieldset`'s first `legend` child, which its being disabled leaves
    /// alone.
    fn first_legend(&self, fieldset: NodeId) -> Option<NodeId> {
        self.children(fieldset).find(|child| {
            self.node(*child)
                .and_then(|node| node.as_element())
                .is_some_and(|element| {
                    element.name.ns == ns!(html) && element.name.local == local_name!("legend")
                })
        })
    }

    /// The controls whose checkedness checking or unchecking `id` can
    /// change ([`Document::set_checked`]): `id`, the other radio buttons
    /// of its group, or the other options of its list. A change journal
    /// notes them before. Empty for any other element.
    pub fn checked_with(&self, id: NodeId) -> Vec<NodeId> {
        match self.kind(id) {
            Ok(Some(Kind::Check)) => {
                let mut with = vec![id];
                with.extend(self.radio_group(id));
                with
            }
            Ok(Some(Kind::Option)) => match self.select_of(id) {
                Some(select) => self.options(select),
                None => vec![id],
            },
            _ => Vec::new(),
        }
    }

    /// The other radio buttons of `id`'s group, if it is a radio button
    /// with a name.
    fn radio_group(&self, id: NodeId) -> Vec<NodeId> {
        let radio = |id: NodeId| {
            self.node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| {
                    element.name.ns == ns!(html)
                        && element.name.local == local_name!("input")
                        && element
                            .attr(&local_name!("type"))
                            .is_some_and(|kind| kind.eq_ignore_ascii_case("radio"))
                })
        };
        let name = |id: NodeId| {
            self.attribute(id, &local_name!("name"))
                .filter(|name| !name.is_empty())
        };
        let (true, Some(own)) = (radio(id), name(id)) else {
            return Vec::new();
        };
        let form = self.form_of(id);
        let mut top = id;
        while let Some(parent) = self.node(top).and_then(|node| node.parent()) {
            top = parent;
        }
        let mut group = Vec::new();
        let mut stack = vec![top];
        while let Some(node) = stack.pop() {
            if node != id
                && radio(node)
                && name(node) == Some(own.clone())
                && self.form_of(node) == form
            {
                group.push(node);
            }
            stack.extend(self.children(node));
        }
        group
    }

    /// The `form` element `id` is in, if any.
    fn form_of(&self, id: NodeId) -> Option<NodeId> {
        let mut ancestor = self.node(id)?.parent();
        while let Some(node) = ancestor {
            let element = self.node(node)?.as_element();
            if element.is_some_and(|element| {
                element.name.ns == ns!(html) && element.name.local == local_name!("form")
            }) {
                return Some(node);
            }
            ancestor = self.node(node)?.parent();
        }
        None
    }
}

/// An `input`'s value as HTML's value sanitization makes it: line breaks
/// dropped, and for `url` and `email` surrounding whitespace too.
fn sanitize(kind: Kind, value: &str) -> String {
    let value: String = value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .collect();
    match kind {
        Kind::Trimmed => value
            .trim_matches(|c: char| c.is_ascii_whitespace())
            .to_owned(),
        _ => value,
    }
}

/// `\r\n` and `\r` as `\n`, as a `textarea`'s value has them.
fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use crate::Document;

    fn find(doc: &Document, id: &str) -> crate::NodeId {
        let mut stack = vec![doc.root()];
        while let Some(node) = stack.pop() {
            if doc
                .node(node)
                .and_then(|node| node.as_element())
                .and_then(|element| element.attr(&html5ever::local_name!("id")))
                == Some(id)
            {
                return node;
            }
            stack.extend(doc.children(node));
        }
        panic!("no #{id}");
    }

    #[test]
    fn one_text_field_composes_at_a_time_and_a_removed_one_no_longer_does() {
        use crate::Composition;
        let mut doc = Document::parse_html(
            "<input id=t value=ab><textarea id=a></textarea><p id=p></p><input id=k type=checkbox>",
        );
        let (t, a, p, k) = (
            find(&doc, "t"),
            find(&doc, "a"),
            find(&doc, "p"),
            find(&doc, "k"),
        );
        let kana = Composition {
            at: 1,
            text: "か".to_owned(),
            caret: 3,
        };
        doc.set_composition(t, Some(kana.clone())).unwrap();
        assert_eq!(doc.composition(t), Some(&kana));
        assert_eq!(doc.composing(), Some(t));
        // Shown, not part of the value.
        assert_eq!(doc.value(t), Ok(Some("ab".to_owned())));
        // Taking another field's away leaves this one's.
        doc.set_composition(a, None).unwrap();
        assert_eq!(doc.composition(t), Some(&kana));
        // Another field composing takes it over.
        doc.set_composition(a, Some(kana.clone())).unwrap();
        assert_eq!((doc.composition(t), doc.composing()), (None, Some(a)));
        doc.set_composition(a, None).unwrap();
        assert_eq!(doc.composing(), None);
        // Only text fields compose.
        for other in [p, k] {
            assert_eq!(
                doc.set_composition(other, Some(kana.clone())),
                Err(crate::MutationError::NotAControl)
            );
        }
        // A field that is removed composes no more.
        doc.set_composition(t, Some(kana)).unwrap();
        doc.remove(t);
        assert_eq!(doc.composing(), None);
    }

    #[test]
    fn a_text_field_reads_its_attribute_until_its_value_is_set() {
        let mut doc =
            Document::parse_html("<input id=t value='a\nb'><input id=u type=url value=' x '>");
        let (t, u) = (find(&doc, "t"), find(&doc, "u"));
        assert_eq!(doc.value(t), Ok(Some("ab".to_owned())));
        assert_eq!(doc.value(u), Ok(Some("x".to_owned())));
        doc.set_value(t, "yeni\r\n").unwrap();
        assert_eq!(doc.value(t), Ok(Some("yeni".to_owned())));
        // The attribute no longer is the value.
        doc.set_attr(t, "value", "eski").unwrap();
        assert_eq!(doc.value(t), Ok(Some("yeni".to_owned())));
        assert_eq!(doc.checked(t), Ok(None));
    }

    #[test]
    fn what_the_user_types_keeps_its_spaces_but_not_an_inputs_line_breaks() {
        let mut doc =
            Document::parse_html("<input id=e type=email><textarea id=a></textarea><p id=p>");
        let (e, a, p) = (find(&doc, "e"), find(&doc, "a"), find(&doc, "p"));
        // An email address being typed keeps its last space.
        doc.set_typed_value(e, "a \n").unwrap();
        assert_eq!(doc.value(e), Ok(Some("a ".to_owned())));
        doc.set_typed_value(a, "b\r\n").unwrap();
        assert_eq!(doc.value(a), Ok(Some("b\n".to_owned())));
        assert_eq!(
            doc.set_typed_value(p, "x"),
            Err(crate::MutationError::NotAControl)
        );
    }

    #[test]
    fn a_textarea_reads_its_text_until_its_value_is_set() {
        let mut doc = Document::parse_html("<textarea id=t>bir\r\niki</textarea>");
        let t = find(&doc, "t");
        assert_eq!(doc.value(t), Ok(Some("bir\niki".to_owned())));
        doc.set_value(t, "üç\rdört").unwrap();
        assert_eq!(doc.value(t), Ok(Some("üç\ndört".to_owned())));
        doc.set_text(t, "beş").unwrap();
        assert_eq!(doc.value(t), Ok(Some("üç\ndört".to_owned())));
        // Text the host wrote, which the parser did not normalize.
        let mut doc = Document::parse_html("<textarea id=t></textarea>");
        let t = find(&doc, "t");
        doc.set_text(t, "a\r\nb\rc").unwrap();
        assert_eq!(doc.value(t), Ok(Some("a\nb\nc".to_owned())));
    }

    #[test]
    fn a_select_reads_its_selected_option_and_selects_by_value() {
        let mut doc = Document::parse_html(
            "<select id=s><option disabled>a<option id=b> b  c <optgroup><option value=d>D</optgroup></select>\
             <select id=m><option selected value=1><option selected value=2></select>\
             <select id=n multiple><option value=1><option value=2 selected><option value=3 selected></select>",
        );
        let (s, m, n, b) = (
            find(&doc, "s"),
            find(&doc, "m"),
            find(&doc, "n"),
            find(&doc, "b"),
        );
        // Nothing marked: the first option that is not disabled.
        assert_eq!(doc.value(s), Ok(Some("b c".to_owned())));
        assert_eq!(doc.value(b), Ok(Some("b c".to_owned())));
        // Two marked: the last.
        assert_eq!(doc.value(m), Ok(Some("2".to_owned())));
        assert_eq!(doc.value(n), Ok(Some("2".to_owned())));
        doc.set_value(s, "d").unwrap();
        assert_eq!(doc.value(s), Ok(Some("d".to_owned())));
        // No option has it: none is selected.
        doc.set_value(s, "yok").unwrap();
        assert_eq!(doc.value(s), Ok(Some(String::new())));
        doc.set_value(n, "3").unwrap();
        assert_eq!(doc.value(n), Ok(Some("3".to_owned())));
    }

    #[test]
    fn an_option_says_whether_it_is_selected() {
        let mut doc = Document::parse_html(
            "<select id=s><option id=a value=x>A<option id=b value=y><option id=c value=x>C</select>",
        );
        let [s, a, b, c] = ["s", "a", "b", "c"].map(|id| find(&doc, id));
        let selected =
            |doc: &Document| [a, b, c].map(|option| doc.checked(option).unwrap().unwrap());
        // Nothing marked: the first.
        assert_eq!(selected(&doc), [true, false, false]);
        doc.set_checked(c, true).unwrap();
        assert_eq!(selected(&doc), [false, false, true]);
        // The first option with the value, and no other.
        doc.set_value(s, "x").unwrap();
        assert_eq!(selected(&doc), [true, false, false]);
        // Unselected: the list falls back to its first.
        doc.set_checked(b, true).unwrap();
        doc.set_checked(b, false).unwrap();
        assert_eq!(selected(&doc), [true, false, false]);
        assert_eq!(doc.value(s), Ok(Some("x".to_owned())));
        // Selecting one unselects the one before: unselected, the list
        // falls back to its first, not to the one before.
        doc.set_checked(b, true).unwrap();
        doc.set_checked(c, true).unwrap();
        doc.set_checked(c, false).unwrap();
        assert_eq!(selected(&doc), [true, false, false]);
        // An option in a group is in the list.
        let doc =
            Document::parse_html("<select><optgroup><option id=g><option></optgroup></select>");
        assert_eq!(doc.checked(find(&doc, "g")), Ok(Some(true)));
        let mut doc =
            Document::parse_html("<select id=m multiple><option id=d><option id=e></select>");
        let (d, e) = (find(&doc, "d"), find(&doc, "e"));
        // Many choices: each on its own.
        doc.set_checked(d, true).unwrap();
        doc.set_checked(e, true).unwrap();
        assert_eq!(doc.checked(d), Ok(Some(true)));
        doc.set_checked(e, false).unwrap();
        assert_eq!(
            (doc.checked(d), doc.checked(e)),
            (Ok(Some(true)), Ok(Some(false)))
        );
    }

    #[test]
    fn checking_a_radio_button_unchecks_its_group() {
        let mut doc = Document::parse_html(
            "<input type=radio name=r id=a checked><input type=radio name=r id=b>\
             <input type=radio name=q id=q checked>\
             <form><input type=radio name=r id=c checked></form>\
             <input type=checkbox id=k value=evet>",
        );
        let q = find(&doc, "q");
        let (a, b, c, k) = (
            find(&doc, "a"),
            find(&doc, "b"),
            find(&doc, "c"),
            find(&doc, "k"),
        );
        assert_eq!(doc.checked(a), Ok(Some(true)));
        doc.set_checked(b, true).unwrap();
        assert_eq!(doc.checked(a), Ok(Some(false)));
        assert_eq!(doc.checked(b), Ok(Some(true)));
        // Another name: another group.
        assert_eq!(doc.checked(q), Ok(Some(true)));
        // Unchecking one leaves the rest.
        doc.set_checked(a, true).unwrap();
        doc.set_checked(b, false).unwrap();
        assert_eq!(doc.checked(a), Ok(Some(true)));
        // In a form: another group.
        assert_eq!(doc.checked(c), Ok(Some(true)));
        assert_eq!(doc.checked(k), Ok(Some(false)));
        doc.set_checked(k, true).unwrap();
        assert_eq!(doc.checked(k), Ok(Some(true)));
        assert_eq!(doc.value(k), Ok(Some("evet".to_owned())));
        assert_eq!(doc.value(a), Ok(Some("on".to_owned())));
        // What a change of each can change.
        let sorted = |mut ids: Vec<crate::NodeId>| {
            ids.sort_by_key(|id| id.index());
            ids
        };
        assert_eq!(sorted(doc.checked_with(b)), sorted(vec![a, b]));
        assert_eq!(doc.checked_with(c), [c]);
        assert_eq!(doc.checked_with(k), [k]);
    }

    #[test]
    fn a_control_is_disabled_by_its_attribute_or_a_fieldset_but_not_in_its_legend() {
        let doc = Document::parse_html(
            "<input id=own disabled><button id=free>b</button><p id=p>p</p>\
             <fieldset id=f disabled>\
               <legend><input id=in-legend><fieldset id=inner><input id=deep></fieldset></legend>\
               <legend><input id=second-legend></legend>\
               <input id=held><fieldset id=nested><legend><input id=nested-legend></legend></fieldset>\
             </fieldset>\
             <select><optgroup id=g disabled><option id=o1>1</optgroup>\
             <option id=o2>2<option id=o3 disabled>3</select>",
        );
        let disabled = |id: &str| doc.disabled(find(&doc, id));
        assert_eq!(disabled("own"), Some(true));
        assert_eq!(disabled("free"), Some(false));
        assert_eq!(disabled("p"), None);
        assert_eq!(disabled("f"), Some(true));
        // The first legend's content is the fieldset's exception, however
        // deep; a second legend is not.
        assert_eq!(disabled("in-legend"), Some(false));
        assert_eq!(disabled("inner"), Some(false));
        assert_eq!(disabled("deep"), Some(false));
        assert_eq!(disabled("second-legend"), Some(true));
        assert_eq!(disabled("held"), Some(true));
        // A fieldset inside a disabled one is disabled with all it holds,
        // its own legend too.
        assert_eq!(disabled("nested"), Some(true));
        assert_eq!(disabled("nested-legend"), Some(true));
        assert_eq!(disabled("g"), Some(true));
        assert_eq!(disabled("o1"), Some(true));
        assert_eq!(disabled("o2"), Some(false));
        assert_eq!(disabled("o3"), Some(true));
    }

    #[test]
    fn what_selecting_an_option_can_change_is_its_lists_options() {
        let doc = Document::parse_html(
            "<select><option id=a>a<optgroup><option id=b>b</optgroup></select>\
             <option id=c>c</option><p id=p>p</p>",
        );
        let (a, b, c, p) = (
            find(&doc, "a"),
            find(&doc, "b"),
            find(&doc, "c"),
            find(&doc, "p"),
        );
        assert_eq!(doc.checked_with(b), [a, b]);
        assert_eq!(doc.checked_with(c), [c]);
        assert!(doc.checked_with(p).is_empty());
    }

    #[test]
    fn an_element_without_a_value_refuses_one() {
        let mut doc = Document::parse_html("<p id=p>metin</p>");
        let p = find(&doc, "p");
        assert_eq!(doc.value(p), Ok(None));
        assert_eq!(
            doc.set_value(p, "x"),
            Err(crate::MutationError::NotAControl)
        );
        let mut doc2 = Document::parse_html("<input id=t>");
        let t = find(&doc2, "t");
        assert_eq!(doc2.checked(t), Ok(None));
        assert_eq!(
            doc2.set_checked(t, true),
            Err(crate::MutationError::NotAControl)
        );
        assert_eq!(
            doc.set_checked(p, true),
            Err(crate::MutationError::NotAControl)
        );
    }

    #[test]
    fn a_removed_control_leaves_no_state_behind() {
        let mut doc = Document::parse_html("<input id=t>");
        let t = find(&doc, "t");
        doc.set_value(t, "x").unwrap();
        assert_eq!(doc.controls.len(), 1);
        doc.remove(t);
        assert!(doc.controls.is_empty());
        assert_eq!(doc.value(t), Err(crate::StaleNode));
        // Nor a document replaced.
        doc.load_html("<input id=t>");
        let t = find(&doc, "t");
        doc.set_value(t, "x").unwrap();
        doc.load_html("<p>yeni</p>");
        assert!(doc.controls.is_empty());
    }
}
