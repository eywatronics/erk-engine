//! A document that lives between frames (M2). It is parsed once, when it is
//! loaded, and every frame styles, lays out and paints it again: the element
//! state input brings (`:hover`, `:focus`), scroll positions and the host's
//! changes need a document that outlasts a frame. Every frame is a full
//! recompute on purpose; incremental work is M5's.

use std::sync::Arc;

use erk_dom::Document;
use erk_style::StyleEngine;

use crate::display::DisplayList;
use crate::layout;
use crate::messages::{Frame, ResourceRequest};
use crate::paint;
use crate::resources::Resources;
use crate::text::{EmbeddedFontMetrics, TextEngine};

pub(crate) struct Page {
    doc: Document,
}

impl Page {
    pub(crate) fn parse(html: &str) -> Self {
        Self {
            doc: Document::parse_html(html),
        }
    }

    /// Style, lay out and paint the page into a `width` × `height` frame of
    /// device pixels with the resources that have arrived; also return
    /// requests for the URLs it names that were not known before.
    pub(crate) fn render(
        &self,
        width: u16,
        height: u16,
        scale: f32,
        resources: &mut Resources,
    ) -> (Frame, Vec<ResourceRequest>) {
        let scale = crate::device_scale(scale);
        // The viewport in CSS pixels.
        let (w, h) = (f32::from(width) / scale, f32::from(height) / scale);
        let doc = &self.doc;
        let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics))
            .with_device_scale(scale)
            .style(doc);
        let requests = resources.requests(doc, &styles);
        let mut text = TextEngine::with_fonts(resources.fonts());
        let layouts = layout::layout(doc, &styles, resources, &mut text, w, h);
        let list = DisplayList::build(doc, &styles, &layouts, resources);
        let pixmap = paint::paint(&list, width, height, scale);
        let frame = Frame::new(
            width,
            height,
            pixmap.data_as_u8_slice().to_vec(),
            list.dump(),
        );
        (frame, requests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<body style="margin: 0"><p style="background: #cde">Erk <b>sayfayı</b> bir kez okur.</p><div style="width: 50%; height: 20px; background: red"></div>"#;

    #[test]
    fn a_page_paints_every_frame_as_a_fresh_parse_would() {
        let page = Page::parse(HTML);
        let mut resources = Resources::default();
        for (width, height, scale) in [
            (200, 100, 1.0),
            (120, 80, 1.0),
            (240, 160, 2.0),
            (200, 100, 1.0),
        ] {
            let (frame, _) = page.render(width, height, scale, &mut resources);
            let (fresh, _) =
                Page::parse(HTML).render(width, height, scale, &mut Resources::default());
            assert_eq!(
                frame.display_list(),
                fresh.display_list(),
                "{width}x{height}@{scale}"
            );
            assert!(frame.rgba() == fresh.rgba(), "{width}x{height}@{scale}");
        }
    }
}
