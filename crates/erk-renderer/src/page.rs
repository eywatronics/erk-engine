//! A document that lives between frames (M2). It is parsed once, when it is
//! loaded, and every frame styles, lays out and paints it again: the element
//! state input brings (`:hover`, `:focus`), scroll positions and the host's
//! changes need a document that outlasts a frame. Every frame is a full
//! recompute on purpose; incremental work is M5's.
//!
//! Pointer input is answered from the last frame: its hit regions, in paint
//! order, say which node is topmost at a point. What the user does (the
//! element under the pointer, the one pressed, the one focused) is kept here
//! and styled into the next frame.

use std::sync::Arc;

use erk_dom::{Document, ElementData, NodeData, NodeId, local_name};
use erk_style::style::computed_values::visibility::T as Visibility;
use erk_style::style::values::computed::Display;
use erk_style::{Interaction, StyleEngine, Styles};

use crate::display::{DisplayItem, DisplayList, Frame as Rect};
use crate::layout;
use crate::messages::{
    Cursor, Event, EventKind, Frame, Key, KeyInput, KeyState, Modifiers, PointerButton,
    PointerInput, PointerKind, ResourceRequest, Status,
};
use crate::paint;
use crate::resources::Resources;
use crate::scroll::{Offsets, Scrolling};
use crate::tables::Tables;
use crate::text::{EmbeddedFontMetrics, TextEngine};

pub(crate) struct Page {
    doc: Document,
    /// The last frame's hit regions, in paint order, and whether each is a
    /// line of text.
    hits: Vec<(NodeId, Rect, bool)>,
    /// Where the pointer is, while it is over the page.
    pointer_at: Option<(f32, f32)>,
    /// The last frame's styles: which elements are shown, for the focus.
    styles: Styles,
    /// The node under the pointer.
    hover: Option<NodeId>,
    /// The node the primary button went down on, until it comes up.
    pressed: Option<NodeId>,
    /// The element that has the keyboard focus.
    focus: Option<NodeId>,
    /// The node the developer tools highlight.
    highlight: Option<NodeId>,
    /// The last frame's viewport, in CSS pixels.
    viewport: (f32, f32),
    /// How far the user has scrolled each scroll container.
    offsets: Offsets,
    /// The last frame's scroll containers the user can scroll, innermost
    /// last, with how far each can go.
    scrollers: Vec<Scroller>,
}

/// A scroll container of the last frame.
struct Scroller {
    node: NodeId,
    max: (f32, f32),
}

impl Page {
    pub(crate) fn parse(html: &str) -> Self {
        Self::with_document(Document::parse_html(html))
    }

    /// Show `html` instead, in the same arena: every id of the document
    /// before is stale from now on (p1-contract §2). What the user was
    /// doing with the old document goes with it.
    pub(crate) fn load(&mut self, html: &str) {
        let mut doc = std::mem::take(&mut self.doc);
        doc.load_html(html);
        *self = Self::with_document(doc);
    }

    fn with_document(doc: Document) -> Self {
        Self {
            doc,
            hits: Vec::new(),
            pointer_at: None,
            styles: Styles::default(),
            hover: None,
            pressed: None,
            focus: None,
            highlight: None,
            viewport: (0.0, 0.0),
            offsets: Offsets::new(),
            scrollers: Vec::new(),
        }
    }

    /// Style, lay out and paint the page into a `width` × `height` frame of
    /// device pixels with the resources that have arrived, bringing the
    /// raster's `tables` up to date for it first; also return
    /// requests for the URLs it names that were not known before.
    pub(crate) fn render(
        &mut self,
        width: u16,
        height: u16,
        scale: f32,
        resources: &mut Resources,
        tables: &mut Tables,
    ) -> (Frame, Vec<ResourceRequest>) {
        let (list, requests) = self.prepare(width, height, scale, resources);
        tables.apply(resources.table_updates(&list));
        let scale = crate::device_scale(scale);
        let pixmap = paint::paint(&list, tables, width, height, scale);
        let frame = Frame::new(
            width,
            height,
            pixmap.data_as_u8_slice().to_vec(),
            list.dump(),
        );
        (frame, requests)
    }

