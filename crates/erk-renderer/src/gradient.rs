//! CSS gradients (CSS Images 3 §3, CSS Images 4 §3) turned into the display
//! list's `Gradient`: the gradient line or ending shape for one copy of the
//! gradient image, and its colour stops placed, fixed up and fitted to
//! [0, 1].
//!
//! Linear and radial gradients and their `repeating-` forms; conic
//! gradients are not painted yet (docs/css-support.md).

use erk_style::style::properties::ComputedValues;
use erk_style::style::values::computed::image::{Gradient as CssGradient, LineDirection};
use erk_style::style::values::computed::{Angle, Color, Length, LengthPercentage, Position};
use erk_style::style::values::generics::image::{
    Circle, Ellipse, EndingShape, GenericGradient, GradientCompatMode, GradientFlags, GradientItem,
    ShapeExtent,
};
use erk_style::style::values::specified::position::{
    HorizontalPositionKeyword, VerticalPositionKeyword,
};

use crate::color::srgb_bytes;
use crate::list::{Frame, Gradient, GradientShape, GradientStop, Rgba};

/// The gradient `css` for the copy of its image that fills `tile`; `None`
/// for a kind not painted yet.
pub(crate) fn gradient(style: &ComputedValues, css: &CssGradient, tile: Frame) -> Option<Gradient> {
    match css {
        GenericGradient::Linear {
            direction,
            items,
            flags,
            compat_mode,
            ..
        } => {
            let (start, end) = line(direction, *compat_mode, tile);
            let length = distance(start, end);
            let stops = stops(style, items, length);
            Some(fit_linear(
                start,
                end,
                stops,
                flags.contains(GradientFlags::REPEATING),
            ))
        }
        GenericGradient::Radial {
            shape,
            position,
            items,
            flags,
            ..
        } => {
            let center = point(position, tile);
            let radii = ending_shape(shape, center, tile);
            let stops = stops(style, items, radii.0);
            Some(fit_radial(
                center,
                radii,
                stops,
                flags.contains(GradientFlags::REPEATING),
            ))
        }
        GenericGradient::Conic { .. } => None,
    }
}

/// The gradient line through the middle of `tile` (CSS Images 3 §3.1.1):
/// long enough that its ends' perpendiculars pass through the corners.
fn line(
    direction: &LineDirection,
    compat: GradientCompatMode,
    tile: Frame,
) -> ((f32, f32), (f32, f32)) {
    let (w, h) = (tile.width, tile.height);
    // The prefixed syntaxes name where the line starts, not where it goes,
    // and measure angles from the east, counter-clockwise.
    let legacy = compat != GradientCompatMode::Modern;
    let flip = if legacy { -1.0 } else { 1.0 };
    let degrees = match direction {
        LineDirection::Angle(angle) => {
            let degrees = Angle::degrees(angle);
            if legacy { 90.0 - degrees } else { degrees }
        }
        LineDirection::Horizontal(keyword) => horizontal(*keyword) * flip * 90.0,
        LineDirection::Vertical(keyword) => {
            if vertical(*keyword) * flip > 0.0 {
                180.0
            } else {
                0.0
            }
        }
        // Perpendicular to the diagonal through the other two corners,
        // pointing into the named corner's quadrant.
        LineDirection::Corner(x, y) => {
            let (sx, sy) = (horizontal(*x) * flip, vertical(*y) * flip);
            (sx * h).atan2(-sy * w).to_degrees()
        }
    };
    let radians = degrees.to_radians();
    let (dx, dy) = (radians.sin(), -radians.cos());
    let length = (w * dx).abs() + (h * dy).abs();
    let center = (tile.x + w / 2.0, tile.y + h / 2.0);
    let half = (dx * length / 2.0, dy * length / 2.0);
    (
        (center.0 - half.0, center.1 - half.1),
        (center.0 + half.0, center.1 + half.1),
    )
}

/// +1 for right, -1 for left.
fn horizontal(keyword: HorizontalPositionKeyword) -> f32 {
    match keyword {
        HorizontalPositionKeyword::Right => 1.0,
        HorizontalPositionKeyword::Left => -1.0,
    }
}

