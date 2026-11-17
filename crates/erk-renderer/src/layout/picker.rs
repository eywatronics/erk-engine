//! The rows of a drop-down select's open list (M5.10).
//!
//! The list is not part of the document's layout: the select's options get
//! no boxes. While it is open, the page shapes a row for each of them,
//! and for each group's label, with the select's font, and draws them over
//! the content below the select (`page/picker.rs`), as Chrome draws its
//! list in a window of its own.

use std::sync::Arc;

use erk_dom::{Document, NodeId};
use erk_style::Styles;

use super::{ShapedText, source_colours, text_language};
use crate::text::{InlineToken, Paragraph, TextEngine};

/// Room around a row's label, across and down: Erk's own look.
pub(crate) const ROW_PADDING: (f32, f32) = (4.0, 1.0);
/// How far an option in a group is indented from its group's label.
pub(crate) const GROUP_INDENT: f32 = super::OPTGROUP_INDENT;

/// One row of the open list.
pub(crate) struct PickerRow {
    /// The option it chooses; `None` for a group's label.
    pub(crate) option: Option<NodeId>,
    /// Whether it cannot be chosen: a disabled option, or a group's label.
    pub(crate) disabled: bool,
    /// How far its label is indented.
    pub(crate) indent: f32,
    /// Its height, padding included.
    pub(crate) height: f32,
    pub(crate) text: Arc<ShapedText>,
}

/// The rows of `select`'s open list, top to bottom, and the width the
/// widest needs, padding included.
pub(crate) struct PickerLayout {
    pub(crate) rows: Vec<PickerRow>,
    pub(crate) width: f32,
}

/// The rows of drop-down select `select`'s open list, shaped by `text` in
/// the select's style: each option, and each group's label before its
/// options. `None` if it has no style.
pub(crate) fn picker_layout(
    doc: &Document,
    styles: &Styles,
    text: &mut TextEngine,
    select: NodeId,
) -> Option<PickerLayout> {
    let style = styles.computed(select)?;
    let language = text_language(doc, select);
    let mut shape = |label: String| {
        let tokens = [InlineToken::Text(
            label,
            style.clone(),
            language.clone(),
            None,
        )];
        let paragraph = Paragraph::new(&tokens, &style);
        let layout = text.shape(&paragraph, None, None, &[]);
        let decorations = layout.decorations(&paragraph);
        let relative = Vec::new();
        Arc::new(ShapedText {
            band: crate::display::text_band(&layout, &relative, &decorations),
            relative,
            text: paragraph.text.clone(),
            sources: paragraph.sources.clone(),
            colours: source_colours(doc, styles, &paragraph.sources),
            preserved: paragraph.preserved.clone(),
            marker: None,
            atom_boxes: Vec::new(),
            decorations,
            layout,
        })
    };
    let mut rows = Vec::new();
    let element = |id: NodeId| doc.node(id).and_then(|node| node.as_element());
    let named = |id: NodeId, name: &erk_dom::LocalName| {
        element(id).is_some_and(|element| {
            element.name.ns == erk_dom::ns!(html) && element.name.local == *name
        })
    };
    for child in doc.children(select) {
        if named(child, &erk_dom::local_name!("optgroup")) {
            let label = element(child)
                .and_then(|group| group.attr(&erk_dom::local_name!("label")))
                .unwrap_or_default()
                .to_owned();
            rows.push((None, true, 0.0, shape(label)));
            for option in doc.children(child) {
                if named(option, &erk_dom::local_name!("option")) {
                    let disabled = doc.disabled(option) == Some(true);
                    let label = doc.option_label(option);
                    rows.push((Some(option), disabled, GROUP_INDENT, shape(label)));
                }
            }
        } else if named(child, &erk_dom::local_name!("option")) {
            let disabled = doc.disabled(child) == Some(true);
            let label = doc.option_label(child);
            rows.push((Some(child), disabled, 0.0, shape(label)));
        }
    }
    let (across, down) = ROW_PADDING;
    let rows: Vec<PickerRow> = rows
        .into_iter()
        .map(|(option, disabled, indent, text)| PickerRow {
            option,
            disabled,
            indent,
            height: text.layout.height + 2.0 * down,
            text,
        })
        .collect();
    let width = rows
        .iter()
        .map(|row| row.indent + row.text.layout.width() + 2.0 * across)
        .fold(0.0_f32, f32::max);
    Some(PickerLayout { rows, width })
}
