//! The transitions running in a document, on the host's clock (M5.12).
//!
//! Stylo's Servo mode does the transitions themselves: when an element's
//! style changes it starts, turns around or cancels them, and computes
//! their value at a time with their timing function. What Servo leaves to
//! its script thread, Erk does here, once a frame before styling: a
//! transition whose end has come is marked finished, and every element
//! with a transition is styled again, so its value follows the clock.

use std::collections::HashSet;

use style::animation::{AnimationState, DocumentAnimationSet, ElementAnimationSet};

/// The transitions of a document, and the time on the host's clock their
/// values are taken at.
#[derive(Default)]
pub struct Transitions {
    /// Shared with Stylo for a traversal; the oracle gets a copy.
    pub(crate) set: DocumentAnimationSet,
    /// Seconds, as Stylo counts.
    pub(crate) now: f64,
}

impl Clone for Transitions {
    /// A copy that shares nothing: styling with it leaves the original as
    /// it was.
    fn clone(&self) -> Self {
        let copy = DocumentAnimationSet::default();
        {
            let mut sets = copy.sets.write();
            for (key, set) in self.set.sets.read().iter() {
                sets.insert(
                    key.clone(),
                    ElementAnimationSet {
                        animations: set.animations.clone(),
                        transitions: set.transitions.clone(),
                        dirty: set.dirty,
                    },
                );
            }
        }
        Self {
            set: copy,
            now: self.now,
        }
    }
}

impl Transitions {
    /// The host's clock is at `now_ns`, in nanoseconds.
    pub(crate) fn set_time(&mut self, now_ns: u64) {
        self.now = now_ns as f64 / 1e9;
    }

    /// Mark the transitions whose end has come as finished and drop the
    /// cancelled ones: styling an element then takes its value from the
    /// clock, or its own once none runs. The arena slots of the elements
    /// that still have transitions.
    pub(crate) fn tick(&self) -> Vec<usize> {
        let mut sets = self.set.sets.write();
        sets.retain(|_, set| {
            for transition in &mut set.transitions {
                let moving = matches!(
                    transition.state,
                    AnimationState::Pending | AnimationState::Running
                );
                if moving && transition.has_ended(self.now) {
                    transition.state = AnimationState::Finished;
                }
            }
            set.clear_canceled_animations();
            !set.is_empty()
        });
        let slots: HashSet<usize> = sets.keys().map(|key| key.node.0).collect();
        slots.into_iter().collect()
    }

    /// Drop the transitions of the elements in arena slots `gone` says are
    /// gone: removed from the document, given to another node, or no
    /// longer styled. `OpaqueNode` is the arena slot, and the next node in
    /// it must not take them over.
    pub(crate) fn forget(&self, gone: impl Fn(usize) -> bool) {
        self.set.sets.write().retain(|key, _| !gone(key.node.0));
    }

    /// Forget every transition: a new document.
    pub(crate) fn clear(&self) {
        self.set.sets.write().clear();
    }

    /// Whether a transition is still to run or to end: a frame is due at
    /// the next tick.
    pub(crate) fn running(&self) -> bool {
        self.set
            .sets
            .read()
            .values()
            .any(ElementAnimationSet::needs_animation_ticks)
    }
}
