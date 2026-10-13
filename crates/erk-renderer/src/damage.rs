//! Which part of a frame changed (M5.6, p2-incremental §3.7): the frame is
//! cut into square tiles, and each tile keeps a digest of the items that
//! may paint in it, in paint order, with the clips, opacity groups and
//! transforms around each. A tile whose digest is the last frame's paints
//! the same pixels; the others are the damage, and only they are painted
//! again (`paint::paint_damage`).
//!
//! No side table from nodes to items is needed: whatever changed (a style,
//! a layout, a scroll, an image that arrived) shows as other items, or the
//! same items elsewhere or in another order, in the tiles they touch.

use std::hash::{Hash, Hasher};

use crate::list::{DisplayItem, DisplayList, Frame, GradientShape};

/// The side of a tile, in device pixels (B11 measures it).
pub(crate) const TILE: u16 = 32;

/// Above this share of the frame damaged, the whole frame is painted: the
/// areas would cost as much.
pub(crate) const WHOLE_ABOVE: f32 = 0.5;

/// A frame's tiles: `columns` × `rows` digests, for a `width` × `height`
/// frame at `scale`.
#[derive(Clone, PartialEq)]
pub(crate) struct Tiles {
    width: u16,
    height: u16,
    scale: f32,
    columns: usize,
    rows: usize,
    digests: Vec<u64>,
}

/// A rectangle of device pixels, `x..x + width` by `y..y + height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

impl Tiles {
    /// The digests of `list` painted into a `width` × `height` frame at
    /// `scale` device pixels per CSS pixel.
    pub(crate) fn of(list: &DisplayList, width: u16, height: u16, scale: f32) -> Self {
        let columns = usize::from(width).div_ceil(usize::from(TILE));
        let rows = usize::from(height).div_ceil(usize::from(TILE));
        // Every tile starts from the canvas colour.
        let seed = digest(&list.canvas);
        let mut digests = vec![seed; columns * rows];
        // The groups the items are in: each push's digest, outermost first.
        let mut open: Vec<u64> = Vec::new();
        // How many of them are transforms: what is inside one may land
        // anywhere.
        let mut transforms = 0usize;
        for item in &list.items {
            match item {
                DisplayItem::PushClip { .. } | DisplayItem::PushOpacity(_) => {
                    open.push(item_digest(item));
                    continue;
                }
                DisplayItem::PushTransform(_) => {
                    open.push(item_digest(item));
                    transforms += 1;
                    continue;
                }
                DisplayItem::PopTransform => {
                    open.pop();
                    transforms = transforms.saturating_sub(1);
                    continue;
                }
                DisplayItem::PopClip | DisplayItem::PopOpacity => {
                    open.pop();
                    continue;
                }
                // Not painted.
                DisplayItem::Hit { .. } => continue,
                _ => {}
            }
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            open.hash(&mut hasher);
            item_digest(item).hash(&mut hasher);
            let contribution = hasher.finish();
            let (x0, y0, x1, y1) = if transforms > 0 {
                (0, 0, columns, rows)
            } else {
                match reach(item) {
                    Some(frame) => cover(frame, scale, columns, rows),
                    None => (0, 0, columns, rows),
                }
            };
            for row in y0..y1 {
                for column in x0..x1 {
                    let tile = &mut digests[row * columns + column];
                    // In order: two items swapping places in a tile change
                    // it.
                    *tile = tile.rotate_left(5).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ contribution;
                }
            }
        }
        Self {
            width,
            height,
            scale,
            columns,
            rows,
            digests,
        }
    }

    /// What changed from `before` to these tiles, as rectangles, row by
    /// row; `None` when it cannot be said (another frame size or scale) or
    /// when so much changed that the whole frame is painted again.
    pub(crate) fn damage_to_paint(&self, before: &Tiles) -> Option<Vec<Area>> {
        let areas = self.damage_since(before)?;
        let damaged: f32 = areas
            .iter()
            .map(|area| f32::from(area.width) * f32::from(area.height))
            .sum();
        let whole = f32::from(self.width) * f32::from(self.height);
        (damaged <= WHOLE_ABOVE * whole).then_some(areas)
    }

