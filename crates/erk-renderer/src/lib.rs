//! Erk Engine renderer: layout, display list and paint.
//!
//! The public surface is deliberately small: HTML goes in, pixels come out,
//! either directly ([`render_html`]) or through the renderer thread
//! ([`spawn`]). DOM, style and layout types stay inside the crate, so the
//! shell cannot come to depend on them before the renderer moves into its
//! own process. The shell only ever uses the thread; `render_html` is for
//! the renderer's own tests. CI checks both
//! (.github/scripts/check-renderer-surface.sh).

mod case;
mod color;
mod display;
mod fonts;
#[cfg(feature = "gpu")]
mod gpu;
mod layout;
mod messages;
mod page;
mod paint;
mod resources;
mod scroll;
mod text;
mod thread;

#[cfg(feature = "gpu")]
pub use gpu::Window;
pub use messages::{
    Cursor, ElementBox, Event, EventKind, FontCatalog, Frame, FromRenderer, GenericFamilies, Key,
    KeyInput, KeyState, Modifiers, PointerButton, PointerInput, PointerKind, Raster, ResourceKind,
    ResourceRequest, ResourceResponse, ScriptFallback, Status, TextBox, ToRenderer,
};
pub use thread::spawn;
#[cfg(feature = "gpu")]
pub use thread::spawn_on_window;

use std::sync::Arc;

use erk_dom::{Document, NodeId, local_name};
use erk_style::StyleEngine;
use vello_cpu::Pixmap;
use vello_cpu::color::PremulRgba8;

use crate::layout::Layouts;
use crate::page::Page;
use crate::resources::Resources;
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
/// at one device pixel per CSS pixel. No resource is loaded: images render
/// as missing.
pub fn render_html(html: &str, width: u16, height: u16) -> Frame {
    render_document(html, width, height, 1.0, &mut Resources::default()).0
}

