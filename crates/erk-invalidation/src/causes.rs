//! Why a node became dirty (p2-incremental §3.3): each seed and each step
//! of a walk as `(order, node, bits, cause)`, in a ring buffer, so that the
//! chain can be read backwards:
//!
//! ```text
//! node 742: STYLE + LAYOUT
//!   ← ChildLayout(743)
//!     ← Text (node 743)
//! ```
//!
//! Kept with the `inspect` feature (and in tests); without it [`Causes`]
//! keeps nothing and costs nothing.

use erk_dom::NodeId;

use crate::Invalidation;

/// What made a node dirty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Text,
    Attribute,
    Class,
    InlineStyle,
    /// Hover, focus, active and the like.
    State,
    /// A child's layout changed: the child.
    ChildLayout(NodeId),
    ParentLayout,
    /// A resource arrived (an image, a font).
    Resource,
    Viewport,
}

/// One step: in which order it was recorded, the node, the bits it got
/// and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub order: u64,
    pub node: NodeId,
    pub bits: Invalidation,
    pub cause: Cause,
}

/// The last records, oldest overwritten first.
#[derive(Debug)]
pub struct Causes {
    #[cfg(any(test, feature = "inspect"))]
    records: Vec<Record>,
    #[cfg(any(test, feature = "inspect"))]
    capacity: usize,
    #[cfg(any(test, feature = "inspect"))]
    next: u64,
}

impl Causes {
    /// A buffer keeping the last `capacity` records (with `inspect`).
    pub fn new(capacity: usize) -> Self {
        let _ = capacity;
        Self {
            #[cfg(any(test, feature = "inspect"))]
            records: Vec::with_capacity(capacity.min(4096)),
            #[cfg(any(test, feature = "inspect"))]
            capacity: capacity.max(1),
            #[cfg(any(test, feature = "inspect"))]
            next: 0,
        }
    }

    /// Note that `node` got `bits` because of `cause`.
    #[inline]
    pub fn record(&mut self, node: NodeId, bits: Invalidation, cause: Cause) {
        #[cfg(any(test, feature = "inspect"))]
        {
            let record = Record {
                order: self.next,
                node,
                bits,
                cause,
            };
            let at = (self.next % self.capacity as u64) as usize;
            if at < self.records.len() {
                self.records[at] = record;
            } else {
                self.records.push(record);
            }
            self.next += 1;
        }
        #[cfg(not(any(test, feature = "inspect")))]
        let _ = (node, bits, cause);
    }

    /// Why `node` is dirty: its latest record, then the record of the
    /// child it came from, and so on down to the seed. Empty without
    /// `inspect`, or when the records have been overwritten.
    pub fn chain(&self, node: NodeId) -> Vec<Record> {
        #[cfg(any(test, feature = "inspect"))]
        {
            let mut chain = Vec::new();
            let mut looking = Some((node, u64::MAX));
            while let Some((node, before)) = looking.take() {
                let Some(record) = self
                    .records
                    .iter()
                    .filter(|record| record.node == node && record.order < before)
                    .max_by_key(|record| record.order)
                else {
                    break;
                };
                chain.push(*record);
                if let Cause::ChildLayout(child) = record.cause {
                    looking = Some((child, record.order));
                }
            }
            chain
        }
        #[cfg(not(any(test, feature = "inspect")))]
        {
            let _ = node;
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use erk_dom::Document;

    #[test]
    fn the_chain_reads_back_from_a_node_to_its_seed() {
        let doc = Document::parse_html("<div><p>a</p></div>");
        let ids: Vec<NodeId> = {
            let mut ids = Vec::new();
            let mut stack = vec![doc.root()];
            while let Some(id) = stack.pop() {
                ids.push(id);
                stack.extend(doc.children(id));
            }
            ids
        };
        let (a, b, c) = (ids[1], ids[2], ids[3]);
        let mut causes = Causes::new(16);
        causes.record(c, Invalidation::TEXT_SHAPE, Cause::Text);
        causes.record(b, Invalidation::LAYOUT_SELF, Cause::ChildLayout(c));
        causes.record(a, Invalidation::LAYOUT_SELF, Cause::ChildLayout(b));
        let chain: Vec<(NodeId, Cause)> =
            causes.chain(a).iter().map(|r| (r.node, r.cause)).collect();
        assert_eq!(
            chain,
            [
                (a, Cause::ChildLayout(b)),
                (b, Cause::ChildLayout(c)),
                (c, Cause::Text)
            ]
        );
    }

    #[test]
    fn the_oldest_records_go_first() {
        let doc = Document::parse_html("");
        let root = doc.root();
        let mut causes = Causes::new(2);
        causes.record(root, Invalidation::PAINT_SELF, Cause::Class);
        causes.record(root, Invalidation::PAINT_SELF, Cause::State);
        causes.record(root, Invalidation::PAINT_SELF, Cause::Viewport);
        let kept: Vec<Cause> = causes.records.iter().map(|r| r.cause).collect();
        assert_eq!(kept, [Cause::Viewport, Cause::State]);
        assert_eq!(causes.chain(root)[0].cause, Cause::Viewport);
    }
}