    /// What changed from `before` to these tiles, as rectangles, row by
    /// row; `None` when it cannot be said: another frame size or scale.
    pub(crate) fn damage_since(&self, before: &Tiles) -> Option<Vec<Area>> {
        if (self.width, self.height, self.scale) != (before.width, before.height, before.scale) {
            return None;
        }
        let tile = usize::from(TILE);
        let mut areas: Vec<Area> = Vec::new();
        for row in 0..self.rows {
            let mut column = 0;
            while column < self.columns {
                let at = row * self.columns + column;
                if self.digests[at] == before.digests[at] {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < self.columns
                    && self.digests[row * self.columns + column]
                        != before.digests[row * self.columns + column]
                {
                    column += 1;
                }
                let x = start * tile;
                let y = row * tile;
                let right = (column * tile).min(usize::from(self.width));
                let bottom = ((row + 1) * tile).min(usize::from(self.height));
                areas.push(Area {
                    x: x as u16,
                    y: y as u16,
                    width: (right - x) as u16,
                    height: (bottom - y) as u16,
                });
            }
        }
        Some(areas)
    }
}

/// The tiles, `x0..x1` by `y0..y1`, that `frame` (CSS pixels) touches at
/// `scale`: antialiasing paints no pixel the shape does not touch.
fn cover(frame: Frame, scale: f32, columns: usize, rows: usize) -> (usize, usize, usize, usize) {
    let tile = f32::from(TILE);
    let span = |start: f32, size: f32, count: usize| {
        let low = ((start * scale) / tile).floor().max(0.0);
        let high = (((start + size) * scale) / tile).floor() + 1.0;
        let low = (low as usize).min(count);
        let high = if high.is_finite() && high > 0.0 {
            (high as usize).min(count)
        } else {
            0
        };
        (low, high.max(low))
    };
    let (x0, x1) = span(frame.x, frame.width, columns);
    let (y0, y1) = span(frame.y, frame.height, rows);
    (x0, y0, x1, y1)
}

/// Where `item` may paint, in CSS pixels; `None` if anywhere.
pub(crate) fn reach(item: &DisplayItem) -> Option<Frame> {
    match item {
        // As the list keeps a run: three ems above the baseline and two
        // below; across, two ems before the first glyph and three after
        // the last.
        DisplayItem::Glyphs(run) => {
            let size = run.size;
            let (mut left, mut right) = (f32::INFINITY, f32::NEG_INFINITY);
            let (mut top, mut bottom) = (f32::INFINITY, f32::NEG_INFINITY);
            for glyph in &run.glyphs {
                left = left.min(glyph.x);
                right = right.max(glyph.x);
                top = top.min(glyph.y);
                bottom = bottom.max(glyph.y);
            }
            (left <= right).then_some(Frame {
                x: left - 2.0 * size,
                y: top - 3.0 * size,
                width: right - left + 5.0 * size,
                height: bottom - top + 5.0 * size,
            })
        }
        DisplayItem::Highlight(frame) => Some(*frame),
        item => crate::display::ink(item),
    }
}

fn digest(value: &impl Hash) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A digest of everything `item` paints with.
fn item_digest(item: &DisplayItem) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let floats = |values: &[f32], h: &mut std::collections::hash_map::DefaultHasher| {
        for value in values {
            value.to_bits().hash(h);
        }
    };
    let frame = |frame: &Frame| [frame.x, frame.y, frame.width, frame.height];
    let radii = |radii: &crate::list::Radii| radii.map(|(x, y)| [x, y]).concat();
    std::mem::discriminant(item).hash(&mut h);
    match item {
        DisplayItem::Rect {
            x,
            y,
            width,
            height,
            color,
        } => {
            floats(&[*x, *y, *width, *height], &mut h);
            color.hash(&mut h);
        }
        DisplayItem::RoundedRect {
            frame: f,
            radii: r,
            color,
        } => {
            floats(&frame(f), &mut h);
            floats(&radii(r), &mut h);
            color.hash(&mut h);
        }
        DisplayItem::Border {
            frame: f,
            widths,
            colors,
            radii: r,
        } => {
            floats(&frame(f), &mut h);
            floats(widths, &mut h);
            colors.hash(&mut h);
            floats(&radii(r), &mut h);
        }
        DisplayItem::Shadow {
            frame: f,
            radius,
            blur,
            color,
            clip,
            clip_radii,
        } => {
            floats(&frame(f), &mut h);
            floats(&[*radius, *blur], &mut h);
            color.hash(&mut h);
            floats(&frame(clip), &mut h);
            floats(&radii(clip_radii), &mut h);
        }
        DisplayItem::Image {
            image,
            size,
            tile,
            repeat,
            area,
            clip,
            clip_radii,
        } => {
            image.hash(&mut h);
            size.hash(&mut h);
            floats(&frame(tile), &mut h);
            repeat.hash(&mut h);
            floats(&frame(area), &mut h);
            floats(&frame(clip), &mut h);
            floats(&radii(clip_radii), &mut h);
        }
        DisplayItem::Gradient {
            gradient,
            tile,
            repeat,
            area,
            clip,
            clip_radii,
        } => {
            match gradient.shape {
                GradientShape::Linear { start, end } => {
                    floats(&[start.0, start.1, end.0, end.1], &mut h);
                }
                GradientShape::Radial {
                    center,
                    radii,
                    inner,
                } => floats(&[center.0, center.1, radii.0, radii.1, inner], &mut h),
            }
            for stop in &gradient.stops {
                floats(&[stop.offset], &mut h);
                stop.color.hash(&mut h);
            }
            gradient.repeating.hash(&mut h);
            floats(&frame(tile), &mut h);
            repeat.hash(&mut h);
            floats(&frame(area), &mut h);
            floats(&frame(clip), &mut h);
            floats(&radii(clip_radii), &mut h);
        }
        DisplayItem::PushOpacity(opacity) => floats(&[*opacity], &mut h),
        DisplayItem::PushTransform(matrix) => floats(matrix, &mut h),
        DisplayItem::PushClip { frame: f, radii: r } => {
            floats(&frame(f), &mut h);
            floats(&radii(r), &mut h);
        }
        DisplayItem::Glyphs(run) => {
            run.font.hash(&mut h);
            floats(&[run.size], &mut h);
            run.color.hash(&mut h);
            for glyph in &run.glyphs {
                glyph.id.hash(&mut h);
                floats(&[glyph.x, glyph.y], &mut h);
            }
        }
        DisplayItem::Hit {
            node,
            frame: f,
            text,
        } => {
            node.hash(&mut h);
            floats(&frame(f), &mut h);
            text.hash(&mut h);
        }
        DisplayItem::Highlight(f) => floats(&frame(f), &mut h),
        DisplayItem::PopOpacity | DisplayItem::PopTransform | DisplayItem::PopClip => {}
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::list::{FontId, GlyphRun, Gradient, GradientStop, ImageId, PositionedGlyph};

    fn frame() -> Frame {
        Frame {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        }
    }

    /// `item`, and each one way it can paint otherwise: every one must have
    /// another digest, or a tile would keep pixels it no longer paints.
    fn differs(item: DisplayItem, changes: Vec<fn(&mut DisplayItem)>) {
        let base = item_digest(&item);
        for (at, change) in changes.into_iter().enumerate() {
            let mut other = item.clone();
            change(&mut other);
            assert_ne!(item_digest(&other), base, "change {at} of {item:?}");
        }
    }

    #[test]
    fn every_field_an_item_paints_with_is_in_its_digest() {
        let radii = [(1.0, 1.0); 4];
        differs(
            DisplayItem::Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                color: [1, 2, 3, 4],
            },
            vec![
                |i| {
                    if let DisplayItem::Rect { x, .. } = i {
                        *x += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Rect { y, .. } = i {
                        *y += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Rect { width, .. } = i {
                        *width += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Rect { height, .. } = i {
                        *height += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Rect { color, .. } = i {
                        color[0] += 1
                    }
                },
            ],
        );
        differs(
            DisplayItem::RoundedRect {
                frame: frame(),
                radii,
                color: [1, 2, 3, 4],
            },
            vec![
                |i| {
                    if let DisplayItem::RoundedRect { frame, .. } = i {
                        frame.y += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::RoundedRect { radii, .. } = i {
                        radii[2].1 += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::RoundedRect { color, .. } = i {
                        color[3] += 1
                    }
                },
            ],
        );
        differs(
            DisplayItem::Border {
                frame: frame(),
                widths: [1.0; 4],
                colors: [[1, 2, 3, 4]; 4],
                radii,
            },
            vec![
                |i| {
                    if let DisplayItem::Border { frame, .. } = i {
                        frame.width += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Border { widths, .. } = i {
                        widths[3] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Border { colors, .. } = i {
                        colors[1][0] += 1
                    }
                },
                |i| {
                    if let DisplayItem::Border { radii, .. } = i {
                        radii[0].0 += 1.0
                    }
                },
            ],
        );
        differs(
            DisplayItem::Shadow {
                frame: frame(),
                radius: 1.0,
                blur: 2.0,
                color: [1, 2, 3, 4],
                clip: frame(),
                clip_radii: radii,
            },
            vec![
                |i| {
                    if let DisplayItem::Shadow { frame, .. } = i {
                        frame.x += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Shadow { radius, .. } = i {
                        *radius += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Shadow { blur, .. } = i {
                        *blur += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Shadow { color, .. } = i {
                        color[1] += 1
                    }
                },
                |i| {
                    if let DisplayItem::Shadow { clip, .. } = i {
                        clip.height += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Shadow { clip_radii, .. } = i {
                        clip_radii[1].1 += 1.0
                    }
                },
            ],
        );
        differs(
            DisplayItem::Image {
                image: ImageId(1),
                size: (2, 3),
                tile: frame(),
                repeat: (false, false),
                area: frame(),
                clip: frame(),
                clip_radii: radii,
            },
            vec![
                |i| {
                    if let DisplayItem::Image { image, .. } = i {
                        image.0 += 1
                    }
                },
                |i| {
                    if let DisplayItem::Image { size, .. } = i {
                        size.1 += 1
                    }
                },
                |i| {
                    if let DisplayItem::Image { tile, .. } = i {
                        tile.y += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Image { repeat, .. } = i {
                        repeat.1 = true
                    }
                },
                |i| {
                    if let DisplayItem::Image { area, .. } = i {
                        area.width += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Image { clip, .. } = i {
                        clip.x += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Image { clip_radii, .. } = i {
                        clip_radii[3].0 += 1.0
                    }
                },
            ],
        );
        let stops = vec![
            GradientStop {
                offset: 0.0,
                color: [1, 2, 3, 4],
            },
            GradientStop {
                offset: 1.0,
                color: [5, 6, 7, 8],
            },
        ];
        let gradient = |shape| DisplayItem::Gradient {
            gradient: Gradient {
                shape,
                stops: stops.clone(),
                repeating: false,
            },
            tile: frame(),
            repeat: (false, false),
            area: frame(),
            clip: frame(),
            clip_radii: radii,
        };
        let linear = GradientShape::Linear {
            start: (1.0, 2.0),
            end: (3.0, 4.0),
        };
        differs(
            gradient(linear),
            vec![
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i
                        && let GradientShape::Linear { start, .. } = &mut gradient.shape
                    {
                        start.1 += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i
                        && let GradientShape::Linear { end, .. } = &mut gradient.shape
                    {
                        end.0 += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i {
                        gradient.stops[1].offset = 0.5
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i {
                        gradient.stops[0].color[2] += 1
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i {
                        gradient.repeating = true
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { tile, .. } = i {
                        tile.x += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { repeat, .. } = i {
                        repeat.0 = true
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { area, .. } = i {
                        area.height += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { clip, .. } = i {
                        clip.y += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { clip_radii, .. } = i {
                        clip_radii[2].0 += 1.0
                    }
                },
            ],
        );
        differs(
            gradient(GradientShape::Radial {
                center: (1.0, 2.0),
                radii: (3.0, 4.0),
                inner: 0.5,
            }),
            vec![
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i
                        && let GradientShape::Radial { center, .. } = &mut gradient.shape
                    {
                        center.0 += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i
                        && let GradientShape::Radial { radii, .. } = &mut gradient.shape
                    {
                        radii.1 += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Gradient { gradient, .. } = i
                        && let GradientShape::Radial { inner, .. } = &mut gradient.shape
                    {
                        *inner = 0.25
                    }
                },
            ],
        );
        differs(
            DisplayItem::PushOpacity(0.5),
            vec![|i| *i = DisplayItem::PushOpacity(0.25)],
        );
        differs(
            DisplayItem::PushTransform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            vec![
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[0] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[1] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[2] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[3] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[4] += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushTransform(m) = i {
                        m[5] += 1.0
                    }
                },
            ],
        );
        differs(
            DisplayItem::PushClip {
                frame: frame(),
                radii,
            },
            vec![
                |i| {
                    if let DisplayItem::PushClip { frame, .. } = i {
                        frame.height += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::PushClip { radii, .. } = i {
                        radii[3].1 += 1.0
                    }
                },
            ],
        );
        differs(
            DisplayItem::Glyphs(GlyphRun {
                font: FontId(1),
                size: 12.0,
                color: [1, 2, 3, 4],
                glyphs: vec![PositionedGlyph {
                    id: 5,
                    x: 1.0,
                    y: 2.0,
                }],
                text: "a".to_owned(),
            }),
            vec![
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.font.0 += 1
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.size += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.color[0] += 1
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.glyphs[0].id += 1
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.glyphs[0].x += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.glyphs[0].y += 1.0
                    }
                },
                |i| {
                    if let DisplayItem::Glyphs(run) = i {
                        run.glyphs.push(run.glyphs[0])
                    }
                },
            ],
        );
        differs(
            DisplayItem::Highlight(frame()),
            vec![|i| {
                if let DisplayItem::Highlight(frame) = i {
                    frame.x += 1.0
                }
            }],
        );
    }

    #[test]
    fn a_glyph_damages_the_tiles_its_ink_reaches() {
        // A glyph set just before a tile's edge reaches into the next tile,
        // and above its baseline into the tile above.
        let list = |id: u32| DisplayList {
            canvas: [255; 4],
            items: vec![DisplayItem::Glyphs(GlyphRun {
                font: FontId(1),
                size: 12.0,
                color: [0, 0, 0, 255],
                glyphs: vec![PositionedGlyph {
                    id,
                    x: f32::from(TILE) - 2.0,
                    y: f32::from(TILE) + 2.0,
                }],
                text: "a".to_owned(),
            })],
        };
        let size = 4 * TILE;
        let before = Tiles::of(&list(1), size, size, 1.0);
        let areas = Tiles::of(&list(2), size, size, 1.0)
            .damage_since(&before)
            .unwrap();
        let covers = |x: u16, y: u16| {
            areas
                .iter()
                .any(|a| a.x <= x && x < a.x + a.width && a.y <= y && y < a.y + a.height)
        };
        assert!(covers(TILE + 4, TILE + 1), "after the glyph: {areas:?}");
        assert!(covers(TILE - 4, TILE - 4), "above the baseline: {areas:?}");
    }
}
