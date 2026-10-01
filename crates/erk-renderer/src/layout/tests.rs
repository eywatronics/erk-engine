use erk_dom::{Document, LocalName, NodeId, local_name};
use erk_style::StyleEngine;
use taffy::Layout;

use super::{Layouts, layout};
use crate::text::TextEngine;

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 600.0;

/// Lay out `body` with the UA body margin removed, so positions inside it are
/// easy to read.
fn lay_out(body: &str) -> (Document, Layouts) {
    let html = format!("<style>body {{ margin: 0 }}</style><body>{body}</body>");
    let doc = Document::parse_html(&html);
    let styles = StyleEngine::with_font_metrics(
        WIDTH,
        HEIGHT,
        std::sync::Arc::new(crate::text::EmbeddedFontMetrics),
    )
    .style(&doc);
    let layouts = layout(&doc, &styles, &mut TextEngine::new(), WIDTH, HEIGHT);
    (doc, layouts)
}

/// Elements with the given tag, in tree order.
fn all(doc: &Document, tag: &LocalName) -> Vec<NodeId> {
    let mut found = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if doc
            .node(id)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.name.local == *tag)
        {
            found.push(id);
        }
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children);
    }
    found
}

fn divs(doc: &Document, layouts: &Layouts) -> Vec<Layout> {
    all(doc, &local_name!("div"))
        .into_iter()
        .map(|id| *layouts.get(id).expect("div has a box"))
        .collect()
}

