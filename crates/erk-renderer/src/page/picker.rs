//! A drop-down select's open list (M5.10).
//!
//! A press on a drop-down select, Space or Alt with an arrow at a focused
//! one opens its list: a row for each option, and for each group's label,
//! drawn over the content below the select, or above it where the
//! viewport has no room below, as Chrome draws its list in a window of its
//! own. The list is the page's, not the document's: the options get no
//! boxes, and the host sees only the select's `input` and `change`.
//!
//! A press on an option that is not disabled chooses it and closes the
//! list; if another option was selected, `input` and `change` follow. A
//! press anywhere else closes it (the select's own press closes it too).
//! While it is open the arrows, Home and End move the highlight, Enter
//! chooses the highlighted option, Escape and Tab close it.

use erk_dom::NodeId;

use super::Page;
use crate::display::{DisplayItem, Frame as Rect};
use crate::layout::picker::{PickerLayout, ROW_PADDING};
use crate::messages::{Event, EventKind, Key, KeyInput, KeyState, Modifiers};
use crate::resources::Resources;
use crate::text::TextEngine;

/// The highlighted row's background, Chrome's accent, and its text.
const HIGHLIGHT: [u8; 4] = [0, 117, 255, 255];
const HIGHLIGHT_TEXT: [u8; 4] = [255, 255, 255, 255];
/// A disabled option's text, and a group's label.
const MUTED_TEXT: [u8; 4] = [16, 16, 16, 77];
/// The list's frame, the select's border colour.
const EDGE: [u8; 4] = [118, 118, 118, 255];

/// An open list: its select and the highlighted option.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Picker {
    select: NodeId,
    highlighted: Option<NodeId>,
}

impl Page {
    /// The open list, if its select can still show one: in the document,
    /// a drop-down select, not disabled. A list whose select stopped being
    /// one is closed in effect, in the frame as in the oracle.
    pub(crate) fn open_picker(&self) -> Option<(NodeId, Option<NodeId>)> {
        let picker = self.picker?;
        (erk_invalidation::journal::connected(&self.doc, picker.select)
            && crate::layout::is_dropdown(&self.doc, picker.select)
            && self.doc.disabled(picker.select) != Some(true))
        .then_some((picker.select, picker.highlighted))
    }

    /// Open `select`'s list, its selected option highlighted, or close it
    /// if it is open: a press on the select. `None` for an element that is
    /// not an enabled drop-down select.
    pub(crate) fn invoke_select(&mut self, select: NodeId) -> Option<Vec<Event>> {
        if !crate::layout::is_dropdown(&self.doc, select) || self.doc.disabled(select) == Some(true)
        {
            return None;
        }
        if self.open_picker().is_some_and(|(open, _)| open == select) {
            self.close_picker();
        } else {
            self.picker = Some(Picker {
                select,
                highlighted: self.doc.selected_options(select).first().copied(),
            });
            self.edited = true;
        }
        Some(Vec::new())
    }

    /// Close the open list, choosing nothing.
    pub(crate) fn close_picker(&mut self) {
        if self.picker.take().is_some() {
            self.edited = true;
        }
    }

    /// A press went down on `target`: anywhere but the open list's select
    /// and its options, it closes the list.
    pub(crate) fn picker_press_down(&mut self, target: Option<NodeId>) {
        let Some((select, _)) = self.open_picker() else {
            return;
        };
        let inside = target.is_some_and(|target| self.element_path(target).contains(&select));
        if !inside {
            self.close_picker();
        }
    }

    /// A click's default at `target`, if it is an option of the open list:
    /// unless it is disabled, it is chosen and the list closes, then
    /// `input` and `change` if the selection changed. A disabled one does
    /// nothing, the list staying open. `None` for any other target.
    pub(crate) fn choose_in_picker(&mut self, target: NodeId) -> Option<Vec<Event>> {
        let (select, _) = self.open_picker()?;
        if self.doc.select_of(target) != Some(select) {
            return None;
        }
        if self.doc.disabled(target) == Some(true) {
            return Some(Vec::new());
        }
        Some(self.choose(select, target))
    }

    /// Close `select`'s list with `option` chosen: `input` and `change` if
    /// it was not selected already.
    fn choose(&mut self, select: NodeId, option: NodeId) -> Vec<Event> {
        self.close_picker();
        if self.doc.selected_options(select).first() == Some(&option) {
            return Vec::new();
        }
        let _ = self.set_checked(option.to_bits(), true);
        self.edited = true;
        [EventKind::Input, EventKind::Change]
            .into_iter()
            .map(|kind| self.event(kind, select, (0.0, 0.0), Modifiers::default()))
            .collect()
    }