/// +1 for bottom, -1 for top.
fn vertical(keyword: VerticalPositionKeyword) -> f32 {
    match keyword {
        VerticalPositionKeyword::Bottom => 1.0,
        VerticalPositionKeyword::Top => -1.0,
    }
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn resolve(value: &LengthPercentage, basis: f32) -> f32 {
    value.resolve(Length::new(basis)).px()
}

/// A position in `tile`, in absolute coordinates.
fn point(position: &Position, tile: Frame) -> (f32, f32) {
    (
        tile.x + resolve(&position.horizontal, tile.width),
        tile.y + resolve(&position.vertical, tile.height),
    )
}

/// The radii of a radial gradient's ending shape (CSS Images 3 §3.2.2).
fn ending_shape(
    shape: &EndingShape<
        erk_style::style::values::computed::NonNegativeLength,
        erk_style::style::values::computed::NonNegativeLengthPercentage,
    >,
    center: (f32, f32),
    tile: Frame,
) -> (f32, f32) {
    let (x, y) = (center.0 - tile.x, center.1 - tile.y);
    // Distances to the left or right side, and to the top or bottom one.
    let sides_x = (x.abs(), (tile.width - x).abs());
    let sides_y = (y.abs(), (tile.height - y).abs());
    let closest = (sides_x.0.min(sides_x.1), sides_y.0.min(sides_y.1));
    let farthest = (sides_x.0.max(sides_x.1), sides_y.0.max(sides_y.1));
    match shape {
        EndingShape::Circle(Circle::Radius(radius)) => (radius.0.px(), radius.0.px()),
        EndingShape::Circle(Circle::Extent(extent)) => {
            let corners = [
                (sides_x.0, sides_y.0),
                (sides_x.1, sides_y.0),
                (sides_x.0, sides_y.1),
                (sides_x.1, sides_y.1),
            ]
            .map(|(dx, dy)| dx.hypot(dy));
            let radius = match extent {
                ShapeExtent::ClosestSide | ShapeExtent::Contain => closest.0.min(closest.1),
                ShapeExtent::FarthestSide => farthest.0.max(farthest.1),
                ShapeExtent::ClosestCorner => corners.into_iter().fold(f32::INFINITY, f32::min),
                ShapeExtent::FarthestCorner | ShapeExtent::Cover => {
                    corners.into_iter().fold(0.0, f32::max)
                }
            };
            (radius, radius)
        }
        EndingShape::Ellipse(Ellipse::Radii(rx, ry)) => {
            (resolve(&rx.0, tile.width), resolve(&ry.0, tile.height))
        }
        // The corner sizes keep the side sizes' aspect ratio and pass
        // through the corner: √2 times them.
        EndingShape::Ellipse(Ellipse::Extent(extent)) => match extent {
            ShapeExtent::ClosestSide | ShapeExtent::Contain => closest,
            ShapeExtent::FarthestSide => farthest,
            ShapeExtent::ClosestCorner => (
                closest.0 * std::f32::consts::SQRT_2,
                closest.1 * std::f32::consts::SQRT_2,
            ),
            ShapeExtent::FarthestCorner | ShapeExtent::Cover => (
                farthest.0 * std::f32::consts::SQRT_2,
                farthest.1 * std::f32::consts::SQRT_2,
            ),
        },
    }
}

/// A colour stop at a distance along the gradient line or ray.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stop {
    at: f32,
    color: Rgba,
}

/// A colour stop or transition hint as written, its position in pixels.
#[derive(Clone, Copy, Debug)]
enum Entry {
    Stop(Rgba, Option<f32>),
    Hint(f32),
}

/// The colour stops of `items` along a line or ray `length` long, in CSS
/// pixels.
fn stops(
    style: &ComputedValues,
    items: &[GradientItem<Color, LengthPercentage>],
    length: f32,
) -> Vec<Stop> {
    let entries: Vec<Entry> = items
        .iter()
        .map(|item| match item {
            GradientItem::SimpleColorStop(color) => {
                Entry::Stop(srgb_bytes(style.resolve_color(color)), None)
            }
            GradientItem::ComplexColorStop { color, position } => Entry::Stop(
                srgb_bytes(style.resolve_color(color)),
                Some(resolve(position, length)),
            ),
            GradientItem::InterpolationHint(position) => Entry::Hint(resolve(position, length)),
        })
        .collect();
    place(entries, length)
}