/// Like [`render_html`], answering the document's resource requests with
/// `provide` before painting: the synchronous form of the host's resource
/// callback (p1-contract §6). `None` means the resource does not exist.
pub fn render_html_with_resources(
    html: &str,
    width: u16,
    height: u16,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> Frame {
    render_html_at_scale(html, width, height, 1.0, provide)
}

/// Like [`render_html_with_resources`] on a screen with `scale` device
/// pixels per CSS pixel: the frame is `width` × `height` device pixels, the
/// page is laid out in a viewport of `width / scale` × `height / scale` CSS
/// pixels and painted at device resolution. A scale that is not a positive
/// number is taken as 1.
pub fn render_html_at_scale(
    html: &str,
    width: u16,
    height: u16,
    scale: f32,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> Frame {
    let mut resources = Resources::default();
    let (frame, requests) = render_document(html, width, height, scale, &mut resources);
    if requests.is_empty() {
        return frame;
    }
    answer(&mut resources, &requests, provide);
    render_document(html, width, height, scale, &mut resources).0
}

/// `scale` if it is a usable number of device pixels per CSS pixel, else 1.
/// Beyond 1/64 and 64 the page would be laid out in a viewport of thousands
/// of CSS pixels per device pixel, or the other way round.
pub(crate) fn device_scale(scale: f32) -> f32 {
    if (1.0 / 64.0..=64.0).contains(&scale) {
        scale
    } else {
        1.0
    }
}

/// Answer `requests` with `provide`, each response under its request's id.
fn answer(
    resources: &mut Resources,
    requests: &[ResourceRequest],
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) {
    for request in requests {
        match provide(request) {
            Some(response) => resources.complete(&ResourceResponse {
                id: request.id,
                ..response
            }),
            None => resources.missing(request.id),
        }
    }
}

/// For the measurements (examples/measure.rs) only: lay out `html` once
/// at `width` × `height` and paint its display list `runs` times, with
/// vello_hybrid offscreen when `gpu`, else with vello_cpu, calling
/// `frame_done` after each frame is finished (on the GPU, finished there).
/// The core reads no clock; the caller times the calls. Returns what
/// painted: the GPU adapter, or "vello_cpu".
#[doc(hidden)]
pub fn paint_repeatedly(
    html: &str,
    width: u16,
    height: u16,
    runs: usize,
    gpu: bool,
    frame_done: &mut dyn FnMut(),
) -> Result<String, String> {
    let mut page = Page::parse(html);
    let (list, _) = page.prepare(width, height, 1.0, &mut Resources::default());
    if gpu {
        #[cfg(feature = "gpu")]
        {
            let mut gpu = gpu::Gpu::offscreen(wgpu::Backends::all())?;
            for _ in 0..runs {
                gpu.render(&list, width, height, 1.0)?;
                frame_done();
            }
            return Ok(gpu.adapter.clone());
        }
        #[cfg(not(feature = "gpu"))]
        return Err("built without the gpu feature".to_owned());
    }
    for _ in 0..runs {
        std::hint::black_box(paint::paint(&list, width, height, 1.0));
        frame_done();
    }
    Ok("vello_cpu".to_owned())
}

/// Parse, style, lay out and paint `html` with the resources that have
/// arrived; also return requests for the URLs it names that were not
/// known before.
pub(crate) fn render_document(
    html: &str,
    width: u16,
    height: u16,
    scale: f32,
    resources: &mut Resources,
) -> (Frame, Vec<ResourceRequest>) {
    Page::parse(html).render(width, height, scale, resources)
}

/// The border box of every element of `html`'s body that generates a box,
/// laid out like [`render_html_with_resources`] would. For the renderer's
/// own tests, which compare the boxes with Chrome's; the inspection queries
/// of M3 replace it.
pub fn element_boxes(
    html: &str,
    width: u16,
    height: u16,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> Vec<ElementBox> {
    let (w, h) = (f32::from(width), f32::from(height));
    let doc = Document::parse_html(html);
    let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics)).style(&doc);
    let mut resources = Resources::default();
    let requests = resources.requests(&doc, &styles);
    answer(&mut resources, &requests, provide);
    let layouts = layout::layout(
        &doc,
        &styles,
        &resources,
        &mut TextEngine::with_fonts(resources.fonts()),
        w,
        h,
    );
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

/// Where each text node of `html`'s body lies, line by line, laid out
/// like [`render_html_with_resources`] would. For the renderer's own tests,
/// which compare the text with Chrome's; the inspection queries of M3
/// replace it.
pub fn text_boxes(
    html: &str,
    width: u16,
    height: u16,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> Vec<TextBox> {
    let (w, h) = (f32::from(width), f32::from(height));
    let doc = Document::parse_html(html);
    let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics)).style(&doc);
    let mut resources = Resources::default();
    let requests = resources.requests(&doc, &styles);
    answer(&mut resources, &requests, provide);
    let layouts = layout::layout(
        &doc,
        &styles,
        &resources,
        &mut TextEngine::with_fonts(resources.fonts()),
        w,
        h,
    );
    let scrolling =
        scroll::Scrolling::new(&doc, &styles, &layouts, (w, h), &scroll::Offsets::new());
    let list = display::DisplayList::build(&doc, &styles, &layouts, &resources, &scrolling, &[]);
    // Each counted text node's position in document order, by arena index.
    let mut order = std::collections::HashMap::new();
    let mut stack = vec![(doc.root(), false)];
    while let Some((id, in_body)) = stack.pop() {
        let node = doc.node(id);
        let element = node.and_then(|node| node.as_element());
        if element.is_some_and(|element| {
            matches!(element.name.local.as_ref(), "script" | "style" | "template")
        }) {
            continue;
        }
        let in_body =
            in_body || element.is_some_and(|element| element.name.local == local_name!("body"));
        if in_body
            && let Some(text) = node.and_then(|node| node.as_text())
            && !text.chars().all(char::is_whitespace)
        {
            let next = order.len();
            order.insert(id.index() as usize, next);
        }
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children.into_iter().map(|child| (child, in_body)));
    }
    list.text
        .iter()
        .filter_map(|fragment| {
            Some(TextBox {
                index: *order.get(&(fragment.node.index() as usize))?,
                x: fragment.x,
                y: fragment.y,
                width: fragment.width,
                height: fragment.height,
            })
        })
        .collect()
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
