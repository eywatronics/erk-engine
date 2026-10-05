//! Text shaping and line breaking with Parley.
//!
//! A paragraph is one run of text with styled ranges: each inline element's
//! text keeps its own size, weight, colour and line height. Text is drawn in
//! the embedded Noto Sans: system fonts are not loaded, so the same page
//! measures and paints the same on every machine. CSS `font-family` is not
//! consulted yet (M1.7).

use std::ops::Range;

use erk_dom::NodeId;
use std::sync::Arc;

use erk_style::style::computed_values::text_wrap_mode::T as TextWrapModeCss;
use erk_style::style::computed_values::white_space_collapse::T as WhiteSpaceCollapse;
use erk_style::style::properties::style_structs::Font as FontStyle;
use erk_style::style::values::computed::font::LineHeight as CssLineHeight;
use erk_style::style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use erk_style::style::values::computed::{CSSPixelLength, Length};
use erk_style::{ComputedValues, FontMetricsProvider, StyleFontMetrics};

use crate::case;
use crate::color::{Rgba, srgb_bytes};
use crate::fonts::{self, Family, HostFonts};
use crate::messages::FontCatalog;
use icu_locale_core::LanguageIdentifier;
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::fontique::{FallbackKey, FamilyId, Language, Script as FontScript};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontWeight, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, StyleProperty,
};
use parley::{FontFamily, FontFamilyName, FontStyle as ParleyFontStyle};
use std::borrow::Cow;
use std::str::FromStr;
#[cfg(test)]
use taffy::{AvailableSpace, Size};

pub(crate) const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
const NOTO_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
const FAMILY: &str = "Noto Sans";

/// What Parley carries with each piece of text: its colour, as straight
/// (non-premultiplied) sRGB bytes, and how far `vertical-align` raises it
/// above the line's baseline. Text whose raise differs gets glyph runs of
/// its own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextBrush {
    pub(crate) color: [u8; 4],
    pub(crate) raise: f32,
}

/// The text properties of one styled range.
#[derive(Clone, Debug, PartialEq)]
struct TextStyle {
    font_size: f32,
    line_height: LineHeight,
    weight: f32,
    italic: bool,
    /// The `font-family` list; the embedded font comes after it.
    families: Arc<[Family]>,
    /// The text's language, for language-specific fallback fonts.
    language: Option<Language>,
    color: TextBrush,
    /// Whether lines may break between words (`text-wrap-mode`).
    wrap: bool,
}

impl TextStyle {
    fn of(style: &ComputedValues) -> Self {
        let font_size = style.get_font().clone_font_size().computed_size().px();
        let weight = style.clone_font_weight().value();
        let line_height = match style.clone_line_height() {
            CssLineHeight::Normal => LineHeight::Absolute(normal_line_height(font_size, weight)),
            CssLineHeight::Number(number) => LineHeight::FontSizeRelative(number.0),
            CssLineHeight::Length(length) => LineHeight::Absolute(length.0.px()),
        };
        Self {
            font_size,
            line_height,
            weight,
            italic: fonts::is_italic(style),
            families: fonts::families(style),
            language: None,
            color: TextBrush {
                color: srgb_bytes(style.clone_color()),
                raise: 0.0,
            },
            wrap: style.get_inherited_text().clone_text_wrap_mode() != TextWrapModeCss::Nowrap,
        }
    }

    /// The used line height in pixels.
    fn line_height_px(&self) -> f32 {
        match self.line_height {
            LineHeight::Absolute(px) => px,
            LineHeight::FontSizeRelative(factor) => factor * self.font_size,
            LineHeight::MetricsRelative(factor) => {
                let (ascent, descent) = font_extents(self.font_size, self.weight);
                factor * (ascent + descent)
            }
        }
    }

    /// The space this style's inline box takes above and below its baseline:
    /// the font's ascent and descent and the half-leading of its line height,
    /// split as Chrome splits it (CSS 2 §10.8.1).
    fn extents(&self) -> (f32, f32) {
        let (ascent, descent) = font_extents(self.font_size, self.weight);
        let leading = self.line_height_px() - (ascent + descent);
        let above = (leading * 0.5).floor();
        let below = leading.round() - above;
        (ascent + above, descent + below)
    }
}

/// What `vertical-align` measures against: the parent inline box's font and
/// how far its own baseline is raised.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ParentBox {
    raise: f32,
    font_size: f32,
    ascent: f32,
    descent: f32,
    x_height: f32,
}

impl ParentBox {
    fn of(style: &TextStyle, raise: f32) -> Self {
        let (ascent, descent) = font_extents(style.font_size, style.weight);
        Self {
            raise,
            font_size: style.font_size,
            ascent,
            descent,
            x_height: x_height(style.font_size, style.weight),
        }
    }
}