/// Stops and hints as written, placed: positioned, fixed up (CSS Images 3
/// §3.4.3) and with the transition hints (CSS Images 4 §3.5.3) turned into
/// stops.
fn place(mut entries: Vec<Entry>, length: f32) -> Vec<Stop> {
    let stop_count = entries
        .iter()
        .filter(|entry| matches!(entry, Entry::Stop(..)))
        .count();
    if stop_count == 0 {
        return Vec::new();
    }
    // 1. The first stop defaults to the start, the last to the end.
    if let Some(Entry::Stop(_, at @ None)) =
        entries.iter_mut().find(|e| matches!(e, Entry::Stop(..)))
    {
        *at = Some(0.0);
    }
    if let Some(Entry::Stop(_, at @ None)) = entries
        .iter_mut()
        .rev()
        .find(|e| matches!(e, Entry::Stop(..)))
    {
        *at = Some(length);
    }
    // 2. Nothing goes back: a position before an earlier one moves up to it.
    let mut furthest = f32::NEG_INFINITY;
    for entry in &mut entries {
        let at = match entry {
            Entry::Stop(_, Some(at)) | Entry::Hint(at) => at,
            Entry::Stop(_, None) => continue,
        };
        *at = at.max(furthest);
        furthest = *at;
    }
    // 3. Stops still without a position spread evenly between their
    //    positioned neighbours.
    let stop_indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| matches!(entry, Entry::Stop(..)))
        .map(|(index, _)| index)
        .collect();
    let position = |entry: &Entry| match entry {
        Entry::Stop(_, at) => *at,
        Entry::Hint(at) => Some(*at),
    };
    let mut run_start = 0;
    while run_start < stop_indices.len() {
        if position(&entries[stop_indices[run_start]]).is_some() {
            run_start += 1;
            continue;
        }
        let run_end = (run_start..stop_indices.len())
            .find(|&i| position(&entries[stop_indices[i]]).is_some())
            .expect("the last stop has a position");
        let before = position(&entries[stop_indices[run_start - 1]]).expect("positioned");
        let after = position(&entries[stop_indices[run_end]]).expect("positioned");
        let steps = (run_end - run_start + 1) as f32;
        for (k, i) in (run_start..run_end).enumerate() {
            if let Entry::Stop(_, at) = &mut entries[stop_indices[i]] {
                *at = Some(before + (after - before) * (k + 1) as f32 / steps);
            }
        }
        run_start = run_end;
    }
    // Transition hints become stops along their curve.
    let mut out: Vec<Stop> = Vec::new();
    let mut hint: Option<f32> = None;
    for entry in &entries {
        match *entry {
            Entry::Hint(at) => hint = Some(at),
            Entry::Stop(color, at) => {
                let stop = Stop {
                    at: at.expect("every stop has a position"),
                    color,
                };
                if let (Some(hint), Some(previous)) = (hint.take(), out.last().copied()) {
                    out.extend(hinted(previous, stop, hint));
                }
                out.push(stop);
            }
        }
    }
    out
}

/// The stops between `a` and `b` that draw the transition hint at `hint`
/// (CSS Images 4 §3.5.3): the colour is halfway at the hint, along the
/// curve `p^(ln 0.5 / ln h)`, sampled as browsers do.
fn hinted(a: Stop, b: Stop, hint: f32) -> Vec<Stop> {
    let span = b.at - a.at;
    if span <= 0.0 {
        return Vec::new();
    }
    let h = (hint - a.at) / span;
    if h <= 0.0 {
        // All the way to b at once.
        return vec![Stop {
            at: a.at,
            color: b.color,
        }];
    }
    if h >= 1.0 {
        return vec![Stop {
            at: b.at,
            color: a.color,
        }];
    }
    if (h - 0.5).abs() < 1e-6 {
        return Vec::new();
    }
    let exponent = 0.5f32.ln() / h.ln();
    const SAMPLES: usize = 16;
    (1..SAMPLES)
        .map(|i| {
            let p = i as f32 / SAMPLES as f32;
            Stop {
                at: a.at + span * p,
                color: mix(a.color, b.color, p.powf(exponent)),
            }
        })
        .collect()
}

