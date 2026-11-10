//! Popovers (HTML §6.12, M5.9): an element with a `popover` attribute
//! shows while it is in the top layer. A button whose `popovertarget`
//! names it opens and closes it; a press outside every open `auto`
//! popover, or Escape, closes them (light dismiss); opening an `auto`
//! popover closes those that are not its ancestors.
//!
//! Opening asks first: a `beforetoggle` event the host can cancel, then a
//! `toggle`. Closing tells first: a `beforetoggle` it cannot cancel, while
//! the popover still shows, then a `toggle`. So a change happens only
//! after the host has seen the event before it, and the page keeps the
//! steps still to take as a plan: each step that asks waits for the host
//! to dispatch its event, which [`Page::after_before_toggle`] says. The
//! erk crate dispatches every event before the next, so a plan is done
//! when the input that started it returns. A modal dialog's `cancel`
//! (dialog.rs) waits in the same plan: Escape closes whichever of them is
//! on top.

use std::collections::VecDeque;

use erk_dom::{NodeId, local_name};

use super::Page;
use crate::messages::{Event, EventKind, Modifiers, Status};

/// What the page keeps for popovers between inputs.
#[derive(Default)]
pub(crate) struct Popovers {
    /// The steps still to take; the first waits for the host when it
    /// asks.
    plan: VecDeque<Step>,
    /// Whether the first step's event went to the host and is awaited.
    awaiting: bool,
    /// Each open popover's or dialog's element that had the focus when it
    /// opened: where the focus goes back when it closes.
    focus_before: Vec<(NodeId, Option<NodeId>)>,
    /// The popover a primary press went down in, or `Some(None)` for a
    /// press in none: light dismiss compares it with the one it comes up
    /// in.
    pressed_in: Option<Option<NodeId>>,
}

/// One step of opening or closing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// A cancelable `beforetoggle`, then the `auto` popovers that are not
    /// its ancestors close and it opens. `invoker` is the button that
    /// opens it, whose popover counts as an ancestor.
    Ask {
        popover: NodeId,
        invoker: Option<NodeId>,
    },
    /// A `beforetoggle`, then it closes, and a `toggle`. With
    /// `focus_back`, a focus inside it goes back where it was before.
    Hide { popover: NodeId, focus_back: bool },
    /// It joins the top layer, and a `toggle`.
    Add(NodeId),
    /// A modal dialog's `cancel`, which the host can prevent, then it
    /// closes, and a `close`.
    Cancel(NodeId),
}

/// The states of the `popover` attribute (HTML §6.12.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PopoverKind {
    /// `auto` or the empty string: one of a stack light dismiss closes.
    /// `hint` is taken as `auto` too.
    Auto,
    /// `manual`, and any value HTML does not know: only its buttons and
    /// the host open and close it.
    Manual,
}

/// What a `popovertargetaction` asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Toggle,
    Show,
    Hide,
}

impl Page {
    /// The state of element `id`'s `popover` attribute; `None` for an
    /// element without one.
    pub(crate) fn popover_kind(&self, id: NodeId) -> Option<PopoverKind> {
        let element = self.doc.node(id)?.as_element()?;
        if element.name.ns != erk_dom::ns!(html) {
            return None;
        }
        let value = element.attr(&local_name!("popover"))?;
        Some(
            if value.is_empty()
                || value.eq_ignore_ascii_case("auto")
                || value.eq_ignore_ascii_case("hint")
            {
                PopoverKind::Auto
            } else {
                PopoverKind::Manual
            },
        )
    }

    /// Whether `id` is an open popover (not a modal dialog that has a
    /// `popover` attribute).
    fn is_open_popover(&self, id: NodeId) -> bool {
        self.doc.in_top_layer(id) && !self.doc.is_modal(id) && self.popover_kind(id).is_some()
    }

    /// The open `auto` popovers, the top one last.
    fn auto_stack(&self) -> Vec<NodeId> {
        self.doc
            .top_layer()
            .iter()
            .copied()
            .filter(|id| {
                !self.doc.is_modal(*id) && self.popover_kind(*id) == Some(PopoverKind::Auto)
            })
            .collect()
    }

