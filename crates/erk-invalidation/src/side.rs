//! Data per node in a flat vector (p2-incremental §3.12): indexed by
//! `NodeId::index()`, each entry keeping the generation of the node it is
//! for. A slot freed and reused by another node does not hand that node
//! the old one's data: the generation tells them apart.

use erk_dom::NodeId;

pub struct SideTable<T> {
    entries: Vec<Option<(NodeId, T)>>,
    /// The slots in use, to clear only those.
    used: Vec<usize>,
}

impl<T> Default for SideTable<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            used: Vec::new(),
        }
    }
}

impl<T> SideTable<T> {
    /// `id`'s entry, if it has one.
    pub fn get(&self, id: NodeId) -> Option<&T> {
        match self.entries.get(id.index() as usize)? {
            Some((owner, value)) if *owner == id => Some(value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut T> {
        match self.entries.get_mut(id.index() as usize)? {
            Some((owner, value)) if *owner == id => Some(value),
            _ => None,
        }
    }

    /// Set `id`'s entry, replacing its own or a dead node's in the slot.
    pub fn insert(&mut self, id: NodeId, value: T) {
        let at = id.index() as usize;
        if self.entries.len() <= at {
            self.entries.resize_with(at + 1, || None);
        }
        if self.entries[at].is_none() {
            self.used.push(at);
        }
        self.entries[at] = Some((id, value));
    }

    /// Take `id`'s entry out.
    pub fn remove(&mut self, id: NodeId) -> Option<T> {
        let entry = self.entries.get_mut(id.index() as usize)?;
        match entry {
            Some((owner, _)) if *owner == id => entry.take().map(|(_, value)| value),
            _ => None,
        }
    }

    /// Every entry, in the order the slots were first used.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &T)> {
        self.used
            .iter()
            .filter_map(|at| self.entries[*at].as_ref().map(|(id, value)| (*id, value)))
    }

    /// Empty the table, touching only the slots in use.
    pub fn clear(&mut self) {
        for at in self.used.drain(..) {
            self.entries[at] = None;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erk_dom::Document;

    #[test]
    fn a_reused_slot_does_not_hand_over_the_old_nodes_entry() {
        let mut doc = Document::parse_html("<p>a</p>");
        let old = doc.create_element("b").unwrap();
        let mut table = SideTable::default();
        table.insert(old, 7);
        assert_eq!(table.get(old), Some(&7));
        doc.remove(old);
        let new = doc.create_element("i").unwrap();
        assert_eq!(new.index(), old.index(), "the slot is reused");
        assert_eq!(table.get(new), None);
        assert_eq!(table.remove(new), None);
        table.insert(new, 8);
        assert_eq!(table.get(new), Some(&8));
        assert_eq!(table.get(old), None);
        table.clear();
        assert!(table.is_empty());
    }
}