/// Where `vertical-align` puts a box (CSS 2 §10.8.1; in Stylo, the
/// `alignment-baseline` and `baseline-shift` longhands of CSS Inline 3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum VerticalAlign {
    /// Relative to the parent's baseline: an anchor, then raised by `shift`
    /// (`sub`, `super`, a length or a percentage of the line height).
    Parent { anchor: Anchor, shift: f32 },
    /// Relative to the line box: `top`, `center`, `bottom`.
    Line(LineAnchor),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Anchor {
    Baseline,
    Middle,
    TextTop,
    TextBottom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LineAnchor {
    Top,
    Center,
    Bottom,
}

impl VerticalAlign {
    /// `style`'s `vertical-align`, with a percentage resolved against its own
    /// line height and `sub`/`super` against the parent's font size, by
    /// Blink's offsets (a fifth plus one pixel down, a third plus one up).
    /// Offsets are cut to Blink's 1/64 px layout unit: unrounded, a `<sup>`
    /// line came out 0.017 px taller than Chrome's, enough to round every
    /// later box the other way (found by the Chrome reference test).
    fn of(style: &ComputedValues, line_height: f32, parent: &ParentBox) -> Self {
        use erk_style::style::values::computed::length::Length;
        use erk_style::style::values::generics::box_::{BaselineShift, BaselineShiftKeyword};
        use erk_style::style::values::specified::box_::AlignmentBaseline;

        let shift = match style.clone_baseline_shift() {
            BaselineShift::Keyword(BaselineShiftKeyword::Top) => {
                return Self::Line(LineAnchor::Top);
            }
            BaselineShift::Keyword(BaselineShiftKeyword::Center) => {
                return Self::Line(LineAnchor::Center);
            }
            BaselineShift::Keyword(BaselineShiftKeyword::Bottom) => {
                return Self::Line(LineAnchor::Bottom);
            }
            BaselineShift::Keyword(BaselineShiftKeyword::Sub) => {
                -layout_unit(parent.font_size / 5.0 + 1.0)
            }
            BaselineShift::Keyword(BaselineShiftKeyword::Super) => {
                layout_unit(parent.font_size / 3.0 + 1.0)
            }
            BaselineShift::Length(length) => {
                layout_unit(length.resolve(Length::new(line_height)).px())
            }
        };
        let anchor = match style.clone_alignment_baseline() {
            AlignmentBaseline::Middle => Anchor::Middle,
            AlignmentBaseline::TextTop => Anchor::TextTop,
            AlignmentBaseline::TextBottom => Anchor::TextBottom,
            _ => Anchor::Baseline,
        };
        Self::Parent { anchor, shift }
    }

    /// How far a box that takes `above` and `below` around its own baseline
    /// is raised above the line's baseline. `None` for the line-relative
    /// values, which are placed once the line box is known.
    fn raise(self, parent: &ParentBox, above: f32, below: f32) -> Option<f32> {
        let Self::Parent { anchor, shift } = self else {
            return None;
        };
        let anchored = match anchor {
            Anchor::Baseline => 0.0,
            // The box's middle on the parent's baseline plus half its
            // x-height.
            Anchor::Middle => layout_unit(parent.x_height / 2.0 + (below - above) / 2.0),
            // The box's top with the top of the parent's content area.
            Anchor::TextTop => parent.ascent - above,
            // The box's bottom with the bottom of the parent's content area.
            Anchor::TextBottom => below - parent.descent,
        };
        Some(parent.raise + anchored + shift)
    }
}

/// One item of a block's inline content, in tree order.
pub(crate) enum InlineToken<S> {
    /// The text of a text node, with the style of its element, the
    /// language it is written in (for `text-transform`) and the text node.
    Text(String, S, LanguageIdentifier, NodeId),
    /// An inline element starts; its style gives its padding, border,
    /// margin and background.
    Open(S),
    /// The innermost open inline element ends.
    Close,
    /// An atomic inline (`inline-block`, `inline-flex`): laid out as a box
    /// of its own and placed in the line like a glyph. The arena index and
    /// its style.
    Atom(usize, S),
    /// An absolutely positioned element that left the flow here: a point
    /// that takes no room, marking its static position (CSS 2 §10.3.7).
    /// The arena index.
    Anchor(usize),
    /// A forced line break (`<br>`).
    Break,
}

/// An inline box in a paragraph's text, in text order.
#[derive(Clone, Debug)]
pub(crate) struct InlineItem {
    /// The byte offset in the text the box sits at.
    index: usize,
    pub(crate) kind: InlineItemKind,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum InlineItemKind {
    /// Horizontal space an inline element takes at its start or end: its
    /// margin, border and padding on that side. An opening one goes with
    /// the element's first character: no line breaks between them.
    Spacer { width: f32, opening: bool },
    /// An atomic inline, by arena index, with its `vertical-align` and the
    /// inline box it is aligned in.
    Atom(usize, VerticalAlign, ParentBox),
    /// Where an absolutely positioned element would have been, by arena
    /// index; no width.
    Anchor(usize),
}

/// The size an atomic inline takes in its line, measured by layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AtomBox {
    /// Margin box width.
    pub(crate) width: f32,
    /// From the top of the margin box to the box's own baseline.
    pub(crate) above: f32,
    /// From the baseline to the bottom of the margin box.
    pub(crate) below: f32,
}

/// An inline element whose background is painted, one rectangle per line
/// it spans.
#[derive(Clone, Debug)]
struct Decoration {
    /// The element's text.
    text: Range<usize>,
    /// The inline boxes inside the element, its own spacers included.
    boxes: Range<usize>,
    /// The element's own start and end spacers, and the margin on each,
    /// which lies outside the background.
    open: Option<(usize, f32)>,
    close: Option<(usize, f32)>,
    /// From the baseline up and down to the edges of the background: the
    /// font's ascent and descent, then padding and border.
    above: f32,
    below: f32,
    /// How far `vertical-align` raises the element.
    raise: f32,
    color: Rgba,
    /// Border widths (top, right, bottom, left) and colours; the left and
    /// right sides belong to the element's first and last line only.
    border: [f32; 4],
    border_colors: [Rgba; 4],
}

/// How an inline element is decorated: what `Decoration` holds besides
/// where it is.
#[derive(Clone, Copy, Debug)]
struct InlineLook {
    above: f32,
    below: f32,
    color: Rgba,
    border: [f32; 4],
    border_colors: [Rgba; 4],
}

/// Text raised or lowered by `vertical-align`: its inline box takes
/// `above` and `below` around a baseline `raise` above the line's.
#[derive(Clone, Debug)]
struct Raised {
    text: Range<usize>,
    raise: f32,
    above: f32,
    below: f32,
}

/// A painted background of an inline element on one line, relative to the
/// paragraph's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecorationRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// The background; transparent when the element has only a border.
    pub(crate) color: Rgba,
    /// The border on this line (top, right, bottom, left) and its colours.
    pub(crate) border: [f32; 4],
    pub(crate) border_colors: [Rgba; 4],
}

/// Everything needed to shape one paragraph.
#[derive(Clone, Debug)]
pub(crate) struct Paragraph {
    pub(crate) text: String,
    /// The block's own style, for text outside any styled range.
    base: TextStyle,
    /// Ranges of `text` styled differently from `base`, in order.
    spans: Vec<(Range<usize>, TextStyle)>,
    /// The block's `text-align`.
    align: Alignment,
    /// Inline boxes in text order; a box's position here is its Parley id.
    pub(crate) items: Vec<InlineItem>,
    decorations: Vec<Decoration>,
    raised: Vec<Raised>,
    /// Where each text node's text went: the node and its range of
    /// `text`, in text order.
    pub(crate) sources: Vec<(NodeId, Range<usize>)>,
    /// A forced line break (`<br>`) not yet written: it becomes a newline
    /// only before more content, so that one ending the block adds no line.
    pending_break: bool,
    /// The spaces `white-space` keeps (`pre`, `pre-wrap`): at the end of a
    /// line they hang, but they are text all the same.
    pub(crate) preserved: Vec<Range<usize>>,
    /// Whether the last character written may not wrap (`nowrap`, `pre`),
    /// and whether the pending space may: see `flush`.
    last_nowrap: bool,
    space_wraps: bool,
}

/// An inline element being read, until its `Close`.
struct OpenElement {
    first_box: usize,
    text_start: usize,
    open: Option<(usize, f32)>,
    end_spacer: f32,
    end_margin: f32,
    decoration: Option<InlineLook>,
    /// How many decorations there were when it opened: its own goes there,
    /// before those of the elements inside it, which are painted over it.
    decorations_before: usize,
    /// Its text style and how far it is raised: the parent box of what it
    /// contains.
    style: TextStyle,
    raise: f32,
    extents: (f32, f32),
}