    /// A press in `target` (a click it was not prevented from doing):
    /// what the nearest button around it does to its popover. `None` when
    /// it is no popover's button; the events that follow otherwise.
    pub(crate) fn invoke_popover(&mut self, button: NodeId) -> Option<Vec<Event>> {
        let (popover, action) = self.popover_target(button)?;
        match (action, self.is_open_popover(popover)) {
            (Action::Toggle | Action::Show, false) => self.popovers.plan.push_back(Step::Ask {
                popover,
                invoker: Some(button),
            }),
            (Action::Toggle | Action::Hide, true) => self.plan_hide(popover, true),
            _ => return Some(Vec::new()),
        }
        Some(self.run_popovers())
    }

    /// Plan to close `popover`; an `auto` one's `auto` popovers above it
    /// close first, the top one first.
    fn plan_hide(&mut self, popover: NodeId, focus_back: bool) {
        let stack = self.auto_stack();
        if let Some(at) = stack.iter().position(|id| *id == popover) {
            for above in stack[at + 1..].iter().rev() {
                self.popovers.plan.push_back(Step::Hide {
                    popover: *above,
                    focus_back,
                });
            }
        }
        self.popovers.plan.push_back(Step::Hide {
            popover,
            focus_back,
        });
    }

    /// The popover `button` opens or closes, and how: a button (or an
    /// `input` that is one) that is not disabled, nor a form's submit
    /// button, whose `popovertarget` is the id of an element in the
    /// document with a `popover` attribute (HTML §6.12.2).
    fn popover_target(&self, button: NodeId) -> Option<(NodeId, Action)> {
        let element = self.doc.node(button)?.as_element()?;
        if element.name.ns != erk_dom::ns!(html) || element.attr(&local_name!("disabled")).is_some()
        {
            return None;
        }
        let kind = element.attr(&local_name!("type"));
        let is = |name: &str| kind.is_some_and(|kind| kind.eq_ignore_ascii_case(name));
        let submits = match element.name.local {
            local_name!("button") => !is("button") && !is("reset"),
            local_name!("input") if is("button") || is("reset") => false,
            local_name!("input") if is("submit") || is("image") => true,
            _ => return None,
        };
        if submits && self.in_form(button) {
            return None;
        }
        let target = element.attr(&local_name!("popovertarget"))?;
        let action = match element.attr(&local_name!("popovertargetaction")) {
            Some(value) if value.eq_ignore_ascii_case("show") => Action::Show,
            Some(value) if value.eq_ignore_ascii_case("hide") => Action::Hide,
            _ => Action::Toggle,
        };
        let popover = self.element_with_id(target)?;
        self.popover_kind(popover)?;
        Some((popover, action))
    }

    /// Whether `node` is inside a `form`, which its submit button submits.
    fn in_form(&self, node: NodeId) -> bool {
        let mut at = self.doc.node(node).and_then(|found| found.parent());
        while let Some(id) = at {
            let found = self.doc.node(id);
            if found
                .and_then(|found| found.as_element())
                .is_some_and(|element| element.name.local == local_name!("form"))
            {
                return true;
            }
            at = found.and_then(|found| found.parent());
        }
        false
    }

    /// The first element in the document, in tree order, whose `id` is
    /// `id`.
    pub(crate) fn element_with_id(&self, id: &str) -> Option<NodeId> {
        if id.is_empty() {
            return None;
        }
        let mut stack = vec![self.doc.root()];
        while let Some(node) = stack.pop() {
            let mut children: Vec<_> = self.doc.children(node).collect();
            children.reverse();
            stack.extend(children);
            let found = self
                .doc
                .node(node)
                .and_then(|found| found.as_element())
                .is_some_and(|element| element.attr(&local_name!("id")) == Some(id));
            if found {
                return Some(node);
            }
        }
        None
    }

    /// The nearest open `auto` popover `node` is in, itself included.
    fn popover_around(&self, node: NodeId) -> Option<NodeId> {
        let mut at = Some(node);
        while let Some(id) = at {
            if self.is_open_popover(id) && self.popover_kind(id) == Some(PopoverKind::Auto) {
                return Some(id);
            }
            at = self.doc.node(id).and_then(|found| found.parent());
        }
        None
    }

