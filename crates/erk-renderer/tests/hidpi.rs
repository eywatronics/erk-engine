//! HiDPI: layout in CSS pixels, painting in device pixels. On a screen with
//! two device pixels per CSS pixel the page has the same layout and twice
//! the pixels, so text and edges are sharp instead of enlarged.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use erk_renderer::{Frame, FromRenderer, ToRenderer, render_html, render_html_at_scale, spawn};

const PATIENCE: Duration = Duration::from_secs(60);

fn rgb(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    [frame.rgba()[i], frame.rgba()[i + 1], frame.rgba()[i + 2]]
}

const RED: [u8; 3] = [255, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

fn scaled(html: &str, width: u16, height: u16, scale: f32) -> Frame {
    render_html_at_scale(html, width, height, scale, &mut |_| None)
}

#[test]
fn a_css_pixel_covers_scale_device_pixels() {
    let html = r#"<body style="margin: 0"><div style="width: 10px; height: 10px; margin: 5px; background: red"></div>"#;
    let frame = scaled(html, 60, 60, 2.0);
    assert_eq!((frame.width(), frame.height()), (60, 60));
    // CSS 5..15 is device 10..30.
    assert_eq!(rgb(&frame, 9, 20), WHITE);
    assert_eq!(rgb(&frame, 10, 20), RED);
    assert_eq!(rgb(&frame, 29, 29), RED);
    assert_eq!(rgb(&frame, 30, 20), WHITE);
    // The display list stays in CSS pixels.
    assert!(
        frame.display_list().contains("rect 5 5 10x10"),
        "{}",
        frame.display_list()
    );
}

#[test]
fn the_viewport_is_the_device_size_in_css_pixels() {
    // 80 device pixels at scale 2 are 40 CSS pixels: the 50% box is 20 CSS
    // pixels wide, 40 device pixels.
    let html =
        r#"<body style="margin: 0"><div style="width: 50%; height: 4px; background: red"></div>"#;
    let frame = scaled(html, 80, 20, 2.0);
    assert_eq!(rgb(&frame, 39, 2), RED);
    assert_eq!(rgb(&frame, 40, 2), WHITE);
}

#[test]
fn text_is_drawn_at_device_resolution() {
    // The same text covers twice the device rows at scale 2, not the same
    // rows enlarged after the fact: it is shaped once, in CSS pixels.
    let html = r#"<body style="margin: 0; font-size: 20px">Hg</body>"#;
    let ink_rows = |frame: &Frame| {
        (0..usize::from(frame.height()))
            .filter(|&y| (0..usize::from(frame.width())).any(|x| rgb(frame, x, y) != WHITE))
            .count()
    };
    let one = render_html(html, 60, 40);
    let two = scaled(html, 120, 80, 2.0);
    let (rows1, rows2) = (ink_rows(&one), ink_rows(&two));
    assert!(
        (2 * rows1).abs_diff(rows2) <= 2,
        "scale 1: {rows1} rows, scale 2: {rows2}"
    );
    assert_eq!(one.display_list(), two.display_list());
}

#[test]
fn a_fractional_scale_and_nonsense_scales_render() {
    let html = r#"<div style="width: 10px; height: 10px; background: red; border-radius: 3px; box-shadow: 2px 2px 4px black">x</div>"#;
    let frame = scaled(html, 75, 75, 1.5);
    assert_eq!(frame.width(), 75);
    // Not a number, not positive, or beyond 1/64 and 64: painted at 1.
    let at_one = render_html(html, 40, 40);
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 1e-30, 0.01, 65.0, 1e30] {
        let frame = scaled(html, 40, 40, scale);
        assert!(frame.rgba() == at_one.rgba(), "{scale}");
    }
}

fn next_frame(from: &Receiver<FromRenderer>) -> Frame {
    loop {
        match from.recv_timeout(PATIENCE) {
            Ok(FromRenderer::Frame(frame)) => break frame,
            Ok(_) => {}
            Err(error) => panic!("no frame: {error:?}"),
        }
    }
}

#[test]
fn the_renderer_thread_repaints_when_the_scale_changes() {
    let (to, from, renderer) = spawn();
    to.send(ToRenderer::Load {
        html: r#"<body style="margin: 0"><div style="width: 10px; height: 10px; background: red"></div>"#
            .to_owned(),
    })
    .unwrap();
    to.send(ToRenderer::Resize {
        width: 40,
        height: 40,
    })
    .unwrap();
    let first = next_frame(&from);
    assert_eq!(rgb(&first, 15, 15), WHITE);

    // The window moved to a HiDPI screen.
    to.send(ToRenderer::Scale { factor: 2.0 }).unwrap();
    let second = next_frame(&from);
    assert_eq!((second.width(), second.height()), (40, 40));
    assert_eq!(rgb(&second, 15, 15), RED);
    assert_eq!(rgb(&second, 25, 15), WHITE);
    to.send(ToRenderer::Shutdown).unwrap();
    renderer.join().unwrap();
}
