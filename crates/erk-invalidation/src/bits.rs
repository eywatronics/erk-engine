//! The dirty bits and how they cross an edge (p2-incremental §3.3, §3.11).

use std::ops::{BitOr, BitOrAssign, Sub};

use erk_dom::{Document, NodeId};

use crate::side::SideTable;

/// What a node needs done again, and in which direction: one vocabulary for
/// every stage. A set of bits; the empty set is "clean".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Invalidation(u16);

impl Invalidation {
    pub const NONE: Self = Self(0);
    /// Its style, seeded from Stylo's restyle hint.
    pub const STYLE_SELF: Self = Self(1 << 0);
    pub const STYLE_SUBTREE: Self = Self(1 << 1);
    /// Its paragraph must be shaped again: the text or the font changed.
    pub const TEXT_SHAPE: Self = Self(1 << 2);
    /// Its layout; lines are broken again, text is not reshaped.
    pub const LAYOUT_SELF: Self = Self(1 << 3);
    /// Its size may leak to its parent.
    pub const LAYOUT_ANCESTOR: Self = Self(1 << 4);
    pub const PAINT_SELF: Self = Self(1 << 5);
    pub const PAINT_SUBTREE: Self = Self(1 << 6);
    pub const A11Y_SELF: Self = Self(1 << 7);
    pub const A11Y_SUBTREE: Self = Self(1 << 8);
    pub const HIT_TEST: Self = Self(1 << 9);
    /// Every bit there is.
    pub const ALL: Self = Self((1 << 10) - 1);

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn from_bits(bits: u16) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every bit of `other` is set here.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any bit of `other` is set here.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Invalidation {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Invalidation {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// The bits of `self` not in `other`.
impl Sub for Invalidation {
    type Output = Self;
    fn sub(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

/// How dirt crosses an edge from a child to its parent: what the parent
/// does not take (`absorb`), and what it takes on besides (`promote`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub absorb: Invalidation,
    pub promote: Invalidation,
}

/// What reaches across an edge with rule `rule` from a child dirty with
/// `dirt` (§3.11): nothing from nothing, else what is not absorbed and what
/// is promoted. No clamp: a boundary's own `LAYOUT_SELF` is its seed's
/// work, not the walk's.
pub fn propagate(dirt: Invalidation, rule: Rule) -> Invalidation {
    if dirt.is_empty() {
        return Invalidation::NONE;
    }
    (dirt - rule.absorb) | rule.promote
}

/// Carry `dirt` up from `node` through its ancestors, each edge by the
/// rule `rule_of` gives for the parent, marking each in `dirty`. The walk
/// stops when nothing gets across or the ancestor already has all of it
/// (convergence: at most as far as the nearest absorbing boundary).
/// Returns how many ancestors it marked.
pub fn propagate_up(
    doc: &Document,
    dirty: &mut SideTable<Invalidation>,
    node: NodeId,
    dirt: Invalidation,
    mut rule_of: impl FnMut(NodeId) -> Rule,
) -> usize {
    let mut carried = dirt;
    let mut at = node;
    let mut marked = 0;
    while let Some(parent) = doc.node(at).and_then(|n| n.parent()) {
        carried = propagate(carried, rule_of(parent));
        // Nothing got across, or the ancestor has all of it already: either
        // way the walk is over (every set contains the empty one).
        let had = dirty.get(parent).copied().unwrap_or_default();
        if had.contains(carried) {
            break;
        }
        dirty.insert(parent, had | carried);
        marked += 1;
        at = parent;
    }
    marked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator: every run sees the same cases.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }
        fn bits(&mut self) -> Invalidation {
            Invalidation::from_bits(self.next() as u16)
        }
        fn rule(&mut self) -> Rule {
            Rule {
                absorb: self.bits(),
                promote: self.bits(),
            }
        }
    }

    const CASES: usize = 10_000;

    fn subset(a: Invalidation, b: Invalidation) -> bool {
        b.contains(a)
    }

    #[test]
    fn nothing_spreads_from_nothing() {
        let mut rng = Rng(1);
        for _ in 0..CASES {
            assert_eq!(
                propagate(Invalidation::NONE, rng.rule()),
                Invalidation::NONE
            );
        }
    }

    #[test]
    fn spreading_twice_is_spreading_once() {
        let mut rng = Rng(2);
        for _ in 0..CASES {
            let (dirt, rule) = (rng.bits(), rng.rule());
            let once = propagate(dirt, rule);
            assert_eq!(propagate(once, rule), once, "{dirt:?} {rule:?}");
        }
    }

    #[test]
    fn more_dirt_spreads_at_least_as_far() {
        let mut rng = Rng(3);
        for _ in 0..CASES {
            let (a, extra, rule) = (rng.bits(), rng.bits(), rng.rule());
            let b = a | extra;
            assert!(
                subset(propagate(a, rule), propagate(b, rule)),
                "{a:?} ⊆ {b:?} under {rule:?}"
            );
        }
    }

    #[test]
    fn spreading_two_changes_together_is_spreading_each() {
        let mut rng = Rng(4);
        for _ in 0..CASES {
            let (a, b, rule) = (rng.bits(), rng.bits(), rng.rule());
            if a.is_empty() || b.is_empty() {
                // Distribution holds for non-empty sets; with an empty one
                // both sides are the other's spread.
                assert_eq!(
                    propagate(a | b, rule),
                    propagate(a, rule) | propagate(b, rule)
                );
                continue;
            }
            assert_eq!(
                propagate(a | b, rule),
                propagate(a, rule) | propagate(b, rule),
                "{a:?} {b:?} {rule:?}"
            );
        }
    }

    /// A chain of `depth` nested divs, the innermost last.
    fn chain(depth: usize) -> (Document, Vec<NodeId>) {
        let html = format!("{}{}", "<div>".repeat(depth), "</div>".repeat(depth));
        let doc = Document::parse_html(&html);
        let mut nodes = Vec::new();
        let mut at = doc.root();
        while let Some(child) = doc.children(at).last() {
            nodes.push(child);
            at = child;
        }
        (doc, nodes)
    }

    #[test]
    fn a_walk_stops_at_the_nearest_boundary_and_where_the_dirt_already_is() {
        let (doc, nodes) = chain(50);
        let leaf = *nodes.last().unwrap();
        let boundary = nodes[nodes.len() - 11];
        let layout = Invalidation::LAYOUT_SELF | Invalidation::LAYOUT_ANCESTOR;
        // Each ancestor takes the size change on; the boundary absorbs it.
        let rule_of = |parent: NodeId| Rule {
            absorb: if parent == boundary {
                Invalidation::LAYOUT_ANCESTOR | Invalidation::LAYOUT_SELF
            } else {
                Invalidation::NONE
            },
            promote: if parent == boundary {
                Invalidation::NONE
            } else {
                Invalidation::LAYOUT_SELF
            },
        };
        let mut dirty = SideTable::default();
        let marked = propagate_up(&doc, &mut dirty, leaf, layout, rule_of);
        // Nine ancestors between the leaf and the boundary, and the walk
        // ends there: O(d), not O(depth).
        assert_eq!(marked, 9);
        assert!(dirty.get(boundary).is_none());
        assert!(dirty.get(nodes[0]).is_none());
        // Again: the ancestors have it all, the walk stops at once.
        assert_eq!(propagate_up(&doc, &mut dirty, leaf, layout, rule_of), 0);
    }
}