/// `a` to `b` at `t`, premultiplied as gradients interpolate (CSS Color 4
/// §12.3), back to straight bytes.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let premultiplied = |c: Rgba| {
        let alpha = f32::from(c[3]) / 255.0;
        [
            f32::from(c[0]) * alpha,
            f32::from(c[1]) * alpha,
            f32::from(c[2]) * alpha,
            alpha,
        ]
    };
    let (pa, pb) = (premultiplied(a), premultiplied(b));
    let lerp = |i: usize| pa[i] + (pb[i] - pa[i]) * t;
    let alpha = lerp(3);
    if alpha <= 0.0 {
        return [0, 0, 0, 0];
    }
    let channel = |i: usize| (lerp(i) / alpha).round().clamp(0.0, 255.0) as u8;
    [
        channel(0),
        channel(1),
        channel(2),
        (alpha * 255.0).round() as u8,
    ]
}

/// One colour all over, on a well-formed line whatever the box.
fn solid(color: Rgba) -> Gradient {
    Gradient {
        shape: GradientShape::Linear {
            start: (0.0, 0.0),
            end: (1.0, 0.0),
        },
        stops: vec![
            GradientStop { offset: 0.0, color },
            GradientStop { offset: 1.0, color },
        ],
        repeating: false,
    }
}

/// The average colour of `stops` over their span, premultiplied: what a
/// repeating gradient whose period rounds to nothing paints (CSS Images 3
/// §3.4.3). Equal spans if all the stops are at one place.
fn average(stops: &[Stop]) -> Rgba {
    let span = stops.last().map_or(0.0, |s| s.at) - stops.first().map_or(0.0, |s| s.at);
    let mut sum = [0.0f32; 4];
    let mut weight = 0.0;
    for pair in stops.windows(2) {
        let length = if span > 0.0 {
            pair[1].at - pair[0].at
        } else {
            1.0
        };
        let middle = mix(pair[0].color, pair[1].color, 0.5);
        let alpha = f32::from(middle[3]) / 255.0;
        for i in 0..3 {
            sum[i] += f32::from(middle[i]) * alpha * length;
        }
        sum[3] += alpha * length;
        weight += length;
    }
    if weight <= 0.0 || sum[3] <= 0.0 {
        return stops.last().map_or([0, 0, 0, 0], |s| s.color);
    }
    let alpha = sum[3] / weight;
    let channel = |i: usize| (sum[i] / sum[3]).round().clamp(0.0, 255.0) as u8;
    [
        channel(0),
        channel(1),
        channel(2),
        (alpha * 255.0).round() as u8,
    ]
}

/// Offsets for `stops` between `first` and `last` (in the stops' units).
fn offsets(stops: &[Stop], first: f32, last: f32) -> Vec<GradientStop> {
    let span = last - first;
    stops
        .iter()
        .map(|stop| GradientStop {
            offset: if span > 0.0 {
                ((stop.at - first) / span).clamp(0.0, 1.0)
            } else {
                0.0
            },
            color: stop.color,
        })
        .collect()
}

/// The smallest span a gradient is drawn over: stops that all sit at one
/// place make a hard edge there.
const EDGE: f32 = 1.0 / 64.0;

/// A linear gradient from `start` to `end` whose stops are at `stops`
/// pixels along it, with the line moved to run from the first stop to the
/// last.
fn fit_linear(start: (f32, f32), end: (f32, f32), stops: Vec<Stop>, repeating: bool) -> Gradient {
    let length = distance(start, end);
    let direction = if length > 0.0 {
        ((end.0 - start.0) / length, (end.1 - start.1) / length)
    } else {
        // No line (an empty box): the stops still order along something.
        (1.0, 0.0)
    };
    let along = |at: f32| (start.0 + direction.0 * at, start.1 + direction.1 * at);
    let (Some(first), Some(last)) = (stops.first().copied(), stops.last().copied()) else {
        return solid([0, 0, 0, 0]);
    };
    let shape = |from: f32, to: f32| GradientShape::Linear {
        start: along(from),
        end: along(to),
    };
    if repeating && last.at - first.at < EDGE {
        return solid(average(&stops));
    }
    if last.at - first.at < EDGE {
        // A hard edge: the first colour before it, the last after.
        return Gradient {
            shape: shape(first.at, first.at + EDGE),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: first.color,
                },
                GradientStop {
                    offset: 1.0,
                    color: last.color,
                },
            ],
            repeating: false,
        };
    }
    Gradient {
        shape: shape(first.at, last.at),
        stops: offsets(&stops, first.at, last.at),
        repeating,
    }
}

