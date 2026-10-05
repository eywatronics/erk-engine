//! Paint behaviour checked through the public API: display-list order and
//! the pixels of a rendered frame.

use erk_renderer::{Frame, render_html};

const WIDTH: u16 = 200;
const HEIGHT: u16 = 120;

/// Premultiplied RGBA at `(x, y)`.
fn pixel(frame: &Frame, x: usize, y: usize) -> [u8; 4] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    frame.rgba()[i..i + 4].try_into().unwrap()
}

#[test]
fn text_is_painted_after_every_background() {
    // The paragraph is 4px tall but its 30px text overflows into the yellow
    // div that follows. CSS 2 Appendix E paints all backgrounds before any
    // text, so the text must show on top of the yellow.
    let frame = render_html(
        r#"<style>body { margin: 0 }</style>
        <p style="margin: 0; height: 4px; font-size: 30px">HHHH</p>
        <div style="background: #ffff00; height: 80px"></div>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    let lines: Vec<&str> = list.lines().collect();
    let (Some(last_rect), Some(first_glyphs)) = (
        lines.iter().rposition(|line| line.starts_with("rect")),
        lines.iter().position(|line| line.starts_with("glyphs")),
    ) else {
        panic!("expected both a rect and glyphs:\n{list}");
    };
    assert!(last_rect < first_glyphs, "rects must come first:\n{list}");

    // The list order alone does not prove the pixels: count text over the
    // yellow. Red tells them apart (yellow has 255, the black text ~0);
    // blue would not, since yellow has none.
    let dark = (4..40)
        .flat_map(|y| (0..WIDTH as usize).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&frame, x, y)[0] < 128)
        .count();
    assert!(
        dark > 100,
        "text should cover part of the yellow, {dark} dark pixels"
    );
}

#[test]
fn a_translucent_canvas_is_blended_over_white() {
    let frame = render_html(
        r#"<html style="background: rgba(255, 0, 0, 0.5)"><body></body></html>"#,
        WIDTH,
        HEIGHT,
    );
    let [r, g, b, a] = pixel(&frame, 5, 5);
    assert_eq!(a, 255, "the frame itself must be opaque");
    assert_eq!(r, 255);
    assert!(
        (126..=128).contains(&g) && (126..=128).contains(&b),
        "got {r} {g} {b}"
    );
}

#[test]
fn an_element_without_a_box_does_not_colour_the_canvas() {
    for html in [
        r#"<html style="display: none; background: blue"><body>x</body></html>"#,
        r#"<body style="display: none; background: blue">x</body>"#,
    ] {
        let frame = render_html(html, WIDTH, HEIGHT);
        assert_eq!(pixel(&frame, 5, 5), [255, 255, 255, 255], "{html}");
    }
}

#[test]
fn visibility_hidden_paints_neither_background_nor_text() {
    let frame = render_html(
        r#"<div style="visibility: hidden; background: red; height: 30px">gizli</div>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    assert!(
        !list.contains("#ff0000"),
        "hidden background painted:\n{list}"
    );
    assert!(!list.contains("gizli"), "hidden text painted:\n{list}");
}

#[test]
fn a_frame_without_pixels_has_no_png() {
    let frame = render_html("<p>x</p>", 0, 100);
    assert!(frame.to_png().is_none());
}