    /// Of `a` and `b`, the one higher in the `auto` stack.
    fn higher(&self, a: Option<NodeId>, b: Option<NodeId>) -> Option<NodeId> {
        let stack = self.auto_stack();
        let at = |id: Option<NodeId>| id.and_then(|id| stack.iter().position(|e| *e == id));
        if at(b) > at(a) { b } else { a }
    }

    /// The open popover a press on `node` is in, for light dismiss: the
    /// one around it, or the one its button opened, whichever is higher
    /// (HTML's topmost clicked popover).
    fn pressed_popover(&self, node: NodeId) -> Option<NodeId> {
        let around = self.popover_around(node);
        let mut at = Some(node);
        let mut opened = None;
        while let Some(id) = at {
            if let Some((popover, _)) = self.popover_target(id)
                && self.is_open_popover(popover)
            {
                opened = Some(popover);
                break;
            }
            at = self.doc.node(id).and_then(|found| found.parent());
        }
        self.higher(around, opened)
    }

    /// The primary button went down on `target`.
    pub(crate) fn popover_press_down(&mut self, target: Option<NodeId>) {
        self.popovers.pressed_in = Some(target.and_then(|node| self.pressed_popover(node)));
    }

    /// The primary button came up on `target`: when it went down in the
    /// same popover, or outside them all, the `auto` popovers above that
    /// one close (light dismiss). The events that follow.
    pub(crate) fn popover_press_up(&mut self, target: Option<NodeId>) -> Vec<Event> {
        let Some(down) = self.popovers.pressed_in.take() else {
            return Vec::new();
        };
        let up = target.and_then(|node| self.pressed_popover(node));
        if down != up {
            return Vec::new();
        }
        let stack = self.auto_stack();
        let keep = up.and_then(|up| stack.iter().position(|id| *id == up));
        let above = keep.map_or(0, |at| at + 1);
        for popover in stack[above..].iter().rev() {
            self.popovers.plan.push_back(Step::Hide {
                popover: *popover,
                focus_back: false,
            });
        }
        self.run_popovers()
    }

    /// Escape, not prevented (HTML's close request): the top `auto`
    /// popover closes, or the top modal dialog is asked to, whichever is
    /// higher. A dialog with `closedby=none` does not listen. `None` when
    /// nothing listens.
    pub(crate) fn close_request(&mut self) -> Option<Vec<Event>> {
        let top = self.doc.top_layer().iter().rev().copied().find(|id| {
            if self.doc.is_modal(*id) {
                !self
                    .doc
                    .node(*id)
                    .and_then(|found| found.as_element())
                    .and_then(|element| element.attr(&local_name!("closedby")))
                    .is_some_and(|value| value.eq_ignore_ascii_case("none"))
            } else {
                self.popover_kind(*id) == Some(PopoverKind::Auto)
            }
        })?;
        if self.doc.is_modal(top) {
            self.popovers.plan.push_back(Step::Cancel(top));
        } else {
            self.plan_hide(top, true);
        }
        Some(self.run_popovers())
    }

    /// The host dispatched the `cancel` event at `target`, and `prevented`
    /// says whether a subscription prevented it: the dialog closes unless
    /// it did. The events that follow.
    pub(crate) fn after_cancel(&mut self, target: NodeId, prevented: bool) -> Vec<Event> {
        if !self.popovers.awaiting || self.popovers.plan.front() != Some(&Step::Cancel(target)) {
            return Vec::new();
        }
        self.popovers.plan.pop_front();
        self.popovers.awaiting = false;
        let mut events = Vec::new();
        if !prevented {
            events.extend(self.close_dialog_now(target, None, true));
        }
        events.extend(self.run_popovers());
        events
    }