impl Paragraph {
    /// A paragraph of `tokens` in a block styled like `block`. Whitespace is
    /// collapsed across the tokens as `white-space: normal` does. A collapsed
    /// space stays inside the element it was written in: `a <b>b</b>` puts
    /// the space before the `<b>`, `<b>a </b>b` inside it.
    pub(crate) fn new<S: AsRef<ComputedValues>>(
        tokens: &[InlineToken<S>],
        block: &ComputedValues,
    ) -> Self {
        let base = TextStyle::of(block);
        let mut paragraph = Self {
            text: String::new(),
            base,
            spans: Vec::new(),
            align: alignment(block),
            items: Vec::new(),
            decorations: Vec::new(),
            raised: Vec::new(),
            sources: Vec::new(),
            pending_break: false,
            preserved: Vec::new(),
            last_nowrap: false,
            space_wraps: true,
        };
        let mut open: Vec<OpenElement> = Vec::new();
        // The box an element or atom is aligned in: the innermost open
        // element, or the block's own (root) inline box.
        let parent_of = |open: &[OpenElement], base: &TextStyle| match open.last() {
            Some(element) => ParentBox::of(&element.style, element.raise),
            None => ParentBox::of(base, 0.0),
        };
        // Whether content (text or an atom) has been seen, whether the last
        // character was a space, and whether a space is due before the next
        // content.
        let (mut has_content, mut last_was_space, mut pending_space) = (false, false, false);
        // The last character of the text so far, white space included, for
        // `text-transform: capitalize`: a word it ends continues in the next
        // text node.
        let mut previous: Option<char> = None;
        // The text node a pending space was written in: the space is its
        // text, though it is emitted only before the next content.
        let mut space_owner: Option<NodeId> = None;
        // Element ends seen since the last content: whether each came after
        // the pending space (the space is then inside the element).
        let mut closes: Vec<(OpenElement, bool)> = Vec::new();

        for token in tokens {
            match token {
                InlineToken::Text(raw, style, lang, node) => {
                    // Case mapping never makes or removes white space, so
                    // it can come before the collapsing below.
                    let case = style
                        .as_ref()
                        .get_inherited_text()
                        .clone_text_transform()
                        .case();
                    let raw = case::transform(raw, case, lang, previous);
                    let mut style = TextStyle::of(style.as_ref());
                    if *lang != LanguageIdentifier::UNKNOWN {
                        style.language = fonts::language(&lang.to_string());
                    }
                    style.color.raise = open.last().map_or(0.0, |element| element.raise);
                    // `white-space`: `pre-line` keeps newlines, `pre` and
                    // `pre-wrap` keep every space too (`break-spaces` is
                    // laid out as `pre-wrap`).
                    let collapse = match token_collapse(token) {
                        Some(collapse) => collapse,
                        None => WhiteSpaceCollapse::Collapse,
                    };
                    let keeps_spaces = !matches!(
                        collapse,
                        WhiteSpaceCollapse::Collapse | WhiteSpaceCollapse::PreserveBreaks
                    );
                    let keeps_breaks = collapse != WhiteSpaceCollapse::Collapse;
                    let mut start = None;
                    for c in raw.chars() {
                        previous = Some(c);
                        if c == '\n' && keeps_breaks {
                            // A forced break, as a `<br>`: white space
                            // around it that collapses goes.
                            pending_space = false;
                            space_owner = None;
                            paragraph.flush(
                                &mut closes,
                                &mut pending_space,
                                &mut last_was_space,
                                &mut space_owner,
                            );
                            paragraph.pending_break = true;
                            has_content = true;
                            last_was_space = true;
                            continue;
                        }
                        // A kept space or tab is text (a tab is one space
                        // wide: tab stops come with `tab-size`, M5).
                        let c = if keeps_spaces && is_document_whitespace(c) {
                            ' '
                        } else {
                            c
                        };
                        if is_document_whitespace(c) && !keeps_spaces {
                            if has_content && !last_was_space && !pending_space {
                                pending_space = true;
                                space_owner = Some(*node);
                                paragraph.space_wraps = style.wrap;
                            }
                            continue;
                        }
                        // A space this node wrote is in its own range: from
                        // the space on if the node starts with it.
                        let own_space = pending_space && space_owner == Some(*node);
                        if own_space {
                            space_owner = None;
                            if start.is_none() {
                                // Element ends add no text: the space is
                                // the next character.
                                start = Some(paragraph.text.len());
                            }
                        }
                        paragraph.flush(
                            &mut closes,
                            &mut pending_space,
                            &mut last_was_space,
                            &mut space_owner,
                        );
                        start.get_or_insert(paragraph.text.len());
                        let at = paragraph.text.len();
                        paragraph.text.push(c);
                        paragraph.last_nowrap = !style.wrap;
                        if keeps_spaces && c == ' ' {
                            match paragraph.preserved.last_mut() {
                                Some(last) if last.end == at => last.end = at + 1,
                                _ => paragraph.preserved.push(at..at + 1),
                            }
                        }
                        has_content = true;
                        // A kept space is not one that collapses: a
                        // collapsible space after it stays.
                        last_was_space = false;
                    }
                    if let Some(start) = start {
                        let end = paragraph.text.len();
                        paragraph.attribute(*node, start..end);
                        if style != paragraph.base {
                            paragraph.spans.push((start..end, style));
                        }
                    }
                }
                InlineToken::Open(style) => {
                    paragraph.flush(
                        &mut closes,
                        &mut pending_space,
                        &mut last_was_space,
                        &mut space_owner,
                    );
                    let style = style.as_ref();
                    let text_style = TextStyle::of(style);
                    let parent = parent_of(&open, &paragraph.base);
                    let extents = text_style.extents();
                    // A line-relative value on an inline element is laid out
                    // as `baseline` (not supported yet).
                    let raise = VerticalAlign::of(style, text_style.line_height_px(), &parent)
                        .raise(&parent, extents.0, extents.1)
                        .unwrap_or(parent.raise);
                    let sides = InlineSides::of(style);
                    let open_box = (sides.start > 0.0).then(|| {
                        paragraph.push_item(InlineItemKind::Spacer {
                            width: sides.start,
                            opening: true,
                        });
                        (paragraph.items.len() - 1, sides.margin_start)
                    });
                    open.push(OpenElement {
                        first_box: open_box.map_or(paragraph.items.len(), |(index, _)| index),
                        text_start: paragraph.text.len(),
                        open: open_box,
                        end_spacer: sides.end,
                        end_margin: sides.margin_end,
                        decoration: decoration_of(style),
                        decorations_before: paragraph.decorations.len(),
                        style: text_style,
                        raise,
                        extents,
                    });
                }
                InlineToken::Anchor(index) => {
                    // Not content: it neither emits nor swallows a space.
                    paragraph.push_item(InlineItemKind::Anchor(*index));
                }
                InlineToken::Break => {
                    // White space before a forced break is removed, and
                    // after it collapses away at the start of the next line
                    // (CSS Text 3 §4.1.2).
                    pending_space = false;
                    space_owner = None;
                    paragraph.flush(
                        &mut closes,
                        &mut pending_space,
                        &mut last_was_space,
                        &mut space_owner,
                    );
                    paragraph.pending_break = true;
                    has_content = true;
                    last_was_space = true;
                }
                InlineToken::Close => {
                    if let Some(element) = open.pop() {
                        closes.push((element, pending_space));
                    }
                }
                InlineToken::Atom(index, style) => {
                    paragraph.flush(
                        &mut closes,
                        &mut pending_space,
                        &mut last_was_space,
                        &mut space_owner,
                    );
                    let parent = parent_of(&open, &paragraph.base);
                    let line_height = TextStyle::of(style.as_ref()).line_height_px();
                    let align = VerticalAlign::of(style.as_ref(), line_height, &parent);
                    paragraph.push_item(InlineItemKind::Atom(*index, align, parent));
                    has_content = true;
                    last_was_space = false;
                }
            }
        }
        // Unclosed elements end with the paragraph; a trailing space is
        // dropped.
        while let Some(element) = open.pop() {
            closes.push((element, false));
        }
        pending_space = false;
        // A break that ends the block ends its last line; alone, it makes
        // the block one empty line, which a zero-width space holds.
        if std::mem::take(&mut paragraph.pending_break) && paragraph.text.is_empty() {
            paragraph.text.push('\u{200B}');
        }
        paragraph.flush(
            &mut closes,
            &mut pending_space,
            &mut last_was_space,
            &mut space_owner,
        );
        paragraph
    }

    /// Before the next content: emit the pending space and the ends of the
    /// elements closed since the last content, each on its side of the
    /// space.
    fn flush(
        &mut self,
        closes: &mut Vec<(OpenElement, bool)>,
        pending_space: &mut bool,
        last_was_space: &mut bool,
        space_owner: &mut Option<NodeId>,
    ) {
        if std::mem::take(&mut self.pending_break) {
            self.text.push('\n');
        }
        let (after, before): (Vec<_>, Vec<_>) =
            closes.drain(..).partition(|(_, after_space)| *after_space);
        for (element, _) in before {
            self.close(element);
        }
        if std::mem::take(pending_space) {
            // Parley decides whether a space may hang at a line's end by
            // the character before it: after text that may not wrap, a
            // space that may would stay on the line and push what is before
            // the no-wrap text down. A zero-width space between them, which
            // adds no break opportunity of its own (UAX #14 LB8), lends the
            // space its wrapping.
            if self.last_nowrap && self.space_wraps {
                self.text.push('\u{200B}');
            }
            let at = self.text.len();
            self.text.push(' ');
            *last_was_space = true;
            if let Some(node) = space_owner.take() {
                self.attribute(node, at..at + 1);
            }
        }
        for (element, _) in after {
            self.close(element);
        }
    }

    /// Text node `node`'s text went to `range`: added to its last range when
    /// the two meet (a collapsed space and the characters around it).
    fn attribute(&mut self, node: NodeId, range: Range<usize>) {
        if let Some((last, so_far)) = self.sources.last_mut()
            && *last == node
            && so_far.end == range.start
        {
            so_far.end = range.end;
        } else {
            self.sources.push((node, range));
        }
    }

