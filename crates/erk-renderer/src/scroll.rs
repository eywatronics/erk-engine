//! Clipping and scrolling (M2.3).
//!
//! A box whose `overflow` is not `visible` clips what it contains to its
//! padding box and, unless it is `clip`, is a scroll container: its content
//! moves by its scroll offset. The viewport is one too, for the document.
//! Each such box opens a *scope*; every box is painted in the innermost
//! scope that contains it. That is its nearest clipping ancestor, except
//! that an absolutely positioned box escapes the clipping ancestors its
//! containing block lies outside of, and a fixed box escapes them all, the
//! viewport included (CSS Overflow 3 §2.1, CSS Position 3 §3).
//!
//! The root element's `overflow`, or the body's when the root's is
//! `visible`, is the viewport's (CSS Overflow 3 §3.3): that element does not
//! clip, and `hidden` there keeps the user from scrolling the document.
//!
//! Positions here are the layout's, made absolute and then scrolled. How
//! far a scope can scroll comes from what it contains, measured from its
//! padding box: border boxes, and lines of text, which may reach past the
//! box they are in.

use std::collections::HashMap;

use erk_dom::{Document, NodeId};
use erk_style::Styles;
use erk_style::style::computed_values::position::T as Position;
use erk_style::style::values::computed::Overflow;

use crate::display::Frame;
use crate::layout::Layouts;

/// Scroll offsets the user has made, by the node that scrolls; the
/// document's is under the document node.
pub(crate) type Offsets = HashMap<NodeId, (f32, f32)>;

pub(crate) struct Scope {
    /// The element that clips and scrolls, or the document node for the
    /// viewport.
    pub(crate) node: NodeId,
    pub(crate) parent: Option<usize>,
    /// The padding box everything in the scope is clipped to, where it is
    /// painted; `None` for the viewport, which the frame itself clips.
    pub(crate) clip: Option<Frame>,
    /// The clip's corner radii: the padding box's, inside a rounded border.
    pub(crate) radii: crate::display::Radii,
    /// The scroll offset, within `0..=max`.
    pub(crate) offset: (f32, f32),
    /// The furthest the content can scroll on each axis.
    pub(crate) max: (f32, f32),
    /// Whether the user scrolls it: `overflow: auto` or `scroll`, and the
    /// viewport. `hidden` clips and scrolls only from a script (none yet).
    pub(crate) user: bool,
    /// How far the scope's content moves: its offset and its ancestors'.
    shift: (f32, f32),
    /// The padding box, unscrolled.
    area: Frame,
    /// How far right and down the content reaches from the padding box's
    /// corner, unscrolled.
    reach: (f32, f32),
}

#[derive(Clone, Copy)]
struct Placed {
    /// The border box's corner, unscrolled.
    x: f32,
    y: f32,
    /// The scope the box is painted in; `None` for a fixed box.
    scope: Option<usize>,
    /// The scope its content is painted in: its own, if it opens one.
    inner: Option<usize>,
}

pub(crate) struct Scrolling {
    boxes: Vec<Option<Placed>>,
    pub(crate) scopes: Vec<Scope>,
}

/// The viewport's scope: the first.
pub(crate) const VIEWPORT: usize = 0;

