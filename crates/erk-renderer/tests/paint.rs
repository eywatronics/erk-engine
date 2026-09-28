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
    let last_rect = lines.iter().rposition(|line| line.starts_with("rect"));
    let first_glyphs = list.lines().position(|line| line.starts_with("glyphs"));
    assert!(last_rect < first_glyphs, "rects must come first:\n{list}");

    let dark = (4..40)
        .flat_map(|y| (0..WIDTH as usize).map(move |x| (x, y)))
        .filter(|&(x, y)| pixel(&frame, x, y)[2] < 128)
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
