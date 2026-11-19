//! A drop-down select's keys (M5.10).
//!
//! A focused drop-down select changes its selection with the keys, as
//! Chrome's does on Windows without opening its list: the arrows go to the
//! next or previous option that is not disabled, Home and End to the first
//! and last. A change tells `input` and `change` at once; a key that
//! changes nothing tells nothing. Alt with an arrow, or Space, opens its
//! list (`picker.rs`).

use erk_dom::NodeId;

use super::Page;
use crate::messages::{Event, EventKind, Key, KeyInput, KeyState, Modifiers};

impl Page {
    /// What key `input` does at `select`, the focused element: `None` if it
    /// is not an enabled drop-down select or the key is not one of its;
    /// otherwise the events of the selection it changed, none if it
    /// changed nothing.
    pub(crate) fn select_key(&mut self, select: NodeId, input: &KeyInput) -> Option<Vec<Event>> {
        if input.state != KeyState::Down
            || !crate::layout::is_dropdown(&self.doc, select)
            || self.doc.disabled(select) == Some(true)
        {
            return None;
        }
        // Alt with an arrow, or Space, opens the list.
        let opens = match input.key {
            Key::ArrowDown | Key::ArrowUp => input.modifiers.alt,
            Key::Space => true,
            _ => false,
        };
        if opens {
            return self.invoke_select(select);
        }
        let options = self.doc.options(select);
        let enabled = |option: &&NodeId| self.doc.disabled(**option) != Some(true);
        let current = self
            .doc
            .selected_options(select)
            .first()
            .and_then(|selected| options.iter().position(|option| option == selected));
        let target = match input.key {
            Key::ArrowDown | Key::ArrowRight => match current {
                Some(at) => options[at + 1..].iter().find(enabled),
                None => options.iter().find(enabled),
            },
            Key::ArrowUp | Key::ArrowLeft => {
                current.and_then(|at| options[..at].iter().rev().find(enabled))
            }
            Key::Home => options.iter().find(enabled),
            Key::End => options.iter().rev().find(enabled),
            _ => return None,
        }
        .copied();
        let Some(target) = target.filter(|target| Some(*target) != current.map(|at| options[at]))
        else {
            return Some(Vec::new());
        };
        let _ = self.set_checked(target.to_bits(), true);
        self.edited = true;
        Some(
            [EventKind::Input, EventKind::Change]
                .into_iter()
                .map(|kind| self.event(kind, select, (0.0, 0.0), Modifiers::default()))
                .collect(),
        )
    }
}