    fn close(&mut self, element: OpenElement) {
        let close = (element.end_spacer > 0.0).then(|| {
            self.push_item(InlineItemKind::Spacer {
                width: element.end_spacer,
                opening: false,
            });
            (self.items.len() - 1, element.end_margin)
        });
        let text = element.text_start..self.text.len();
        if element.raise != 0.0 && !text.is_empty() {
            self.raised.push(Raised {
                text: text.clone(),
                raise: element.raise,
                above: element.extents.0,
                below: element.extents.1,
            });
        }
        if let Some(look) = element.decoration {
            self.decorations.insert(
                element.decorations_before,
                Decoration {
                    text,
                    boxes: element.first_box..self.items.len(),
                    open: element.open,
                    close,
                    above: look.above,
                    below: look.below,
                    raise: element.raise,
                    color: look.color,
                    border: look.border,
                    border_colors: look.border_colors,
                },
            );
        }
    }

    fn push_item(&mut self, kind: InlineItemKind) {
        self.items.push(InlineItem {
            index: self.text.len(),
            kind,
        });
    }

    /// Whether the paragraph has nothing to lay out.
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty() && self.items.is_empty()
    }

    /// The arena indices of the absolutely positioned elements anchored in
    /// the paragraph, in order.
    pub(crate) fn anchors(&self) -> Vec<usize> {
        self.items
            .iter()
            .filter_map(|item| match item.kind {
                InlineItemKind::Anchor(index) => Some(index),
                _ => None,
            })
            .collect()
    }

    /// The arena indices of the atomic inlines, in order.
    pub(crate) fn atoms(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().filter_map(|item| match item.kind {
            InlineItemKind::Atom(index, ..) => Some(index),
            InlineItemKind::Spacer { .. } | InlineItemKind::Anchor(_) => None,
        })
    }

    /// The strut: the space above and below the baseline that the block's
    /// own font and line height give every line (CSS 2 §10.8.1).
    fn strut(&self) -> (f32, f32) {
        self.base.extents()
    }
}

/// An inline element's horizontal margin, border and padding.
struct InlineSides {
    start: f32,
    end: f32,
    margin_start: f32,
    margin_end: f32,
}

impl InlineSides {
    /// Percentages resolve to zero: they would need the containing block's
    /// width, which line breaking does not have yet.
    fn of(style: &ComputedValues) -> Self {
        use erk_style::style::values::generics::length::GenericMargin;
        let margin = |value: &GenericMargin<_>| match value {
            GenericMargin::LengthPercentage(length) => fixed(length),
            _ => 0.0,
        };
        let margins = style.get_margin();
        let (margin_start, margin_end) =
            (margin(&margins.margin_left), margin(&margins.margin_right));
        let padding = style.get_padding();
        let border = style.get_border();
        let border_width =
            |width: &erk_style::style::values::computed::BorderSideWidth,
             line: erk_style::style::values::computed::BorderStyle| {
                if line.none_or_hidden() {
                    0.0
                } else {
                    width.0.to_f32_px()
                }
            };
        Self {
            start: margin_start
                + border_width(&border.border_left_width, border.border_left_style)
                + fixed(&padding.padding_left.0),
            end: margin_end
                + border_width(&border.border_right_width, border.border_right_style)
                + fixed(&padding.padding_right.0),
            margin_start,
            margin_end,
        }
    }
}

fn fixed(length: &erk_style::style::values::computed::LengthPercentage) -> f32 {
    length.to_length().map_or(0.0, |length| length.px())
}

/// How an inline element is painted: its background and border, the
/// extent of both around the baseline; `None` if it paints neither.
fn decoration_of(style: &ComputedValues) -> Option<InlineLook> {
    use erk_style::style::computed_values::visibility::T as Visibility;
    if style.clone_visibility() != Visibility::Visible {
        return None;
    }
    let color = srgb_bytes(style.resolve_color(&style.get_background().background_color));
    let font = TextStyle::of(style);
    let (ascent, descent) = font_extents(font.font_size, font.weight);
    let padding = style.get_padding();
    let border = style.get_border();
    let border_width =
        |width: &erk_style::style::values::computed::BorderSideWidth,
         line: erk_style::style::values::computed::BorderStyle| {
            if line.none_or_hidden() {
                0.0
            } else {
                width.0.to_f32_px()
            }
        };
    let widths = [
        border_width(&border.border_top_width, border.border_top_style),
        border_width(&border.border_right_width, border.border_right_style),
        border_width(&border.border_bottom_width, border.border_bottom_style),
        border_width(&border.border_left_width, border.border_left_style),
    ];
    let border_colors = [
        &border.border_top_color,
        &border.border_right_color,
        &border.border_bottom_color,
        &border.border_left_color,
    ]
    .map(|color| srgb_bytes(style.resolve_color(color)));
    let has_border = widths
        .iter()
        .zip(&border_colors)
        .any(|(width, color)| *width > 0.0 && color[3] != 0);
    if color[3] == 0 && !has_border {
        return None;
    }
    Some(InlineLook {
        above: ascent + fixed(&padding.padding_top.0) + widths[0],
        below: descent + fixed(&padding.padding_bottom.0) + widths[2],
        color,
        border: widths,
        border_colors,
    })
}

/// A shaped, line-broken paragraph, with the line boxes adjusted for
/// atomic inlines and raised text.
pub(crate) struct InlineLayout {
    pub(crate) layout: Layout<TextBrush>,
    /// How far each line moved down from where Parley put it: atomic
    /// inlines and raised text can make a line box taller than its text.
    pub(crate) shifts: Vec<f32>,
    pub(crate) height: f32,
    /// Where each atomic inline sits: `(arena index, x, top of its margin
    /// box)`, relative to the content box.
    atoms: Vec<(usize, f32, f32)>,
    /// Where each anchor sits: `(arena index, x, top of its line, bottom of
    /// its line)`, relative to the content box.
    anchors: Vec<(usize, f32, f32, f32)>,
    /// The width the lines were broken at.
    max_advance: Option<f32>,
}

impl InlineLayout {
    pub(crate) fn width(&self) -> f32 {
        self.layout.width()
    }

    /// Whether the lines were broken at `max_advance`.
    pub(crate) fn broken_at(&self, max_advance: Option<f32>) -> bool {
        self.max_advance == max_advance
    }

    /// The baseline of line `index`, from the top of the content box.
    pub(crate) fn baseline(&self, index: usize) -> Option<f32> {
        let line = self.layout.get(index)?;
        Some(line.metrics().baseline + self.shifts.get(index).copied().unwrap_or(0.0))
    }

    pub(crate) fn line_count(&self) -> usize {
        self.layout.len()
    }