#[test]
fn an_inline_background_lies_between_its_block_and_its_text() {
    // Blue block, red span background, black text: the red must cover the
    // blue, and the text the red.
    let frame = render_html(
        r#"<style>body { margin: 0 }</style>
        <div style="background: #0000ff; height: 80px; font-size: 30px">
        <span style="background: #ff0000; padding: 0 10px">HH</span></div>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    let lines: Vec<&str> = list.lines().collect();
    let red = lines
        .iter()
        .position(|line| line.starts_with("rect") && line.ends_with("#ff0000ff"))
        .unwrap_or_else(|| panic!("no red background:\n{list}"));
    let blue = lines
        .iter()
        .position(|line| line.ends_with("#0000ffff"))
        .unwrap_or_else(|| panic!("no blue background:\n{list}"));
    let glyphs = lines
        .iter()
        .position(|line| line.starts_with("glyphs"))
        .unwrap_or_else(|| panic!("no text:\n{list}"));
    assert!(blue < red && red < glyphs, "blue, red, then text:\n{list}");
    // Its edges follow the text but are snapped to whole pixels, as Chrome
    // snaps them: `rect x y WxH colour`.
    let geometry: Vec<&str> = lines[red].split(' ').skip(1).take(3).collect();
    assert!(
        geometry.iter().all(|value| !value.contains('.')),
        "{}",
        lines[red]
    );

    let count = |test: fn([u8; 4]) -> bool| {
        (0..80)
            .flat_map(|y| (0..WIDTH as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| test(pixel(&frame, x, y)))
            .count()
    };
    let reds = count(|[r, g, b, _]| r > 200 && g < 60 && b < 60);
    let darks = count(|[r, g, b, _]| r < 80 && g < 80 && b < 80);
    assert!(
        reds > 300,
        "the span background should show, {reds} red pixels"
    );
    assert!(
        darks > 100,
        "the text should show over it, {darks} dark pixels"
    );
}

#[test]
fn an_inline_block_inside_an_inline_element_is_painted() {
    let frame = render_html(
        r#"<style>body { margin: 0 }</style>
        <p style="margin: 0">a <b>b <span style="display: inline-block; width: 30px; height: 20px; background: #00ff00"></span></b></p>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    assert!(
        list.lines()
            .any(|line| line.starts_with("rect") && line.ends_with("#00ff00ff")),
        "the inline-block's background is missing:\n{list}"
    );
}

#[test]
fn text_after_a_tall_line_is_painted_lower() {
    // The second line's baseline is below the first line's 40px box and the
    // strut under it (5px), plus its own 17px ascent: 62.
    let frame = render_html(
        r#"<style>body { margin: 0 }</style>
        <div style="width: 60px"><p style="margin: 0">a <span style="display: inline-block; width: 10px; height: 40px"></span> bbbbbb cc</p></div>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    let second = list
        .lines()
        .find(|line| line.starts_with("glyphs") && line.contains("bbbbbb"))
        .unwrap_or_else(|| panic!("no second line:\n{list}"));
    let y: f32 = second.split(' ').nth(2).unwrap().parse().unwrap();
    assert_eq!(y, 62.0, "{second}");
}

#[test]
fn superscript_glyphs_are_painted_raised() {
    let frame = render_html(
        r#"<style>body { margin: 0 }</style><p style="margin: 0">x<sup>2</sup></p>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    let y_of = |text: &str| -> f32 {
        let line = list
            .lines()
            .find(|line| line.starts_with("glyphs") && line.ends_with(&format!("{text:?}")))
            .unwrap_or_else(|| panic!("no glyphs for {text:?}:\n{list}"));
        line.split(' ').nth(2).unwrap().parse().unwrap()
    };
    // Raised by a third of the 16px font plus one pixel, in 1/64 px.
    assert_eq!(y_of("x") - y_of("2"), 405.0 / 64.0);
}

/// The colour at the middle of the overlap, as RGB.
fn rgb_at(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let [r, g, b, _] = pixel(frame, x, y);
    [r, g, b]
}

#[test]
fn positioned_boxes_paint_after_the_flow() {
    // The red box comes first in the tree but is positioned: it is painted
    // after the blue in-flow block that overlaps it.
    let frame = render_html(
        r#"<style>body { margin: 0 }</style>
        <div style="position: relative; background: #ff0000; height: 40px"></div>
        <div style="margin-top: -20px; background: #0000ff; height: 40px"></div>"#,
        WIDTH,
        HEIGHT,
    );
    assert_eq!(rgb_at(&frame, 10, 30), [255, 0, 0]);
    assert_eq!(rgb_at(&frame, 10, 50), [0, 0, 255]);
}

#[test]
fn z_index_orders_positioned_boxes() {
    let frame = render_html(
        r#"<style>body { margin: 0 } div { position: absolute; width: 40px; height: 40px }</style>
        <div style="left: 0; top: 0; z-index: 2; background: #ff0000"></div>
        <div style="left: 20px; top: 20px; z-index: 1; background: #0000ff"></div>
        <p style="margin: 0; position: relative; height: 0"></p>
        <div style="left: 100px; top: 0; z-index: -1; background: #00ff00"></div>
        <section style="margin-left: 110px; width: 40px; height: 40px; background: #ffff00"></section>"#,
        WIDTH,
        HEIGHT,
    );
    // The higher z-index wins although it comes first.
    assert_eq!(rgb_at(&frame, 30, 30), [255, 0, 0]);
    // A negative z-index is painted below the in-flow yellow block.
    assert_eq!(rgb_at(&frame, 120, 10), [255, 255, 0]);
    assert_eq!(rgb_at(&frame, 105, 10), [0, 255, 0]);
}

fn render(body: &str) -> Frame {
    render_html(
        &format!("<style>body {{ margin: 0 }}</style>{body}"),
        WIDTH,
        HEIGHT,
    )
}

#[test]
fn a_solid_border_is_painted_around_the_padding_box() {
    let frame =
        render(r#"<div style="width: 40px; height: 40px; border: 5px solid #ff0000"></div>"#);
    assert_eq!(rgb_at(&frame, 2, 20), [255, 0, 0]);
    assert_eq!(rgb_at(&frame, 47, 20), [255, 0, 0]);
    assert_eq!(rgb_at(&frame, 25, 25), [255, 255, 255]);
    assert_eq!(rgb_at(&frame, 52, 20), [255, 255, 255]);
}

#[test]
fn each_border_side_keeps_its_own_colour() {
    let frame = render(
        r#"<div style="width: 40px; height: 40px; border: 6px solid #ff0000; border-top-color: #0000ff"></div>"#,
    );
    assert_eq!(rgb_at(&frame, 26, 2), [0, 0, 255]);
    assert_eq!(rgb_at(&frame, 2, 26), [255, 0, 0]);
    assert_eq!(rgb_at(&frame, 26, 49), [255, 0, 0]);
}

#[test]
fn rounded_corners_clip_the_background() {
    let frame = render(
        r#"<div style="width: 40px; height: 40px; background: #0000ff; border-radius: 50%"></div>"#,
    );
    assert_eq!(rgb_at(&frame, 20, 20), [0, 0, 255]);
    // Outside the circle, inside the box.
    assert_eq!(rgb_at(&frame, 2, 2), [255, 255, 255]);
    assert_eq!(rgb_at(&frame, 37, 37), [255, 255, 255]);
    let list = frame.display_list().to_owned();
    assert!(
        list.lines()
            .any(|line| line.starts_with("rrect") && line.contains(" 20,20,20,20 ")),
        "{list}"
    );
}

#[test]
fn overlapping_radii_are_scaled_down_together() {
    // 100px radii on a 40px box: scaled by 40 / 200, to 20px each.
    let frame = render(
        r#"<div style="width: 40px; height: 40px; background: #0000ff; border-radius: 100px"></div>"#,
    );
    let list = frame.display_list().to_owned();
    assert!(
        list.lines()
            .any(|line| line.starts_with("rrect") && line.contains(" 20,20,20,20 ")),
        "{list}"
    );
}

#[test]
fn a_box_shadow_is_cast_outside_the_box_only() {
    // A box with no background: the shadow must not show through it.
    let frame = render(
        r#"<div style="margin: 20px; width: 20px; height: 20px; box-shadow: 0 0 0 5px #000000"></div>"#,
    );
    assert_eq!(rgb_at(&frame, 17, 30), [0, 0, 0]);
    assert_eq!(rgb_at(&frame, 30, 30), [255, 255, 255]);
    assert_eq!(rgb_at(&frame, 12, 30), [255, 255, 255]);
}

#[test]
fn an_offset_blurred_shadow_lies_behind_the_background() {
    let frame = render(
        r#"<div style="margin: 20px; width: 30px; height: 30px; background: #ffffff; box-shadow: 15px 15px 6px #000000"></div>"#,
    );
    // Inside the box: its background, not the shadow.
    assert_eq!(rgb_at(&frame, 40, 40), [255, 255, 255]);
    // Below-right of the box, inside the offset shadow.
    let [r, g, b] = rgb_at(&frame, 60, 60);
    assert!(r < 40 && g < 40 && b < 40, "{r} {g} {b}");
    // The blur softens the shadow's edge.
    let [edge, _, _] = rgb_at(&frame, 65, 52);
    assert!(edge > 40 && edge < 220, "{edge}");
}

#[test]
fn opacity_composites_an_element_as_one_group() {
    // A red child covers its parent's blue: at half opacity the group is
    // half red over white, with no blue mixed in.
    let frame = render(
        r#"<div style="opacity: 0.5; background: #0000ff; width: 40px; height: 40px"><div style="background: #ff0000; height: 40px"></div></div>"#,
    );
    let [r, g, b] = rgb_at(&frame, 20, 20);
    assert!(
        r == 255 && (126..=129).contains(&g) && (126..=129).contains(&b),
        "{r} {g} {b}"
    );
}

#[test]
fn an_inline_border_closes_only_the_first_and_last_line() {
    let frame = render(
        r#"<div style="width: 90px"><span style="border: 3px solid #00ff00">uzun bir metin satırlara bölünür</span></div>"#,
    );
    let list = frame.display_list().to_owned();
    let borders: Vec<&str> = list
        .lines()
        .filter(|line| line.starts_with("border"))
        .collect();
    assert!(borders.len() >= 2, "{list}");
    assert!(
        borders[0].contains("[3.0, 0.0, 3.0, 3.0]"),
        "{}",
        borders[0]
    );
    assert!(
        borders[borders.len() - 1].contains("[3.0, 3.0, 3.0, 0.0]"),
        "{}",
        borders[borders.len() - 1]
    );
}

#[test]
fn the_canvas_element_still_paints_its_border() {
    let frame = render(
        r#"<div></div><style>body { background: #ffff00; border: 4px solid #ff0000; height: 50px }</style>"#,
    );
    assert_eq!(rgb_at(&frame, 1, 20), [255, 0, 0]);
    assert_eq!(rgb_at(&frame, 100, 20), [255, 255, 0]);
}

#[test]
fn an_inline_element_that_wraps_takes_its_opening_edge_with_it() {
    // The span fits on no line with the words before it: it starts the
    // second line, border and padding included, and leaves no sliver of
    // its background or border at the end of the first.
    let frame = render_html(
        r#"<style>body { margin: 0; font-family: 'Noto Sans'; font-size: 16px }</style>
        <p style="margin: 0; width: 120px">aaaa bbbb <span style="background: #ff0000; padding-left: 6px; border-left: 4px solid #0000ff">cccccccccc</span></p>"#,
        WIDTH,
        HEIGHT,
    );
    let coloured = |x: usize, y: usize| {
        let [r, g, b, _] = pixel(&frame, x, y);
        (r > 200 && g < 60 && b < 60) || (b > 200 && r < 60 && g < 60)
    };
    let first_line = (0..WIDTH as usize).flat_map(|x| (0..20).map(move |y| (x, y)));
    assert_eq!(
        first_line.filter(|(x, y)| coloured(*x, *y)).count(),
        0,
        "a sliver is left"
    );
    assert!(coloured(1, 30), "the border starts the second line");
    assert!(coloured(7, 30), "then the padding");
}

#[test]
fn a_nested_inline_background_is_painted_over_its_parents() {
    let frame = render_html(
        r#"<style>body { margin: 0; font-family: 'Noto Sans'; font-size: 16px }</style>
        <p style="margin: 0"><span style="background: #0000ff; padding: 0 20px">a <span style="background: #00ff00">bbbbbb</span> c</span></p>"#,
        WIDTH,
        HEIGHT,
    );
    // Inside the inner span, between its letters' strokes: green, not blue.
    let green = (0..WIDTH as usize)
        .filter(|x| {
            let [r, g, b, _] = pixel(&frame, *x, 3);
            r < 60 && g > 200 && b < 60
        })
        .count();
    assert!(green > 30, "the inner background is covered: {green}");
}

#[test]
fn a_relatively_positioned_inline_element_moves_what_it_holds() {
    let html = |position: &str, inner: &str| {
        format!(
            r#"<style>body {{ margin: 0; font-family: 'Noto Sans'; font-size: 16px }}</style>
            <p style="margin: 0; width: 300px; height: 100px">Önce <span style="{position}; background: #ff0000">kayan <b style="{inner}">metin</b> <span style="display: inline-block; width: 10px; height: 10px; background: #0000ff"></span></span> sonra</p>"#
        )
    };
    let still = html("position: static", "");
    // The bold word moves with the span and by its own offset too.
    let moved = html(
        "position: relative; left: 10%; top: 10%",
        "position: relative; top: 5px",
    );
    let boxes = |page: &str| erk_renderer::text_boxes(page, WIDTH, HEIGHT, &mut |_| None);
    let (before, after) = (boxes(&still), boxes(&moved));
    // "Önce" and "sonra" stay; the span's text moves 30px right (10% of
    // the 300px block) and 10px (10% of its 100px) down: no line changes.
    assert_eq!(before.len(), after.len());
    for (a, b) in before.iter().zip(&after) {
        let shift = match a.index {
            0 | 3 => (0.0, 0.0),
            2 => (30.0, 15.0),
            _ => (30.0, 10.0),
        };
        assert!(
            (b.x - a.x - shift.0).abs() < 0.01 && (b.y - a.y - shift.1).abs() < 0.01,
            "{a:?} -> {b:?}"
        );
    }
    // The background and the inline-block move with it.
    let rect_at = |page: &str, colour: &str| {
        let frame = render_html(page, WIDTH, HEIGHT);
        frame
            .display_list()
            .lines()
            .find(|line| line.starts_with("rect") && line.ends_with(colour))
            .map(|line| {
                let words: Vec<f32> = line
                    .split(' ')
                    .skip(1)
                    .take(2)
                    .map(|w| w.parse().unwrap())
                    .collect();
                (words[0], words[1])
            })
            .unwrap()
    };
    for colour in ["#ff0000ff", "#0000ffff"] {
        let (a, b) = (rect_at(&still, colour), rect_at(&moved, colour));
        assert_eq!((b.0 - a.0, b.1 - a.1), (30.0, 10.0), "{colour}");
    }
    // And so do the glyphs drawn: the run of the span's first word, against
    // the same span positioned with no offset (its text a run of its own
    // there too).
    let unmoved = html("position: relative; left: 0; top: 0", "position: relative");
    let glyphs_at = |page: &str| {
        let frame = render_html(page, WIDTH, HEIGHT);
        frame
            .display_list()
            .lines()
            .find(|line| line.starts_with("glyphs") && line.contains("kayan"))
            .map(|line| {
                let words: Vec<f32> = line
                    .split(' ')
                    .skip(1)
                    .take(2)
                    .map(|w| w.parse().unwrap())
                    .collect();
                (words[0], words[1])
            })
            .unwrap()
    };
    let (a, b) = (glyphs_at(&unmoved), glyphs_at(&moved));
    assert!(
        (b.0 - a.0 - 30.0).abs() < 0.01 && (b.1 - a.1 - 10.0).abs() < 0.01,
        "{a:?} -> {b:?}"
    );
}