/// A radial gradient around `center` whose stops are at `stops` pixels
/// along its horizontal ray of length `radii.0`.
fn fit_radial(
    center: (f32, f32),
    radii: (f32, f32),
    mut stops: Vec<Stop>,
    repeating: bool,
) -> Gradient {
    let shape = |from: f32, to: f32| GradientShape::Radial {
        center,
        radii: (to, radii.1 * to / radii.0),
        inner: if to > 0.0 { from / to } else { 0.0 },
    };
    let (Some(first), Some(last)) = (stops.first().copied(), stops.last().copied()) else {
        return solid([0, 0, 0, 0]);
    };
    // An ending shape with no area: the last colour everywhere, as a very
    // small one would draw.
    if radii.0 <= 0.0 || radii.1 <= 0.0 {
        return solid(last.color);
    }
    if repeating {
        let period = last.at - first.at;
        if period < EDGE {
            return solid(average(&stops));
        }
        // A ray has no negative side: shift by whole periods until the
        // first stop is on it.
        if first.at < 0.0 {
            let shift = (-first.at / period).ceil() * period;
            for stop in &mut stops {
                stop.at += shift;
            }
        }
    } else if first.at < 0.0 {
        // The colour at the centre is what the stops make there.
        let at_zero = colour_at(&stops, 0.0);
        stops.retain(|stop| stop.at > 0.0);
        stops.insert(
            0,
            Stop {
                at: 0.0,
                color: at_zero,
            },
        );
    }
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    if last.at <= 0.0 {
        return solid(last.color);
    }
    if last.at - first.at < EDGE {
        return Gradient {
            shape: shape(first.at, first.at + EDGE),
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: first.color,
                },
                GradientStop {
                    offset: 1.0,
                    color: last.color,
                },
            ],
            repeating: false,
        };
    }
    Gradient {
        shape: shape(first.at, last.at),
        stops: offsets(&stops, first.at, last.at),
        repeating,
    }
}