impl Scrolling {
    /// Where every box of `doc` is with the scroll offsets `offsets`, in a
    /// viewport of `viewport` CSS pixels. Offsets beyond what a scope can
    /// scroll are clamped.
    pub(crate) fn new(
        doc: &Document,
        styles: &Styles,
        layouts: &Layouts,
        viewport: (f32, f32),
        offsets: &Offsets,
    ) -> Self {
        let area = Frame {
            x: 0.0,
            y: 0.0,
            width: viewport.0,
            height: viewport.1,
        };
        let (propagated, overflow) = viewport_overflow(doc, styles);
        let mut scrolling = Self {
            boxes: Vec::new(),
            scopes: vec![Scope {
                node: doc.root(),
                parent: None,
                clip: None,
                radii: [(0.0, 0.0); 4],
                offset: (0.0, 0.0),
                max: (0.0, 0.0),
                user: !overflow
                    .iter()
                    .any(|o| matches!(o, Overflow::Hidden | Overflow::Clip)),
                shift: (0.0, 0.0),
                area,
                reach: (0.0, 0.0),
            }],
        };
        scrolling.boxes.resize(doc.capacity_hint(), None);
        let mut walk = Walk {
            doc,
            styles,
            layouts,
            propagated,
            scrolling: &mut scrolling,
        };
        let mut active = vec![VIEWPORT];
        for child in doc.children(doc.root()) {
            walk.visit(child, (0.0, 0.0), doc.root(), &mut active);
        }

        // Parents come before their children.
        for index in 0..scrolling.scopes.len() {
            let parent_shift = scrolling.scopes[index]
                .parent
                .map_or((0.0, 0.0), |parent| scrolling.scopes[parent].shift);
            let scope = &mut scrolling.scopes[index];
            scope.max = (
                (scope.reach.0 - scope.area.width).max(0.0),
                (scope.reach.1 - scope.area.height).max(0.0),
            );
            let wanted = offsets.get(&scope.node).copied().unwrap_or_default();
            scope.offset = (
                wanted.0.clamp(0.0, scope.max.0),
                wanted.1.clamp(0.0, scope.max.1),
            );
            scope.shift = (
                parent_shift.0 + scope.offset.0,
                parent_shift.1 + scope.offset.1,
            );
            if let Some(clip) = &mut scope.clip {
                clip.x -= parent_shift.0;
                clip.y -= parent_shift.1;
            }
        }
        scrolling
    }

    /// The scrolled position of `id`'s border box.
    pub(crate) fn position(&self, id: NodeId) -> Option<(f32, f32)> {
        let placed = self.placed(id)?;
        let shift = self.shift(placed.scope);
        Some((placed.x - shift.0, placed.y - shift.1))
    }

    /// Where `id`'s content is measured from: its border box's corner,
    /// moved by its own scroll offset when it scrolls.
    pub(crate) fn content_origin(&self, id: NodeId) -> Option<(f32, f32)> {
        let placed = self.placed(id)?;
        let shift = self.shift(placed.inner);
        Some((placed.x - shift.0, placed.y - shift.1))
    }

    /// The scope `id`'s box is painted in.
    pub(crate) fn scope(&self, id: NodeId) -> Option<usize> {
        self.placed(id)?.scope
    }

    /// The scope `id`'s content is painted in.
    pub(crate) fn inner_scope(&self, id: NodeId) -> Option<usize> {
        self.placed(id)?.inner
    }

    /// The viewport, in CSS pixels.
    pub(crate) fn viewport(&self) -> Frame {
        self.scopes[VIEWPORT].area
    }

    fn placed(&self, id: NodeId) -> Option<Placed> {
        *self.boxes.get(id.index() as usize)?
    }

    fn shift(&self, scope: Option<usize>) -> (f32, f32) {
        scope.map_or((0.0, 0.0), |scope| self.scopes[scope].shift)
    }
}

struct Walk<'a> {
    doc: &'a Document,
    styles: &'a Styles,
    layouts: &'a Layouts,
    /// The element whose `overflow` is the viewport's.
    propagated: Option<NodeId>,
    scrolling: &'a mut Scrolling,
}