    /// The backgrounds of the paragraph's inline elements, one rectangle per
    /// line each element spans, in tree order: an element under those inside
    /// it, as browsers paint them.
    pub(crate) fn decorations(&self, paragraph: &Paragraph) -> Vec<DecorationRect> {
        let mut rects = Vec::new();
        if paragraph.decorations.is_empty() {
            return rects;
        }
        for (index, line) in self.layout.lines().enumerate() {
            let segments = line_segments(&line);
            // Trailing whitespace hangs past the line's end and has no
            // background.
            let content_end = segments
                .iter()
                .filter(|segment| !matches!(segment.kind, SegmentKind::Cluster { space: true, .. }))
                .map(|segment| segment.x1)
                .fold(f32::NEG_INFINITY, f32::max);
            let baseline = line.metrics().baseline + self.shifts.get(index).copied().unwrap_or(0.0);
            for decoration in &paragraph.decorations {
                let mut extent: Option<(f32, f32)> = None;
                let (mut starts, mut ends) = (false, false);
                for segment in &segments {
                    let (mut x0, mut x1) = (segment.x0, segment.x1);
                    let inside = match segment.kind {
                        SegmentKind::Cluster { start, .. } => decoration.text.contains(&start),
                        SegmentKind::Box(id) => {
                            if decoration.open.is_some_and(|(open, _)| open == id) {
                                x0 += decoration.open.map_or(0.0, |(_, margin)| margin);
                                starts = true;
                            }
                            if decoration.close.is_some_and(|(close, _)| close == id) {
                                x1 -= decoration.close.map_or(0.0, |(_, margin)| margin);
                                ends = true;
                            }
                            decoration.boxes.contains(&id)
                        }
                    };
                    if inside {
                        x1 = x1.min(content_end.max(x0));
                        extent = Some(extent.map_or((x0, x1), |(a, b)| (a.min(x0), b.max(x1))));
                    }
                }
                if let Some((x0, x1)) = extent
                    && x1 > x0
                {
                    // The left and right borders close the element's first
                    // and last line only (box-decoration-break: slice).
                    let [top, right, bottom, left] = decoration.border;
                    rects.push(DecorationRect {
                        x: x0,
                        y: baseline - decoration.raise - decoration.above,
                        width: x1 - x0,
                        height: decoration.above + decoration.below,
                        color: decoration.color,
                        border: [
                            top,
                            if ends { right } else { 0.0 },
                            bottom,
                            if starts { left } else { 0.0 },
                        ],
                        border_colors: decoration.border_colors,
                    });
                }
            }
        }
        rects
    }

    /// Where each atomic inline sits: `(arena index, x, top of its margin
    /// box)`, relative to the content box.
    pub(crate) fn atom_positions(&self) -> &[(usize, f32, f32)] {
        &self.atoms
    }

    /// Where each absolutely positioned element of the paragraph would have
    /// been: `(arena index, x, top of its line, bottom of its line)`,
    /// relative to the content box.
    pub(crate) fn anchor_positions(&self) -> &[(usize, f32, f32, f32)] {
        &self.anchors
    }
}

/// A piece of a line, left to right: a cluster of text or an inline box.
struct Segment {
    x0: f32,
    x1: f32,
    kind: SegmentKind,
}

enum SegmentKind {
    /// A cluster, by the start of its text, and whether it is a space.
    Cluster { start: usize, space: bool },
    /// An inline box, by its Parley id.
    Box(usize),
}

/// The clusters and inline boxes of `line` with their horizontal extents.
/// The glyph runs of one run come one after another, so each takes the
/// run's next clusters until it has its glyphs (as in `glyph_run_ranges`).
fn line_segments(line: &parley::Line<'_, TextBrush>) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor: Option<(usize, usize)> = None;
    for item in line.items() {
        match item {
            PositionedLayoutItem::GlyphRun(glyph_run) => {
                let run = glyph_run.run();
                let skip = match cursor {
                    Some((index, used)) if index == run.index() => used,
                    _ => 0,
                };
                let wanted = glyph_run.glyphs().count();
                let (mut glyphs, mut taken) = (0, 0);
                let mut x = glyph_run.offset();
                for cluster in run.visual_clusters().skip(skip) {
                    if glyphs >= wanted {
                        break;
                    }
                    glyphs += cluster.glyphs().count();
                    taken += 1;
                    let advance = cluster.advance();
                    segments.push(Segment {
                        x0: x,
                        x1: x + advance,
                        kind: SegmentKind::Cluster {
                            start: cluster.text_range().start,
                            space: cluster.is_space_or_nbsp(),
                        },
                    });
                    x += advance;
                }
                cursor = Some((run.index(), skip + taken));
            }
            PositionedLayoutItem::InlineBox(inline_box) => segments.push(Segment {
                x0: inline_box.x,
                x1: inline_box.x + inline_box.width,
                kind: SegmentKind::Box(inline_box.id as usize),
            }),
        }
    }
    segments
}

/// The text range of each glyph run on `line`, in the order `line.items()`
/// yields them. Parley splits a run into glyph runs where the style changes
/// (a colour change does not split the shaping run) but gives the text range
/// only of the whole run; the glyph runs of one run come one after another,
/// so each takes the next clusters until it has its glyphs.
pub(crate) fn glyph_run_ranges(line: &parley::Line<'_, TextBrush>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    // The run the previous glyph run came from, and how many of its visual
    // clusters are used up.
    let mut cursor: Option<(usize, usize)> = None;
    for item in line.items() {
        let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
            continue;
        };
        let run = glyph_run.run();
        let skip = match cursor {
            Some((index, used)) if index == run.index() => used,
            _ => 0,
        };
        let wanted = glyph_run.glyphs().count();
        let (mut glyphs, mut taken) = (0, 0);
        let mut range: Option<Range<usize>> = None;
        for cluster in run.visual_clusters().skip(skip) {
            if glyphs >= wanted {
                break;
            }
            glyphs += cluster.glyphs().count();
            taken += 1;
            let r = cluster.text_range();
            range = Some(match range {
                None => r,
                Some(so_far) => so_far.start.min(r.start)..so_far.end.max(r.end),
            });
        }
        cursor = Some((run.index(), skip + taken));
        let start = run.text_range().start;
        ranges.push(range.unwrap_or(start..start));
    }
    ranges
}

fn wrap_mode(style: &TextStyle) -> parley::TextWrapMode {
    if style.wrap {
        parley::TextWrapMode::Wrap
    } else {
        parley::TextWrapMode::NoWrap
    }
}

/// The `white-space-collapse` of a text token.
fn token_collapse<S: AsRef<ComputedValues>>(token: &InlineToken<S>) -> Option<WhiteSpaceCollapse> {
    match token {
        InlineToken::Text(_, style, ..) => Some(
            style
                .as_ref()
                .get_inherited_text()
                .clone_white_space_collapse(),
        ),
        _ => None,
    }
}

/// Break `layout`'s lines at `max_advance`, keeping each inline element's
/// edges with its text, as browsers do. Erk's element edges (border,
/// padding, margin) are inline boxes, and Parley breaks next to an inline
/// box: after it, so that a line can end with an opening edge whose text
/// starts the next line, leaving the edge behind as a sliver; and before
/// it when it does not fit, so that a closing edge can start a line alone.
/// Such a line is broken again, narrower: before the opening edge, or
/// before the last word the closing edge belongs to. It keeps its full
/// width for alignment.
fn break_lines(layout: &mut Layout<TextBrush>, paragraph: &Paragraph, max_advance: Option<f32>) {
    let Some(max) = max_advance else {
        // Nothing wraps.
        layout.break_all_lines(None);
        return;
    };
    // Narrower limits for the lines found to end with an edge.
    let mut limits: Vec<Option<f32>> = Vec::new();
    loop {
        let mut breaker = layout.break_lines();
        breaker.state_mut().set_layout_max_advance(max);
        let mut line = 0;
        loop {
            let limit = limits.get(line).copied().flatten();
            breaker
                .state_mut()
                .set_line_max_advance(limit.unwrap_or(max));
            if breaker.break_next().is_none() {
                break;
            }
            if limit.is_some() {
                breaker.set_prior_line_width(max);
            }
            line += 1;
        }
        breaker.finish();
        match dangling_edge(layout, paragraph, max) {
            Some((line, x)) if limits.get(line).copied().flatten().is_none() => {
                if limits.len() <= line {
                    limits.resize(line + 1, None);
                }
                limits[line] = Some(x);
            }
            _ => return,
        }
    }
}

