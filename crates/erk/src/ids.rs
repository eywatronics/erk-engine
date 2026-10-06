//! Node ids as the host sees them (p1-contract §2): the engine's id, its
//! index half mixed with a key of the app's own. An id from another app, or
//! from an app since destroyed, then almost always names a slot that does
//! not exist or holds another generation, and comes back as a stale node
//! instead of quietly meaning some other node.
//!
//! This is namespacing, not security: the key is not secret, it follows
//! from the app's serial number, so tests are deterministic.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

/// The serial number of the next app in this process; never repeats.
static NEXT_APP: AtomicU64 = AtomicU64::new(1);

/// SplitMix64's output function: consecutive serials give unrelated keys.
fn splitmix64(serial: u64) -> u64 {
    let mut z = serial.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The key of a new app. It mixes only the index half: the upper half of an
/// id is the node's generation, never 0, so no mixed id is 0 either.
pub(crate) fn new_app_key() -> u64 {
    key_for(NEXT_APP.fetch_add(1, Ordering::Relaxed))
}

fn key_for(serial: u64) -> u64 {
    splitmix64(serial) & 0xFFFF_FFFF
}

/// A node of an app's document, as the host holds it: opaque, never 0, and
/// meaningful only to the app that gave it. Keep it and give it back; do not
/// read its bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Node(NonZeroU64);

impl Node {
    /// The id as a number, for a host that keeps ids outside Rust (the C
    /// ABI's `ErkNodeId`).
    pub fn to_raw(self) -> u64 {
        self.0.get()
    }

    /// A node from [`Node::to_raw`]'s number; 0 is no node.
    pub fn from_raw(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }
}

/// The engine's id of `node` in the app with `key`.
pub(crate) fn inward(node: Node, key: u64) -> u64 {
    node.0.get() ^ key
}

/// The host's id of the engine's `id` in the app with `key`. Every id the
/// engine gives has a generation in its upper half; one without is not an
/// id and gives no node.
pub(crate) fn outward(id: u64, key: u64) -> Option<Node> {
    if id >> 32 == 0 {
        return None;
    }
    Node::from_raw(id ^ key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mixed_id_is_zero_and_every_one_comes_back() {
        // Ids of every shape (index anywhere, generation from 1 up) under
        // the keys of many apps.
        for serial in 0..256 {
            let key = key_for(serial);
            assert_eq!(key >> 32, 0, "the key leaves the generation alone");
            let mut x = splitmix64(serial ^ 0xe7c3);
            for _ in 0..256 {
                x = splitmix64(x);
                let index = x & 0xFFFF_FFFF;
                let generation = (x >> 32).max(1);
                let id = index | (generation << 32);
                let node = outward(id, key).expect("an id with a generation");
                assert_ne!(node.to_raw(), 0);
                assert_eq!(inward(node, key), id);
            }
        }
        assert_eq!(outward(5, key_for(1)), None, "no generation, no id");
    }

    #[test]
    fn apps_get_different_keys() {
        let keys: std::collections::HashSet<u64> = (1..1000).map(key_for).collect();
        assert_eq!(keys.len(), 999);
    }
}
