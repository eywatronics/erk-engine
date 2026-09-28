//! Painting a display list with vello_cpu.

use vello_cpu::kurbo::Rect;
use vello_cpu::peniko::Color;
use vello_cpu::{Glyph, Level, Pixmap, RenderContext, RenderSettings, Resources};

use crate::color::Rgba;
use crate::display::{DisplayItem, DisplayList};

/// Paint `list` into a new `width` × `height` pixmap.
///
/// Rendering uses vello_cpu's scalar fallback, not a SIMD level: levels may
/// round differently, and golden images must come out identical on every
/// machine CI runs on. `Level::baseline()` is not enough, because it is
/// scalar on x86_64 but NEON on aarch64. Speed is the GPU path's job (M2).
pub(crate) fn paint(list: &DisplayList, width: u16, height: u16) -> Pixmap {
    let settings = RenderSettings {
        level: Level::fallback(),
        num_threads: 0,
    };
    let mut ctx = RenderContext::new_with(width, height, settings);
    let mut resources = Resources::new();

    // The canvas colour is laid over white, as browsers do: a translucent
    // root background must not make the frame itself translucent.
    let page = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
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

fn color([r, g, b, a]: Rgba) -> Color {
    Color::from_rgba8(r, g, b, a)
}
