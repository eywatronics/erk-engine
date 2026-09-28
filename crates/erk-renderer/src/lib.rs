//! Erk Engine renderer: layout, display list and paint.
//!
//! The public surface is deliberately small: HTML goes in, pixels come out,
//! either directly ([`render_html`]) or through the renderer thread
//! ([`spawn`]). DOM, style and layout types stay inside the crate, so the
//! shell cannot come to depend on them before the renderer moves into its
//! own process.

mod color;
mod display;
mod layout;
mod paint;
mod text;
mod thread;

pub use thread::{FromRenderer, ToRenderer, spawn};

use std::sync::Arc;

use erk_dom::Document;
use erk_style::StyleEngine;
use vello_cpu::Pixmap;

use crate::display::DisplayList;
use crate::text::{EmbeddedFontMetrics, TextEngine};

/// A rendered page.
pub struct Frame {
    pixmap: Pixmap,
    display_list: String,
}

impl Frame {
    pub fn width(&self) -> u16 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u16 {
        self.pixmap.height()
    }

    /// Pixels as premultiplied RGBA8, row by row.
    pub fn rgba(&self) -> &[u8] {
        self.pixmap.data_as_u8_slice()
    }

    /// The frame encoded as PNG, or `None` for a frame with no pixels (a
    /// minimised window reports 0 × 0; PNG cannot encode that).
    pub fn to_png(&self) -> Option<Vec<u8>> {
        if self.width() == 0 || self.height() == 0 {
            return None;
        }
        Some(
            self.pixmap
                .clone()
                .into_png()
                .expect("a non-empty in-memory pixmap encodes"),
        )
    }

    /// The display list the frame was painted from, as text.
    pub fn display_list(&self) -> &str {
        &self.display_list
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
    Frame {
        pixmap: paint::paint(&list, width, height),
        display_list: list.dump(),
    }
}
