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

use erk_dom::{Document, ElementData, NodeId, local_name};
use erk_style::style::computed_values::visibility::T as Visibility;
use erk_style::style::values::computed::Display;
use erk_style::{Interaction, StyleEngine, Styles};

use crate::display::{DisplayItem, DisplayList, Frame as Rect};
use crate::layout;
use crate::messages::{
    Event, EventKind, Frame, Key, KeyInput, KeyState, Modifiers, PointerButton, PointerInput,
    PointerKind, ResourceRequest,
};
use crate::paint;
use crate::resources::Resources;
use crate::text::{EmbeddedFontMetrics, TextEngine};

pub(crate) struct Page {
    doc: Document,
    /// The last frame's hit regions, in paint order.
    hits: Vec<(NodeId, Rect)>,
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
}

impl Page {
    pub(crate) fn parse(html: &str) -> Self {
        Self {
            doc: Document::parse_html(html),
            hits: Vec::new(),
            styles: Styles::default(),
            hover: None,
            pressed: None,
            focus: None,
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
        let interaction = self.interaction();
        let doc = &self.doc;
        let styles = StyleEngine::with_font_metrics(w, h, Arc::new(EmbeddedFontMetrics))
            .with_device_scale(scale)
            .style_with(doc, &interaction);
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
        self.styles = styles;
        (frame, requests)
    }

    /// Whether what the user is doing now looks different from `before`
    /// under the last frame's styles.
    pub(crate) fn shows_change_from(&self, before: &Interaction) -> bool {
        self.styles.react_to(before, &self.interaction())
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
