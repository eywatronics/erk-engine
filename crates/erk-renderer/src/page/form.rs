//! Submitting a form (M5.10).
//!
//! A click on a submit button that the host did not prevent submits the
//! button's form: a `submit` event at the form, with the button as its
//! `source` (DOM's `SubmitEvent.submitter`). Enter in a one-line text
//! field submits implicitly (HTML §4.10.21.2): it clicks the form's
//! default button, its first submit button, or with none submits the
//! form itself when the field is the only one that would block it.
//!
//! Erk does not navigate: what a submission sends is the host's. A
//! `submit` the host lets through does one thing by default, for a form
//! whose method is `dialog`: it closes the dialog it is in, with the
//! button's value as the return value (HTML §4.10.21.3).

use erk_dom::{ElementData, NodeId, local_name};

use super::Page;
use crate::messages::{Event, EventKind, Modifiers};

/// The input types that make a field block implicit submission (HTML
/// §4.10.21.2): with more than one in a form without a submit button,
/// Enter submits nothing.
const BLOCKING: [&str; 12] = [
    "text",
    "search",
    "url",
    "tel",
    "email",
    "password",
    "date",
    "month",
    "week",
    "time",
    "datetime-local",
    "number",
];

/// The type of `input` element `element`, lowercase, `text` when it has
/// none.
fn input_type(element: &ElementData) -> String {
    element
        .attr(&local_name!("type"))
        .unwrap_or("text")
        .to_ascii_lowercase()
}

impl Page {
    /// The form `node` belongs to: its nearest `form` ancestor.
    pub(crate) fn form_owner(&self, node: NodeId) -> Option<NodeId> {
        let mut at = self.doc.node(node).and_then(|found| found.parent());
        while let Some(id) = at {
            let found = self.doc.node(id)?;
            if found.as_element().is_some_and(|element| {
                element.name.ns == erk_dom::ns!(html) && element.name.local == local_name!("form")
            }) {
                return Some(id);
            }
            at = found.parent();
        }
        None
    }

    /// Whether `node` is a submit button: a `button` HTML's rules make one
    /// (`type=submit`, or none and no `commandfor`), or an `input` of type
    /// `submit` or `image`.
    pub(crate) fn is_submit_button(&self, node: NodeId) -> bool {
        let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
            return false;
        };
        if element.name.ns != erk_dom::ns!(html) {
            return false;
        }
        match element.name.local {
            local_name!("button") => super::command::submits(element),
            local_name!("input") => matches!(input_type(element).as_str(), "submit" | "image"),
            _ => false,
        }
    }

    /// A click's default at `button`: if it is a submit button in a form,
    /// the form's `submit` event, the button its source. `None` for any
    /// other element.
    pub(crate) fn invoke_submit(&mut self, button: NodeId) -> Option<Vec<Event>> {
        if !self.is_submit_button(button) {
            return None;
        }
        let form = self.form_owner(button)?;
        Some(vec![self.submit_event(form, Some(button))])
    }

    /// Enter in text field `field`: if it is a one-line field in a form,
    /// a click on the form's default button, its first submit button in
    /// tree order (none if that one is disabled); with no submit button,
    /// the form's `submit` when no other field would block it. `None`
    /// when Enter submits nothing here.
    pub(crate) fn implicit_submit(&mut self, field: NodeId) -> Option<Vec<Event>> {
        let element = self.doc.node(field)?.as_element()?;
        if element.name.ns != erk_dom::ns!(html)
            || element.name.local != local_name!("input")
            || !BLOCKING.contains(&input_type(element).as_str())
        {
            return None;
        }
        let form = self.form_owner(field)?;
        let mut blocking = 0;
        let mut stack: Vec<NodeId> = self.doc.children(form).collect();
        stack.reverse();
        while let Some(id) = stack.pop() {
            if self.is_submit_button(id) {
                // The default button: its click submits, if it takes one.
                return Some(
                    self.click(id, (0.0, 0.0), Modifiers::default())
                        .into_iter()
                        .collect(),
                );
            }
            let blocks = self
                .doc
                .node(id)
                .and_then(|found| found.as_element())
                .is_some_and(|element| {
                    element.name.ns == erk_dom::ns!(html)
                        && element.name.local == local_name!("input")
                        && BLOCKING.contains(&input_type(element).as_str())
                });
            blocking += usize::from(blocks);
            let mut children: Vec<NodeId> = self.doc.children(id).collect();
            children.reverse();
            stack.extend(children);
        }
        Some(if blocking > 1 {
            Vec::new()
        } else {
            vec![self.submit_event(form, None)]
        })
    }

    /// What follows a `submit` at `form` sent by `submitter`, once the host
    /// dispatched it: unless prevented, a form whose method is `dialog`
    /// (the submitter's `formmethod`, else the form's `method`) closes the
    /// dialog it is in, with the submitter's value as the return value.
    pub(crate) fn after_submit(
        &mut self,
        form: NodeId,
        submitter: Option<NodeId>,
        prevented: bool,
    ) -> Vec<Event> {
        if prevented || !erk_invalidation::journal::connected(&self.doc, form) {
            return Vec::new();
        }
        let attr = |node: NodeId, name: &erk_dom::LocalName| {
            self.doc
                .node(node)
                .and_then(|found| found.as_element())
                .and_then(|element| element.attr(name))
                .map(str::to_owned)
        };
        let method = submitter
            .and_then(|button| attr(button, &local_name!("formmethod")))
            .or_else(|| attr(form, &local_name!("method")));
        if !method.is_some_and(|method| method.eq_ignore_ascii_case("dialog")) {
            return Vec::new();
        }
        let mut at = self.doc.node(form).and_then(|found| found.parent());
        while let Some(id) = at {
            if self.is_dialog(id) {
                let value = submitter.and_then(|button| attr(button, &local_name!("value")));
                return self.close_dialog_now(id, value.as_deref(), true);
            }
            at = self.doc.node(id).and_then(|found| found.parent());
        }
        Vec::new()
    }

    /// A `submit` event at `form`, from `submitter` if a button sent it.
    fn submit_event(&self, form: NodeId, submitter: Option<NodeId>) -> Event {
        let mut event = self.event(EventKind::Submit, form, (0.0, 0.0), Modifiers::default());
        event.source = submitter.map(NodeId::to_bits);
        event
    }
}
