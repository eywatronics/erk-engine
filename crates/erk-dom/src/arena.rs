use std::num::NonZeroU32;

/// A handle to a value in an [`Arena`].
///
/// The generation makes a handle to a removed value stay invalid even after
/// its slot is reused, instead of silently pointing at the new occupant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId {
    index: u32,
    generation: NonZeroU32,
}

impl NodeId {
    /// The slot index, for side tables that store per-node data outside the
    /// arena. It is only unique among live nodes; pair it with the id itself
    /// when staleness matters.
    pub fn index(self) -> u32 {
        self.index
    }
}

struct Slot<T> {
    generation: NonZeroU32,
    value: Option<T>,
}

/// Contiguous storage addressed by [`NodeId`].
pub struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Arena<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, value: T) -> NodeId {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return NodeId {
                index,
                generation: slot.generation,
            };
        }
        let index = u32::try_from(self.slots.len()).expect("arena holds at most u32::MAX values");
        let generation = NonZeroU32::MIN;
        self.slots.push(Slot {
            generation,
            value: Some(value),
        });
        NodeId { index, generation }
    }

    pub fn remove(&mut self, id: NodeId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        // A wrapped generation would make a stale NodeId valid again, so an
        // exhausted slot is retired instead of reused.
        if let Some(next) = slot.generation.checked_add(1) {
            slot.generation = next;
            self.free.push(id.index);
        }
        Some(value)
    }

    pub fn get(&self, id: NodeId) -> Option<&T> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_ref()
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.value.as_mut()
    }

    /// Number of slots ever allocated, live or not. Side tables indexed by
    /// [`NodeId::index`] need this many entries.
    pub fn capacity_hint(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserted_value_can_be_read_back() {
        let mut arena = Arena::new();
        let id = arena.insert("a");
        assert_eq!(arena.get(id), Some(&"a"));
    }

    #[test]
    fn removed_value_is_gone() {
        let mut arena = Arena::new();
        let id = arena.insert("a");
        assert_eq!(arena.remove(id), Some("a"));
        assert_eq!(arena.get(id), None);
        assert_eq!(arena.remove(id), None);
    }

    #[test]
    fn reused_slot_gets_a_new_generation() {
        let mut arena = Arena::new();
        let old = arena.insert("a");
        arena.remove(old);
        let new = arena.insert("b");

        assert_eq!(new.index(), old.index());
        assert_ne!(new, old);
        assert_eq!(
            arena.get(old),
            None,
            "a stale id must not see the new occupant"
        );
        assert_eq!(arena.get(new), Some(&"b"));
    }

    #[test]
    fn exhausted_slot_is_retired() {
        let mut arena = Arena::new();
        let id = arena.insert("a");
        arena.slots[id.index as usize].generation = NonZeroU32::MAX;
        let id = NodeId {
            index: id.index,
            generation: NonZeroU32::MAX,
        };

        arena.remove(id);
        let next = arena.insert("b");

        assert_ne!(
            next.index(),
            id.index(),
            "a slot at u32::MAX must not be reused"
        );
        assert_eq!(arena.get(id), None);
    }

    #[test]
    fn optional_node_id_costs_no_extra_space() {
        assert_eq!(size_of::<NodeId>(), 8);
        assert_eq!(size_of::<Option<NodeId>>(), 8);
    }
}
