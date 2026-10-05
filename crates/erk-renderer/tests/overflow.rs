//! Clipping (M2.3): a box whose `overflow` is not `visible` clips what it
//! contains to its padding box, and which boxes escape the clip.

use erk_renderer::{Frame, render_html};

const WIDTH: u16 = 200;
const HEIGHT: u16 = 150;

fn rgb(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    [frame.rgba()[i], frame.rgba()[i + 1], frame.rgba()[i + 2]]
}

const WHITE: [u8; 3] = [255, 255, 255];
const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn page(body: &str) -> Frame {
    render_html(
        &format!(
            r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px">{body}</body>"#
        ),
        WIDTH,
        HEIGHT,
    )
}

#[test]
fn overflow_clips_what_a_box_holds_to_its_padding_box() {
    for overflow in ["hidden", "auto", "scroll", "clip"] {
        let frame = page(&format!(
            r#"<div style="overflow: {overflow}; width: 60px; height: 40px; border: 5px solid #0000ff">
                <div style="width: 150px; height: 120px; background: #ff0000"></div>
            </div>"#
        ));
        assert_eq!(rgb(&frame, 30, 30), RED, "{overflow}: inside");
        assert_eq!(
            rgb(&frame, 67, 30),
            BLUE,
            "{overflow}: the border is not covered"
        );
        assert_eq!(rgb(&frame, 100, 30), WHITE, "{overflow}: right of the box");
        assert_eq!(rgb(&frame, 30, 80), WHITE, "{overflow}: below the box");
        assert!(
            frame.display_list().contains("clip 5 5 60x40\n"),
            "{overflow}:\n{}",
            frame.display_list()
        );
    }
    // `visible` clips nothing.
    let frame = page(
        r#"<div style="width: 60px; height: 40px"><div style="width: 150px; height: 120px; background: #ff0000"></div></div>"#,
    );
    assert_eq!(rgb(&frame, 100, 80), RED);
    assert!(!frame.display_list().contains("clip"));
}

#[test]
fn text_that_overflows_is_clipped() {
    let html = |overflow: &str| {
        format!(
            r#"<div style="overflow: {overflow}; width: 40px; height: 20px; white-space: nowrap; font-size: 30px">HHHHHHHH</div>"#
        )
    };
    let clipped = page(&html("hidden"));
    let shown = page(&html("visible"));
    let dark = |frame: &Frame| {
        (45..200)
            .flat_map(|x| (0..40).map(move |y| (x, y)))
            .filter(|(x, y)| rgb(frame, *x, *y)[0] < 128)
            .count()
    };
    assert!(dark(&shown) > 100, "the text reaches past the box");
    assert_eq!(dark(&clipped), 0, "and is clipped there");
}

#[test]
fn clips_nest() {
    let frame = page(
        r#"<div style="overflow: hidden; width: 100px; height: 100px">
            <div style="overflow: hidden; margin-left: 50px; width: 100px; height: 50px">
                <div style="width: 300px; height: 300px; background: #ff0000"></div>
            </div>
        </div>"#,
    );
    assert_eq!(rgb(&frame, 70, 20), RED);
    assert_eq!(rgb(&frame, 120, 20), WHITE, "the outer clip");
    assert_eq!(rgb(&frame, 70, 70), WHITE, "the inner clip");
    assert_eq!(rgb(&frame, 20, 20), WHITE, "outside the inner box");
}

