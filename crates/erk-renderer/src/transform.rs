//! CSS transforms (CSS Transforms 1, and the individual `translate`,
//! `rotate` and `scale` of CSS Transforms 2) as one 2D affine matrix for
//! the display list: what the element's border box and everything in it go
//! through, about its `transform-origin`.
//!
//! 3D functions are flattened onto the page (their z is dropped) and
//! `perspective` is not applied; docs/css-support.md lists both.

use erk_style::style::properties::ComputedValues;
use erk_style::style::values::computed::transform::{Transform, TransformOperation};
use erk_style::style::values::computed::{Length, LengthPercentage};
use erk_style::style::values::generics::transform::{
    GenericRotate, GenericScale, GenericTranslate,
};
use euclid::default::{Point2D, Rect, Size2D, Transform3D};

use crate::list::Frame;

/// A 2D affine matrix `[a, b, c, d, e, f]`: a point `(x, y)` goes to
/// `(a x + c y + e, b x + d y + f)`.
pub(crate) type Matrix = [f32; 6];

/// Whether `style` transforms its element at all.
pub(crate) fn transforms(style: &ComputedValues) -> bool {
    let ops = style.get_box();
    !ops.transform.0.is_empty()
        || ops.translate != GenericTranslate::None
        || ops.rotate != GenericRotate::None
        || ops.scale != GenericScale::None
}

/// The matrix `style` applies to its element, whose border box is `frame`
/// in absolute coordinates: `None` if it has no transform.
pub(crate) fn matrix(style: &ComputedValues, frame: Frame) -> Option<Matrix> {
    if !transforms(style) {
        return None;
    }
    let ops = style.get_box();
    let resolve = |value: &LengthPercentage, basis: f32| value.resolve(Length::new(basis)).px();
    // translate, rotate and scale, then transform (CSS Transforms 2 §6),
    // as the same operations Stylo reads `transform` with.
    let mut list: Vec<TransformOperation> = Vec::new();
    if let GenericTranslate::Translate(x, y, z) = &ops.translate {
        list.push(TransformOperation::Translate3D(x.clone(), y.clone(), *z));
    }
    match &ops.rotate {
        GenericRotate::None => {}
        GenericRotate::Rotate(angle) => list.push(TransformOperation::Rotate(*angle)),
        GenericRotate::Rotate3D(x, y, z, angle) => {
            list.push(TransformOperation::Rotate3D(*x, *y, *z, *angle));
        }
    }
    if let GenericScale::Scale(x, y, z) = ops.scale {
        list.push(TransformOperation::Scale3D(x, y, z));
    }
    list.extend(ops.transform.0.iter().cloned());
    // Percentages in translations are of the border box (the reference box).
    let reference = Rect::new(
        Point2D::new(Length::new(0.0), Length::new(0.0)),
        Size2D::new(Length::new(frame.width), Length::new(frame.height)),
    );
    let (m, _) = Transform::components_to_transform_3d_matrix(&list, Some(&reference)).ok()?;
    let origin = &style.get_box().transform_origin;
    let (ox, oy) = (
        frame.x + resolve(&origin.horizontal, frame.width),
        frame.y + resolve(&origin.vertical, frame.height),
    );
    let m = Transform3D::translation(-ox, -oy, 0.0)
        .then(&m)
        .then(&Transform3D::translation(ox, oy, 0.0));
    Some([m.m11, m.m12, m.m21, m.m22, m.m41, m.m42])
}

/// Whether `matrix` can be undone: a transform that cannot (a scale by 0)
/// hides its element and everything in it (CSS Transforms 1 §6).
pub(crate) fn invertible(matrix: &Matrix) -> bool {
    let [a, b, c, d, ..] = *matrix;
    let det = a * d - b * c;
    det.is_finite() && det.abs() > 1e-9 && matrix.iter().all(|v| v.is_finite())
}

/// Where `matrix` takes `(x, y)`.
#[cfg(test)]
pub(crate) fn apply(matrix: &Matrix, (x, y): (f32, f32)) -> (f32, f32) {
    let [a, b, c, d, e, f] = *matrix;
    (a * x + c * y + e, b * x + d * y + f)
}

/// The point `matrix` takes to `(x, y)`; `None` if it cannot be undone.
pub(crate) fn unapply(matrix: &Matrix, (x, y): (f32, f32)) -> Option<(f32, f32)> {
    if !invertible(matrix) {
        return None;
    }
    let [a, b, c, d, e, f] = *matrix;
    let det = a * d - b * c;
    let (x, y) = (x - e, y - f);
    Some(((d * x - c * y) / det, (a * y - b * x) / det))
}

/// `outer` after `inner`: a point goes through `inner` first.
pub(crate) fn then(inner: &Matrix, outer: &Matrix) -> Matrix {
    let [a1, b1, c1, d1, e1, f1] = *inner;
    let [a2, b2, c2, d2, e2, f2] = *outer;
    [
        a2 * a1 + c2 * b1,
        b2 * a1 + d2 * b1,
        a2 * c1 + c2 * d1,
        b2 * c1 + d2 * d1,
        a2 * e1 + c2 * f1 + e2,
        b2 * e1 + d2 * f1 + f2,
    ]
}

/// The matrix that changes nothing.
pub(crate) const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn unapply_undoes_apply_and_then_composes_in_order() {
        // A quarter turn about (10, 10), then a move by (5, 0).
        let turn = [0.0, 1.0, -1.0, 0.0, 20.0, 0.0];
        let shift = [1.0, 0.0, 0.0, 1.0, 5.0, 0.0];
        let both = then(&turn, &shift);
        let p = (13.0, 10.0);
        assert!(close(apply(&both, p), apply(&shift, apply(&turn, p))));
        assert!(close(apply(&turn, p), (10.0, 13.0)));
        assert!(close(unapply(&both, apply(&both, p)).unwrap(), p));
        assert_eq!(unapply(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0], p), None);
        assert_eq!(then(&IDENTITY, &turn), turn);
    }
}
