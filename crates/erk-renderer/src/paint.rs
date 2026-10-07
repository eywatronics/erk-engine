//! Painting a display list: with vello_cpu into a pixmap (the reference,
//! the source of golden images), or with vello_hybrid on the GPU (gpu.rs).
//! Both take the same calls; [`Canvas`] is the part of them a display list
//! needs.

use std::sync::Arc;

use vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vello_cpu::peniko::{Color, Extend, Fill, Gradient as GradientPaint};
use vello_cpu::{
    Glyph, Image as VelloImage, ImageSource, Level, Pixmap, RenderContext, RenderSettings,
    Resources,
};

use crate::color::Rgba;
use crate::display::{DisplayItem, DisplayList, Frame, GlyphRun, Gradient, GradientShape, Radii};
use crate::list::ImageId;
use crate::tables::Tables;

/// What painting a display list asks of a rasterizer.
pub(crate) trait Canvas {
    fn set_transform(&mut self, transform: Affine);
    fn set_color(&mut self, color: Rgba);
    /// Paint with image `id`, `image`, repeated along an axis where
    /// `repeat` says so.
    fn set_image(&mut self, id: ImageId, image: &Arc<Pixmap>, repeat: (bool, bool));
    fn set_gradient(&mut self, gradient: GradientPaint);
    fn set_paint_transform(&mut self, transform: Affine);
    fn reset_paint_transform(&mut self);
    fn set_fill_rule(&mut self, rule: Fill);
    fn fill_rect(&mut self, rect: &Rect);
    fn fill_path(&mut self, path: &BezPath);
    fn fill_blurred_rounded_rect(&mut self, rect: &Rect, radius: f32, std_dev: f32);
    fn push_clip_layer(&mut self, path: &BezPath);
    fn push_opacity_layer(&mut self, opacity: f32);
    fn pop_layer(&mut self);
    fn glyphs(&mut self, run: &GlyphRun, font: &parley::FontData);
}

/// The image paint both rasterizers take, around a source each makes.
pub(crate) fn image_paint(source: ImageSource, repeat: (bool, bool)) -> VelloImage {
    use vello_cpu::peniko::{ImageQuality, ImageSampler};
    let extend = |repeat: bool| if repeat { Extend::Repeat } else { Extend::Pad };
    VelloImage {
        image: source,
        sampler: ImageSampler {
            x_extend: extend(repeat.0),
            y_extend: extend(repeat.1),
            quality: ImageQuality::Medium,
            alpha: 1.0,
        },
    }
}

/// The gradient paint both rasterizers take, and the paint transform that
/// stretches a radial gradient's circle into its ellipse.
pub(crate) fn gradient_paint(gradient: &Gradient) -> (GradientPaint, Affine) {
    let point = |(x, y): (f32, f32)| Point::new(f64::from(x), f64::from(y));
    let (paint, shape) = match gradient.shape {
        GradientShape::Linear { start, end } => (
            GradientPaint::new_linear(point(start), point(end)),
            Affine::IDENTITY,
        ),
        GradientShape::Radial {
            center,
            radii,
            inner,
        } => {
            let c = point(center);
            let around = Vec2::new(c.x, c.y);
            (
                GradientPaint::new_two_point_radial(c, radii.0 * inner, c, radii.0),
                Affine::translate(around)
                    * Affine::scale_non_uniform(1.0, f64::from(radii.1 / radii.0))
                    * Affine::translate(-around),
            )
        }
    };
    let stops: Vec<(f32, Color)> = gradient
        .stops
        .iter()
        .map(|stop| (stop.offset, color(stop.color)))
        .collect();
    let extend = if gradient.repeating {
        Extend::Repeat
    } else {
        Extend::Pad
    };
    (
        paint.with_extend(extend).with_stops(stops.as_slice()),
        shape,
    )
}

/// The most copies of a tiled gradient painted in one item: beyond it, a
/// tile far smaller than its box would cost more than it can show.
const MAX_COPIES: usize = 65_536;

