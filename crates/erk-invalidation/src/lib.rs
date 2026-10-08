//! What changed since the last frame, and what it makes dirty (M5.2,
//! p2-incremental §3.3, §3.11, §3.12).
//!
//! - [`journal`]: the document's changes since the last frame, coalesced
//!   (M5.1).
//! - [`Invalidation`]: one vocabulary of dirty bits for style, text,
//!   layout, paint and accessibility, the direction inside the bits.
//! - [`Rule`] and [`propagate`]: how dirt crosses an edge of the tree, an
//!   algebra its properties are tested for.
//! - [`SideTable`]: data per node, indexed by `NodeId::index()` with the
//!   generation checked; no map keyed by node.
//! - [`Causes`]: why each node became dirty, a ring buffer behind the
//!   `inspect` feature.
//!
//! The crate depends on `erk-dom` alone (checked in CI): the style, layout
//! and display list stages consume it, it knows none of them.

mod bits;
mod causes;
pub mod journal;
mod side;

pub use bits::{Invalidation, Rule, propagate, propagate_up};
pub use causes::{Cause, Causes, Record};
pub use side::SideTable;
