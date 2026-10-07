//! 2D transforms (M4.4): what they paint, what they hide, and the
//! containing block they make. Hit testing and the box query through a
//! transform are checked from the host's side, in crates/erk/tests.

use erk_renderer::{Frame, element_boxes, render_html};

const WIDTH: u16 = 200;
const HEIGHT: u16 = 120;

/// Straight RGB at `(x, y)` (the frame is opaque).
fn rgb(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    frame.rgba()[i..i + 3].try_into().unwrap()
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

fn page(style: &str) -> Frame {
    render_html(
        &format!(
            r#"<body style="margin: 0"><div style="margin: 40px 0 0 40px; width: 40px; height: 20px; background: #ff0000; {style}"></div>"#
        ),
        WIDTH,
        HEIGHT,
    )
}

#[test]
fn a_rotation_turns_the_box_about_its_centre() {
    // 40×20 at (40, 40), centre (60, 50): a quarter turn stands it up,
    // 20 wide and 40 tall about the same centre.
    let frame = page("transform: rotate(90deg)");
    let list = frame.display_list();
    let lines: Vec<&str> = list.lines().collect();
    let push = lines
        .iter()
        .position(|l| l.starts_with("transform"))
        .unwrap_or_else(|| panic!("{list}"));
    let rect = lines
        .iter()
        .position(|l| l.starts_with("rect"))
        .unwrap_or_else(|| panic!("{list}"));
    let pop = lines
        .iter()
        .position(|l| *l == "end transform")
        .unwrap_or_else(|| panic!("{list}"));
    assert!(push < rect && rect < pop, "{list}");
    // Inside the turned box, outside the untransformed one.
    assert_eq!(rgb(&frame, 60, 65), RED);
    assert_eq!(rgb(&frame, 60, 35), RED);
    // Inside the untransformed box, outside the turned one.
    assert_eq!(rgb(&frame, 45, 45), WHITE);
    assert_eq!(rgb(&frame, 75, 55), WHITE);
}

#[test]
fn translation_percentages_are_of_the_border_box_and_the_origin_moves() {
    // translate(50%, 100%) of 40×20 moves by (20, 20).
    let moved = page("transform: translate(50%, 100%)");
    assert_eq!(rgb(&moved, 45, 45), WHITE);
    assert_eq!(rgb(&moved, 65, 65), RED);
    assert_eq!(rgb(&moved, 105, 75), WHITE);
    // Half size about the top-left corner: 20×10 at (40, 40).
    let scaled = page("transform: scale(0.5); transform-origin: 0 0");
    assert_eq!(rgb(&scaled, 45, 45), RED);
    assert_eq!(rgb(&scaled, 65, 45), WHITE);
    assert_eq!(rgb(&scaled, 45, 55), WHITE);
}

#[test]
fn translate_rotate_and_scale_come_before_transform_in_that_order() {
    // scale: 0.5 about the corner, then translate: 40px; with transform
    // naming the same steps in the other order the box would land
    // elsewhere (at 20px, not 40px, to the right).
    let individual = page("transform-origin: 0 0; translate: 40px; scale: 0.5");
    assert_eq!(rgb(&individual, 85, 45), RED);
    assert_eq!(rgb(&individual, 65, 45), WHITE);
    let listed = page("transform-origin: 0 0; transform: scale(0.5) translate(40px)");
    assert_eq!(rgb(&listed, 65, 45), RED);
    assert_eq!(rgb(&listed, 85, 45), WHITE);
}

#[test]
fn a_transform_that_cannot_be_undone_hides_the_box_and_what_it_holds() {
    let frame = render_html(
        r#"<body style="margin: 0"><div style="width: 80px; height: 40px; background: #ff0000; transform: scale(0)"><p style="background: #0000ff; margin: 0; height: 10px"></p></div>"#,
        WIDTH,
        HEIGHT,
    );
    let list = frame.display_list();
    assert!(!list.contains("rect"), "{list}");
    assert_eq!(rgb(&frame, 10, 5), WHITE);
}

#[test]
fn a_transformed_box_contains_its_absolute_and_fixed_descendants() {
    let mut provide = |_: &_| None;
    for position in ["absolute", "fixed"] {
        let html = format!(
            r#"<body style="margin: 0"><div style="margin: 30px 0 0 50px; height: 40px; transform: translate(0)"><span>a</span><div style="position: {position}; left: 10px; top: 5px; width: 10px; height: 10px"></div></div>"#
        );
        let boxes = element_boxes(&html, WIDTH, HEIGHT, &mut provide);
        let inner = boxes
            .iter()
            .rfind(|b| b.tag == "div")
            .expect("the inner div");
        // Placed from the transformed box's corner, not the viewport's.
        assert_eq!((inner.x, inner.y), (60.0, 35.0), "{position}");
    }
}

#[test]
fn without_a_transform_a_fixed_box_is_placed_in_the_viewport() {
    let mut provide = |_: &_| None;
    let html = r#"<body style="margin: 0"><div style="margin: 30px 0 0 50px; height: 40px; position: relative"><div style="position: fixed; left: 10px; top: 5px; width: 10px; height: 10px"></div></div>"#;
    let boxes = element_boxes(html, WIDTH, HEIGHT, &mut provide);
    let inner = boxes
        .iter()
        .rfind(|b| b.tag == "div")
        .expect("the inner div");
    assert_eq!((inner.x, inner.y), (10.0, 5.0));
}

#[test]
fn a_clip_inside_a_transform_turns_with_it() {
    // A 40×20 box clipping a taller child, turned a quarter: the child
    // shows only inside the turned clip.
    let frame = render_html(
        r#"<body style="margin: 0"><div style="margin: 40px 0 0 40px; width: 40px; height: 20px; overflow: hidden; transform: rotate(90deg)"><div style="height: 200px; background: #ff0000"></div></div>"#,
        WIDTH,
        HEIGHT,
    );
    assert_eq!(rgb(&frame, 60, 65), RED);
    // Where the unturned clip would have let the child through.
    assert_eq!(rgb(&frame, 45, 45), WHITE);
    assert_eq!(rgb(&frame, 75, 55), WHITE);
}

#[test]
fn an_out_of_flow_box_in_a_transform_stays_in_the_clips_around_it() {
    // The transformed box is its containing block, inside the clip: the
    // child, placed outside the clip, is cut away. Were its containing
    // block the viewport, it would escape the clip and show.
    for position in ["absolute", "fixed"] {
        let frame = render_html(
            &format!(
                r#"<body style="margin: 0"><div style="overflow: hidden; width: 50px; height: 50px"><div style="transform: translate(0)"><div style="position: {position}; left: 60px; top: 10px; width: 20px; height: 20px; background: #ff0000"></div></div></div>"#
            ),
            WIDTH,
            HEIGHT,
        );
        assert_eq!(rgb(&frame, 70, 20), WHITE, "{position}");
    }
}