/// The copies of a tile at `start`, `size` long, that reach `low..high`:
/// only the first unless it repeats.
fn copies(start: f64, size: f64, low: f64, high: f64, repeat: bool) -> std::ops::Range<i64> {
    if !repeat {
        return 0..1;
    }
    let first = ((low - start) / size).floor() as i64;
    let last = ((high - start) / size).ceil() as i64;
    first..last.max(first)
}

pub(crate) fn glyphs(run: &GlyphRun) -> impl Iterator<Item = Glyph> + Clone + '_ {
    run.glyphs.iter().map(|glyph| Glyph {
        id: glyph.id,
        x: glyph.x,
        y: glyph.y,
    })
}

/// vello_cpu, with the resources its glyph runs cache.
struct Cpu {
    ctx: RenderContext,
    resources: Resources,
}

impl Canvas for Cpu {
    fn set_transform(&mut self, transform: Affine) {
        self.ctx.set_transform(transform);
    }
    fn set_color(&mut self, rgba: Rgba) {
        self.ctx.set_paint(color(rgba));
    }
    fn set_image(&mut self, _: ImageId, image: &Arc<Pixmap>, repeat: (bool, bool)) {
        self.ctx
            .set_paint(image_paint(ImageSource::Pixmap(image.clone()), repeat));
    }
    fn set_gradient(&mut self, gradient: GradientPaint) {
        self.ctx.set_paint(gradient);
    }
    fn set_paint_transform(&mut self, transform: Affine) {
        self.ctx.set_paint_transform(transform);
    }
    fn reset_paint_transform(&mut self) {
        self.ctx.reset_paint_transform();
    }
    fn set_fill_rule(&mut self, rule: Fill) {
        self.ctx.set_fill_rule(rule);
    }
    fn fill_rect(&mut self, rect: &Rect) {
        self.ctx.fill_rect(rect);
    }
    fn fill_path(&mut self, path: &BezPath) {
        self.ctx.fill_path(path);
    }
    fn fill_blurred_rounded_rect(&mut self, rect: &Rect, radius: f32, std_dev: f32) {
        self.ctx
            .fill_blurred_rounded_rect(rect, radius, std_dev, false);
    }
    fn push_clip_layer(&mut self, path: &BezPath) {
        self.ctx.push_clip_layer(path);
    }
    fn push_opacity_layer(&mut self, opacity: f32) {
        self.ctx.push_opacity_layer(opacity);
    }
    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }
    fn glyphs(&mut self, run: &GlyphRun, font: &parley::FontData) {
        self.ctx
            .glyph_run(&mut self.resources, font)
            .font_size(run.size)
            .hint(true)
            .fill_glyphs(glyphs(run));
    }
}

/// Paint `list` into a new `width` × `height` pixmap.
///
/// Rendering uses vello_cpu's scalar fallback, not a SIMD level: levels may
/// round differently, and golden images must come out identical on every
/// machine CI runs on. `Level::baseline()` is not enough, because it is
/// scalar on x86_64 but NEON on aarch64. Speed is the GPU path's job (M2).
/// Paint `list`, in CSS pixels, into a `width` × `height` pixmap of device
/// pixels, `scale` device pixels per CSS pixel. Everything is drawn through
/// one scale transform, so glyphs are rasterised at device resolution.
pub(crate) fn paint(
    list: &DisplayList,
    tables: &Tables,
    width: u16,
    height: u16,
    scale: f32,
) -> Pixmap {
    let settings = RenderSettings {
        level: Level::fallback(),
        num_threads: 0,
    };
    let mut cpu = Cpu {
        ctx: RenderContext::new_with(width, height, settings),
        resources: Resources::new(),
    };
    paint_list(&mut cpu, list, tables, width, height, scale);
    let mut pixmap = Pixmap::new(width, height);
    cpu.ctx.render(&mut pixmap, &mut cpu.resources);
    pixmap
}

