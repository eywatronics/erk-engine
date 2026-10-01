//! Text shaping and line breaking with Parley.
//!
//! A paragraph is one run of text with styled ranges: each inline element's
//! text keeps its own size, weight, colour and line height. Text is drawn in
//! the embedded Noto Sans: system fonts are not loaded, so the same page
//! measures and paints the same on every machine. CSS `font-family` is not
//! consulted yet (M1.7).

use std::ops::Range;
use std::sync::Arc;

use erk_style::style::properties::style_structs::Font as FontStyle;
use erk_style::style::values::computed::font::LineHeight as CssLineHeight;
use erk_style::style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use erk_style::style::values::computed::{CSSPixelLength, Length};
use erk_style::{ComputedValues, FontMetricsProvider, StyleFontMetrics};

use crate::color::{Rgba, srgb_bytes};
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontWeight, Layout, LayoutContext, LineHeight,
    PositionedLayoutItem, StyleProperty,
};
#[cfg(test)]
use taffy::{AvailableSpace, Size};

const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
const NOTO_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
const FAMILY: &str = "Noto Sans";

/// Text colour, as straight (non-premultiplied) sRGB bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextBrush(pub(crate) [u8; 4]);

/// The text properties of one styled range.
#[derive(Clone, Debug, PartialEq)]
struct TextStyle {
    font_size: f32,
    line_height: LineHeight,
    weight: f32,
    color: TextBrush,
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
            color: TextBrush(srgb_bytes(style.clone_color())),
        }
    }
}

/// One item of a block's inline content, in tree order.
pub(crate) enum InlineToken<S> {
    /// The text of a text node, with the style of its element.
    Text(String, S),
    /// An inline element starts; its style gives its padding, border,
    /// margin and background.
    Open(S),
    /// The innermost open inline element ends.
    Close,
    /// An atomic inline (`inline-block`, `inline-flex`): laid out as a box
    /// of its own and placed in the line like a glyph. The arena index.
    Atom(usize),
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
    /// margin, border and padding on that side.
    Spacer(f32),
    /// An atomic inline, by arena index.
    Atom(usize),
}

/// The size an atomic inline takes in its line, measured by layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AtomBox {
    /// Margin box width.
    pub(crate) width: f32,
    /// From the top of the margin box to the box's baseline, which sits on
    /// the line's baseline.
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
    color: Rgba,
}

/// A painted background of an inline element on one line, relative to the
/// paragraph's content box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DecorationRect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) color: Rgba,
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
}

