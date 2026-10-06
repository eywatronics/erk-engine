//! Events and subscriptions (p1-contract §5): the host subscribes to kinds
//! of events on nodes; an event goes through DOM's capture, target and
//! bubble phases along the path from the root element to its target.

use std::collections::BTreeMap;

use crate::Context;
use crate::ids::Node;

pub use erk_renderer::{EventKind, Modifiers};

/// Where an event is on its way when a callback gets it; the numbers are
/// `erk.h`'s `ERK_PHASE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Phase {
    /// On an ancestor of the target, on the way down from the root.
    Capture = 1,
    /// On the target itself.
    Target = 2,
    /// On an ancestor of the target, on the way back up.
    Bubble = 3,
}

/// What happened, as a callback gets it. Valid only during the callback.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: EventKind,
    pub phase: Phase,
    /// The node the event happened to.
    pub target: Node,
    /// The node whose subscription is being called.
    pub current_target: Node,
    /// Where the pointer was, in CSS pixels, for pointer events.
    pub x: f32,
    pub y: f32,
    pub modifiers: Modifiers,
}

/// A subscription, to cancel with [`Context::off`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Subscription(pub(crate) u64);

impl Subscription {
    /// The subscription as a number, for a host that keeps it outside Rust
    /// (the C ABI's `ErkSubscription`); never 0.
    pub fn to_raw(self) -> u64 {
        self.0
    }

    /// A subscription from [`Subscription::to_raw`]'s number.
    pub fn from_raw(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }
}

pub(crate) type Callback = Box<dyn FnMut(&mut Context, &Event)>;

pub(crate) struct Listener {
    /// The engine's id of the node subscribed to.
    pub(crate) node: u64,
    pub(crate) kind: EventKind,
    /// Called in the capture phase (and at the target) rather than the
    /// bubble phase (and at the target).
    pub(crate) capture: bool,
    /// Taken out while it runs: `None` then.
    pub(crate) callback: Option<Callback>,
}

/// The subscriptions, in the order they were made.
#[derive(Default)]
pub(crate) struct Listeners {
    by_id: BTreeMap<Subscription, Listener>,
    next: u64,
}

impl Listeners {
    pub(crate) fn add(&mut self, listener: Listener) -> Subscription {
        self.next += 1;
        let id = Subscription(self.next);
        self.by_id.insert(id, listener);
        id
    }

    /// Cancel `id`. Its callback is dropped here, or, if it is running, when
    /// it returns (p1-contract §5: `destroy` after the callback).
    pub(crate) fn remove(&mut self, id: Subscription) -> Option<Listener> {
        self.by_id.remove(&id)
    }

    /// Cancel every subscription whose node is gone.
    pub(crate) fn retain_nodes(&mut self, mut alive: impl FnMut(u64) -> bool) {
        self.by_id.retain(|_, listener| alive(listener.node));
    }

    /// The subscriptions called at `node` for `kind` in one pass, in the
    /// order they were made: a snapshot, as DOM takes one per node.
    pub(crate) fn at(&self, node: u64, kind: EventKind, capture: bool) -> Vec<Subscription> {
        self.by_id
            .iter()
            .filter(|(_, listener)| {
                listener.node == node && listener.kind == kind && listener.capture == capture
            })
            .map(|(id, _)| *id)
            .collect()
    }

    pub(crate) fn take(&mut self, id: Subscription) -> Option<Callback> {
        self.by_id.get_mut(&id)?.callback.take()
    }

    /// Put `callback` back after it ran, if its subscription still stands;
    /// else it is dropped here.
    pub(crate) fn put_back(&mut self, id: Subscription, callback: Callback) {
        if let Some(listener) = self.by_id.get_mut(&id) {
            listener.callback = Some(callback);
        }
    }
}

/// Whether `kind` goes back up the path after its target. Focus and blur
/// do not bubble (UI Events §5.2.2); a click does.
pub(crate) fn bubbles(kind: EventKind) -> bool {
    matches!(kind, EventKind::Click)
}
