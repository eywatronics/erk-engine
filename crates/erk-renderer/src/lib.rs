//! Erk Engine renderer: layout, display list and paint.
//!
//! The public surface is deliberately small: HTML goes in, pixels come out,
//! either directly ([`render_html`]) or through the renderer thread
//! ([`spawn`]). DOM, style and layout types stay inside the crate, so the
//! shell cannot come to depend on them before the renderer moves into its
//! own process. The shell only ever uses the thread; `render_html` is for
//! the renderer's own tests. CI checks both
//! (.github/scripts/check-renderer-surface.sh).

mod color;
mod display;
mod layout;
mod messages;
mod paint;
mod text;
mod thread;

pub use messages::{ElementBox, Frame, FromRenderer, ToRenderer};
pub use thread::spawn;

use std::sync::Arc;

use erk_dom::{Document, NodeId, local_name};
use erk_style::StyleEngine;
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::display::DisplayList;
use crate::layout::Layouts;
use crate::text::{EmbeddedFontMetrics, TextEngine};

impl Frame {
    /// The frame encoded as PNG, or `None` for a frame with no pixels (a
    /// minimised window reports 0 × 0; PNG cannot encode that).
    pub fn to_png(&self) -> Option<Vec<u8>> {
        if self.width() == 0 || self.height() == 0 {
            return None;
        }
        let pixels = self
            .rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, a]| PremulRgba8 { r, g, b, a })
            .collect();
        Some(
            Pixmap::from_parts(pixels, self.width(), self.height())
                .into_png()
                .expect("a non-empty in-memory pixmap encodes"),
        )
    }
}

/// Parse, style, lay out and paint `html` in a `width` × `height` viewport
/// of CSS pixels (1 CSS pixel = 1 device pixel in M0).
pub fn render_html(html: &str, width: u16, height: u16) -> Frame {
    let (w, h) = (f32::from(width), f32::from(height));
    let doc = Document::parse_html(html);
    let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics)).style(&doc);
    let mut text = TextEngine::new();
    let layouts = layout::layout(&doc, &styles, &mut text, w, h);
    let list = DisplayList::build(&doc, &styles, &layouts);
    let pixmap = paint::paint(&list, width, height);
    Frame::new(
        width,
        height,
        pixmap.data_as_u8_slice().to_vec(),
        list.dump(),
    )
}

/// The border box of every element of `html`'s body that generates a box,
/// laid out like [`render_html`] would. For the renderer's own tests, which
/// compare the boxes with Chrome's; the inspection queries of M3 replace it.
pub fn element_boxes(html: &str, width: u16, height: u16) -> Vec<ElementBox> {
    let (w, h) = (f32::from(width), f32::from(height));
    let doc = Document::parse_html(html);
    let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics)).style(&doc);
    let layouts = layout::layout(&doc, &styles, &mut TextEngine::new(), w, h);
    let mut boxes = Vec::new();
    let mut index = 0;
    collect_boxes(
        &doc,
        &layouts,
        doc.root(),
        (0.0, 0.0),
        false,
        &mut index,
        &mut boxes,
    );
    boxes
}

/// Walk the tree in document order, adding each box's offset to its
/// parent's position. Counting starts at the body.
fn collect_boxes(
    doc: &Document,
    layouts: &Layouts,
    id: NodeId,
    origin: (f32, f32),
    in_body: bool,
    index: &mut usize,
    out: &mut Vec<ElementBox>,
) {
    let layout = layouts.get(id);
    let here = layout.map_or(origin, |layout| {
        (origin.0 + layout.location.x, origin.1 + layout.location.y)
    });
    let element = doc.node(id).and_then(|node| node.as_element());
    let counting = in_body || element.is_some_and(|e| e.name.local == local_name!("body"));
    if let Some(element) = element
        && counting
    {
        if let Some(layout) = layout {
            out.push(ElementBox {
                index: *index,
                tag: element.name.local.to_string(),
                x: here.0,
                y: here.1,
                width: layout.size.width,
                height: layout.size.height,
            });
        }
        *index += 1;
    }
    for child in doc.children(id) {
        collect_boxes(doc, layouts, child, here, counting, index, out);
    }
}