    /// The host dispatched the `beforetoggle` event at `target`, and
    /// `prevented` says whether a subscription cancelled it: the plan goes
    /// on. The events that follow.
    pub(crate) fn after_before_toggle(&mut self, target: NodeId, prevented: bool) -> Vec<Event> {
        let waiting = match self.popovers.plan.front() {
            Some(Step::Ask { popover, .. } | Step::Hide { popover, .. })
                if self.popovers.awaiting && *popover == target =>
            {
                self.popovers.plan.pop_front()
            }
            _ => return Vec::new(),
        };
        self.popovers.awaiting = false;
        let mut events = Vec::new();
        match waiting {
            Some(Step::Ask { popover, invoker }) => {
                // Unless cancelled, or no longer something to open.
                if !prevented && self.can_open(popover) {
                    let mut steps = vec![Step::Add(popover)];
                    if self.popover_kind(popover) == Some(PopoverKind::Auto) {
                        let ancestor = self.higher(
                            self.popover_ancestor(popover),
                            invoker.and_then(|b| self.popover_around(b)),
                        );
                        let stack = self.auto_stack();
                        let keep = ancestor.and_then(|a| stack.iter().position(|id| *id == a));
                        let above = keep.map_or(0, |at| at + 1);
                        steps.splice(
                            0..0,
                            stack[above..].iter().rev().map(|popover| Step::Hide {
                                popover: *popover,
                                focus_back: false,
                            }),
                        );
                    }
                    for step in steps.into_iter().rev() {
                        self.popovers.plan.push_front(step);
                    }
                }
            }
            // Not closed meanwhile.
            Some(Step::Hide {
                popover,
                focus_back,
            }) if self.is_open_popover(popover) => {
                events.extend(self.close_popover(popover, focus_back));
            }
            _ => {}
        }
        events.extend(self.run_popovers());
        events
    }

    /// Whether `popover` can open now: a closed popover in the document,
    /// not an open dialog.
    fn can_open(&self, popover: NodeId) -> bool {
        self.popover_kind(popover).is_some()
            && !self.doc.in_top_layer(popover)
            && !(self.is_dialog(popover) && self.has_open(popover))
            && erk_invalidation::journal::connected(&self.doc, popover)
    }

    /// The nearest open `auto` popover above `popover` in the document.
    fn popover_ancestor(&self, popover: NodeId) -> Option<NodeId> {
        let parent = self.doc.node(popover)?.parent()?;
        self.popover_around(parent)
    }

    /// Take the plan's steps up to the next that asks the host: that one's
    /// `beforetoggle` goes out and the plan waits. The events.
    fn run_popovers(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while !self.popovers.awaiting
            && let Some(step) = self.popovers.plan.front().copied()
        {
            match step {
                Step::Ask { popover, .. } if self.can_open(popover) => {
                    self.popovers.awaiting = true;
                    events.push(self.popover_event(EventKind::BeforeToggle, popover));
                }
                Step::Hide { popover, .. } if self.is_open_popover(popover) => {
                    self.popovers.awaiting = true;
                    events.push(self.popover_event(EventKind::BeforeToggle, popover));
                }
                Step::Add(popover) => {
                    self.popovers.plan.pop_front();
                    if self.can_open(popover) {
                        events.extend(self.open_popover(popover));
                    }
                }
                Step::Cancel(dialog) if self.doc.is_modal(dialog) => {
                    self.popovers.awaiting = true;
                    events.push(self.popover_event(EventKind::Cancel, dialog));
                }
                // Opened or closed already.
                Step::Ask { .. } | Step::Hide { .. } | Step::Cancel(_) => {
                    self.popovers.plan.pop_front();
                }
            }
        }
        events
    }

    /// Put `popover` in the top layer: a `toggle`.
    fn open_popover(&mut self, popover: NodeId) -> Vec<Event> {
        self.touch_top_layer(Some(popover));
        let added = self.doc.add_to_top_layer(popover, false);
        debug_assert!(added.is_ok());
        let focus = self.focus.filter(|focus| !self.inside(*focus, popover));
        self.remember_focus(popover, focus);
        vec![self.popover_event(EventKind::Toggle, popover)]
    }

    /// `focus` had the focus when `id` opened.
    pub(crate) fn remember_focus(&mut self, id: NodeId, focus: Option<NodeId>) {
        self.popovers.focus_before.retain(|(entry, _)| *entry != id);
        self.popovers.focus_before.push((id, focus));
    }

    /// What had the focus when `id` opened, forgotten.
    pub(crate) fn forget_focus(&mut self, id: NodeId) -> Option<NodeId> {
        let at = self
            .popovers
            .focus_before
            .iter()
            .position(|(entry, _)| *entry == id)?;
        self.popovers.focus_before.remove(at).1
    }