#[test]
fn explicit_width_is_used() {
    let (doc, layouts) = lay_out(r#"<div style="width: 100px; height: 10px"></div>"#);
    let div = divs(&doc, &layouts)[0];
    assert_eq!((div.size.width, div.size.height), (100.0, 10.0));
}

#[test]
fn a_block_without_width_fills_its_container() {
    let (doc, layouts) = lay_out(r#"<div style="height: 10px"></div>"#);
    assert_eq!(divs(&doc, &layouts)[0].size.width, WIDTH);
}

#[test]
fn block_siblings_stack_vertically() {
    let (doc, layouts) =
        lay_out(r#"<div style="height: 50px"></div><div style="height: 30px"></div>"#);
    let [first, second] = divs(&doc, &layouts)[..] else {
        panic!("expected two divs");
    };
    assert_eq!(first.location.y, 0.0);
    assert_eq!(second.location.y, 50.0);

    let body = all(&doc, &local_name!("body"))[0];
    assert_eq!(layouts.get(body).unwrap().size.height, 80.0);
}

#[test]
fn margin_top_offsets_the_box() {
    let (doc, layouts) = lay_out(
        r#"<div style="height: 10px"></div><div style="margin-top: 20px; height: 10px"></div>"#,
    );
    assert_eq!(divs(&doc, &layouts)[1].location.y, 30.0);
}

#[test]
fn adjacent_vertical_margins_collapse() {
    // CSS 2 §8.3.1: the gap between siblings is max(20, 30) = 30, not 50.
    let (doc, layouts) = lay_out(
        r#"<div style="margin-bottom: 20px; height: 10px"></div>
           <div style="margin-top: 30px; height: 10px"></div>"#,
    );
    assert_eq!(divs(&doc, &layouts)[1].location.y, 40.0);
}

#[test]
fn display_none_generates_no_box() {
    let (doc, layouts) = lay_out(
        r#"<div style="display: none; height: 50px"></div><div style="height: 10px"></div>"#,
    );
    let ids = all(&doc, &local_name!("div"));
    assert!(layouts.get(ids[0]).is_none());
    assert_eq!(layouts.get(ids[1]).unwrap().location.y, 0.0);
}

#[test]
fn calc_widths_resolve_against_the_container() {
    let (doc, layouts) = lay_out(r#"<div style="width: calc(50% - 20px); height: 10px"></div>"#);
    assert_eq!(divs(&doc, &layouts)[0].size.width, WIDTH / 2.0 - 20.0);
}

#[test]
fn padding_and_border_widen_the_border_box() {
    let (doc, layouts) = lay_out(
        r#"<div style="width: 100px; height: 10px; padding: 5px; border: 2px solid"></div>"#,
    );
    let div = divs(&doc, &layouts)[0];
    assert_eq!(div.size.width, 100.0 + 2.0 * 5.0 + 2.0 * 2.0);
    assert_eq!(div.padding.left, 5.0);
    assert_eq!(div.border.left, 2.0);
}

fn boxes(doc: &Document, layouts: &Layouts, tag: &LocalName) -> Vec<Layout> {
    all(doc, tag)
        .into_iter()
        .map(|id| *layouts.get(id).expect("element has a box"))
        .collect()
}

/// Height of one line of 16px Noto Sans with `line-height: normal`.
fn one_line() -> f32 {
    let (doc, layouts) = lay_out("<p>x</p>");
    boxes(&doc, &layouts, &local_name!("p"))[0].size.height
}

#[test]
fn a_paragraph_is_one_line_high() {
    // Noto Sans at 16px: ascent 17.10 rounds to 17, descent 4.69 to 5, no
    // line gap, as Chrome computes `line-height: normal`. Unrounded it
    // would be 21.79.
    assert_eq!(one_line(), 22.0);
}

#[test]
fn a_narrow_container_wraps_the_paragraph() {
    let (doc, layouts) = lay_out(
        r#"<div style="width: 120px"><p>Erk sayfayı önce çizer, sonra izole eder,
        en son betik çalıştırır.</p></div>"#,
    );
    let p = boxes(&doc, &layouts, &local_name!("p"))[0];
    let lines = (p.size.height / one_line()).round();
    assert!(lines >= 3.0, "expected at least 3 lines, got {lines}");
    assert_eq!(p.size.width, 120.0);
}

#[test]
fn paragraphs_stack_with_collapsed_margins() {
    // Both paragraphs have 1em (16px) vertical margins from the UA sheet; the
    // gap between them collapses to 16px.
    let (doc, layouts) = lay_out("<p>bir</p><p>iki</p>");
    let [first, second] = boxes(&doc, &layouts, &local_name!("p"))[..] else {
        panic!("expected two paragraphs");
    };
    assert_eq!(
        second.location.y,
        first.location.y + first.size.height + 16.0
    );
}

#[test]
fn inline_elements_contribute_their_text() {
    let (doc, layouts) = lay_out("<p>Merhaba <b>dünya</b></p>");
    let p = all(&doc, &local_name!("p"))[0];
    let text = &layouts
        .text(p)
        .expect("paragraph has shaped text")
        .layout
        .layout;
    assert_eq!(text.len(), 1);
    assert!(
        text.width() > 50.0,
        "both words should be shaped, width {}",
        text.width()
    );
    assert!(
        layouts.get(all(&doc, &local_name!("b"))[0]).is_none(),
        "inline boxes are not in the tree yet"
    );
}

#[test]
fn headings_are_larger_than_paragraphs() {
    let (doc, layouts) = lay_out("<h1>Başlık</h1>");
    let h1 = boxes(&doc, &layouts, &local_name!("h1"))[0];
    // 2em = 32px font size: a line about twice as tall as body text.
    assert!(h1.size.height > 1.8 * one_line());
}

#[test]
fn ex_and_ch_come_from_the_embedded_font() {
    // Noto Sans: x-height 536 and '0' advance 572 font units per 1000, so at
    // 16px 10ex = 85.8px and 10ch = 91.5px. The fixed fallback would give 80.
    let (doc, layouts) = lay_out(
        r#"<div style="width: 10ex; height: 1px"></div><div style="width: 10ch; height: 1px"></div>"#,
    );
    let [ex, ch] = divs(&doc, &layouts)[..] else {
        panic!("expected two divs");
    };
    assert_eq!(ex.size.width, 86.0);
    assert_eq!(ch.size.width, 92.0);
}

#[test]
fn text_beside_blocks_gets_anonymous_boxes() {
    // M0 kept the <p> and dropped "önce" and "sonra".
    let (doc, layouts) = lay_out(r#"<div>önce<p style="margin: 0">blok</p>sonra</div>"#);
    let div = all(&doc, &local_name!("div"))[0];
    let anonymous = layouts.anonymous(div);
    let texts: Vec<&str> = anonymous.iter().map(|a| a.text.text.as_str()).collect();
    assert_eq!(texts, ["önce", "sonra"]);

    // Anonymous box, paragraph, anonymous box: stacked in tree order.
    let p = *layouts.get(all(&doc, &local_name!("p"))[0]).unwrap();
    let (before, after) = (anonymous[0].layout, anonymous[1].layout);
    assert_eq!(before.location.y, 0.0);
    assert_eq!(p.location.y, before.size.height);
    assert_eq!(after.location.y, p.location.y + p.size.height);
    assert_eq!(
        layouts.get(div).unwrap().size.height,
        before.size.height + p.size.height + after.size.height
    );
}

#[test]
fn whitespace_between_blocks_makes_no_anonymous_box() {
    let (doc, layouts) = lay_out("<div>\n  <p>a</p>\n  <p>b</p>\n</div>");
    let div = all(&doc, &local_name!("div"))[0];
    assert!(layouts.anonymous(div).is_empty());
}

/// The text of each glyph run of `id`'s paragraph, with whether it was
/// shaped with the bold face and its colour.
fn runs(layouts: &Layouts, id: NodeId) -> Vec<(String, bool, [u8; 4])> {
    let shaped = layouts.text(id).expect("a paragraph");
    let mut runs = Vec::new();
    for line in shaped.layout.layout.lines() {
        let ranges = crate::text::glyph_run_ranges(&line);
        let glyph_runs = line.items().filter_map(|item| match item {
            parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
            parley::PositionedLayoutItem::InlineBox(_) => None,
        });
        for (run, range) in glyph_runs.zip(ranges) {
            let bold = crate::text::is_bold_face(run.run().font());
            runs.push((shaped.text[range].to_owned(), bold, run.style().brush.color));
        }
    }
    runs
}

#[test]
fn an_inline_element_keeps_its_own_weight() {
    let (doc, layouts) = lay_out("<p>a <b>kalın</b> c</p>");
    let p = all(&doc, &local_name!("p"))[0];
    assert_eq!(layouts.text(p).unwrap().text, "a kalın c");
    let runs = runs(&layouts, p);
    let bold: Vec<&str> = runs
        .iter()
        .filter(|(_, bold, _)| *bold)
        .map(|(text, ..)| text.as_str())
        .collect();
    // The collapsed spaces belong to the regular text around the <b>.
    assert_eq!(bold, ["kalın"]);
}

#[test]
fn an_inline_element_keeps_its_own_colour() {
    let (doc, layouts) =
        lay_out(r#"<p style="color: black">x<span style="color: red">kırmızı</span>y</p>"#);
    let p = all(&doc, &local_name!("p"))[0];
    let red: Vec<String> = runs(&layouts, p)
        .into_iter()
        .filter(|(.., colour)| *colour == [255, 0, 0, 255])
        .map(|(text, ..)| text)
        .collect();
    assert_eq!(red, ["kırmızı"]);
}

/// Each line of `id`'s paragraph: where it starts and how wide it is
/// without its trailing whitespace, relative to the content box.
fn lines(layouts: &Layouts, id: NodeId) -> Vec<(f32, f32)> {
    let shaped = layouts.text(id).expect("a paragraph");
    shaped
        .layout
        .layout
        .lines()
        .map(|line| {
            // Justification widens the clusters, not the line's metrics, so
            // the width comes from where the glyph runs actually end.
            let runs: Vec<(f32, f32)> = line
                .items()
                .filter_map(|item| match item {
                    parley::PositionedLayoutItem::GlyphRun(run) => {
                        Some((run.offset(), run.advance()))
                    }
                    parley::PositionedLayoutItem::InlineBox(_) => None,
                })
                .collect();
            let start = runs.first().expect("a glyph run").0;
            let end = runs
                .iter()
                .map(|(offset, advance)| offset + advance)
                .fold(start, f32::max);
            (start, end - start - line.metrics().trailing_whitespace)
        })
        .collect()
}

fn paragraph_lines(style: &str) -> Vec<(f32, f32)> {
    let (doc, layouts) = lay_out(&format!(
        r#"<p style="width: 400px; margin: 0; {style}">Erk ortalar.</p>"#
    ));
    lines(&layouts, all(&doc, &local_name!("p"))[0])
}

#[test]
fn text_align_moves_the_line_within_the_box() {
    let [(start, width)] = paragraph_lines("")[..] else {
        panic!("one line");
    };
    assert!(start.abs() < 0.5, "left by default, started at {start}");
    let [(center, _)] = paragraph_lines("text-align: center")[..] else {
        panic!("one line");
    };
    assert!(
        (center - (400.0 - width) / 2.0).abs() < 0.5,
        "centre at {center}"
    );
    let [(right, _)] = paragraph_lines("text-align: right")[..] else {
        panic!("one line");
    };
    assert!((right - (400.0 - width)).abs() < 0.5, "right at {right}");
}

#[test]
fn the_align_attribute_aligns_text() {
    let (doc, layouts) = lay_out(r#"<p align="center" style="width: 400px; margin: 0">Erk</p>"#);
    let [(start, width)] = lines(&layouts, all(&doc, &local_name!("p"))[0])[..] else {
        panic!("one line");
    };
    assert!(
        (start - (400.0 - width) / 2.0).abs() < 0.5,
        "started at {start}"
    );
}

#[test]
fn justified_lines_fill_the_box_except_the_last() {
    let (doc, layouts) = lay_out(
        r#"<p style="width: 300px; margin: 0; text-align: justify">Erk bu paragrafı iki yana yaslar: her satır kutunun iki kenarına dayanır, kelimeler arasındaki boşluklar büyür; son satır ise doğal genişliğinde kalır.</p>"#,
    );
    let lines = lines(&layouts, all(&doc, &local_name!("p"))[0]);
    assert!(lines.len() >= 3, "{lines:?}");
    let (last, full) = lines.split_last().unwrap();
    for (start, width) in full {
        assert!(
            start.abs() < 0.5 && (width - 300.0).abs() < 1.0,
            "{lines:?}"
        );
    }
    assert!(last.1 < 290.0, "the last line is stretched: {lines:?}");
}

/// The width of a paragraph's widest line.
fn line_width(body: &str) -> f32 {
    let (doc, layouts) = lay_out(body);
    let p = all(&doc, &local_name!("p"))[0];
    layouts.text(p).expect("a paragraph").layout.width()
}

#[test]
fn inline_padding_border_and_margin_take_room_in_the_line() {
    let plain = line_width("<p><span>a</span>b</p>");
    let padded = line_width(r#"<p><span style="padding: 0 20px">a</span>b</p>"#);
    let all_sides = line_width(
        r#"<p><span style="padding: 0 5px; border: 3px solid; margin: 0 2px">a</span>b</p>"#,
    );
    assert_eq!(padded - plain, 40.0);
    assert_eq!(all_sides - plain, 20.0);
}

#[test]
fn inline_padding_extends_the_background_but_not_the_line() {
    let (doc, layouts) =
        lay_out(r#"<p><span style="background: red; padding: 5px 10px">abc</span></p>"#);
    let p = all(&doc, &local_name!("p"))[0];
    let shaped = layouts.text(p).expect("a paragraph");
    let [rect] = shaped.decorations[..] else {
        panic!("expected one background, got {:?}", shaped.decorations);
    };
    // The content area is the font's ascent and descent (17 + 5), then the
    // padding.
    assert_eq!(rect.height, 22.0 + 10.0);
    assert_eq!(rect.x, 0.0);
    assert!((rect.width - shaped.layout.width()).abs() < 0.01);
    let baseline = shaped.layout.baseline(0).unwrap();
    assert_eq!(rect.y, baseline - 17.0 - 5.0);
    assert_eq!(
        boxes(&doc, &layouts, &local_name!("p"))[0].size.height,
        one_line()
    );
}

#[test]
fn a_wrapped_inline_background_gets_one_rectangle_per_line() {
    let (doc, layouts) = lay_out(
        r#"<div style="width: 120px"><p><span style="background: red">uzun bir metin satırlara bölünür</span></p></div>"#,
    );
    let p = all(&doc, &local_name!("p"))[0];
    let shaped = layouts.text(p).expect("a paragraph");
    let lines = shaped.layout.line_count();
    assert!(lines > 1);
    assert_eq!(shaped.decorations.len(), lines);
    for (index, rect) in shaped.decorations.iter().enumerate() {
        let baseline = shaped.layout.baseline(index).unwrap();
        assert_eq!(rect.y, baseline - 17.0, "line {index}");
        assert!(
            rect.width > 0.0 && rect.x + rect.width <= 120.0,
            "line {index}: {rect:?}"
        );
    }
}

#[test]
fn a_collapsed_space_stays_in_the_element_it_was_written_in() {
    let rect = |body: &str| {
        let (doc, layouts) = lay_out(body);
        let p = all(&doc, &local_name!("p"))[0];
        layouts.text(p).expect("a paragraph").decorations[0]
    };
    let a = line_width("<p>a</p>");
    let a_space_b = line_width("<p>a b</p>");
    // `a <span>b</span>`: the space is before the span.
    let after = rect(r#"<p>a <span style="background: red">b</span></p>"#);
    assert!(after.x > a, "{after:?}");
    // `<span>a </span>b`: the space is inside it.
    let inside = rect(r#"<p><span style="background: red">a </span>b</p>"#);
    assert!(inside.width > a && inside.width < a_space_b, "{inside:?}");
}

#[test]
fn an_inline_block_is_laid_out_and_sits_on_the_baseline() {
    let (doc, layouts) = lay_out(
        r#"<p>ab <span style="display: inline-block; width: 50px; height: 30px; margin-left: 4px"></span> cd</p>"#,
    );
    let p = all(&doc, &local_name!("p"))[0];
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    assert_eq!((span.size.width, span.size.height), (50.0, 30.0));
    let shaped = layouts.text(p).expect("a paragraph");
    // Without line boxes, an inline-block's baseline is its bottom margin
    // edge: the box stands on the line's baseline.
    let baseline = shaped.layout.baseline(0).unwrap();
    assert_eq!(span.location.y + span.size.height, baseline);
    assert!(span.location.x >= line_width("<p>ab </p>") + 4.0);
    // The line grows to hold it: 30px above the baseline, the strut's 5px
    // below.
    assert_eq!(
        boxes(&doc, &layouts, &local_name!("p"))[0].size.height,
        35.0
    );
}

#[test]
fn an_inline_block_with_text_aligns_its_text_with_the_line() {
    let (doc, layouts) =
        lay_out(r#"<p>x<span style="display: inline-block; padding: 4px">OK</span>y</p>"#);
    let p = all(&doc, &local_name!("p"))[0];
    let span_id = all(&doc, &local_name!("span"))[0];
    let span = *layouts.get(span_id).unwrap();
    let outer = layouts.text(p).unwrap().layout.baseline(0).unwrap();
    let inner = layouts.text(span_id).unwrap().layout.baseline(0).unwrap();
    assert_eq!(span.location.y + 4.0 + inner, outer);
    // 4px of padding above and below the text's line: the line box is 8px
    // taller than one line of text.
    assert_eq!(
        boxes(&doc, &layouts, &local_name!("p"))[0].size.height,
        one_line() + 8.0
    );
    assert_eq!(span.size.height, one_line() + 8.0);
}

#[test]
fn text_after_a_tall_line_moves_down() {
    let (doc, layouts) = lay_out(
        r#"<div style="width: 60px"><p>a <span style="display: inline-block; width: 10px; height: 40px"></span> bbbbbb cc</p></div>"#,
    );
    let p = all(&doc, &local_name!("p"))[0];
    let shaped = &layouts.text(p).unwrap().layout;
    assert!(shaped.line_count() >= 2);
    let first = shaped.baseline(0).unwrap();
    let second = shaped.baseline(1).unwrap();
    // The second line starts below the first line's 40px box and the strut
    // below its baseline, then has its own 17px above its baseline.
    assert_eq!(first, 40.0);
    assert_eq!(second, 40.0 + 5.0 + 17.0);
}

#[test]
fn an_inline_block_beside_blocks_is_placed_relative_to_its_block() {
    let (doc, layouts) = lay_out(
        r#"<div><p style="margin: 0">blok</p>metin <span style="display: inline-block; width: 10px; height: 10px"></span></div>"#,
    );
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    // Below the block paragraph's line, standing on the anonymous line's
    // baseline (17px down in its 22px line).
    assert_eq!(span.location.y, one_line() + 17.0 - 10.0);
}

#[test]
fn an_inline_block_inside_an_inline_element_gets_a_box() {
    let (doc, layouts) = lay_out(
        r#"<p>a <b>kalın <span style="display: inline-block; width: 12px; height: 12px"></span></b></p>"#,
    );
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    assert_eq!((span.size.width, span.size.height), (12.0, 12.0));
    assert!(span.location.x > 0.0);
}

#[test]
fn a_background_stops_before_the_space_a_line_breaks_at() {
    let (doc, layouts) = lay_out(
        r#"<div style="width: 120px"><p><span style="background: red">uzun bir metin satırlara bölünür</span></p></div>"#,
    );
    let p = all(&doc, &local_name!("p"))[0];
    let rect = layouts.text(p).unwrap().decorations[0];
    let (start, width) = lines(&layouts, p)[0];
    assert!((rect.x - start).abs() < 0.01, "{rect:?}");
    assert!(
        (rect.x + rect.width - (start + width)).abs() < 0.01,
        "{rect:?}"
    );
}

/// The `vertical-align` raise of each glyph run in `id`'s paragraph, with
/// its text.
fn raises(layouts: &Layouts, id: NodeId) -> Vec<(String, f32)> {
    let shaped = layouts.text(id).expect("a paragraph");
    let mut runs = Vec::new();
    for line in shaped.layout.layout.lines() {
        let ranges = crate::text::glyph_run_ranges(&line);
        let glyph_runs = line.items().filter_map(|item| match item {
            parley::PositionedLayoutItem::GlyphRun(run) => Some(run),
            parley::PositionedLayoutItem::InlineBox(_) => None,
        });
        for (run, range) in glyph_runs.zip(ranges) {
            runs.push((shaped.text[range].to_owned(), run.style().brush.raise));
        }
    }
    runs
}

#[test]
fn sup_and_sub_move_their_text_off_the_baseline() {
    let (doc, layouts) = lay_out("<p>x<sup>2</sup> y<sub>i</sub></p>");
    let p = all(&doc, &local_name!("p"))[0];
    let runs = raises(&layouts, p);
    let raise_of = |text: &str| {
        runs.iter()
            .find(|(run, _)| run.contains(text))
            .unwrap_or_else(|| panic!("no run with {text:?} in {runs:?}"))
            .1
    };
    // Blink's offsets, from the parent's 16px font: a third plus one up, a
    // fifth plus one down, each cut to Blink's 1/64 px layout unit (6.333
    // to 405/64, 4.2 to 268/64).
    assert_eq!(raise_of("2"), 405.0 / 64.0);
    assert_eq!(raise_of("i"), -268.0 / 64.0);
    assert_eq!(raise_of("x"), 0.0);
    // Raised text needs room: the line is taller than one line of text.
    assert!(boxes(&doc, &layouts, &local_name!("p"))[0].size.height > one_line());
}

#[test]
fn a_raised_background_moves_with_its_text() {
    let (doc, layouts) =
        lay_out(r#"<p>x<span style="vertical-align: 6px; background: red">y</span></p>"#);
    let p = all(&doc, &local_name!("p"))[0];
    let shaped = layouts.text(p).unwrap();
    let rect = shaped.decorations[0];
    let baseline = shaped.layout.baseline(0).unwrap();
    assert_eq!(rect.y, baseline - 6.0 - 17.0);
}

/// The first span's box and its paragraph's first baseline.
fn aligned_box(body: &str) -> (Layout, f32) {
    let (doc, layouts) = lay_out(body);
    let p = all(&doc, &local_name!("p"))[0];
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    (span, layouts.text(p).unwrap().layout.baseline(0).unwrap())
}

#[test]
fn vertical_align_places_an_inline_block_against_the_parent() {
    let block = |align: &str, height: u32| {
        format!(
            r#"<p>x<span style="display: inline-block; width: 10px; height: {height}px; vertical-align: {align}"></span></p>"#
        )
    };
    // A length raises the bottom (its baseline) off the line's baseline.
    let (span, baseline) = aligned_box(&block("5px", 10));
    assert_eq!(span.location.y + 10.0, baseline - 5.0);
    // `middle`: the box's middle half the parent's x-height above the
    // baseline.
    let (span, baseline) = aligned_box(&block("middle", 20));
    let x_height = crate::text::x_height(16.0, 400.0);
    assert!((span.location.y + 10.0 - (baseline - x_height / 2.0)).abs() < 1.0);
    // `text-top`: the top with the top of the parent's content area, the
    // font's 17px ascent above the baseline.
    let (span, baseline) = aligned_box(&block("text-top", 30));
    assert_eq!(span.location.y, baseline - 17.0);
    // `text-bottom`: the bottom with the bottom of the content area, 5px
    // below the baseline.
    let (span, baseline) = aligned_box(&block("text-bottom", 30));
    assert_eq!(span.location.y + 30.0, baseline + 5.0);
}

#[test]
fn vertical_align_top_and_bottom_follow_the_line_box() {
    let block = |align: &str| {
        format!(
            r#"<p style="line-height: 40px">x<span style="display: inline-block; width: 10px; height: 10px; vertical-align: {align}"></span></p>"#
        )
    };
    let (top, _) = aligned_box(&block("top"));
    assert_eq!(top.location.y, 0.0);
    let (bottom, _) = aligned_box(&block("bottom"));
    assert_eq!(bottom.location.y + 10.0, 40.0);
}

#[test]
fn a_line_aligned_box_taller_than_the_line_grows_it() {
    let (doc, layouts) = lay_out(
        r#"<p>x<span style="display: inline-block; width: 10px; height: 50px; vertical-align: top"></span></p>"#,
    );
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    assert_eq!(span.location.y, 0.0);
    assert_eq!(
        boxes(&doc, &layouts, &local_name!("p"))[0].size.height,
        50.0
    );
}

#[test]
fn positions_snap_to_absolute_pixels() {
    // The second div starts at 10.4px and its child 0.4px below that, at
    // 10.8px: Chrome draws the child at pixel 11. Rounding each offset on
    // its own would put it at 10 + 0.
    let (doc, layouts) = lay_out(
        r#"<div style="height: 10.4px"></div><div style="padding-top: 0.4px"><div style="height: 5px"></div></div>"#,
    );
    let divs = divs(&doc, &layouts);
    assert_eq!(divs[1].location.y, 10.0);
    assert_eq!(divs[2].location.y, 1.0);
}

#[test]
fn raised_and_lowered_text_make_room_on_their_own_side() {
    let height = |body: &str| {
        let (doc, layouts) = lay_out(body);
        let p = all(&doc, &local_name!("p"))[0];
        layouts.text(p).unwrap().layout.height
    };
    // `<sup>` (13.33px, 14px ascent, 4px descent) raised 405/64: the line
    // reaches 14 + 405/64 above the baseline, the strut's 5 below.
    assert_eq!(height("<p>x<sup>2</sup></p>"), 14.0 + 405.0 / 64.0 + 5.0);
    // `<sub>` lowered 268/64: the strut's 17 above, 4 + 268/64 below.
    assert_eq!(height("<p>x<sub>2</sub></p>"), 17.0 + 4.0 + 268.0 / 64.0);
}

#[test]
fn relative_position_offsets_the_box_but_not_the_flow() {
    let (doc, layouts) = lay_out(
        r#"<div style="position: relative; top: 10px; left: 5px; height: 20px"></div><div style="height: 10px"></div>"#,
    );
    let divs = divs(&doc, &layouts);
    assert_eq!((divs[0].location.x, divs[0].location.y), (5.0, 10.0));
    assert_eq!(divs[1].location.y, 20.0);
}

#[test]
fn static_position_ignores_insets() {
    let (doc, layouts) = lay_out(r#"<div style="top: 50px; left: 9px; height: 10px"></div>"#);
    let div = divs(&doc, &layouts)[0];
    assert_eq!((div.location.x, div.location.y), (0.0, 0.0));
}

#[test]
fn an_absolute_box_is_placed_in_its_nearest_positioned_ancestor() {
    // The static middle div is the DOM parent; the relative outer div is the
    // containing block.
    let (doc, layouts) = lay_out(
        r#"<div style="position: relative; margin-left: 30px; width: 200px; height: 100px"><div style="margin-left: 15px; height: 10px"><div style="position: absolute; right: 10px; bottom: 10px; width: 20px; height: 20px"></div></div></div>"#,
    );
    let [outer, middle, inner] = divs(&doc, &layouts)[..] else {
        panic!("three divs");
    };
    assert_eq!(outer.location.x, 30.0);
    assert_eq!(middle.location.x, 15.0);
    // 200 - 10 - 20 from the outer div's left, relative to the middle div.
    assert_eq!(inner.location.x, 170.0 - 15.0);
    assert_eq!(inner.location.y, 70.0);
}

#[test]
fn without_a_positioned_ancestor_the_viewport_contains() {
    let (doc, layouts) = lay_out(
        r#"<div style="height: 30px"><div style="position: absolute; bottom: 0; right: 0; width: 20px; height: 20px"></div></div>"#,
    );
    let inner = divs(&doc, &layouts)[1];
    assert_eq!(
        (inner.location.x, inner.location.y),
        (WIDTH - 20.0, HEIGHT - 20.0)
    );
}

#[test]
fn a_fixed_box_ignores_its_positioned_ancestors() {
    let (doc, layouts) = lay_out(
        r#"<div style="position: relative; margin: 40px; height: 30px"><div style="position: fixed; top: 0; left: 0; width: 20px; height: 20px"></div></div>"#,
    );
    let inner = divs(&doc, &layouts)[1];
    // At the viewport's corner: 40px up and left of its parent.
    assert_eq!((inner.location.x, inner.location.y), (-40.0, -40.0));
}

#[test]
fn an_absolute_element_does_not_split_its_paragraph() {
    let (doc, layouts) =
        lay_out(r#"<p>önce <span style="position: absolute; top: 0">x</span>sonra</p>"#);
    let p = all(&doc, &local_name!("p"))[0];
    let shaped = layouts.text(p).expect("still one paragraph");
    assert_eq!(shaped.text, "önce sonra");
    assert!(layouts.anonymous(p).is_empty());
    // The span has a box of its own, its text in an anonymous paragraph.
    let span = all(&doc, &local_name!("span"))[0];
    assert!(layouts.get(span).is_some());
    assert_eq!(layouts.anonymous(span).len(), 1);
}

#[test]
fn a_positioned_block_of_text_lays_out_its_absolute_children() {
    let (doc, layouts) = lay_out(
        r#"<div style="position: relative; padding: 4px">metin <span style="position: absolute; top: 0; left: 0; width: 5px; height: 5px"></span></div>"#,
    );
    let div = all(&doc, &local_name!("div"))[0];
    assert_eq!(
        layouts.anonymous(div).len(),
        1,
        "its text in an anonymous box"
    );
    let span = boxes(&doc, &layouts, &local_name!("span"))[0];
    assert_eq!((span.location.x, span.location.y), (0.0, 0.0));
    assert_eq!((span.size.width, span.size.height), (5.0, 5.0));
}

#[test]
fn a_float_is_laid_out_as_if_not_floated() {
    let (doc, layouts) = lay_out(
        r#"<div style="float: left; width: 50px; height: 50px"></div><p style="margin: 0">metin</p>"#,
    );
    let p = boxes(&doc, &layouts, &local_name!("p"))[0];
    // Below the float, not beside it: Parley's lines do not flow around
    // floats, so text beside one would be drawn over it.
    assert_eq!(p.location.y, 50.0);
    assert!(layouts.text(all(&doc, &local_name!("p"))[0]).is_some());
}