#[test]
fn positioned_boxes_escape_a_clip_their_containing_block_is_outside_of() {
    let child = r#"<div style="position: absolute; left: 0; top: 0; width: 150px; height: 120px; background: #ff0000"></div>"#;
    // The containing block is the viewport, outside the clip.
    let frame = page(&format!(
        r#"<div style="overflow: hidden; width: 50px; height: 50px">{child}</div>"#
    ));
    assert_eq!(rgb(&frame, 100, 100), RED, "escapes");
    // The clipping box is the containing block.
    let frame = page(&format!(
        r#"<div style="overflow: hidden; position: relative; width: 50px; height: 50px">{child}</div>"#
    ));
    assert_eq!(rgb(&frame, 20, 20), RED);
    assert_eq!(rgb(&frame, 100, 100), WHITE, "clipped");
    // A positioned box between them is the containing block, inside the clip.
    let frame = page(&format!(
        r#"<div style="overflow: hidden; width: 50px; height: 50px"><div style="position: relative">{child}</div></div>"#
    ));
    assert_eq!(
        rgb(&frame, 100, 100),
        WHITE,
        "clipped through its containing block"
    );
    // A fixed box escapes every clip.
    let frame = page(&format!(
        r#"<div style="overflow: hidden; position: relative; width: 50px; height: 50px">{}</div>"#,
        child.replace("absolute", "fixed")
    ));
    assert_eq!(rgb(&frame, 100, 100), RED, "fixed");
}

#[test]
fn positioned_and_translucent_content_is_clipped_where_it_is_painted() {
    // A relatively positioned child with a z-index and a translucent one are
    // painted after everything else, still inside the clip.
    for child in [
        r#"<div style="position: relative; z-index: 1; width: 150px; height: 120px; background: #ff0000"></div>"#,
        r#"<div style="opacity: 0.5; width: 150px; height: 120px; background: #ff0000"></div>"#,
    ] {
        let frame = page(&format!(
            r#"<div style="overflow: hidden; width: 50px; height: 50px">{child}</div><div style="height: 20px; background: #0000ff"></div>"#
        ));
        assert_ne!(rgb(&frame, 20, 20), WHITE, "{child}");
        assert_eq!(rgb(&frame, 100, 20), WHITE, "{child}");
        assert_eq!(
            rgb(&frame, 100, 60),
            BLUE,
            "{child}: the next block is not covered"
        );
    }
}

#[test]
fn the_root_or_body_overflow_belongs_to_the_viewport() {
    // The body is 10px tall; its overflow goes to the viewport, so what
    // overflows the body is not clipped to it.
    for (html_style, body_style) in [("", "overflow: hidden"), ("overflow: hidden", "")] {
        let frame = render_html(
            &format!(
                r#"<html style="{html_style}"><body style="margin: 0; height: 10px; {body_style}">
                    <div style="width: 150px; height: 120px; background: #ff0000"></div>
                </body></html>"#
            ),
            WIDTH,
            HEIGHT,
        );
        assert_eq!(rgb(&frame, 100, 100), RED, "{html_style:?} {body_style:?}");
        assert!(
            !frame.display_list().contains("clip"),
            "{html_style:?} {body_style:?}"
        );
    }
    // With both set, the body's own overflow clips.
    let frame = render_html(
        r#"<html style="overflow: hidden"><body style="margin: 0; height: 10px; overflow: hidden">
            <div style="width: 150px; height: 120px; background: #ff0000"></div>
        </body></html>"#,
        WIDTH,
        HEIGHT,
    );
    assert_eq!(rgb(&frame, 100, 5), RED);
    assert_eq!(rgb(&frame, 100, 100), WHITE);
}

#[test]
fn a_box_escaping_a_clip_inside_a_translucent_box_stays_translucent() {
    // The child's containing block is the viewport, outside the clip; it is
    // painted inside its translucent parent's group all the same.
    let frame = page(
        r#"<div style="overflow: hidden; width: 50px; height: 50px">
            <div style="opacity: 0.5">
                <div style="position: absolute; left: 0; top: 0; width: 150px; height: 120px; background: #ff0000"></div>
            </div>
        </div>"#,
    );
    let [r, g, b] = rgb(&frame, 20, 20);
    assert_eq!(r, 255);
    assert!(
        (120..=135).contains(&g) && g == b,
        "half red over white: {r} {g} {b}"
    );
}