impl Walk<'_> {
    /// Place `id` and what it contains. `origin` is the unscrolled corner of
    /// the nearest box above, `containing` the containing block an
    /// absolutely positioned box in it would have, and `active` the scopes
    /// open around it, outermost first.
    fn visit(
        &mut self,
        id: NodeId,
        origin: (f32, f32),
        containing: NodeId,
        active: &mut Vec<usize>,
    ) {
        let style = self.styles.computed(id);
        let position = style.as_ref().map(|style| style.clone_position());
        let containing_inside = if position.is_some_and(|p| p != Position::Static) {
            id
        } else {
            containing
        };
        let Some(layout) = self.layouts.get(id) else {
            // No box of its own (an inline element): what it holds is placed
            // relative to the box above.
            for child in self.doc.children(id) {
                self.visit(child, origin, containing_inside, active);
            }
            return;
        };
        let (x, y) = (origin.0 + layout.location.x, origin.1 + layout.location.y);

        // The scopes that contain this box, and with it what it holds.
        let keep = match position {
            Some(Position::Fixed) => 0,
            Some(Position::Absolute) => active
                .iter()
                .rposition(|scope| {
                    contains(self.doc, self.scrolling.scopes[*scope].node, containing)
                })
                .map_or(0, |at| at + 1),
            _ => active.len(),
        };
        let escaped = active.split_off(keep);
        let scope = active.last().copied();
        self.reach(scope, x + layout.size.width, y + layout.size.height);

        let clips = Some(id) != self.propagated
            && style.as_ref().is_some_and(|style| {
                style.clone_overflow_x() != Overflow::Visible
                    || style.clone_overflow_y() != Overflow::Visible
            });
        let mut inner = scope;
        if let Some(style) = style.as_ref().filter(|_| clips) {
            let user = [style.clone_overflow_x(), style.clone_overflow_y()]
                .iter()
                .any(|overflow| matches!(overflow, Overflow::Auto | Overflow::Scroll));
            let area = Frame {
                x: x + layout.border.left,
                y: y + layout.border.top,
                width: (layout.size.width - layout.border.left - layout.border.right).max(0.0),
                height: (layout.size.height - layout.border.top - layout.border.bottom).max(0.0),
            };
            let border_box = Frame {
                x,
                y,
                width: layout.size.width,
                height: layout.size.height,
            };
            let widths = [
                layout.border.top,
                layout.border.right,
                layout.border.bottom,
                layout.border.left,
            ];
            self.scrolling.scopes.push(Scope {
                node: id,
                parent: scope,
                clip: Some(area),
                radii: crate::display::padding_radii(style, border_box, widths),
                offset: (0.0, 0.0),
                max: (0.0, 0.0),
                user,
                shift: (0.0, 0.0),
                area,
                reach: (0.0, 0.0),
            });
            inner = Some(self.scrolling.scopes.len() - 1);
            active.extend(inner);
        }
        self.scrolling.boxes[id.index() as usize] = Some(Placed { x, y, scope, inner });

        // Its text, its own or in anonymous boxes, is its content: as far as
        // the lines go.
        if let Some(shaped) = self.layouts.text(id) {
            let left = x + layout.border.left + layout.padding.left;
            let top = y + layout.border.top + layout.padding.top;
            let text = &shaped.layout;
            self.reach(inner, left + text.width(), top + text.height);
        }
        for anonymous in self.layouts.anonymous(id) {
            let (left, top) = (
                x + anonymous.layout.location.x,
                y + anonymous.layout.location.y,
            );
            let text = &anonymous.text.layout;
            self.reach(
                inner,
                left + anonymous.layout.size.width.max(text.width()),
                top + anonymous.layout.size.height.max(text.height),
            );
        }
        for child in self.doc.children(id) {
            self.visit(child, (x, y), containing_inside, active);
        }
        if inner != scope {
            active.pop();
        }
        active.extend(escaped);
    }
}

/// The element whose `overflow` goes to the viewport, and that overflow:
/// the root element's unless it is `visible`, then the body's.
fn viewport_overflow(doc: &Document, styles: &Styles) -> (Option<NodeId>, [Overflow; 2]) {
    let element = |parent: NodeId, name: erk_dom::LocalName| {
        doc.children(parent).find(|child| {
            doc.node(*child)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.name.local == name)
        })
    };
    let overflow = |id: NodeId| {
        styles.computed(id).map_or([Overflow::Visible; 2], |style| {
            [style.clone_overflow_x(), style.clone_overflow_y()]
        })
    };
    let visible = [Overflow::Visible; 2];
    let Some(html) = element(doc.root(), erk_dom::local_name!("html")) else {
        return (None, visible);
    };
    if overflow(html) != visible {
        return (Some(html), overflow(html));
    }
    match element(html, erk_dom::local_name!("body")) {
        Some(body) if overflow(body) != visible => (Some(body), overflow(body)),
        _ => (None, visible),
    }
}

impl Walk<'_> {
    /// `scope`'s content reaches at least `right`, `bottom`.
    fn reach(&mut self, scope: Option<usize>, right: f32, bottom: f32) {
        if let Some(scope) = scope {
            let scope = &mut self.scrolling.scopes[scope];
            scope.reach.0 = scope.reach.0.max(right - scope.area.x);
            scope.reach.1 = scope.reach.1.max(bottom - scope.area.y);
        }
    }
}

/// Whether `node` is `ancestor` or inside it.
fn contains(doc: &Document, ancestor: NodeId, node: NodeId) -> bool {
    let mut current = Some(node);
    while let Some(id) = current {
        if id == ancestor {
            return true;
        }
        current = doc.node(id).and_then(|found| found.parent());
    }
    false
}
