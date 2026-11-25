//! A box's layout cache: what Taffy's [`taffy::Cache`] answers, in less
//! room (M5.13).
//!
//! Taffy's cache holds its final layout and nine measurements in every
//! box, 384 bytes, most of them empty: on B16's page of 10 000 cards that
//! was 11.5 MB of 30 000 boxes. This one holds its final layout and as
//! many measurements as were written. Which input a stored result answers,
//! and which measurement a new one replaces, are Taffy 0.14's (its
//! `tree/cache.rs`); Taffy keeps its key private, so it is made here from
//! the input's public fields. The test below holds the two to the same
//! answers: an upgrade of Taffy that changes them breaks it.

use taffy::{
    AvailableSpace, CollapsibleMarginSet, LayoutInput, LayoutOutput, RequestedAxis, RunMode, Size,
};

/// How many measurements a box keeps: Taffy's.
const MEASURES: u8 = 9;

/// `f32::INFINITY`, `f32::NEG_INFINITY` as bits.
const INFINITY: u32 = f32::INFINITY.to_bits();
const NEG_INFINITY: u32 = f32::NEG_INFINITY.to_bits();

/// The two sizes in a `u64` are never negative: their sign bits carry the
/// axis asked for.
const SIGN_HIGH: u64 = 1 << 63;
const SIGN_LOW: u64 = 1 << 31;
const SIGNS: u64 = SIGN_HIGH | SIGN_LOW;
/// The width of the two sizes in a `u64`, without its sign bit.
const WIDTH: u64 = ((u32::MAX as u64) << 32) & !SIGNS;

/// What a stored result was computed for.
#[derive(Clone, Copy, PartialEq)]
struct Key {
    /// Each axis's known dimension, or its available space without one.
    known_or_available: u64,
    /// The parent's size, and in the sign bits the axis asked for.
    parent: u64,
    /// Whether each known dimension is definite; an axis without one is.
    definite: Size<bool>,
}

fn option_bits(value: Option<f32>) -> u32 {
    value.map_or(INFINITY, f32::to_bits)
}

fn space_bits(space: AvailableSpace) -> u32 {
    match space {
        AvailableSpace::Definite(value) => (-value).to_bits(),
        AvailableSpace::MinContent => NEG_INFINITY,
        AvailableSpace::MaxContent => INFINITY,
    }
}

fn pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

impl Key {
    fn of(input: &LayoutInput) -> Self {
        let axis = match input.axis {
            RequestedAxis::Horizontal => SIGN_HIGH,
            RequestedAxis::Vertical => SIGN_LOW,
            RequestedAxis::Both => SIGNS,
        };
        let mixed = |known: Option<f32>, space: AvailableSpace| {
            known.map_or_else(|| space_bits(space), f32::to_bits)
        };
        let (known, space) = (input.known_dimensions, input.available_space);
        Self {
            known_or_available: pair(
                mixed(known.width, space.width),
                mixed(known.height, space.height),
            ),
            parent: (pair(
                option_bits(input.parent_size.width),
                option_bits(input.parent_size.height),
            ) & !SIGNS)
                | axis,
            definite: Size {
                width: input.known_dimensions_are_definite.width || known.width.is_none(),
                height: input.known_dimensions_are_definite.height || known.height.is_none(),
            },
        }
    }

    /// Whether a measurement stored for `self` answers `asked`: the same
    /// known dimensions or space, the same parent width, and a size of
    /// the axis asked for (one computed for one axis may hold anything in
    /// the other).
    fn measures(&self, asked: &Key) -> bool {
        let axis = self.parent & SIGNS;
        self.known_or_available == asked.known_or_available
            && self.definite == asked.definite
            && self.parent & WIDTH == asked.parent & WIDTH
            && (axis == SIGNS || axis == asked.parent & SIGNS)
    }
}

/// A box's cache: its final layout, and its measurements.
#[derive(Default)]
pub(super) struct BoxCache {
    final_layout: Option<(Key, LayoutOutput)>,
    /// Taffy's measurement slots, as many as were written: most boxes
    /// write none or one. An empty slot is `None`.
    measures: Vec<Option<(Key, Size<f32>)>>,
    /// The slots used since the replacing cursor last passed them.
    recently_used: u16,
    /// The slot to consider replacing next.
    next: u8,
}