    /// Show popover `node` for the host, as DOM's `showPopover()` does but
    /// with no event: an `auto` one closes the `auto` popovers that are
    /// not its ancestors. Showing an open one does nothing;
    /// `InvalidArgument` for an element without `popover`, or one that
    /// cannot open (outside the document, an open dialog).
    pub(crate) fn show_popover(&mut self, node: u64) -> Result<(), Status> {
        let id = self.node(node)?;
        self.popover_kind(id).ok_or(Status::InvalidArgument)?;
        if self.is_open_popover(id) {
            return Ok(());
        }
        if !self.can_open(id) {
            return Err(Status::InvalidArgument);
        }
        if self.popover_kind(id) == Some(PopoverKind::Auto) {
            let ancestor = self.popover_ancestor(id);
            let stack = self.auto_stack();
            let keep = ancestor.and_then(|a| stack.iter().position(|entry| *entry == a));
            for above in stack[keep.map_or(0, |at| at + 1)..].iter().rev() {
                self.close_popover(*above, false);
            }
        }
        self.open_popover(id);
        Ok(())
    }

    /// Hide popover `node` for the host, as DOM's `hidePopover()` does but
    /// with no event: an `auto` one's `auto` popovers above it close
    /// first. Hiding a closed one does nothing; `InvalidArgument` for an
    /// element without `popover`.
    pub(crate) fn hide_popover(&mut self, node: u64) -> Result<(), Status> {
        let id = self.node(node)?;
        self.popover_kind(id).ok_or(Status::InvalidArgument)?;
        if !self.is_open_popover(id) {
            return Ok(());
        }
        let stack = self.auto_stack();
        if let Some(at) = stack.iter().position(|entry| *entry == id) {
            for above in stack[at + 1..].iter().rev() {
                self.close_popover(*above, true);
            }
        }
        self.close_popover(id, true);
        Ok(())
    }

    /// Take `popover` out of the top layer. A focus inside it goes where it
    /// was before it opened with `focus_back`, else away. The events: the
    /// focus's, then a `toggle`.
    fn close_popover(&mut self, popover: NodeId, focus_back: bool) -> Vec<Event> {
        self.touch_top_layer(None);
        self.doc.remove_from_top_layer(popover);
        let before = self.forget_focus(popover);
        let mut events = Vec::new();
        if let Some(focus) = self.focus
            && self.inside(focus, popover)
        {
            let back = before.filter(|node| {
                focus_back
                    && erk_invalidation::journal::connected(&self.doc, *node)
                    && self.focusable(*node)
            });
            events.extend(self.move_focus(back, Modifiers::default(), false));
        }
        events.push(self.popover_event(EventKind::Toggle, popover));
        events
    }

    /// Whether `node` is `ancestor` or inside it.
    pub(crate) fn inside(&self, node: NodeId, ancestor: NodeId) -> bool {
        let mut at = Some(node);
        while let Some(id) = at {
            if id == ancestor {
                return true;
            }
            at = self.doc.node(id).and_then(|found| found.parent());
        }
        false
    }

    /// The top layer is about to change: every element in it, and `also`,
    /// may move in it.
    pub(crate) fn touch_top_layer(&mut self, also: Option<NodeId>) {
        let entries: Vec<NodeId> = self.doc.top_layer().iter().copied().chain(also).collect();
        for id in entries {
            self.journal.touch(&self.doc, id);
        }
    }

    /// The host set or removed attribute `name` of `node`, whose popover
    /// state was `before`: an open popover whose state changed closes,
    /// without events, as when the host sets a `details`' `open`.
    pub(crate) fn popover_attribute_changed(
        &mut self,
        node: NodeId,
        name: &str,
        before: Option<PopoverKind>,
    ) {
        if name.eq_ignore_ascii_case("popover")
            && self.doc.in_top_layer(node)
            && self.popover_kind(node) != before
        {
            self.touch_top_layer(None);
            self.doc.remove_from_top_layer(node);
        }
    }

    fn popover_event(&self, kind: EventKind, popover: NodeId) -> Event {
        self.event(kind, popover, (0.0, 0.0), Modifiers::default())
    }
}
