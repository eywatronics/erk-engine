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
    let text = &layouts.text(p).expect("paragraph has shaped text").layout;
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
