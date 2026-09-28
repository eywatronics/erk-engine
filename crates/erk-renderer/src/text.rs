//! Text shaping and line breaking with Parley.
//!
//! M0 renders every paragraph with one style, in the embedded Noto Sans:
//! system fonts are not loaded, so the same page measures and paints the
//! same on every machine. CSS `font-family` is not consulted yet.

use std::sync::Arc;

use erk_style::style::properties::style_structs::Font as FontStyle;
use erk_style::style::values::computed::font::LineHeight as CssLineHeight;
use erk_style::style::values::computed::font::{GenericFontFamily, QueryFontMetricsFlags};
use erk_style::style::values::computed::{CSSPixelLength, Length};
use erk_style::{ComputedValues, FontMetricsProvider, StyleFontMetrics};

use crate::color::srgb_bytes;
use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
use parley::{
    Alignment, AlignmentOptions, FontContext, FontWeight, Layout, LayoutContext, LineHeight,
    StyleProperty,
};
use taffy::{AvailableSpace, Size};

const NOTO_SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/NotoSans-Regular.ttf");
const NOTO_SANS_BOLD: &[u8] = include_bytes!("../assets/fonts/NotoSans-Bold.ttf");
const FAMILY: &str = "Noto Sans";

/// Text colour, as straight (non-premultiplied) sRGB bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TextBrush(pub(crate) [u8; 4]);

/// Everything needed to shape one paragraph.
#[derive(Clone, Debug)]
pub(crate) struct Paragraph {
    pub(crate) text: String,
    font_size: f32,
    line_height: LineHeight,
    weight: f32,
    pub(crate) color: TextBrush,
}

impl Paragraph {
    /// A paragraph of `text` styled like `style`, with whitespace collapsed as
    /// `white-space: normal` does.
    pub(crate) fn new(text: &str, style: &ComputedValues) -> Self {
        let font_size = style.get_font().clone_font_size().computed_size().px();
        let weight = style.clone_font_weight().value();
        let line_height = match style.clone_line_height() {
            CssLineHeight::Normal => LineHeight::Absolute(normal_line_height(font_size, weight)),
            CssLineHeight::Number(number) => LineHeight::FontSizeRelative(number.0),
            CssLineHeight::Length(length) => LineHeight::Absolute(length.0.px()),
        };
        Self {
            text: collapse_whitespace(text),
            font_size,
            line_height,
            weight,
            color: TextBrush(srgb_bytes(style.clone_color())),
        }
    }
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

fn face_for(weight: f32) -> &'static [u8] {
    if weight >= 600.0 {
        NOTO_SANS_BOLD
    } else {
        NOTO_SANS_REGULAR
    }
}

/// Collapse runs of document whitespace to one space and trim both ends.
fn collapse_whitespace(text: &str) -> String {
    text.split([' ', '\t', '\n', '\r', '\u{c}'])
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
    /// `max_advance` (no limit when `None`).
    pub(crate) fn shape(
        &mut self,
        paragraph: &Paragraph,
        max_advance: Option<f32>,
    ) -> Layout<TextBrush> {
        let mut builder = self
            .layouts
            .ranged_builder(&mut self.fonts, &paragraph.text, 1.0, true);
        builder.push_default(StyleProperty::FontFamily(FAMILY.into()));
        builder.push_default(StyleProperty::FontSize(paragraph.font_size));
        builder.push_default(StyleProperty::LineHeight(paragraph.line_height));
        builder.push_default(StyleProperty::FontWeight(FontWeight::new(paragraph.weight)));
        builder.push_default(StyleProperty::Brush(paragraph.color));
        let mut layout = builder.build(&paragraph.text);
        layout.break_all_lines(max_advance);
        layout.align(Alignment::Start, AlignmentOptions::default());
        layout
    }

    /// Taffy's measure function for a paragraph leaf.
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
        let layout = self.shape(paragraph, max_advance);
        Size {
            width: known.width.unwrap_or_else(|| layout.width()),
            height: known.height.unwrap_or_else(|| layout.height()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str, weight: f32) -> Paragraph {
        Paragraph {
            text: collapse_whitespace(text),
            font_size: 16.0,
            line_height: LineHeight::MetricsRelative(1.0),
            weight,
            color: TextBrush::default(),
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
        use parley::PositionedLayoutItem;

        let mut engine = TextEngine::new();
        let layout = engine.shape(&paragraph("İstanbul Işık ğüşöç ĞÜŞÖÇ", 400.0), None);
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
        let one_line = engine.shape(&text, None);
        let wrapped = engine.shape(&text, Some(120.0));

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
        let regular = engine.shape(&paragraph("Merhaba dünya", 400.0), None);
        let bold = engine.shape(&paragraph("Merhaba dünya", 700.0), None);
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
        let word = engine.shape(&paragraph("uzunkelime", 400.0), None).width();
        assert!((min.width - word).abs() < 0.5);
    }
}
