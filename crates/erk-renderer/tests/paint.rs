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
