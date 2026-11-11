//! What a click does to a checkbox, a radio button and a label (M5.10).
//!
//! A checkbox or radio button changes before its click is dispatched, as
//! HTML's "legacy-pre-activation behavior" has it (HTML §4.10.5.1.15-16):
//! the host's click callback reads the new checkedness. If the host
//! prevents the click, the checkedness goes back; otherwise `input` and
//! `change` follow, if it changed. A radio button already checked changes
//! nothing and tells nothing.
//!
//! A click on a label clicks its control: the one its `for` names, or the
//! first it holds (HTML §4.10.4). No click reaches a disabled control
//! (HTML §4.10.18.5), nor what it holds.

use erk_dom::{NodeId, local_name};

use super::Page;
use crate::messages::{Event, EventKind, Modifiers};

/// A checkbox or radio button changed by a click not yet dispatched.
#[derive(Debug)]
pub(crate) struct Activation {
    control: NodeId,
    /// The checkedness of all the click could change, as it was before.
    before: Vec<(NodeId, bool)>,
}

impl Page {
    /// A click at `node`, from a press or a key, at `at` with `modifiers`:
    /// its event, with a checkbox or radio button changed already. `None`
    /// when `node` is in a disabled control.
    pub(crate) fn click(
        &mut self,
        node: NodeId,
        at: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Event> {
        if self.in_disabled_control(node) {
            return None;
        }
        self.activation = None;
        if self.is_checkable(node) {
            let before = self
                .doc
                .checked_with(node)
                .into_iter()
                .map(|id| (id, self.doc.checked(id) == Ok(Some(true))))
                .collect();
            let checked = self.doc.checked(node) == Ok(Some(true));
            let radio = self.input_type(node).as_deref() == Some("radio");
            let _ = self.set_checked(node.to_bits(), radio || !checked);
            self.edited = true;
            self.activation = Some(Activation {
                control: node,
                before,
            });
        }
        Some(self.event(EventKind::Click, node, at, modifiers))
    }

    /// What follows a click at `target` the host dispatched, `prevented`
    /// or not: a checkbox or radio button changed goes back, or tells
    /// `input` and `change`; then the click's default action.
    pub(crate) fn after_click(&mut self, target: NodeId, prevented: bool) -> Vec<Event> {
        let activation = self
            .activation
            .take()
            .filter(|activation| activation.control == target);
        if prevented {
            if let Some(activation) = activation {
                self.restore(&activation);
            }
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(Activation { control, before }) = activation {
            let was = before
                .iter()
                .find(|(id, _)| *id == control)
                .map(|(_, checked)| *checked);
            let now = self.doc.checked(control) == Ok(Some(true));
            if was != Some(now) && erk_invalidation::journal::connected(&self.doc, control) {
                for kind in [EventKind::Input, EventKind::Change] {
                    events.push(self.event(kind, control, (0.0, 0.0), Modifiers::default()));
                }
            }
        }
        events.extend(self.click_default(target));
        events
    }

    /// Put back the checkedness a prevented click changed.
    fn restore(&mut self, activation: &Activation) {
        for (id, _) in &activation.before {
            self.journal.touch(&self.doc, *id);
        }
        // The unchecked first: checking a radio button unchecks its
        // group.
        for checked in [false, true] {
            for (id, was) in &activation.before {
                if *was == checked {
                    let _ = self.doc.set_checked(*id, checked);
                }
            }
        }
        self.edited = true;
    }

    /// The control `label` labels, if it is a label and has one that a
    /// click can reach: the element its `for` names, or else the first
    /// labelable element it holds.
    pub(crate) fn labeled_control(&self, label: NodeId) -> Option<NodeId> {
        let element = self.doc.node(label)?.as_element()?;
        if element.name.ns != erk_dom::ns!(html) || element.name.local != local_name!("label") {
            return None;
        }
        let control = match element.attr(&local_name!("for")) {
            Some(id) => self.element_with_id(id).filter(|id| self.labelable(*id))?,
            None => {
                let mut stack: Vec<NodeId> = self.doc.children(label).collect();
                stack.reverse();
                let mut found = None;
                while let Some(id) = stack.pop() {
                    if self.labelable(id) {
                        found = Some(id);
                        break;
                    }
                    let mut children: Vec<NodeId> = self.doc.children(id).collect();
                    children.reverse();
                    stack.extend(children);
                }
                found?
            }
        };
        Some(control)
    }

    /// Whether `node` is a labelable element (HTML §4.10.2): a button, an
    /// input that is not hidden, a meter, an output, a progress bar, a
    /// select or a textarea.
    fn labelable(&self, node: NodeId) -> bool {
        let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
            return false;
        };
        if element.name.ns != erk_dom::ns!(html) {
            return false;
        }
        match element.name.local {
            local_name!("input") => self.input_type(node).as_deref() != Some("hidden"),
            local_name!("button")
            | local_name!("meter")
            | local_name!("output")
            | local_name!("progress")
            | local_name!("select")
            | local_name!("textarea") => true,
            _ => false,
        }
    }

    /// Whether `node` is a checkbox or a radio button.
    fn is_checkable(&self, node: NodeId) -> bool {
        matches!(self.input_type(node).as_deref(), Some("checkbox" | "radio"))
    }

    /// The type of `node`, an `input`, in lowercase; `None` for any other
    /// node.
    fn input_type(&self, node: NodeId) -> Option<String> {
        let element = self.doc.node(node)?.as_element()?;
        (element.name.ns == erk_dom::ns!(html) && element.name.local == local_name!("input")).then(
            || {
                element
                    .attr(&local_name!("type"))
                    .unwrap_or("text")
                    .to_ascii_lowercase()
            },
        )
    }

    /// Whether `node` is a disabled control or in one: the nearest
    /// `button`, `input`, `select` or `textarea` at or above it.
    fn in_disabled_control(&self, node: NodeId) -> bool {
        let mut current = Some(node);
        while let Some(id) = current {
            let control = self
                .doc
                .node(id)
                .and_then(|found| found.as_element())
                .is_some_and(|element| {
                    element.name.ns == erk_dom::ns!(html)
                        && matches!(
                            element.name.local,
                            local_name!("button")
                                | local_name!("input")
                                | local_name!("select")
                                | local_name!("textarea")
                        )
                });
            if control {
                return self.doc.disabled(id) == Some(true);
            }
            current = self.doc.node(id).and_then(|found| found.parent());
        }
        false
    }
}