impl BoxCache {
    /// What was stored for `input`, if anything.
    pub(super) fn get(&mut self, input: &LayoutInput) -> Option<LayoutOutput> {
        let key = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => self
                .final_layout
                .filter(|(stored, _)| *stored == key)
                .map(|(_, output)| output),
            RunMode::ComputeSize => {
                let (index, size) =
                    self.measures
                        .iter()
                        .enumerate()
                        .find_map(|(index, slot)| match slot {
                            Some((stored, size)) if stored.measures(&key) => Some((index, *size)),
                            _ => None,
                        })?;
                self.recently_used |= 1 << index;
                Some(LayoutOutput::from_outer_size(size))
            }
            RunMode::PerformHiddenLayout => None,
        }
    }

    /// Keep `output` as computed for `input`.
    pub(super) fn store(&mut self, input: &LayoutInput, output: LayoutOutput) {
        let key = Key::of(input);
        match input.run_mode {
            RunMode::PerformLayout => self.final_layout = Some((key, output)),
            RunMode::ComputeSize => {
                // A measurement answers with its size alone, which drops
                // what margin collapsing carries: one that carries any is
                // not kept.
                if output.margins_can_collapse_through
                    || output.top_margin != CollapsibleMarginSet::ZERO
                    || output.bottom_margin != CollapsibleMarginSet::ZERO
                {
                    return;
                }
                if let Some(index) = self
                    .measures
                    .iter()
                    .position(|slot| slot.is_some_and(|(stored, _)| stored == key))
                {
                    self.measures[index] = Some((key, output.size));
                    self.recently_used |= 1 << index;
                    return;
                }
                // A second chance for each slot used since the cursor
                // last passed it.
                while self.recently_used & (1 << self.next) != 0 {
                    self.recently_used &= !(1 << self.next);
                    self.next = (self.next + 1) % MEASURES;
                }
                let index = usize::from(self.next);
                if index >= self.measures.len() {
                    // One slot at a time: a Vec grows to four at first.
                    self.measures.reserve_exact(index + 1 - self.measures.len());
                    self.measures.resize(index + 1, None);
                }
                self.measures[index] = Some((key, output.size));
                self.recently_used |= 1 << index;
                self.next = (self.next + 1) % MEASURES;
            }
            RunMode::PerformHiddenLayout => {}
        }
    }

    /// Forget everything stored.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use taffy::{Line, SizingMode};

    use super::*;

    /// A fixed-seed generator: the same sequence every run.
    struct Seeded(u64);

    impl Seeded {
        fn next(&mut self) -> u64 {
            // xorshift64*
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn pick<T: Copy>(&mut self, from: &[T]) -> T {
            from[(self.next() % from.len() as u64) as usize]
        }
    }

    /// An input from few values, the common ones more often, so that
    /// inputs meet again.
    fn input(seeded: &mut Seeded) -> LayoutInput {
        let known = [None, None, Some(10.0)];
        let parent = [Some(100.0), Some(100.0), None];
        let space = [
            AvailableSpace::Definite(100.0),
            AvailableSpace::MinContent,
            AvailableSpace::MaxContent,
        ];
        LayoutInput {
            run_mode: seeded.pick(&[
                RunMode::PerformLayout,
                RunMode::ComputeSize,
                RunMode::ComputeSize,
                RunMode::PerformHiddenLayout,
            ]),
            sizing_mode: seeded.pick(&[SizingMode::InherentSize, SizingMode::ContentSize]),
            axis: seeded.pick(&[
                RequestedAxis::Horizontal,
                RequestedAxis::Vertical,
                RequestedAxis::Both,
            ]),
            known_dimensions: Size {
                width: seeded.pick(&known),
                height: seeded.pick(&known),
            },
            known_dimensions_are_definite: Size {
                width: seeded.pick(&[true, true, false]),
                height: seeded.pick(&[true, true, false]),
            },
            parent_size: Size {
                width: seeded.pick(&parent),
                height: seeded.pick(&parent),
            },
            available_space: Size {
                width: seeded.pick(&space),
                height: seeded.pick(&space),
            },
            vertical_margins_are_collapsible: Line::FALSE,
        }
    }

    fn output(seeded: &mut Seeded) -> LayoutOutput {
        let size = [1.0, 2.0, 30.0];
        let mut output = LayoutOutput::from_outer_size(Size {
            width: seeded.pick(&size),
            height: seeded.pick(&size),
        });
        output.baselines.first = seeded.pick(&[None, Some(12.0)]);
        match seeded.next() % 8 {
            0 => output.margins_can_collapse_through = true,
            1 => output.top_margin = CollapsibleMarginSet::from_margin(5.0),
            2 => output.bottom_margin = CollapsibleMarginSet::from_margin(-3.0),
            _ => {}
        }
        output
    }

    #[test]
    fn it_answers_what_taffys_cache_answers() {
        let (mut hits, mut measured, mut full) = (0, 0, 0);
        for seed in 1..=200u64 {
            let mut seeded = Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let (mut taffy, mut ours) = (taffy::Cache::new(), BoxCache::default());
            for step in 0..2000 {
                // Cleared seldom, so that the nine slots fill and are
                // replaced.
                match seeded.next() % 400 {
                    0 => {
                        taffy.clear();
                        ours.clear();
                    }
                    1..=180 => {
                        let (input, output) = (input(&mut seeded), output(&mut seeded));
                        taffy.store(&input, output);
                        ours.store(&input, output);
                        full += usize::from(ours.measures.len() == usize::from(MEASURES));
                    }
                    _ => {
                        let input = input(&mut seeded);
                        let expected = taffy.get(&input);
                        assert_eq!(
                            ours.get(&input),
                            expected,
                            "seed {seed}, step {step}: {input:?}"
                        );
                        if expected.is_some() {
                            hits += 1;
                            if input.run_mode == RunMode::ComputeSize {
                                measured += 1;
                            }
                        }
                    }
                }
            }
        }
        // The inputs met often enough to hit, final layouts and
        // measurements both, and the slots were full often.
        eprintln!("{hits} hits, {measured} measured; full at {full} stores");
        assert!(
            // Measured 2026-11-25: 11 535, 250 and 147 859.
            measured > 5000 && hits - measured > 120 && full > 70_000,
            "{hits} hits, {measured} measured; full at {full} stores"
        );
    }

    #[test]
    fn it_holds_room_for_what_was_written() {
        assert!(
            size_of::<BoxCache>() <= 128,
            "{} bytes",
            size_of::<BoxCache>()
        );
        let mut seeded = Seeded(7);
        let mut cache = BoxCache::default();
        let mut one = input(&mut seeded);
        one.run_mode = RunMode::ComputeSize;
        cache.store(
            &one,
            LayoutOutput::from_outer_size(Size {
                width: 4.0,
                height: 2.0,
            }),
        );
        assert_eq!(cache.measures.capacity(), 1);
        cache.clear();
        assert_eq!(cache.measures.capacity(), 0);
    }
}