/// The first line that an element's edge is cut off from its text at, and
/// the width to break it at instead: a line (not the last) ending with
/// opening edges breaks before the first of them; the line before one that
/// starts with closing edges breaks before its content's end, which puts
/// its last word on the next line. White space does not count as text. A
/// line that begins with the opening edges, or holds nothing before the
/// closing ones, is left: no earlier break would help.
fn dangling_edge(
    layout: &Layout<TextBrush>,
    paragraph: &Paragraph,
    max: f32,
) -> Option<(usize, f32)> {
    let edge_of = |id: u64| match paragraph.items.get(id as usize).map(|item| &item.kind) {
        Some(InlineItemKind::Spacer { opening, .. }) => Some(*opening),
        _ => None,
    };
    let is_text = |run: &parley::GlyphRun<'_, TextBrush>| {
        !paragraph
            .text
            .get(run.run().text_range())
            .unwrap_or_default()
            .trim()
            .is_empty()
    };
    let lines: Vec<_> = layout.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        // Opening edges at the end, and where the content before them ends.
        let mut opening: Option<f32> = None;
        let mut content_end = 0.0_f32;
        for item in line.items() {
            match item {
                PositionedLayoutItem::InlineBox(inline_box) => {
                    if edge_of(inline_box.id) == Some(true) {
                        opening.get_or_insert(inline_box.x);
                    } else {
                        opening = None;
                    }
                    content_end = content_end.max(inline_box.x + inline_box.width);
                }
                PositionedLayoutItem::GlyphRun(run) => {
                    if is_text(&run) {
                        opening = None;
                        content_end = content_end.max(run.offset() + run.advance());
                    }
                }
            }
        }
        let Some(next) = lines.get(index + 1) else {
            break;
        };
        // A narrower width helps only if it is narrower than the line's.
        let useful = |x: &f32| *x > 0.0 && *x < max;
        if let Some(x) = opening.filter(useful) {
            return Some((index, x));
        }
        // Closing edges before any text on the next line.
        let mut starts_closed = false;
        for item in next.items() {
            match item {
                PositionedLayoutItem::InlineBox(inline_box) => {
                    if edge_of(inline_box.id) == Some(false) {
                        starts_closed = true;
                    } else {
                        break;
                    }
                }
                PositionedLayoutItem::GlyphRun(run) => {
                    if is_text(&run) {
                        break;
                    }
                }
            }
        }
        if starts_closed && useful(&(content_end - 0.5)) {
            return Some((index, content_end - 0.5));
        }
    }
    None
}

/// One cluster of a line as it was placed: its range of the paragraph's
/// text, its left and right edges and the top and bottom of its font's box
/// (ascent and descent around its baseline, raised by `vertical-align`), in
/// the layout's coordinates; and whether it is white space.
pub(crate) struct PlacedCluster {
    pub(crate) text: Range<usize>,
    pub(crate) left: f32,
    pub(crate) right: f32,
    pub(crate) top: f32,
    pub(crate) bottom: f32,
    pub(crate) space: bool,
}

/// The clusters of `line` in visual order, each where its glyph run put it,
/// and whether an inline box that takes room follows the last of them. A
/// glyph run takes its clusters from its run the way [`glyph_run_ranges`]
/// counts them.
pub(crate) fn placed_clusters(line: &parley::Line<'_, TextBrush>) -> (Vec<PlacedCluster>, bool) {
    let mut placed = Vec::new();
    let mut cursor: Option<(usize, usize)> = None;
    let mut then_a_box = false;
    for item in line.items() {
        let glyph_run = match item {
            PositionedLayoutItem::GlyphRun(glyph_run) => {
                then_a_box = false;
                glyph_run
            }
            PositionedLayoutItem::InlineBox(inline_box) => {
                then_a_box |= inline_box.width > 0.0;
                continue;
            }
        };
        let run = glyph_run.run();
        let skip = match cursor {
            Some((index, used)) if index == run.index() => used,
            _ => 0,
        };
        let wanted = glyph_run.glyphs().count();
        let metrics = run.metrics();
        let baseline = glyph_run.baseline() - glyph_run.style().brush.raise;
        let (mut glyphs, mut taken) = (0, 0);
        let mut x = glyph_run.offset();
        for cluster in run.visual_clusters().skip(skip) {
            if glyphs >= wanted {
                break;
            }
            glyphs += cluster.glyphs().count();
            taken += 1;
            let advance = cluster.advance();
            placed.push(PlacedCluster {
                text: cluster.text_range(),
                left: x,
                right: x + advance,
                top: baseline - metrics.ascent,
                bottom: baseline + metrics.descent,
                space: cluster.is_space_or_nbsp(),
            });
            x += advance;
        }
        cursor = Some((run.index(), skip + taken));
    }
    (placed, then_a_box)
}

/// The block's `text-align` as Parley's alignment. The legacy `-moz-` values
/// come from the `align` presentational attribute.
fn alignment(block: &ComputedValues) -> Alignment {
    use erk_style::style::values::computed::TextAlign;
    match block.get_inherited_text().clone_text_align() {
        TextAlign::End => Alignment::End,
        TextAlign::Left | TextAlign::MozLeft => Alignment::Left,
        TextAlign::Right | TextAlign::MozRight => Alignment::Right,
        TextAlign::Center | TextAlign::MozCenter => Alignment::Center,
        TextAlign::Justify => Alignment::Justify,
        _ => Alignment::Start,
    }
}

fn is_document_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Whether a shaped run uses the embedded bold face.
#[cfg(test)]
pub(crate) fn is_bold_face(font: &parley::FontData) -> bool {
    font.data.as_ref() == NOTO_SANS_BOLD
}

/// `line-height: normal`: the font's ascent, descent and line gap, each
/// rounded to whole pixels before they are added. The rounding is what
/// Chrome does, and it matters: unrounded, a 16px Noto Sans line is 21.79px
/// instead of 22, and the shortfall accumulates down the page (found by the
/// Chrome reference test).
fn normal_line_height(font_size: f32, weight: f32) -> f32 {
    use skrifa::instance::{LocationRef, Size as FontSize};
    use skrifa::{FontRef, MetadataProvider};

    let font = FontRef::new(face_for(weight)).expect("embedded font parses");
    let metrics = font.metrics(FontSize::new(font_size), LocationRef::default());
    metrics.ascent.round() + (-metrics.descent).round() + metrics.leading.round()
}

/// `value` cut toward zero to Blink's layout unit, 1/64 px.
fn layout_unit(value: f32) -> f32 {
    (value * 64.0).trunc() / 64.0
}

/// The font's x-height, for `vertical-align: middle`.
pub(crate) fn x_height(font_size: f32, weight: f32) -> f32 {
    use skrifa::instance::{LocationRef, Size as FontSize};
    use skrifa::{FontRef, MetadataProvider};

    let font = FontRef::new(face_for(weight)).expect("embedded font parses");
    let metrics = font.metrics(FontSize::new(font_size), LocationRef::default());
    metrics.x_height.unwrap_or(font_size / 2.0)
}

/// The font's ascent and descent, each rounded to whole pixels as Chrome
/// rounds them: the height of an inline box's content area.
fn font_extents(font_size: f32, weight: f32) -> (f32, f32) {
    use skrifa::instance::{LocationRef, Size as FontSize};
    use skrifa::{FontRef, MetadataProvider};

    let font = FontRef::new(face_for(weight)).expect("embedded font parses");
    let metrics = font.metrics(FontSize::new(font_size), LocationRef::default());
    (metrics.ascent.round(), (-metrics.descent).round())
}

fn face_for(weight: f32) -> &'static [u8] {
    if weight >= 600.0 {
        NOTO_SANS_BOLD
    } else {
        NOTO_SANS_REGULAR
    }
}

