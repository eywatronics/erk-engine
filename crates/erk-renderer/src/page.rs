//! A document that lives between frames (M2). It is parsed once, when it is
//! loaded, and every frame styles, lays out and paints it again: the element
//! state input brings (`:hover`, `:focus`), scroll positions and the host's
//! changes need a document that outlasts a frame. Every frame is a full
//! recompute on purpose; incremental work is M5's.
//!
//! Pointer input is answered from the last frame: its hit regions, in paint
//! order, say which node is topmost at a point.

use std::sync::Arc;

use erk_dom::{Document, NodeId, local_name};
use erk_style::StyleEngine;

use crate::display::{DisplayItem, DisplayList, Frame as Rect};
use crate::layout;
use crate::messages::{
    Event, EventKind, Frame, PointerButton, PointerInput, PointerKind, ResourceRequest,
};
use crate::paint;
use crate::resources::Resources;
use crate::text::{EmbeddedFontMetrics, TextEngine};

pub(crate) struct Page {
    doc: Document,
    /// The last frame's hit regions, in paint order.
    hits: Vec<(NodeId, Rect)>,
    /// The node the primary button went down on, until it comes up.
    pressed: Option<NodeId>,
    /// The node the developer tools highlight.
    highlight: Option<NodeId>,
    /// The last frame's viewport, in CSS pixels.
    viewport: (f32, f32),
}

impl Page {
    pub(crate) fn parse(html: &str) -> Self {
        Self {
            doc: Document::parse_html(html),
            hits: Vec::new(),
            pressed: None,
            highlight: None,
            viewport: (0.0, 0.0),
        }
    }

    /// Style, lay out and paint the page into a `width` × `height` frame of
    /// device pixels with the resources that have arrived; also return
    /// requests for the URLs it names that were not known before.
    pub(crate) fn render(
        &mut self,
        width: u16,
        height: u16,
        scale: f32,
        resources: &mut Resources,
    ) -> (Frame, Vec<ResourceRequest>) {
        let scale = crate::device_scale(scale);
        // The viewport in CSS pixels.
        let (w, h) = (f32::from(width) / scale, f32::from(height) / scale);
        self.viewport = (w, h);
        let doc = &self.doc;
        let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics))
            .with_device_scale(scale)
            .style(doc);
        let requests = resources.requests(doc, &styles);
        let mut text = TextEngine::with_fonts(resources.fonts());
        let layouts = layout::layout(doc, &styles, resources, &mut text, w, h);
        let mut list = DisplayList::build(doc, &styles, &layouts, resources);
        self.hits = list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Hit { node, frame } => Some((*node, *frame)),
                _ => None,
            })
            .collect();
        if let Some(node) = self.highlight {
            list.items.extend(
                self.hits
                    .iter()
                    .filter(|(hit, _)| *hit == node)
                    .map(|(_, frame)| DisplayItem::Highlight(*frame)),
            );
        }
        let pixmap = paint::paint(&list, width, height, scale);
        let frame = Frame::new(
            width,
            height,
            pixmap.data_as_u8_slice().to_vec(),
            list.dump(),
        );
        (frame, requests)
    }

    /// The topmost node that takes pointer input at `x`, `y` (CSS pixels) in
    /// the last frame. A point in the viewport that no box covers is the
    /// root element's, as in browsers: the canvas belongs to it.
    pub(crate) fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|(_, frame)| {
                x >= frame.x
                    && y >= frame.y
                    && x < frame.x + frame.width
                    && y < frame.y + frame.height
            })
            .map(|(node, _)| *node);
        let in_viewport = x >= 0.0 && y >= 0.0 && x < self.viewport.0 && y < self.viewport.1;
        hit.or_else(|| {
            in_viewport
                .then(|| {
                    self.doc.children(self.doc.root()).find(|child| {
                        self.doc
                            .node(*child)
                            .is_some_and(|n| n.as_element().is_some())
                    })
                })
                .flatten()
        })
    }

    /// Pointer input: a press and a release of the primary button make a
    /// click on the deepest node both are in (UI Events §3.5, as browsers
    /// do when the pointer moves between the two).
    pub(crate) fn pointer(&mut self, input: &PointerInput) -> Vec<Event> {
        let target = self.hit_test(input.x, input.y);
        match (input.kind, input.button) {
            (PointerKind::Down, PointerButton::Primary) => {
                self.pressed = target;
                Vec::new()
            }
            (PointerKind::Up, PointerButton::Primary) => {
                let (Some(down), Some(up)) = (self.pressed.take(), target) else {
                    return Vec::new();
                };
                let Some(clicked) = self.common_ancestor(down, up) else {
                    return Vec::new();
                };
                vec![Event {
                    kind: EventKind::Click,
                    target: clicked.to_bits(),
                    path: self
                        .element_path(clicked)
                        .into_iter()
                        .map(NodeId::to_bits)
                        .collect(),
                    x: input.x,
                    y: input.y,
                    modifiers: input.modifiers,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// `node` and its ancestor elements, up to the root element.
    fn element_path(&self, node: NodeId) -> Vec<NodeId> {
        let mut path = Vec::new();
        let mut current = Some(node);
        while let Some(id) = current {
            let Some(found) = self.doc.node(id) else {
                break;
            };
            if found.as_element().is_some() {
                path.push(id);
            }
            current = found.parent();
        }
        path
    }

    fn common_ancestor(&self, a: NodeId, b: NodeId) -> Option<NodeId> {
        let ancestors_of_b = self.element_path(b);
        self.element_path(a)
            .into_iter()
            .find(|node| ancestors_of_b.contains(node))
    }

    /// Draw the developer tools' highlight over `node`'s boxes from the next
    /// frame on; a node that is not in the document highlights nothing.
    pub(crate) fn set_highlight(&mut self, node: Option<u64>) {
        self.highlight = node
            .and_then(NodeId::from_bits)
            .filter(|id| self.doc.node(*id).is_some());
    }

    /// The first element matching `selector` in document order. Only `#id`
    /// is understood for now.
    pub(crate) fn query(&self, selector: &str) -> Option<NodeId> {
        let id = selector.trim().strip_prefix('#')?;
        if id.is_empty() {
            return None;
        }
        let mut stack = vec![self.doc.root()];
        while let Some(node) = stack.pop() {
            if self
                .doc
                .node(node)
                .and_then(|found| found.as_element())
                .and_then(|element| element.attr(&local_name!("id")))
                == Some(id)
            {
                return Some(node);
            }
            let mut children: Vec<_> = self.doc.children(node).collect();
            children.reverse();
            stack.extend(children);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<body style="margin: 0"><p style="background: #cde">Erk <b>sayfayı</b> bir kez okur.</p><div style="width: 50%; height: 20px; background: red"></div>"#;

    #[test]
    fn a_page_paints_every_frame_as_a_fresh_parse_would() {
        let mut page = Page::parse(HTML);
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