/// An inline element being read, until its `Close`.
struct OpenElement {
    first_box: usize,
    text_start: usize,
    open: Option<(usize, f32)>,
    end_spacer: f32,
    end_margin: f32,
    decoration: Option<(f32, f32, Rgba)>,
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
        };
        let mut open: Vec<OpenElement> = Vec::new();
        // Whether content (text or an atom) has been seen, whether the last
        // character was a space, and whether a space is due before the next
        // content.
        let (mut has_content, mut last_was_space, mut pending_space) = (false, false, false);
        // Element ends seen since the last content: whether each came after
        // the pending space (the space is then inside the element).
        let mut closes: Vec<(OpenElement, bool)> = Vec::new();

        for token in tokens {
            match token {
                InlineToken::Text(raw, style) => {
                    let style = TextStyle::of(style.as_ref());
                    let mut start = None;
                    for c in raw.chars() {
                        if is_document_whitespace(c) {
                            pending_space |= has_content && !last_was_space;
                            continue;
                        }
                        paragraph.flush(&mut closes, &mut pending_space, &mut last_was_space);
                        start.get_or_insert(paragraph.text.len());
                        paragraph.text.push(c);
                        has_content = true;
                        last_was_space = false;
                    }
                    if let Some(start) = start
                        && style != paragraph.base
                    {
                        let end = paragraph.text.len();
                        paragraph.spans.push((start..end, style));
                    }
                }
                InlineToken::Open(style) => {
                    paragraph.flush(&mut closes, &mut pending_space, &mut last_was_space);
                    let style = style.as_ref();
                    let sides = InlineSides::of(style);
                    let open_box = (sides.start > 0.0).then(|| {
                        paragraph.push_item(InlineItemKind::Spacer(sides.start));
                        (paragraph.items.len() - 1, sides.margin_start)
                    });
                    open.push(OpenElement {
                        first_box: open_box.map_or(paragraph.items.len(), |(index, _)| index),
                        text_start: paragraph.text.len(),
                        open: open_box,
                        end_spacer: sides.end,
                        end_margin: sides.margin_end,
                        decoration: decoration_of(style),
                    });
                }
                InlineToken::Close => {
                    if let Some(element) = open.pop() {
                        closes.push((element, pending_space));
                    }
                }
                InlineToken::Atom(index) => {
                    paragraph.flush(&mut closes, &mut pending_space, &mut last_was_space);
                    paragraph.push_item(InlineItemKind::Atom(*index));
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
        paragraph.flush(&mut closes, &mut pending_space, &mut last_was_space);
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
    ) {
        let (after, before): (Vec<_>, Vec<_>) =
            closes.drain(..).partition(|(_, after_space)| *after_space);
        for (element, _) in before {
            self.close(element);
        }
        if std::mem::take(pending_space) {
            self.text.push(' ');
            *last_was_space = true;
        }
        for (element, _) in after {
            self.close(element);
        }
    }

    fn close(&mut self, element: OpenElement) {
        let close = (element.end_spacer > 0.0).then(|| {
            self.push_item(InlineItemKind::Spacer(element.end_spacer));
            (self.items.len() - 1, element.end_margin)
        });
        if let Some((above, below, color)) = element.decoration {
            self.decorations.push(Decoration {
                text: element.text_start..self.text.len(),
                boxes: element.first_box..self.items.len(),
                open: element.open,
                close,
                above,
                below,
                color,
            });
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

    /// The arena indices of the atomic inlines, in order.
    pub(crate) fn atoms(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().filter_map(|item| match item.kind {
            InlineItemKind::Atom(index) => Some(index),
            InlineItemKind::Spacer(_) => None,
        })
    }

    /// The strut: the space above and below the baseline that the block's
    /// own font and line height give every line (CSS 2 §10.8.1), with the
    /// half-leading split as Chrome splits it.
    fn strut(&self) -> (f32, f32) {
        let (ascent, descent) = font_extents(self.base.font_size, self.base.weight);
        let line_height = match self.base.line_height {
            LineHeight::Absolute(px) => px,
            LineHeight::FontSizeRelative(factor) => factor * self.base.font_size,
            LineHeight::MetricsRelative(factor) => factor * (ascent + descent),
        };
        let leading = line_height - (ascent + descent);
        let above = (leading * 0.5).floor();
        let below = leading.round() - above;
        (ascent + above, descent + below)
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

/// An inline element's painted background, as `(above, below, colour)`, or
/// `None` if it has none.
fn decoration_of(style: &ComputedValues) -> Option<(f32, f32, Rgba)> {
    use erk_style::style::computed_values::visibility::T as Visibility;
    let color = srgb_bytes(style.resolve_color(&style.get_background().background_color));
    if color[3] == 0 || style.clone_visibility() != Visibility::Visible {
        return None;
    }
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
    Some((
        ascent
            + fixed(&padding.padding_top.0)
            + border_width(&border.border_top_width, border.border_top_style),
        descent
            + fixed(&padding.padding_bottom.0)
            + border_width(&border.border_bottom_width, border.border_bottom_style),
        color,
    ))
}

/// A shaped, line-broken paragraph, with the line boxes adjusted for
/// atomic inlines.
pub(crate) struct InlineLayout {
    pub(crate) layout: Layout<TextBrush>,
    /// How far each line moved down from where Parley put it: atomic
    /// inlines can make a line box taller than its text.
    pub(crate) shifts: Vec<f32>,
    pub(crate) height: f32,
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
    /// line each element spans, in the order the elements end.
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
                for segment in &segments {
                    let (mut x0, mut x1) = (segment.x0, segment.x1);
                    let inside = match segment.kind {
                        SegmentKind::Cluster { start, .. } => decoration.text.contains(&start),
                        SegmentKind::Box(id) => {
                            if decoration.open.is_some_and(|(open, _)| open == id) {
                                x0 += decoration.open.map_or(0.0, |(_, margin)| margin);
                            }
                            if decoration.close.is_some_and(|(close, _)| close == id) {
                                x1 -= decoration.close.map_or(0.0, |(_, margin)| margin);
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
                    rects.push(DecorationRect {
                        x: x0,
                        y: baseline - decoration.above,
                        width: x1 - x0,
                        height: decoration.above + decoration.below,
                        color: decoration.color,
                    });
                }
            }
        }
        rects
    }

    /// Where each atomic inline sits: `(arena index, x, baseline)`, relative
    /// to the content box.
    pub(crate) fn atom_positions(&self, paragraph: &Paragraph) -> Vec<(usize, f32, f32)> {
        let mut positions = Vec::new();
        for (index, line) in self.layout.lines().enumerate() {
            let baseline = line.metrics().baseline + self.shifts.get(index).copied().unwrap_or(0.0);
            for item in line.items() {
                if let PositionedLayoutItem::InlineBox(inline_box) = item
                    && let Some(InlineItemKind::Atom(atom)) = paragraph
                        .items
                        .get(inline_box.id as usize)
                        .map(|item| item.kind)
                {
                    positions.push((atom, inline_box.x, baseline));
                }
            }
        }
        positions
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

/// Parley's font and layout contexts, with the embedded fonts registered.
pub(crate) struct TextEngine {
    fonts: FontContext,
    layouts: LayoutContext<TextBrush>,
}

impl TextEngine {
    pub(crate) fn new() -> Self {
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
        Self {
            fonts,
            layouts: LayoutContext::new(),
        }
    }

    /// Shape `paragraph` and break it into lines no wider than
    /// `max_advance` (no limit when `None`). `atoms` are the sizes of the
    /// paragraph's atomic inlines, in order.
    ///
    /// Inline boxes reach Parley with no height: Parley gives a line one
    /// line height and splits its leading around the tallest content, where
    /// CSS gives each inline box its own place around the baseline. Lines
    /// holding atoms are therefore sized here, from the strut, the text's
    /// own extent and each atom's extent above and below the baseline (CSS 2
    /// §10.8), and the lines after them move down.
    pub(crate) fn shape(
        &mut self,
        paragraph: &Paragraph,
        max_advance: Option<f32>,
        atoms: &[AtomBox],
    ) -> InlineLayout {
        let mut layout = self.shape_text(paragraph, max_advance, atoms);
        let mut shifts = vec![0.0; layout.len()];
        let mut height = layout.height();
        if atoms.iter().any(|atom| *atom != AtomBox::default()) {
            let atom_of = |id: u64| {
                let mut seen = 0;
                for (index, item) in paragraph.items.iter().enumerate() {
                    if let InlineItemKind::Atom(_) = item.kind {
                        if index as u64 == id {
                            return atoms.get(seen).copied();
                        }
                        seen += 1;
                    }
                }
                None
            };
            let (strut_above, strut_below) = paragraph.strut();
            let mut y = 0.0_f32;
            for (index, line) in layout.lines().enumerate() {
                let metrics = line.metrics();
                let baseline = metrics.baseline;
                let mut above = (baseline - metrics.block_min_coord).max(strut_above);
                let mut below = (metrics.block_max_coord - baseline).max(strut_below);
                let mut has_atom = false;
                for item in line.items() {
                    if let PositionedLayoutItem::InlineBox(inline_box) = item
                        && let Some(atom) = atom_of(inline_box.id)
                    {
                        has_atom = true;
                        above = above.max(atom.above);
                        below = below.max(atom.below);
                    }
                }
                if !has_atom {
                    above = baseline - metrics.block_min_coord;
                    below = metrics.block_max_coord - baseline;
                }
                // The line's top moves to `y`, its baseline `above` below that.
                shifts[index] = y + above - baseline;
                y += above + below;
            }
            height = y;
        }
        layout.align(paragraph.align, AlignmentOptions::default());
        InlineLayout {
            layout,
            shifts,
            height,
            max_advance,
        }
    }

    fn shape_text(
        &mut self,
        paragraph: &Paragraph,
        max_advance: Option<f32>,
        atoms: &[AtomBox],
    ) -> Layout<TextBrush> {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, &paragraph.text, 1.0, true);
        let base = &paragraph.base;
        builder.push_default(StyleProperty::FontFamily(FAMILY.into()));
        builder.push_default(StyleProperty::FontSize(base.font_size));
        builder.push_default(StyleProperty::LineHeight(base.line_height));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(base.weight)));
        builder.push_default(StyleProperty::Brush(base.color));
        for (range, style) in &paragraph.spans {
            builder.push(StyleProperty::FontSize(style.font_size), range.clone());
            builder.push(StyleProperty::LineHeight(style.line_height), range.clone());
            builder.push(
                StyleProperty::FontWeight(FontWeight::new(style.weight)),
                range.clone(),
            );
            builder.push(StyleProperty::Brush(style.color), range.clone());
        }
        let mut atoms = atoms.iter();
        for (id, item) in paragraph.items.iter().enumerate() {
            let width = match item.kind {
                InlineItemKind::Spacer(width) => width,
                InlineItemKind::Atom(_) => atoms.next().map_or(0.0, |atom| atom.width),
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
        layout.break_all_lines(max_advance);
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
                color: TextBrush::default(),
            },
            spans: Vec::new(),
            align: Alignment::Start,
            items: Vec::new(),
            decorations: Vec::new(),
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