/// Collapse runs of document whitespace to one space and trim both ends.
#[cfg(test)]
fn collapse_whitespace(text: &str) -> String {
    text.split(is_document_whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Answers Stylo's font metric queries (for `ex`, `ch`, `cap`) from the
/// embedded Noto Sans, so font-relative units agree with the text that is
/// actually drawn.
#[derive(Debug)]
pub(crate) struct EmbeddedFontMetrics;

impl FontMetricsProvider for EmbeddedFontMetrics {
    fn query_font_metrics(
        &self,
        _vertical: bool,
        font: &FontStyle,
        font_size: CSSPixelLength,
        _flags: QueryFontMetricsFlags,
    ) -> StyleFontMetrics {
        use skrifa::instance::{LocationRef, Size as FontSize};
        use skrifa::{FontRef, MetadataProvider};

        let font_ref =
            FontRef::new(face_for(font.font_weight.value())).expect("embedded font parses");
        let size = FontSize::new(font_size.px());
        let metrics = font_ref.metrics(size, LocationRef::default());
        let zero_advance = font_ref.charmap().map('0').and_then(|glyph| {
            font_ref
                .glyph_metrics(size, LocationRef::default())
                .advance_width(glyph)
        });
        StyleFontMetrics {
            ascent: CSSPixelLength::new(metrics.ascent),
            x_height: metrics.x_height.map(CSSPixelLength::new),
            cap_height: metrics.cap_height.map(CSSPixelLength::new),
            zero_advance_measure: zero_advance.map(CSSPixelLength::new),
            // Noto Sans has no CJK water ideograph, which `ic` is defined by.
            ic_width: None,
            script_percent_scale_down: None,
            script_script_percent_scale_down: None,
        }
    }

    fn base_size_for_generic(&self, generic: GenericFontFamily) -> Length {
        let px = match generic {
            GenericFontFamily::Monospace => 13.0,
            _ => 16.0,
        };
        Length::new(px)
    }
}

/// Parley's font and layout contexts, with the embedded fonts registered,
/// and the host's fonts that have arrived.
pub(crate) struct TextEngine {
    fonts: FontContext,
    layouts: LayoutContext<TextBrush>,
    catalogue: Option<FontCatalog>,
}

impl TextEngine {
    /// The embedded fonts only.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_fonts(&HostFonts::default())
    }

    /// The embedded fonts, the host's faces that have arrived, and what its
    /// catalogue says the generic families and the fallback lists are.
    pub(crate) fn with_fonts(host: &HostFonts) -> Self {
        let mut fonts = FontContext {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            source_cache: SourceCache::default(),
        };
        for font in [NOTO_SANS_REGULAR, NOTO_SANS_BOLD] {
            fonts
                .collection
                .register_fonts(Blob::new(Arc::new(font)), None);
        }
        for blob in host.loaded() {
            fonts.collection.register_fonts(blob.clone(), None);
        }
        if let Some(catalogue) = host.catalogue() {
            let collection = &mut fonts.collection;
            let mut ids = |names: &[String]| -> Vec<FamilyId> {
                names
                    .iter()
                    .filter_map(|name| collection.family_id(name))
                    .collect()
            };
            let generics: Vec<_> = catalogue
                .generic
                .iter()
                .filter_map(|entry| {
                    Some((fonts::generic_named(&entry.generic)?, ids(&entry.families)))
                })
                .collect();
            let fallbacks: Vec<_> = catalogue
                .fallback
                .iter()
                .filter_map(|entry| {
                    let script = FontScript::from_str(&entry.script).ok()?;
                    let key = FallbackKey::new(script, fonts::language(&entry.language).as_ref());
                    Some((key, ids(&entry.families)))
                })
                .collect();
            for (generic, families) in generics {
                fonts
                    .collection
                    .set_generic_families(generic, families.into_iter());
            }
            for (key, families) in fallbacks {
                fonts.collection.set_fallbacks(key, families.into_iter());
            }
        }
        Self {
            fonts,
            layouts: LayoutContext::new(),
            catalogue: host.catalogue().cloned(),
        }
    }

    /// The font family list Parley gets for `style`: the CSS list, then the
    /// embedded font.
    fn family_list(style: &TextStyle) -> FontFamily<'static> {
        let mut list: Vec<FontFamilyName<'static>> = style
            .families
            .iter()
            .map(|family| match family {
                Family::Named(name) => FontFamilyName::Named(Cow::Owned(name.clone())),
                Family::Generic(generic) => FontFamilyName::Generic(*generic),
            })
            .collect();
        list.push(FontFamilyName::Named(Cow::Borrowed(FAMILY)));
        FontFamily::List(Cow::Owned(list))
    }

    /// The language Parley is told for `style`: only one the catalogue has
    /// a fallback list for, so that every other text uses the lists for any
    /// language (fontique finds nothing for a language it keys but has no
    /// list for).
    fn locale(&self, style: &TextStyle) -> Option<Language> {
        let catalogue = self.catalogue.as_ref()?;
        style
            .language
            .filter(|language| fonts::has_language_fallback(catalogue, language))
    }

    /// Shape `paragraph` and break it into lines no wider than
    /// `max_advance` (no limit when `None`). `atoms` are the sizes of the
    /// paragraph's atomic inlines, in order.
    ///
    /// Inline boxes reach Parley with no height: Parley gives a line one
    /// line height and splits its leading around the tallest content, where
    /// CSS gives each inline box its own place around the baseline. Lines
    /// holding atoms or raised text are therefore sized here, from the
    /// strut, the text's own extent and each box's extent above and below
    /// the baseline (CSS 2 §10.8); the lines after them move down.
    pub(crate) fn shape(
        &mut self,
        paragraph: &Paragraph,
        max_advance: Option<f32>,
        atoms: &[AtomBox],
    ) -> InlineLayout {
        let mut layout = self.shape_text(paragraph, max_advance, atoms);
        layout.align(paragraph.align, AlignmentOptions::default());
        let mut shifts = vec![0.0; layout.len()];
        let mut height = layout.height();
        let mut placed = Vec::new();
        let has_atoms = paragraph.atoms().next().is_some();
        // Styled ranges too: Parley 0.11 gives a line the height of its last
        // run when that run is smaller than the rest, so a line ending in
        // small text came out short. The strut corrects it below.
        if has_atoms || !paragraph.raised.is_empty() || !paragraph.spans.is_empty() {
            // Parley ids are positions in `items`; atoms are measured in order.
            let mut sizes = atoms.iter();
            let atom_of: Vec<Option<(usize, AtomBox, VerticalAlign, ParentBox)>> = paragraph
                .items
                .iter()
                .map(|item| match item.kind {
                    InlineItemKind::Atom(index, align, parent) => Some((
                        index,
                        sizes.next().copied().unwrap_or_default(),
                        align,
                        parent,
                    )),
                    InlineItemKind::Spacer { .. } | InlineItemKind::Anchor(_) => None,
                })
                .collect();
            let (strut_above, strut_below) = paragraph.strut();
            let mut y = 0.0_f32;
            for (index, line) in layout.lines().enumerate() {
                let metrics = line.metrics();
                let baseline = metrics.baseline;
                let mut above = baseline - metrics.block_min_coord;
                let mut below = metrics.block_max_coord - baseline;
                let mut special = false;
                let text = line.text_range();
                for raised in &paragraph.raised {
                    if raised.text.start < text.end && text.start < raised.text.end {
                        special = true;
                        above = above.max(raised.raise + raised.above);
                        below = below.max(raised.below - raised.raise);
                    }
                }
                // Atoms on this line: (index, x, size, raise or the line anchor).
                let mut on_line = Vec::new();
                for item in line.items() {
                    if let PositionedLayoutItem::InlineBox(inline_box) = item
                        && let Some(Some((atom, size, align, parent))) =
                            atom_of.get(inline_box.id as usize)
                    {
                        special = true;
                        let raise = align.raise(parent, size.above, size.below);
                        if let Some(raise) = raise {
                            above = above.max(size.above + raise);
                            below = below.max(size.below - raise);
                        }
                        on_line.push((*atom, inline_box.x, *size, raise, *align));
                    }
                }
                // Every line box starts with the strut (CSS 2 §10.8.1).
                if special || above < strut_above || below < strut_below {
                    special = true;
                    above = above.max(strut_above);
                    below = below.max(strut_below);
                }
                // Boxes aligned to the line box take part only in its height:
                // a taller one grows the line on the side away from its edge.
                for &(_, _, size, raise, align) in &on_line {
                    let total = size.above + size.below;
                    if raise.is_none() && total > above + below {
                        let grow = total - (above + below);
                        match align {
                            VerticalAlign::Line(LineAnchor::Bottom) => above += grow,
                            _ => below += grow,
                        }
                    }
                }
                let line_height = above + below;
                for (atom, x, size, raise, align) in on_line {
                    let total = size.above + size.below;
                    let top = match (raise, align) {
                        (Some(raise), _) => y + above - raise - size.above,
                        (None, VerticalAlign::Line(LineAnchor::Bottom)) => y + line_height - total,
                        (None, VerticalAlign::Line(LineAnchor::Center)) => {
                            y + (line_height - total) / 2.0
                        }
                        (None, _) => y,
                    };
                    placed.push((atom, x, top));
                }
                if !special {
                    above = baseline - metrics.block_min_coord;
                    below = metrics.block_max_coord - baseline;
                }
                // The line's top moves to `y`, its baseline `above` below that.
                shifts[index] = y + above - baseline;
                y += above + below;
            }
            height = y;
        }
        let mut anchors = Vec::new();
        for (index, line) in layout.lines().enumerate() {
            let metrics = line.metrics();
            let shift = shifts[index];
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(inline_box) = item
                    && let Some(InlineItemKind::Anchor(element)) = paragraph
                        .items
                        .get(inline_box.id as usize)
                        .map(|item| item.kind)
                {
                    anchors.push((
                        element,
                        inline_box.x,
                        metrics.block_min_coord + shift,
                        metrics.block_max_coord + shift,
                    ));
                }
            }
        }
        InlineLayout {
            layout,
            shifts,
            height,
            atoms: placed,
            anchors,
            max_advance,
        }
    }

    fn shape_text(
        &mut self,
        paragraph: &Paragraph,
        max_advance: Option<f32>,
        atoms: &[AtomBox],
    ) -> Layout<TextBrush> {
        let base_locale = self.locale(&paragraph.base);
        let locales: Vec<Option<Language>> = paragraph
            .spans
            .iter()
            .map(|(_, style)| self.locale(style))
            .collect();
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, &paragraph.text, 1.0, true);
        let base = &paragraph.base;
        builder.push_default(StyleProperty::FontFamily(Self::family_list(base)));
        if base.italic {
            builder.push_default(StyleProperty::FontStyle(ParleyFontStyle::Italic));
        }
        builder.push_default(StyleProperty::Locale(base_locale));
        builder.push_default(StyleProperty::FontSize(base.font_size));
        builder.push_default(StyleProperty::LineHeight(base.line_height));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(base.weight)));
        builder.push_default(StyleProperty::Brush(base.color));
        builder.push_default(StyleProperty::TextWrapMode(wrap_mode(base)));
        for ((range, style), locale) in paragraph.spans.iter().zip(locales) {
            builder.push(
                StyleProperty::FontFamily(Self::family_list(style)),
                range.clone(),
            );
            let font_style = if style.italic {
                ParleyFontStyle::Italic
            } else {
                ParleyFontStyle::Normal
            };
            builder.push(StyleProperty::FontStyle(font_style), range.clone());
            builder.push(StyleProperty::Locale(locale), range.clone());
            builder.push(StyleProperty::FontSize(style.font_size), range.clone());
            builder.push(StyleProperty::LineHeight(style.line_height), range.clone());
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(style.weight)),
                range.clone(),
            );
            builder.push(StyleProperty::Brush(style.color), range.clone());
            builder.push(StyleProperty::TextWrapMode(wrap_mode(style)), range.clone());
        }
        let mut atoms = atoms.iter();
        for (id, item) in paragraph.items.iter().enumerate() {
            let width = match item.kind {
                InlineItemKind::Spacer { width, .. } => width,
                InlineItemKind::Anchor(_) => 0.0,
                InlineItemKind::Atom(..) => atoms.next().map_or(0.0, |atom| atom.width),
            };
            builder.push_inline_box(parley::InlineBox {
                id: id as u64,
                kind: parley::InlineBoxKind::InFlow,
                index: item.index,
                width,
                height: 0.0,
            });
        }
        let mut layout = builder.build(&paragraph.text);
        break_lines(&mut layout, paragraph, max_advance);
        layout
    }

    /// The size of an atom-free paragraph, as Taffy's measure function
    /// would report it.
    #[cfg(test)]
    pub(crate) fn measure(
        &mut self,
        paragraph: &Paragraph,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
    ) -> Size<f32> {
        let max_advance = known.width.or(match available.width {
            AvailableSpace::Definite(width) => Some(width),
            // Break at every opportunity: the result is as wide as the
            // longest unbreakable run.
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
        let layout = self.shape(paragraph, max_advance, &[]);
        Size {
            width: known.width.unwrap_or_else(|| layout.width()),
            height: known.height.unwrap_or(layout.height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str, weight: f32) -> Paragraph {
        Paragraph {
            text: collapse_whitespace(text),
            base: TextStyle {
                font_size: 16.0,
                line_height: LineHeight::MetricsRelative(1.0),
                weight,
                italic: false,
                families: Arc::new([]),
                language: None,
                color: TextBrush::default(),
                wrap: true,
            },
            spans: Vec::new(),
            align: Alignment::Start,
            items: Vec::new(),
            decorations: Vec::new(),
            raised: Vec::new(),
            pending_break: false,
            preserved: Vec::new(),
            last_nowrap: false,
            space_wraps: true,
            sources: Vec::new(),
        }
    }

    #[test]
    fn whitespace_collapses_and_trims() {
        assert_eq!(
            collapse_whitespace("\n  Merhaba \t\n dünya  "),
            "Merhaba dünya"
        );
    }

    #[test]
    fn turkish_letters_all_have_glyphs() {
        let mut engine = TextEngine::new();
        let layout = engine
            .shape(&paragraph("İstanbul Işık ğüşöç ĞÜŞÖÇ", 400.0), None, &[])
            .layout;
        let mut glyphs = 0;
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    for glyph in run.positioned_glyphs() {
                        assert_ne!(glyph.id, 0, "missing glyph (.notdef) in the embedded font");
                        glyphs += 1;
                    }
                }
            }
        }
        assert!(glyphs > 0);
    }

    #[test]
    fn narrow_width_breaks_into_more_lines() {
        let mut engine = TextEngine::new();
        let text = paragraph(
            "Erk sayfayı önce çizer, sonra izole eder, en son betik çalıştırır.",
            400.0,
        );
        let one_line = engine.shape(&text, None, &[]).layout;
        let wrapped = engine.shape(&text, Some(120.0), &[]).layout;

        assert_eq!(one_line.len(), 1);
        assert!(
            wrapped.len() > 2,
            "expected several lines, got {}",
            wrapped.len()
        );
        assert!(wrapped.width() <= 120.0);
        let line = one_line.height();
        assert!((wrapped.height() - line * wrapped.len() as f32).abs() < 0.5);
    }

    #[test]
    fn bold_uses_the_bold_face() {
        let mut engine = TextEngine::new();
        let regular = engine.shape(&paragraph("Merhaba dünya", 400.0), None, &[]);
        let bold = engine.shape(&paragraph("Merhaba dünya", 700.0), None, &[]);
        // Noto Sans Bold's advances are wider than Regular's; a synthesized
        // bold of the Regular face would keep Regular's advances.
        assert!(bold.width() > regular.width() + 1.0);
    }

    #[test]
    fn min_content_is_the_longest_word() {
        let mut engine = TextEngine::new();
        let text = paragraph("a bb uzunkelime", 400.0);
        let min = engine.measure(
            &text,
            Size::NONE,
            Size {
                width: AvailableSpace::MinContent,
                height: AvailableSpace::MaxContent,
            },
        );
        let word = engine
            .shape(&paragraph("uzunkelime", 400.0), None, &[])
            .width();
        assert!((min.width - word).abs() < 0.5);
    }
}
