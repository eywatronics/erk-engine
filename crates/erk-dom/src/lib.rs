//! Erk Engine DOM: an arena of nodes addressed by generational ids, built
//! from HTML by html5ever.
//!
//! Nodes never point at each other; they hold [`NodeId`]s into the owning
//! [`Document`]'s arena. See docs/design/p0-architecture.md §5.

mod arena;
mod document;
mod node;
mod sink;

pub use arena::{Arena, NodeId};
pub use document::{Children, Document};
pub use node::{Attribute, ElementData, Node, NodeData};

// Re-exported so consumers name elements with the same atom types this crate
// stores; a second copy of the atom crates would not compare equal.
pub use html5ever::tree_builder::QuirksMode;
pub use html5ever::{LocalName, Namespace, Prefix, QualName, local_name, ns};