    /// What key `input` does to the open list: the arrows, Home and End
    /// move the highlight to an option that is not disabled, Enter chooses
    /// the highlighted option, Escape closes the list. Tab closes it and
    /// goes on to move the focus (`None`), as does any key while no list
    /// is open.
    pub(crate) fn picker_key(&mut self, input: &KeyInput) -> Option<Vec<Event>> {
        let (select, highlighted) = self.open_picker()?;
        if input.state != KeyState::Down {
            return None;
        }
        let options: Vec<NodeId> = self
            .doc
            .options(select)
            .into_iter()
            .filter(|option| self.doc.disabled(*option) != Some(true))
            .collect();
        let at = highlighted.and_then(|option| options.iter().position(|o| *o == option));
        let moved = match input.key {
            Key::ArrowDown => {
                Some(at.map_or(0, |at| (at + 1).min(options.len().saturating_sub(1))))
            }
            Key::ArrowUp => Some(at.map_or(0, |at| at.saturating_sub(1))),
            Key::Home => Some(0),
            Key::End => Some(options.len().saturating_sub(1)),
            Key::Enter => {
                return Some(match highlighted {
                    Some(option) if options.contains(&option) => self.choose(select, option),
                    _ => {
                        self.close_picker();
                        Vec::new()
                    }
                });
            }
            Key::Escape => {
                self.close_picker();
                return Some(Vec::new());
            }
            Key::Tab => {
                self.close_picker();
                return None;
            }
            _ => None,
        };
        if let Some(index) = moved
            && let Some(option) = options.get(index)
        {
            self.picker = Some(Picker {
                select,
                highlighted: Some(*option),
            });
            self.edited = true;
        }
        Some(Vec::new())
    }

    /// The open list over the content, if there is one: its rows shaped in
    /// the select's style, below the select's box `select_frame` or above
    /// it where `viewport` has no room below, as wide as the select or its
    /// widest row; and each option's hit region, so that a press on a row
    /// is one on its option.
    pub(crate) fn picker_items(
        &self,
        styles: &erk_style::Styles,
        resources: &Resources,
        select_frame: Option<Rect>,
        viewport: (f32, f32),
    ) -> Vec<DisplayItem> {
        let (Some((select, highlighted)), Some(anchor)) = (self.open_picker(), select_frame) else {
            return Vec::new();
        };
        let mut text = TextEngine::with_fonts(resources.fonts());
        let Some(PickerLayout { rows, width }) =
            crate::layout::picker::picker_layout(&self.doc, styles, &mut text, select)
        else {
            return Vec::new();
        };
        let color = styles.computed(select).map_or([0, 0, 0, 255], |style| {
            crate::color::srgb_bytes(style.clone_color())
        });
        // Its frame around its widest row, but no narrower than the select.
        let width = (width + 2.0).max(anchor.width);
        let height = rows.iter().map(|row| row.height).sum::<f32>() + 2.0;
        let below = anchor.y + anchor.height;
        let y = if below + height > viewport.1 && anchor.y - height >= 0.0 {
            anchor.y - height
        } else {
            below
        };
        let frame = Rect {
            x: anchor.x,
            y,
            width,
            height,
        };
        let mut items = vec![DisplayItem::Rect {
            x: frame.x,
            y: frame.y,
            width: frame.width,
            height: frame.height,
            color: [255, 255, 255, 255],
        }];
        let (across, down) = ROW_PADDING;
        let mut top = frame.y + 1.0;
        for row in &rows {
            let row_frame = Rect {
                x: frame.x + 1.0,
                y: top,
                width: frame.width - 2.0,
                height: row.height,
            };
            let lit = row.option.is_some() && row.option == highlighted;
            if lit {
                items.push(DisplayItem::Rect {
                    x: row_frame.x,
                    y: row_frame.y,
                    width: row_frame.width,
                    height: row_frame.height,
                    color: HIGHLIGHT,
                });
            }
            if let Some(option) = row.option {
                items.push(DisplayItem::Hit {
                    node: option.to_bits(),
                    frame: row_frame,
                    text: false,
                });
            }
            let text_color = if lit {
                HIGHLIGHT_TEXT
            } else if row.disabled {
                MUTED_TEXT
            } else {
                color
            };
            let origin = (row_frame.x + across + row.indent, row_frame.y + down);
            items.extend(crate::display::glyph_runs(
                &row.text,
                origin,
                resources,
                &|_, _| text_color,
                None,
            ));
            top += row.height;
        }
        items.push(DisplayItem::Border {
            frame,
            widths: [1.0; 4],
            colors: [EDGE; 4],
            radii: [(0.0, 0.0); 4],
        });
        items
    }
}
