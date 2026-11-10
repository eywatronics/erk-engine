//! Commands (HTML §4.10.6, M5.9): a `button` whose `commandfor` names an
//! element runs its `command` on it when pressed. The built-in commands
//! open and close popovers and dialogs, so a menu or a confirmation works
//! without the host; a custom one (`--name`) is for the host alone.
//!
//! The press sends a cancelable `command` event to the element first,
//! carrying the command and the button; the command runs once the host
//! has dispatched it, unless prevented. The event waits in the popovers'
//! plan (popover.rs), and the popover a command opens or closes asks with
//! its own `beforetoggle` from there.

use erk_dom::{ElementData, NodeId, local_name};

use super::Page;
use super::popover::Step;
use crate::messages::Event;

/// A built-in command (HTML's `command` attribute states).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    TogglePopover,
    ShowPopover,
    HidePopover,
    ShowModal,
    Close,
    RequestClose,
}

impl Command {
    /// The built-in commands, by the keywords that name them.
    const ALL: [(&str, Command); 6] = [
        ("toggle-popover", Command::TogglePopover),
        ("show-popover", Command::ShowPopover),
        ("hide-popover", Command::HidePopover),
        ("show-modal", Command::ShowModal),
        ("close", Command::Close),
        ("request-close", Command::RequestClose),
    ];

    fn is_popover_command(self) -> bool {
        matches!(
            self,
            Command::TogglePopover | Command::ShowPopover | Command::HidePopover
        )
    }
}

/// What a `command` attribute asks: a built-in command with its keyword,
/// or a custom one (`None`) as written. `None` for a missing one or one
/// HTML does not know, which does nothing.
fn parse(value: Option<&str>) -> Option<(String, Option<Command>)> {
    let value = value?;
    if value.starts_with("--") {
        return Some((value.to_owned(), None));
    }
    Command::ALL
        .iter()
        .find(|(keyword, _)| value.eq_ignore_ascii_case(keyword))
        .map(|(keyword, command)| ((*keyword).to_owned(), Some(*command)))
}

/// Whether a `button` is a submit button: `type=submit`, or a type it
/// does not know or none, when it has no `commandfor` (HTML's Auto state).
pub(crate) fn submits(button: &ElementData) -> bool {
    match button.attr(&local_name!("type")) {
        Some(kind) if kind.eq_ignore_ascii_case("submit") => true,
        Some(kind) if kind.eq_ignore_ascii_case("button") || kind.eq_ignore_ascii_case("reset") => {
            false
        }
        _ => button.attr(&local_name!("commandfor")).is_none(),
    }
}

impl Page {
    /// The element `button`'s `commandfor` names: a `button` that is not
    /// disabled, nor a submit or reset button of a form, whose
    /// `commandfor` is the id of an element in the document.
    pub(crate) fn commandfor_target(&self, button: NodeId) -> Option<NodeId> {
        let element = self.doc.node(button)?.as_element()?;
        if element.name.ns != erk_dom::ns!(html)
            || element.name.local != local_name!("button")
            || element.attr(&local_name!("disabled")).is_some()
        {
            return None;
        }
        let resets = element
            .attr(&local_name!("type"))
            .is_some_and(|kind| kind.eq_ignore_ascii_case("reset"));
        if (submits(element) || resets) && self.in_form(button) {
            return None;
        }
        self.element_with_id(element.attr(&local_name!("commandfor"))?)
    }

    /// The popover `button`'s popover command opens or closes, for light
    /// dismiss: a press on its button counts as one in it.
    pub(crate) fn command_popover(&self, button: NodeId) -> Option<NodeId> {
        let target = self.commandfor_target(button)?;
        let element = self.doc.node(button)?.as_element()?;
        let (_, command) = parse(element.attr(&local_name!("command")))?;
        (command?.is_popover_command() && self.popover_kind(target).is_some()).then_some(target)
    }

    /// A press on `button` (a click it was not prevented from doing): when
    /// its `commandfor` names an element, its `command` asks to run there.
    /// `None` when it names none; the events that follow otherwise, none
    /// for a command HTML does not know or one the element does not take.
    pub(crate) fn invoke_command(&mut self, button: NodeId) -> Option<Vec<Event>> {
        let target = self.commandfor_target(button)?;
        let element = self.doc.node(button)?.as_element()?;
        let Some((name, command)) = parse(element.attr(&local_name!("command"))) else {
            return Some(Vec::new());
        };
        // A built-in command at an element without `popover` only where
        // the element takes it: a dialog's three.
        if let Some(command) = command
            && self.popover_kind(target).is_none()
            && (command.is_popover_command() || !self.is_dialog(target))
        {
            return Some(Vec::new());
        }
        self.plan_push(Step::Command {
            target,
            source: button,
            name,
            command,
        });
        Some(self.run_popovers())
    }

    /// Run built-in `command` at `target`, sent by `source`, once the host
    /// let its `command` event through: the steps it takes go in the plan,
    /// or the events it makes come back.
    pub(crate) fn run_command(
        &mut self,
        target: NodeId,
        source: NodeId,
        command: Command,
    ) -> Vec<Event> {
        let open_popover = self.is_open_popover(target);
        match command {
            Command::HidePopover | Command::TogglePopover if open_popover => {
                self.plan_hide(target, true);
            }
            Command::ShowPopover | Command::TogglePopover
                if !open_popover && self.popover_kind(target).is_some() =>
            {
                self.plan_push(Step::Ask {
                    popover: target,
                    invoker: Some(source),
                });
            }
            // A dialog's commands, but not while it shows as a popover.
            _ if !self.is_dialog(target) || open_popover => {}
            // The `auto` popovers that are not its ancestors close first,
            // each telling with its `beforetoggle`.
            Command::ShowModal if !self.has_open(target) => {
                let ancestor = self.popover_ancestor(target);
                for popover in self.autos_above(ancestor).into_iter().rev() {
                    self.plan_push(Step::Hide {
                        popover,
                        focus_back: false,
                    });
                }
                self.plan_push(Step::ShowModal(target));
            }
            Command::Close => {
                return self.close_dialog_now(target, button_value(self, source).as_deref(), true);
            }
            Command::RequestClose if self.has_open(target) => {
                let value = button_value(self, source);
                self.plan_push(Step::Cancel {
                    dialog: target,
                    value,
                });
            }
            _ => {}
        }
        Vec::new()
    }
}

/// A button's optional value: its `value` attribute, if it has one.
fn button_value(page: &Page, button: NodeId) -> Option<String> {
    page.doc
        .node(button)?
        .as_element()?
        .attr(&local_name!("value"))
        .map(str::to_owned)
}