/// The colour `stops` make at `at`.
fn colour_at(stops: &[Stop], at: f32) -> Rgba {
    let Some(after) = stops.iter().position(|stop| stop.at >= at) else {
        return stops.last().map_or([0, 0, 0, 0], |s| s.color);
    };
    if after == 0 {
        return stops[0].color;
    }
    let (a, b) = (stops[after - 1], stops[after]);
    let span = b.at - a.at;
    let t = if span > 0.0 { (at - a.at) / span } else { 1.0 };
    mix(a.color, b.color, t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgba = [255, 0, 0, 255];
    const BLUE: Rgba = [0, 0, 255, 255];

    fn stop(at: f32, color: Rgba) -> Stop {
        Stop { at, color }
    }

    #[test]
    fn a_linear_gradient_is_fitted_to_its_first_and_last_stop() {
        let fitted = fit_linear(
            (0.0, 0.0),
            (100.0, 0.0),
            vec![stop(20.0, RED), stop(60.0, BLUE)],
            false,
        );
        assert_eq!(
            fitted.shape,
            GradientShape::Linear {
                start: (20.0, 0.0),
                end: (60.0, 0.0)
            }
        );
        assert_eq!(
            fitted.stops.iter().map(|s| s.offset).collect::<Vec<_>>(),
            [0.0, 1.0]
        );
    }

    #[test]
    fn stops_at_one_place_make_a_hard_edge_or_a_repeating_average() {
        let edge = fit_linear(
            (0.0, 0.0),
            (100.0, 0.0),
            vec![stop(50.0, RED), stop(50.0, BLUE)],
            false,
        );
        assert_eq!(edge.stops[0].color, RED);
        assert_eq!(edge.stops[1].color, BLUE);
        let average = fit_linear(
            (0.0, 0.0),
            (100.0, 0.0),
            vec![stop(50.0, RED), stop(50.0, BLUE)],
            true,
        );
        assert_eq!(average.stops[0].color, average.stops[1].color);
        assert_eq!(average.stops[0].color, [128, 0, 128, 255]);
    }

    #[test]
    fn a_radial_gradient_starts_at_the_centre_with_the_colour_its_stops_make_there() {
        let fitted = fit_radial(
            (50.0, 50.0),
            (50.0, 25.0),
            vec![stop(-50.0, RED), stop(50.0, BLUE)],
            false,
        );
        assert_eq!(fitted.stops[0].color, [128, 0, 128, 255]);
        assert_eq!(
            fitted.shape,
            GradientShape::Radial {
                center: (50.0, 50.0),
                radii: (50.0, 25.0),
                inner: 0.0
            }
        );
    }

    #[test]
    fn a_repeating_radial_gradient_is_shifted_onto_the_ray_by_whole_periods() {
        let fitted = fit_radial(
            (0.0, 0.0),
            (100.0, 100.0),
            vec![stop(-30.0, RED), stop(10.0, BLUE)],
            true,
        );
        // Period 40: shifted by 40, to 10..50.
        assert_eq!(
            fitted.shape,
            GradientShape::Radial {
                center: (0.0, 0.0),
                radii: (50.0, 50.0),
                inner: 0.2
            }
        );
        assert!(fitted.repeating);
    }

    #[test]
    fn stops_default_to_the_ends_never_go_back_and_spread_evenly() {
        let placed = place(
            vec![
                Entry::Stop(RED, None),
                Entry::Stop(BLUE, Some(60.0)),
                // Before the stop above: moves up to it.
                Entry::Stop(RED, Some(20.0)),
                Entry::Stop(BLUE, None),
                Entry::Stop(RED, None),
                Entry::Stop(BLUE, None),
            ],
            100.0,
        );
        let at: Vec<f32> = placed.iter().map(|stop| stop.at).collect();
        assert_eq!(at, [0.0, 60.0, 60.0, 73.333336, 86.666664, 100.0]);
    }

    #[test]
    fn a_hint_halfway_changes_nothing_and_elsewhere_bends_the_transition() {
        assert!(hinted(stop(0.0, RED), stop(100.0, BLUE), 50.0).is_empty());
        let bent = hinted(stop(0.0, RED), stop(100.0, BLUE), 25.0);
        // At the hint the colour is halfway.
        let at_hint = bent.iter().find(|s| (s.at - 25.0).abs() < 1e-3).unwrap();
        assert_eq!(at_hint.color, [128, 0, 128, 255]);
    }

    #[test]
    fn prefixed_gradients_name_where_the_line_starts_and_measure_angles_from_the_east() {
        let tile = Frame {
            x: 0.0,
            y: 0.0,
            width: 120.0,
            height: 80.0,
        };
        assert_eq!(
            line(
                &LineDirection::Vertical(VerticalPositionKeyword::Top),
                GradientCompatMode::WebKit,
                tile
            ),
            line(
                &LineDirection::Vertical(VerticalPositionKeyword::Bottom),
                GradientCompatMode::Modern,
                tile
            ),
        );
        assert_eq!(
            line(
                &LineDirection::Angle(Angle::from_degrees(0.0)),
                GradientCompatMode::Moz,
                tile
            ),
            line(
                &LineDirection::Angle(Angle::from_degrees(90.0)),
                GradientCompatMode::Modern,
                tile
            ),
        );
    }

    #[test]
    fn corners_point_into_their_quadrant_perpendicular_to_the_other_diagonal() {
        let tile = Frame {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 100.0,
        };
        let (start, end) = line(
            &LineDirection::Corner(
                HorizontalPositionKeyword::Right,
                VerticalPositionKeyword::Top,
            ),
            GradientCompatMode::Modern,
            tile,
        );
        // The middle line passes through the other two corners: the
        // direction is perpendicular to (200, 100).
        let direction = (end.0 - start.0, end.1 - start.1);
        assert!((direction.0 * 200.0 + direction.1 * 100.0).abs() < 1e-2);
        assert!(direction.0 > 0.0 && direction.1 < 0.0);
        // The ends' perpendiculars pass through the corners themselves.
        let length = distance(start, end);
        let unit = (direction.0 / length, direction.1 / length);
        let project = |p: (f32, f32)| (p.0 - start.0) * unit.0 + (p.1 - start.1) * unit.1;
        assert!(project((0.0, 100.0)).abs() < 1e-2);
        assert!((project((200.0, 0.0)) - length).abs() < 1e-2);
    }
}
