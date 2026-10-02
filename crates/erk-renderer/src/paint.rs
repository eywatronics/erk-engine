//! Painting a display list with vello_cpu.

use vello_cpu::kurbo::{Affine, BezPath, Point, Rect};
use vello_cpu::peniko::{Color, Extend, Fill, ImageQuality, ImageSampler};
use vello_cpu::{
    Glyph, Image as VelloImage, ImageSource, Level, Pixmap, RenderContext, RenderSettings,
    Resources,
};

use crate::color::Rgba;
use crate::display::{DisplayItem, DisplayList, Frame, Radii};

/// Paint `list` into a new `width` × `height` pixmap.
///
/// Rendering uses vello_cpu's scalar fallback, not a SIMD level: levels may
/// round differently, and golden images must come out identical on every
/// machine CI runs on. `Level::baseline()` is not enough, because it is
/// scalar on x86_64 but NEON on aarch64. Speed is the GPU path's job (M2).
/// Paint `list`, in CSS pixels, into a `width` × `height` pixmap of device
/// pixels, `scale` device pixels per CSS pixel. Everything is drawn through
/// one scale transform, so glyphs are rasterised at device resolution.
pub(crate) fn paint(list: &DisplayList, width: u16, height: u16, scale: f32) -> Pixmap {
    let settings = RenderSettings {
        level: Level::fallback(),
        num_threads: 0,
    };
    let mut ctx = RenderContext::new_with(width, height, settings);
    let mut resources = Resources::new();

    // The canvas colour is laid over white, as browsers do: a translucent
    // root background must not make the frame itself translucent.
    let scale = f64::from(scale);
    ctx.set_transform(Affine::scale(scale));
    // The page in CSS pixels.
    let page = Rect::new(
        0.0,
        0.0,
        f64::from(width) / scale,
        f64::from(height) / scale,
    );
    ctx.set_paint(color([255, 255, 255, 255]));
    ctx.fill_rect(&page);
    ctx.set_paint(color(list.canvas));
    ctx.fill_rect(&page);

    for item in &list.items {
        match item {
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color: fill,
            } => {
                ctx.set_paint(color(*fill));
                let (x, y) = (f64::from(*x), f64::from(*y));
                ctx.fill_rect(&Rect::new(
                    x,
                    y,
                    x + f64::from(*width),
                    y + f64::from(*height),
                ));
            }
            DisplayItem::RoundedRect {
                frame,
                radii,
                color: fill,
            } => {
                ctx.set_paint(color(*fill));
                ctx.fill_path(&rounded_rect(*frame, radii));
            }
            DisplayItem::Border {
                frame,
                widths,
                colors,
                radii,
            } => border(&mut ctx, *frame, *widths, *colors, radii),
            DisplayItem::Shadow {
                frame,
                radius,
                blur,
                color: fill,
                clip,
                clip_radii,
            } => {
                // Outside the casting box only: a page-sized rectangle with
                // the box cut out, by the even-odd rule.
                let mut outside = rounded_rect(*clip, clip_radii);
                outside.extend(rect_path(page));
                ctx.set_fill_rule(Fill::EvenOdd);
                ctx.push_clip_layer(&outside);
                ctx.set_fill_rule(Fill::NonZero);
                ctx.set_paint(color(*fill));
                ctx.fill_blurred_rounded_rect(&rect(*frame), *radius, blur_parameter(*blur), false);
                ctx.pop_layer();
            }
            DisplayItem::Image {
                image,
                tile,
                repeat,
                area,
                clip,
                clip_radii,
            } => {
                let rounded = clip_radii.iter().any(|(x, y)| *x > 0.0 && *y > 0.0);
                if rounded {
                    ctx.push_clip_layer(&rounded_rect(*clip, clip_radii));
                }
                let extend = |repeat: bool| if repeat { Extend::Repeat } else { Extend::Pad };
                ctx.set_paint(VelloImage {
                    image: ImageSource::Pixmap(image.clone()),
                    sampler: ImageSampler {
                        x_extend: extend(repeat.0),
                        y_extend: extend(repeat.1),
                        quality: ImageQuality::Medium,
                        alpha: 1.0,
                    },
                });
                // One copy of the image maps onto the tile.
                ctx.set_paint_transform(
                    Affine::translate((f64::from(tile.x), f64::from(tile.y)))
                        * Affine::scale_non_uniform(
                            f64::from(tile.width) / f64::from(image.width()),
                            f64::from(tile.height) / f64::from(image.height()),
                        ),
                );
                let painted = rect(*area).intersect(rect(*clip));
                if painted.width() > 0.0 && painted.height() > 0.0 {
                    ctx.fill_rect(&painted);
                }
                ctx.reset_paint_transform();
                if rounded {
                    ctx.pop_layer();
                }
            }
            DisplayItem::PushOpacity(opacity) => ctx.push_opacity_layer(*opacity),
            DisplayItem::PopOpacity => ctx.pop_layer(),
            DisplayItem::Glyphs(run) => {
                ctx.set_paint(color(run.color));
                ctx.glyph_run(&mut resources, &run.font)
                    .font_size(run.size)
                    .hint(true)
                    .fill_glyphs(run.glyphs.iter().map(|glyph| Glyph {
                        id: glyph.id,
                        x: glyph.x,
                        y: glyph.y,
                    }));
            }
        }
    }

    let mut pixmap = Pixmap::new(width, height);
    ctx.render(&mut pixmap, &mut resources);
    pixmap
}

