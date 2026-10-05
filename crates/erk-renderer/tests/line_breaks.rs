//! Forced line breaks (`<br>`), checked through the boxes and text lines
//! layout gives; the `line-breaks` reference page checks the same against
//! Chrome.

use erk_renderer::{element_boxes, text_boxes};

fn page(body: &str) -> String {
    format!(
        r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px"><div>{body}</div></body>"#
    )
}

/// The div's height, in lines of the font's height.
fn lines(body: &str) -> f32 {
    let one = div_height("a");
    div_height(body) / one
}

fn div_height(body: &str) -> f32 {
    element_boxes(&page(body), 400, 300, &mut |_| None)
        .into_iter()
        .find(|b| b.tag == "div")
        .unwrap()
        .height
}

#[test]
fn a_br_ends_its_line_as_chrome_does() {
    for (body, expected) in [
        ("a<br>b", 2.0),
        ("a<br><br>b", 3.0),
        // A break ending the block ends its last line: no line after it.
        ("a<br>", 1.0),
        ("a<br><br>", 2.0),
        // Alone, it is one empty line.
        ("<br>", 1.0),
        ("<br><br>", 2.0),
        ("<b>a<br>b</b>", 2.0),
        ("a <br> <br> b", 3.0),
    ] {
        assert_eq!(lines(body), expected, "{body}");
    }
}

#[test]
fn white_space_around_a_br_is_removed() {
    // The space before the break is not at the end of the first line, and
    // the one after it does not start the second: `b` starts at 0.
    let boxes = text_boxes(&page("a <br>   b"), 400, 300, &mut |_| None);
    let [first, second] = boxes.as_slice() else {
        panic!("{boxes:?}");
    };
    let alone = &text_boxes(&page("a"), 400, 300, &mut |_| None)[0];
    assert_eq!(first.width, alone.width, "no trailing space");
    assert_eq!(second.x, 0.0, "no leading space");
    assert!(second.y > first.y);
}

#[test]
fn a_space_before_a_br_does_not_move_a_right_aligned_line() {
    let line = |body: &str| {
        let html = format!(
            r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px"><div style="width: 100px; text-align: right">{body}</div></body>"#
        );
        text_boxes(&html, 400, 300, &mut |_| None)[0].x
    };
    assert_eq!(line("a <br>b"), line("a<br>b"));
}

#[test]
fn white_space_keeps_what_its_value_says() {
    let lines = |white_space: &str, text: &str| {
        let html = format!(
            r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px"><div style="width: 100px; white-space: {white_space}">{text}</div></body>"#
        );
        element_boxes(&html, 400, 300, &mut |_| None)
            .into_iter()
            .find(|b| b.tag == "div")
            .unwrap()
            .height
            / div_height("a")
    };
    let long = "bir iki üç dört beş altı yedi sekiz";
    // Wrapping: normal, pre-wrap and pre-line wrap; nowrap and pre do not.
    assert!(lines("normal", long) > 1.0);
    assert_eq!(lines("nowrap", long), 1.0);
    assert_eq!(lines("pre", long), 1.0);
    assert!(lines("pre-wrap", long) > 1.0);
    // Newlines: pre, pre-wrap and pre-line keep them; a final one adds no
    // line.
    for (white_space, expected) in [
        ("normal", 1.0),
        ("nowrap", 1.0),
        ("pre", 3.0),
        ("pre-wrap", 3.0),
        ("pre-line", 3.0),
    ] {
        assert_eq!(lines(white_space, "a\nb\nc\n"), expected, "{white_space}");
    }
    // Spaces: pre keeps them all, pre-line collapses them.
    let width = |white_space: &str, text: &str| {
        let html = format!(
            r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px"><div style="white-space: {white_space}">{text}</div></body>"#
        );
        text_boxes(&html, 400, 300, &mut |_| None)[0].width
    };
    assert!(width("pre", "a    b") > width("pre", "a b") + 10.0);
    assert_eq!(width("pre-line", "a    b"), width("pre-line", "a b"));
    assert_eq!(width("normal", "a    b"), width("normal", "a b"));
}
