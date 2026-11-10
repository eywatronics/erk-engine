//! Dialogs (HTML §4.11.4, M5.9). A `dialog` with `open` shows; the host
//! opens one, modal or not, and closes it. A modal dialog is in the top
//! layer with a backdrop behind it, and the rest of the page is inert: a
//! press there goes to the dialog, and the focus stays inside it. Escape
//! asks to close it: a `cancel` event the host can prevent, then it
//! closes, and a `close`.
//!
//! What the host does itself raises no event, as with a `details`' `open`
//! or a popover it shows: it knows what it did. A button's command
//! (command.rs) shows, closes or asks to close one for the user.

use erk_dom::{NodeId, local_name};

use super::{Page, status};
use crate::messages::{Event, EventKind, Modifiers, Status};

impl Page {
    /// Whether `id` is an HTML `dialog` element.
    pub(crate) fn is_dialog(&self, id: NodeId) -> bool {
        self.doc
            .node(id)
            .and_then(|found| found.as_element())
            .is_some_and(|element| {
                element.name.ns == erk_dom::ns!(html) && element.name.local == local_name!("dialog")
            })
    }

    /// Whether `id` has an `open` attribute.
    pub(crate) fn has_open(&self, id: NodeId) -> bool {
        self.doc
            .node(id)
            .and_then(|found| found.as_element())
            .is_some_and(|element| element.attr(&local_name!("open")).is_some())
    }

    /// Show dialog `node` as DOM's `show()` does, or `showModal()` with
    /// `modal`: it gets `open`, a modal one joins the top layer, and the
    /// focus goes into it. A modal one closes the open `auto` popovers that
    /// are not its ancestors. Showing one already shown the same way does
    /// nothing; `InvalidArgument` for an element that is not a dialog, one
    /// shown the other way, an open popover, or a modal one outside the
    /// document.
    pub(crate) fn show_dialog(&mut self, node: u64, modal: bool) -> Result<(), Status> {
        let id = self.node(node)?;
        self.show_dialog_now(id, modal, false).map(drop)
    }

    /// Show dialog `id` (see [`Page::show_dialog`]). With `events`, the
    /// user showed it (a button's command, whose plan closed the popovers
    /// first): the events of the focus and of popovers closing now come
    /// back. The host's own change sends none.
    pub(crate) fn show_dialog_now(
        &mut self,
        id: NodeId,
        modal: bool,
        events: bool,
    ) -> Result<Vec<Event>, Status> {
        if !self.is_dialog(id) {
            return Err(Status::InvalidArgument);
        }
        if self.has_open(id) {
            return if self.doc.is_modal(id) == modal {
                Ok(Vec::new())
            } else {
                Err(Status::InvalidArgument)
            };
        }
        if self.doc.in_top_layer(id)
            || (modal && !erk_invalidation::journal::connected(&self.doc, id))
        {
            return Err(Status::InvalidArgument);
        }
        let mut out = Vec::new();
        if modal {
            let ancestor = self.popover_ancestor(id);
            for popover in self.autos_above(ancestor).into_iter().rev() {
                out.extend(self.close_popover(popover, false));
            }
        }
        self.journal.touch(&self.doc, id);
        self.doc.set_attr(id, "open", "").map_err(status)?;
        if modal {
            self.touch_top_layer(Some(id));
            self.doc.add_to_top_layer(id, true).map_err(status)?;
        }
        let focus = self.focus.filter(|focus| !self.inside(*focus, id));
        self.remember_focus(id, focus);
        // It shows unless the user's last input was the pointer's, as a
        // focus a script moves does in browsers.
        let control = self.focus_delegate(id).unwrap_or(id);
        let visible = !self.pointer_last;
        out.extend(self.move_focus(Some(control), Modifiers::default(), visible));
        Ok(if events { out } else { Vec::new() })
    }

    /// Where the focus goes in a dialog that opens (HTML's dialog focusing
    /// steps): the dialog itself if it has `autofocus`, else the first
    /// element in it with `autofocus` that takes the focus, else the first
    /// that Tab would visit. What it holds was hidden until now, so a
    /// descendant counts as shown unless the last frame styled it hidden.
    fn focus_delegate(&self, dialog: NodeId) -> Option<NodeId> {
        let autofocus = |id: NodeId| {
            self.doc
                .node(id)
                .and_then(|found| found.as_element())
                .is_some_and(|element| element.attr(&local_name!("autofocus")).is_some())
        };
        if autofocus(dialog) {
            return Some(dialog);
        }
        let mut descendants = Vec::new();
        let mut stack: Vec<NodeId> = self.doc.children(dialog).collect();
        stack.reverse();
        while let Some(id) = stack.pop() {
            descendants.push(id);
            let mut children: Vec<NodeId> = self.doc.children(id).collect();
            children.reverse();
            stack.extend(children);
        }
        let takes = |id: NodeId| self.focusable_markup(id).is_some();
        descendants
            .iter()
            .copied()
            .find(|id| autofocus(*id) && takes(*id))
            .or_else(|| {
                descendants
                    .iter()
                    .copied()
                    .find(|id| self.focusable_markup(*id).is_some_and(|index| index >= 0))
            })
    }