/// A solid border: the ring between the border box and the padding box.
/// One colour fills the ring; several colours each fill their side's
/// trapezoid, from the outer to the inner corner, inside the ring.
fn border(
    ctx: &mut RenderContext,
    frame: Frame,
    widths: [f32; 4],
    colors: [Rgba; 4],
    radii: &Radii,
) {
    let [top, right, bottom, left] = widths;
    let inner = Frame {
        x: frame.x + left,
        y: frame.y + top,
        width: (frame.width - left - right).max(0.0),
        height: (frame.height - top - bottom).max(0.0),
    };
    // The padding box's corners curve with what is left of the radius.
    let [tl, tr, br, bl] = *radii;
    let inner_radii = [
        ((tl.0 - left).max(0.0), (tl.1 - top).max(0.0)),
        ((tr.0 - right).max(0.0), (tr.1 - top).max(0.0)),
        ((br.0 - right).max(0.0), (br.1 - bottom).max(0.0)),
        ((bl.0 - left).max(0.0), (bl.1 - bottom).max(0.0)),
    ];
    let mut ring = rounded_rect(frame, radii);
    ring.extend(rounded_rect(inner, &inner_radii));

    let shown: Vec<usize> = (0..4)
        .filter(|side| widths[*side] > 0.0 && colors[*side][3] != 0)
        .collect();
    let single = shown.iter().all(|side| colors[*side] == colors[shown[0]]);
    ctx.set_fill_rule(Fill::EvenOdd);
    if single && shown.len() == 4 {
        ctx.set_paint(color(colors[shown[0]]));
        ctx.fill_path(&ring);
        ctx.set_fill_rule(Fill::NonZero);
        return;
    }
    ctx.push_clip_layer(&ring);
    ctx.set_fill_rule(Fill::NonZero);
    let (x0, y0) = (f64::from(frame.x), f64::from(frame.y));
    let (x1, y1) = (x0 + f64::from(frame.width), y0 + f64::from(frame.height));
    let (ix0, iy0) = (f64::from(inner.x), f64::from(inner.y));
    let (ix1, iy1) = (ix0 + f64::from(inner.width), iy0 + f64::from(inner.height));
    let sides = [
        [(x0, y0), (x1, y0), (ix1, iy0), (ix0, iy0)],
        [(x1, y0), (x1, y1), (ix1, iy1), (ix1, iy0)],
        [(x1, y1), (x0, y1), (ix0, iy1), (ix1, iy1)],
        [(x0, y1), (x0, y0), (ix0, iy0), (ix0, iy1)],
    ];
    for side in shown {
        let [a, b, c, d] = sides[side];
        let mut path = BezPath::new();
        path.move_to(a);
        path.line_to(b);
        path.line_to(c);
        path.line_to(d);
        path.close_path();
        ctx.set_paint(color(colors[side]));
        ctx.fill_path(&path);
    }
    ctx.pop_layer();
}