    /// Style and lay out the page for a `width` × `height` frame of device
    /// pixels and build its display list, as [`Page::render`] paints it.
    pub(crate) fn prepare(
        &mut self,
        width: u16,
        height: u16,
        scale: f32,
        resources: &mut Resources,
    ) -> (DisplayList, Vec<ResourceRequest>) {
        let scale = crate::device_scale(scale);
        // The viewport in CSS pixels.
        let (w, h) = (f32::from(width) / scale, f32::from(height) / scale);
        self.viewport = (w, h);
        let interaction = self.interaction();
        let doc = &self.doc;
        let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics))
            .with_device_scale(scale)
            .style_with(doc, &interaction);
        let requests = resources.requests(doc, &styles);
        let mut text = TextEngine::with_fonts(resources.fonts());
        let layouts = layout::layout(doc, &styles, resources, &mut text, w, h);
        let scrolling = Scrolling::new(doc, &styles, &layouts, (w, h), &self.offsets);
        let bars = self.bars(self.hover);
        let (mut list, _) =
            DisplayList::build(doc, &styles, &layouts, resources, &scrolling, &bars);
        // What the user scrolled, as far as it still goes.
        self.offsets = scrolling
            .scopes
            .iter()
            .filter(|scope| scope.offset != (0.0, 0.0))
            .map(|scope| (scope.node, scope.offset))
            .collect();
        self.scrollers = scrolling
            .scopes
            .iter()
            .filter(|scope| scope.user && (scope.max.0 > 0.0 || scope.max.1 > 0.0))
            .map(|scope| Scroller {
                node: scope.node,
                max: scope.max,
            })
            .collect();
        self.hits = hits(&list.items);
        if let Some(node) = self.highlight {
            list.items.extend(
                self.hits
                    .iter()
                    .filter(|(hit, _, _)| *hit == node)
                    .map(|(_, frame, _)| DisplayItem::Highlight(*frame)),
            );
        }
        self.styles = styles;
        (list, requests)
    }

    /// Whether what the user is doing now looks different from `before`
    /// under the last frame's styles, or shows other scroll bars.
    pub(crate) fn shows_change_from(&self, before: &Interaction) -> bool {
        self.styles.react_to(before, &self.interaction())
            || self.bars(before.hover) != self.bars(self.hover)
    }

    /// The scroll containers whose scroll bars show while the pointer is
    /// over `hover`: those it is in, the document's among them.
    fn bars(&self, hover: Option<NodeId>) -> Vec<NodeId> {
        let Some(hover) = hover else {
            return Vec::new();
        };
        let mut path = self.element_path(hover);
        path.push(self.doc.root());
        self.scrollers
            .iter()
            .map(|scroller| scroller.node)
            .filter(|node| path.contains(node))
            .collect()
    }

    /// The wheel turned by `dx`, `dy` CSS pixels (positive: towards the
    /// end) at `x`, `y`: the innermost scroll container there that can move
    /// that way scrolls, and what it cannot take goes to the one around it,
    /// up to the document. Whether anything scrolled.
    pub(crate) fn wheel(&mut self, (dx, dy): (f32, f32), (x, y): (f32, f32)) -> bool {
        let mut path = self
            .hit_test(x, y)
            .map(|target| self.element_path(target))
            .unwrap_or_default();
        path.push(self.doc.root());
        let mut left = (dx, dy);
        let mut moved = false;
        for node in path {
            let Some(scroller) = self.scrollers.iter().find(|s| s.node == node) else {
                continue;
            };
            let offset = self.offsets.get(&node).copied().unwrap_or_default();
            let next = (
                (offset.0 + left.0).clamp(0.0, scroller.max.0),
                (offset.1 + left.1).clamp(0.0, scroller.max.1),
            );
            left = (left.0 - (next.0 - offset.0), left.1 - (next.1 - offset.1));
            if next != offset {
                moved = true;
                self.offsets.insert(node, next);
            }
            if left == (0.0, 0.0) {
                break;
            }
        }
        moved
    }

    /// What the user is doing with the page, as the next frame styles it.
    pub(crate) fn interaction(&self) -> Interaction {
        Interaction {
            hover: self.hover,
            active: self.pressed,
            focus: self.focus,
        }
    }

    /// The topmost node that takes pointer input at `x`, `y` (CSS pixels) in
    /// the last frame. A point in the viewport that no box covers is the
    /// root element's, as in browsers: the canvas belongs to it.
    pub(crate) fn hit_test(&self, x: f32, y: f32) -> Option<NodeId> {
        self.hit(x, y).map(|(node, _)| node)
    }

    /// Like [`Page::hit_test`], and whether the point is on a line of text.
    fn hit(&self, x: f32, y: f32) -> Option<(NodeId, bool)> {
        let hit = self
            .hits
            .iter()
            .rev()
            .find(|(_, frame, _)| {
                x >= frame.x
                    && y >= frame.y
                    && x < frame.x + frame.width
                    && y < frame.y + frame.height
            })
            .map(|(node, _, text)| (*node, *text));
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
                .map(|root| (root, false))
        })
    }

    /// How the pointer should look where it is: the `cursor` of what it is
    /// over; `auto` is a text cursor over text and the arrow elsewhere.
    pub(crate) fn cursor(&self) -> Cursor {
        use erk_style::style::values::specified::ui::CursorKind as Kind;
        let Some((node, text)) = self.pointer_at.and_then(|(x, y)| self.hit(x, y)) else {
            return Cursor::Default;
        };
        let Some(style) = self.styles.computed(node) else {
            return Cursor::Default;
        };
        match style.get_inherited_ui().cursor.keyword {
            Kind::Auto if text => Cursor::Text,
            Kind::Auto | Kind::Default => Cursor::Default,
            Kind::None => Cursor::None,
            Kind::ContextMenu => Cursor::ContextMenu,
            Kind::Help => Cursor::Help,
            Kind::Pointer => Cursor::Pointer,
            Kind::Progress => Cursor::Progress,
            Kind::Wait => Cursor::Wait,
            Kind::Cell => Cursor::CellSelect,
            Kind::Crosshair => Cursor::Crosshair,
            Kind::Text => Cursor::Text,
            Kind::VerticalText => Cursor::VerticalText,
            Kind::Alias => Cursor::Alias,
            Kind::Copy => Cursor::Copy,
            Kind::Move => Cursor::Move,
            Kind::NoDrop => Cursor::NoDrop,
            Kind::NotAllowed => Cursor::NotAllowed,
            Kind::Grab => Cursor::Grab,
            Kind::Grabbing => Cursor::Grabbing,
            Kind::EResize => Cursor::EResize,
            Kind::NResize => Cursor::NResize,
            Kind::NeResize => Cursor::NeResize,
            Kind::NwResize => Cursor::NwResize,
            Kind::SResize => Cursor::SResize,
            Kind::SeResize => Cursor::SeResize,
            Kind::SwResize => Cursor::SwResize,
            Kind::WResize => Cursor::WResize,
            Kind::EwResize => Cursor::EwResize,
            Kind::NsResize => Cursor::NsResize,
            Kind::NeswResize => Cursor::NeswResize,
            Kind::NwseResize => Cursor::NwseResize,
            Kind::ColResize => Cursor::ColResize,
            Kind::RowResize => Cursor::RowResize,
            Kind::AllScroll => Cursor::AllScroll,
            Kind::ZoomIn => Cursor::ZoomIn,
            Kind::ZoomOut => Cursor::ZoomOut,
        }
    }

    /// Pointer input: the node under the pointer hovers; a press of the
    /// primary button makes it active and moves the focus; a press and a
    /// release make a click on the deepest node both are in (UI Events §3.5,
    /// as browsers do when the pointer moves between the two).
    pub(crate) fn pointer(&mut self, input: &PointerInput) -> Vec<Event> {
        let target = self.hit_test(input.x, input.y);
        self.hover = match input.kind {
            PointerKind::Leave => None,
            _ => target,
        };
        self.pointer_at = match input.kind {
            PointerKind::Leave => None,
            _ => Some((input.x, input.y)),
        };
        match (input.kind, input.button) {
            (PointerKind::Down, PointerButton::Primary) => {
                self.pressed = target;
                // The focus goes to the focusable element pressed, or away
                // when there is none, as in browsers.
                let focus = target.and_then(|target| {
                    self.element_path(target)
                        .into_iter()
                        .find(|node| self.focusable(*node))
                });
                self.move_focus(focus, input.modifiers)
            }
            (PointerKind::Up, PointerButton::Primary) => {
                let (Some(down), Some(up)) = (self.pressed.take(), target) else {
                    return Vec::new();
                };
                let Some(clicked) = self.common_ancestor(down, up) else {
                    return Vec::new();
                };
                vec![self.event(
                    EventKind::Click,
                    clicked,
                    (input.x, input.y),
                    input.modifiers,
                )]
            }
            _ => Vec::new(),
        }
    }

    /// Keyboard input: Tab and Shift+Tab move the focus through the page in
    /// sequential focus order (HTML §6.6.3), wrapping around; Enter on a
    /// focused link or button and Space released on a focused button click
    /// it, as browsers do. Keyboard clicks are at 0, 0.
    pub(crate) fn key(&mut self, input: &KeyInput) -> Vec<Event> {
        let focused = self.focus.filter(|node| self.doc.node(*node).is_some());
        match (&input.key, input.state) {
            (Key::Tab, KeyState::Down) => {
                let order = self.focus_order();
                let Some(last) = order.len().checked_sub(1) else {
                    return Vec::new();
                };
                let at = focused.and_then(|node| order.iter().position(|n| *n == node));
                let next = match (at, input.modifiers.shift) {
                    (None, false) => 0,
                    (None, true) => last,
                    (Some(i), false) => {
                        if i == last {
                            0
                        } else {
                            i + 1
                        }
                    }
                    (Some(i), true) => {
                        if i == 0 {
                            last
                        } else {
                            i - 1
                        }
                    }
                };
                self.move_focus(Some(order[next]), input.modifiers)
            }
            (Key::Enter, KeyState::Down) | (Key::Space, KeyState::Up) => {
                let Some(node) = focused else {
                    return Vec::new();
                };
                let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
                    return Vec::new();
                };
                let name = &element.name.local;
                let activates = *name == local_name!("button")
                    || (input.key == Key::Enter
                        && *name == local_name!("a")
                        && element.attr(&local_name!("href")).is_some());
                if activates {
                    vec![self.event(EventKind::Click, node, (0.0, 0.0), input.modifiers)]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Move the focus to `focus`: a blur event for the element that had it,
    /// then a focus event for the one that has it.
    fn move_focus(&mut self, focus: Option<NodeId>, modifiers: Modifiers) -> Vec<Event> {
        if focus == self.focus {
            return Vec::new();
        }
        let mut events = Vec::new();
        if let Some(old) = self
            .focus
            .take()
            .filter(|node| self.doc.node(*node).is_some())
        {
            events.push(self.event(EventKind::Blur, old, (0.0, 0.0), modifiers));
        }
        self.focus = focus;
        if let Some(new) = focus {
            events.push(self.event(EventKind::Focus, new, (0.0, 0.0), modifiers));
        }
        events
    }

    fn event(
        &self,
        kind: EventKind,
        target: NodeId,
        (x, y): (f32, f32),
        modifiers: Modifiers,
    ) -> Event {
        Event {
            kind,
            target: target.to_bits(),
            path: self
                .element_path(target)
                .into_iter()
                .map(NodeId::to_bits)
                .collect(),
            x,
            y,
            modifiers,
        }
    }

    /// Whether `node` can take the focus (HTML §6.6.2): a link, an enabled
    /// form control or an element with a `tabindex`, shown in the last frame.
    fn focusable(&self, node: NodeId) -> bool {
        let Some(element) = self.doc.node(node).and_then(|found| found.as_element()) else {
            return false;
        };
        let shown = self.styles.computed(node).is_some_and(|style| {
            style.get_box().clone_display() != Display::None
                && style.clone_visibility() == Visibility::Visible
        });
        let name = &element.name.local;
        let control = [
            local_name!("button"),
            local_name!("input"),
            local_name!("select"),
            local_name!("textarea"),
        ]
        .contains(name);
        let focusable = if control {
            element.attr(&local_name!("disabled")).is_none()
                && !(*name == local_name!("input")
                    && element
                        .attr(&local_name!("type"))
                        .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden")))
        } else {
            tabindex(element).is_some()
                || (*name == local_name!("a") && element.attr(&local_name!("href")).is_some())
        };
        shown && focusable
    }

    /// The elements Tab visits, in order: positive `tabindex` values from
    /// the lowest, then the rest in document order; a negative `tabindex`
    /// takes the focus only from the pointer.
    fn focus_order(&self) -> Vec<NodeId> {
        let mut first = Vec::new();
        let mut rest = Vec::new();
        let mut stack = vec![self.doc.root()];
        while let Some(node) = stack.pop() {
            let mut children: Vec<_> = self.doc.children(node).collect();
            children.reverse();
            stack.extend(children);
            if !self.focusable(node) {
                continue;
            }
            let index = self
                .doc
                .node(node)
                .and_then(|found| found.as_element())
                .and_then(tabindex);
            match index {
                Some(index) if index < 0 => {}
                Some(index) if index > 0 => first.push((index, node)),
                _ => rest.push(node),
            }
        }
        first.sort_by_key(|(index, _)| *index);
        first
            .into_iter()
            .map(|(_, node)| node)
            .chain(rest)
            .collect()
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

    /// The first element inside `scope` (the document for `None`) matching
    /// the selector list `selector`, in document order.
    pub(crate) fn query(
        &self,
        scope: Option<u64>,
        selector: &str,
    ) -> Result<Option<NodeId>, Status> {
        let scope = match scope {
            None => self.doc.root(),
            Some(bits) => self.node(bits)?,
        };
        let found = erk_style::query(&self.doc, scope, selector, &self.interaction())
            .map_err(|_| Status::InvalidArgument)?;
        Ok(found.first().copied())
    }

    /// The document node.
    pub(crate) fn root(&self) -> NodeId {
        self.doc.root()
    }

    /// `node`'s text as `textContent` reads it: a text node's own text,
    /// else the text of every text node under it, in document order. The
    /// document node has none (DOM §4.4). Walked without recursion: the
    /// caller's stack may be a UI thread's.
    pub(crate) fn text(&self, node: u64) -> Result<String, Status> {
        let node = self.node(node)?;
        if node == self.doc.root() {
            return Ok(String::new());
        }
        let mut text = String::new();
        let mut stack = vec![node];
        while let Some(id) = stack.pop() {
            match self.doc.node(id).map(|node| &node.data) {
                Some(NodeData::Text(own)) => text.push_str(own),
                Some(NodeData::Element(_) | NodeData::DocumentFragment) => {
                    let mut children: Vec<_> = self.doc.children(id).collect();
                    children.reverse();
                    stack.extend(children);
                }
                _ => {}
            }
        }
        Ok(text)
    }

    /// Set `node`'s text as `textContent` does; the next frame shows it.
    pub(crate) fn set_text(&mut self, node: u64, text: &str) -> Result<(), Status> {
        let node = self.node(node)?;
        self.doc.set_text(node, text).map_err(|_| Status::StaleNode)
    }

    /// The node `bits` names: 0 and other numbers no id has are invalid, an
    /// id whose node is gone is stale.
    fn node(&self, bits: u64) -> Result<NodeId, Status> {
        let id = NodeId::from_bits(bits).ok_or(Status::InvalidArgument)?;
        self.doc.node(id).map(|_| id).ok_or(Status::StaleNode)
    }
}

/// The hit regions of `items` in paint order, each cut to the clips around
/// it; a region clipped away entirely is gone.
fn hits(items: &[DisplayItem]) -> Vec<(NodeId, Rect, bool)> {
    let mut clips: Vec<Rect> = Vec::new();
    let mut hits = Vec::new();
    for item in items {
        match item {
            // A rounded clip is taken as its rectangle: a point in a cut-off
            // corner still finds what is under it.
            DisplayItem::PushClip { frame, .. } => {
                let clip = clips.last().map_or(*frame, |outer| intersect(outer, frame));
                clips.push(clip);
            }
            DisplayItem::PopClip => {
                clips.pop();
            }
            DisplayItem::Hit { node, frame, text } => {
                let frame = clips.last().map_or(*frame, |clip| intersect(clip, frame));
                if frame.width > 0.0
                    && frame.height > 0.0
                    && let Some(node) = NodeId::from_bits(*node)
                {
                    hits.push((node, frame, *text));
                }
            }
            _ => {}
        }
    }
    hits
}

fn intersect(a: &Rect, b: &Rect) -> Rect {
    let (left, top) = (a.x.max(b.x), a.y.max(b.y));
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect {
        x: left,
        y: top,
        width: (right - left).max(0.0),
        height: (bottom - top).max(0.0),
    }
}

/// An element's `tabindex`, read as HTML reads integers: leading white
/// space, a sign, digits, and anything after them ignored.
fn tabindex(element: &ElementData) -> Option<i32> {
    let value = element
        .attr(&local_name!("tabindex"))?
        .trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match value.as_bytes().first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    let magnitude: i64 = digits[..end].parse().ok()?;
    let value = if negative { -magnitude } else { magnitude };
    Some(i32::try_from(value).unwrap_or(if negative { i32::MIN } else { i32::MAX }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<body style="margin: 0"><p style="background: #cde">Erk <b>sayfayı</b> bir kez okur.</p><div style="width: 50%; height: 20px; background: red"></div>"#;

    #[test]
    fn a_page_paints_every_frame_as_a_fresh_parse_would() {
        let mut page = Page::parse(HTML);
        let mut resources = Resources::default();
        let mut tables = Tables::default();
        for (width, height, scale) in [
            (200, 100, 1.0),
            (120, 80, 1.0),
            (240, 160, 2.0),
            (200, 100, 1.0),
        ] {
            let (frame, _) = page.render(width, height, scale, &mut resources, &mut tables);
            let (fresh, _) = Page::parse(HTML).render(
                width,
                height,
                scale,
                &mut Resources::default(),
                &mut Tables::default(),
            );
            assert_eq!(
                frame.display_list(),
                fresh.display_list(),
                "{width}x{height}@{scale}"
            );
            assert!(frame.rgba() == fresh.rgba(), "{width}x{height}@{scale}");
        }
    }
}