/// Paint `list` onto `ctx`, a `width` × `height` target of device pixels,
/// with the faces and images in `tables`. An item whose face or image is
/// not there is left out.
pub(crate) fn paint_list(
    ctx: &mut impl Canvas,
    list: &DisplayList,
    tables: &Tables,
    width: u16,
    height: u16,
    scale: f32,
) {
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
    ctx.set_color([255, 255, 255, 255]);
    ctx.fill_rect(&page);
    ctx.set_color(list.canvas);
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
                ctx.set_color(*fill);
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
                ctx.set_color(*fill);
                ctx.fill_path(&rounded_rect(*frame, radii));
            }
            DisplayItem::Border {
                frame,
                widths,
                colors,
                radii,
            } => border(ctx, *frame, *widths, *colors, radii),
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
                ctx.set_color(*fill);
                ctx.fill_blurred_rounded_rect(&rect(*frame), *radius, blur_parameter(*blur));
                ctx.pop_layer();
            }
            DisplayItem::Image {
                image: id,
                tile,
                repeat,
                area,
                clip,
                clip_radii,
                ..
            } => {
                let Some(image) = tables.image(*id) else {
                    continue;
                };
                let rounded = clip_radii.iter().any(|(x, y)| *x > 0.0 && *y > 0.0);
                if rounded {
                    ctx.push_clip_layer(&rounded_rect(*clip, clip_radii));
                }
                ctx.set_image(*id, image, *repeat);
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
            DisplayItem::Gradient {
                gradient,
                tile,
                repeat,
                area,
                clip,
                clip_radii,
            } => {
                let painted = rect(*area).intersect(rect(*clip));
                if painted.width() <= 0.0
                    || painted.height() <= 0.0
                    || tile.width <= 0.0
                    || tile.height <= 0.0
                {
                    continue;
                }
                let rounded = clip_radii.iter().any(|(x, y)| *x > 0.0 && *y > 0.0);
                if rounded {
                    ctx.push_clip_layer(&rounded_rect(*clip, clip_radii));
                }
                let (paint, shape) = gradient_paint(gradient);
                ctx.set_gradient(paint);
                let one = rect(*tile);
                let (w, h) = (one.width(), one.height());
                let columns = copies(one.x0, w, painted.x0, painted.x1, repeat.0);
                let rows = copies(one.y0, h, painted.y0, painted.y1, repeat.1);
                let mut painted_copies = 0;
                'copies: for row in rows {
                    for column in columns.clone() {
                        if painted_copies == MAX_COPIES {
                            break 'copies;
                        }
                        painted_copies += 1;
                        let offset = Vec2::new(column as f64 * w, row as f64 * h);
                        let copy = (one + offset).intersect(painted);
                        if copy.width() <= 0.0 || copy.height() <= 0.0 {
                            continue;
                        }
                        // The gradient's geometry is the first copy's.
                        ctx.set_paint_transform(Affine::translate(offset) * shape);
                        ctx.fill_rect(&copy);
                    }
                }
                ctx.reset_paint_transform();
                if rounded {
                    ctx.pop_layer();
                }
            }
            DisplayItem::PushOpacity(opacity) => ctx.push_opacity_layer(*opacity),
            DisplayItem::PopOpacity | DisplayItem::PopClip => ctx.pop_layer(),
            DisplayItem::PushClip { frame, radii } => {
                ctx.push_clip_layer(&rounded_rect(*frame, radii));
            }
            DisplayItem::Hit { .. } => {}
            DisplayItem::Highlight(frame) => {
                ctx.set_color(crate::display::HIGHLIGHT);
                ctx.fill_rect(&rect(*frame));
            }
            DisplayItem::Glyphs(run) => {
                if let Some(font) = tables.font(run.font) {
                    ctx.set_color(run.color);
                    ctx.glyphs(run, font);
                }
            }
        }
    }
}

/// A solid border: the ring between the border box and the padding box.
/// One colour fills the ring; several colours each fill their side's
/// trapezoid, from the outer to the inner corner, inside the ring.
fn border(ctx: &mut impl Canvas, frame: Frame, widths: [f32; 4], colors: [Rgba; 4], radii: &Radii) {
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
        ctx.set_color(colors[shown[0]]);
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
        ctx.set_color(colors[side]);
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

pub(crate) fn color([r, g, b, a]: Rgba) -> Color {
    Color::from_rgba8(r, g, b, a)
}