/// A rectangle with elliptical corners, clockwise. Each quarter ellipse is
/// one cubic Bézier with the usual 0.5523 handle length.
fn rounded_rect(frame: Frame, radii: &Radii) -> BezPath {
    const KAPPA: f64 = 0.552_284_75;
    let (x0, y0) = (f64::from(frame.x), f64::from(frame.y));
    let (x1, y1) = (x0 + f64::from(frame.width), y0 + f64::from(frame.height));
    let r = radii.map(|(x, y)| (f64::from(x), f64::from(y)));
    let [tl, tr, br, bl] = r;
    let mut path = BezPath::new();
    path.move_to((x0 + tl.0, y0));
    path.line_to((x1 - tr.0, y0));
    corner(&mut path, (x1 - tr.0, y0), (x1, y0 + tr.1), (x1, y0), KAPPA);
    path.line_to((x1, y1 - br.1));
    corner(&mut path, (x1, y1 - br.1), (x1 - br.0, y1), (x1, y1), KAPPA);
    path.line_to((x0 + bl.0, y1));
    corner(&mut path, (x0 + bl.0, y1), (x0, y1 - bl.1), (x0, y1), KAPPA);
    path.line_to((x0, y0 + tl.1));
    corner(&mut path, (x0, y0 + tl.1), (x0 + tl.0, y0), (x0, y0), KAPPA);
    path.close_path();
    path
}

/// A quarter ellipse from `from` to `to` bulging towards the box corner
/// `towards`; nothing when the radius is zero.
fn corner(path: &mut BezPath, from: (f64, f64), to: (f64, f64), towards: (f64, f64), kappa: f64) {
    if from == to {
        return;
    }
    let lerp = |a: (f64, f64), b: (f64, f64)| -> Point {
        Point::new(a.0 + (b.0 - a.0) * kappa, a.1 + (b.1 - a.1) * kappa)
    };
    path.curve_to(
        lerp(from, towards),
        lerp(to, towards),
        Point::new(to.0, to.1),
    );
}

/// vello_cpu's blur parameter for a CSS blur radius. CSS makes the blur's
/// standard deviation half the radius (CSS Backgrounds 3 §7.1.1).
/// vello_cpu's parameter measured as σ·√2: given 3 and 5.6, the drawn
/// edges had σ ≈ 2.1 and ≈ 4.0, a ratio of 1/√2 both times. Found by the
/// Chrome reference test: passed σ itself, the shadow fell off a third
/// faster than Chrome's.
fn blur_parameter(radius: f32) -> f32 {
    (radius / 2.0).max(0.0) * std::f32::consts::SQRT_2
}

fn rect_path(rect: Rect) -> BezPath {
    let mut path = BezPath::new();
    path.move_to((rect.x0, rect.y0));
    path.line_to((rect.x1, rect.y0));
    path.line_to((rect.x1, rect.y1));
    path.line_to((rect.x0, rect.y1));
    path.close_path();
    path
}

fn rect(frame: Frame) -> Rect {
    let (x, y) = (f64::from(frame.x), f64::from(frame.y));
    Rect::new(
        x,
        y,
        x + f64::from(frame.width),
        y + f64::from(frame.height),
    )
}

fn color([r, g, b, a]: Rgba) -> Color {
    Color::from_rgba8(r, g, b, a)
}
