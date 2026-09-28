//! calc() values without dereferencing pointers.
//!
//! stylo_taffy passes a calc() length to Taffy as a raw pointer to Stylo's
//! `CalcLengthPercentage`, and Taffy hands that pointer back when it needs the
//! value resolved against a percentage basis. Blitz resolves it by
//! dereferencing the pointer, which needs `unsafe`, and this crate forbids
//! unsafe code.
//!
//! Instead, every calc() value is copied into a table keyed by its address
//! while the computed styles are still borrowed safely. Taffy's pointer is
//! then only ever used as a key: it is compared, never followed.

use std::collections::HashMap;

use erk_style::ComputedValues;
use erk_style::style::values::computed::length_percentage::{
    CalcLengthPercentage, Unpacked as UnpackedLengthPercentage,
};
use erk_style::style::values::computed::{CSSPixelLength, LengthPercentage};
use erk_style::style::values::generics::length::{GenericMargin, GenericMaxSize, GenericSize};
use erk_style::style::values::generics::position::GenericInset;

#[derive(Default)]
pub(crate) struct CalcTable {
    values: HashMap<usize, CalcLengthPercentage>,
}

impl CalcTable {
    /// Record the calc() values of every property stylo_taffy converts for
    /// box layout: sizes, min/max sizes, margins, paddings and insets.
    pub(crate) fn record(&mut self, style: &ComputedValues) {
        let position = style.get_position();
        for size in [&position.width, &position.height] {
            if let GenericSize::LengthPercentage(length) = size {
                self.note(&length.0);
            }
        }
        for size in [&position.min_width, &position.min_height] {
            if let GenericSize::LengthPercentage(length) = size {
                self.note(&length.0);
            }
        }
        for size in [&position.max_width, &position.max_height] {
            if let GenericMaxSize::LengthPercentage(length) = size {
                self.note(&length.0);
            }
        }
        for inset in [
            &position.top,
            &position.right,
            &position.bottom,
            &position.left,
        ] {
            if let GenericInset::LengthPercentage(length) = inset {
                self.note(length);
            }
        }

        let margin = style.get_margin();
        for side in [
            &margin.margin_top,
            &margin.margin_right,
            &margin.margin_bottom,
            &margin.margin_left,
        ] {
            if let GenericMargin::LengthPercentage(length) = side {
                self.note(length);
            }
        }

        let padding = style.get_padding();
        for side in [
            &padding.padding_top,
            &padding.padding_right,
            &padding.padding_bottom,
            &padding.padding_left,
        ] {
            self.note(&side.0);
        }
    }

    fn note(&mut self, length: &LengthPercentage) {
        if let UnpackedLengthPercentage::Calc(calc) = length.unpack() {
            self.values
                .insert(std::ptr::from_ref(calc) as usize, calc.clone());
        }
    }

    /// Resolve the calc() value Taffy identified by `ptr` against `basis`.
    /// An unknown pointer means a property this table does not record yet;
    /// it resolves to zero, which is also what Taffy does without a resolver.
    pub(crate) fn resolve(&self, ptr: *const (), basis: f32) -> f32 {
        debug_assert!(
            self.values.contains_key(&(ptr as usize)),
            "calc() value from a property the calc table does not record"
        );
        self.values
            .get(&(ptr as usize))
            .map_or(0.0, |calc| calc.resolve(CSSPixelLength::new(basis)).px())
    }
}