    /// Close dialog `node` as DOM's `close()` does, setting its return
    /// value to `value` if there is one: it loses `open` and leaves the top
    /// layer, and the focus goes back where it was. Nothing for a closed
    /// one; `InvalidArgument` for an element that is not a dialog.
    pub(crate) fn close_dialog(&mut self, node: u64, value: Option<&str>) -> Result<(), Status> {
        let id = self.node(node)?;
        if !self.is_dialog(id) {
            return Err(Status::InvalidArgument);
        }
        self.close_dialog_now(id, value, false);
        Ok(())
    }

    /// Dialog `node`'s return value: what it was last closed with, empty
    /// until then. `InvalidArgument` for an element that is not a dialog.
    pub(crate) fn dialog_return_value(&self, node: u64) -> Result<String, Status> {
        let id = self.node(node)?;
        if !self.is_dialog(id) {
            return Err(Status::InvalidArgument);
        }
        Ok(self
            .dialogs
            .iter()
            .find(|(dialog, _)| *dialog == id)
            .map(|(_, value)| value.clone())
            .unwrap_or_default())
    }

    /// Close `dialog` if it is open. A modal one, or one with the focus in
    /// it, gives the focus back to the element that had it when it opened.
    /// With `events`, the user closed it: the focus's events and a `close`.
    pub(crate) fn close_dialog_now(
        &mut self,
        dialog: NodeId,
        value: Option<&str>,
        events: bool,
    ) -> Vec<Event> {
        if !self.has_open(dialog) {
            return Vec::new();
        }
        self.journal.touch(&self.doc, dialog);
        let removed = self.doc.remove_attr(dialog, "open");
        debug_assert!(removed.is_ok());
        let modal = self.doc.is_modal(dialog);
        if self.doc.in_top_layer(dialog) {
            self.touch_top_layer(None);
            self.doc.remove_from_top_layer(dialog);
        }
        if let Some(value) = value {
            self.dialogs.retain(|(id, _)| *id != dialog);
            self.dialogs.push((dialog, value.to_owned()));
        }
        let before = self.forget_focus(dialog);
        let focus_inside = self.focus.is_some_and(|focus| self.inside(focus, dialog));
        let mut out = Vec::new();
        if modal || focus_inside {
            let back = before.filter(|node| {
                erk_invalidation::journal::connected(&self.doc, *node) && self.focusable(*node)
            });
            if back.is_some() || focus_inside {
                out.extend(self.move_focus(back, Modifiers::default(), false));
            }
        }
        if !events {
            return Vec::new();
        }
        out.push(self.event(EventKind::Close, dialog, (0.0, 0.0), Modifiers::default()));
        out
    }

    /// The modal dialog on top of the top layer, which makes the page
    /// behind it inert.
    pub(crate) fn blocking_modal(&self) -> Option<NodeId> {
        self.doc
            .top_layer()
            .iter()
            .rev()
            .find(|id| self.doc.is_modal(**id))
            .copied()
    }

    /// Whether `node` is inert behind a modal dialog: outside it and
    /// outside what the top layer holds above it.
    pub(crate) fn inert(&self, node: NodeId) -> bool {
        let Some(modal) = self.blocking_modal() else {
            return false;
        };
        let layer = self.doc.top_layer();
        let at = layer.iter().position(|id| *id == modal).unwrap_or(0);
        !layer[at..].iter().any(|top| self.inside(node, *top))
    }

    /// The host set or removed attribute `name` of `node`: a modal dialog
    /// that lost `open` leaves the top layer, without events.
    pub(crate) fn dialog_attribute_changed(&mut self, node: NodeId, name: &str) {
        if name.eq_ignore_ascii_case("open") && self.doc.is_modal(node) && !self.has_open(node) {
            self.touch_top_layer(None);
            self.doc.remove_from_top_layer(node);
            self.forget_focus(node);
        }
    }

    /// Whether `node` could take the focus by what it is, and its
    /// `tabindex` (0 for the elements that take it without one): the last
    /// frame's styles count only when they hide it.
    fn focusable_markup(&self, node: NodeId) -> Option<i32> {
        let element = self.doc.node(node)?.as_element()?;
        let hidden = self.styles.computed(node).is_some_and(|style| {
            style.get_box().clone_display() == erk_style::style::values::computed::Display::None
                || style.clone_visibility()
                    != erk_style::style::computed_values::visibility::T::Visible
        });
        if hidden {
            return None;
        }
        let name = &element.name.local;
        let control = [
            local_name!("button"),
            local_name!("input"),
            local_name!("select"),
            local_name!("textarea"),
        ]
        .contains(name);
        if control {
            let hidden_input = *name == local_name!("input")
                && element
                    .attr(&local_name!("type"))
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"));
            if element.attr(&local_name!("disabled")).is_some() || hidden_input {
                return None;
            }
            return Some(super::tabindex(element).unwrap_or(0));
        }
        if let Some(index) = super::tabindex(element) {
            return Some(index);
        }
        (*name == local_name!("a") && element.attr(&local_name!("href")).is_some()).then_some(0)
    }
}
