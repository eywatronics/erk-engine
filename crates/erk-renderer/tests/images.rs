//! Images through the resource protocol (p1-contract §6): requested by URL
//! and kind, provided by the host, decoded, laid out and painted.

mod support;

use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

use erk_renderer::{
    Frame, ResourceKind, ResourceRequest, ResourceResponse, element_boxes, render_html,
    render_html_with_resources,
};
use support::protocol::{FromRenderer, ToRenderer, spawn};

const WIDTH: u16 = 120;
const HEIGHT: u16 = 80;
const PATIENCE: Duration = Duration::from_secs(60);

/// An opaque `width` × `height` PNG whose pixel colours come from `color`.
fn png(width: u32, height: u32, color: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = color(x, y);
            data.extend([r, g, b, 255]);
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }
    out
}

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

/// A provider with one PNG at `name`.
fn serving(
    name: &'static str,
    image: Vec<u8>,
    mime: &'static str,
) -> impl FnMut(&ResourceRequest) -> Option<ResourceResponse> {
    move |request| {
        (request.url == name && request.kind == ResourceKind::Image).then(|| ResourceResponse {
            id: request.id,
            mime: mime.to_owned(),
            data: image.clone(),
        })
    }
}

fn page(body: &str) -> String {
    format!("<style>body {{ margin: 0 }}</style>{body}")
}

fn rgb(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    [frame.rgba()[i], frame.rgba()[i + 1], frame.rgba()[i + 2]]
}

fn img_box(
    html: &str,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> (f32, f32) {
    let boxes = element_boxes(html, WIDTH, HEIGHT, provide);
    let img = boxes
        .iter()
        .find(|b| b.tag == "img")
        .expect("the img has a box");
    (img.width, img.height)
}

fn img_origin(
    html: &str,
    provide: &mut dyn FnMut(&ResourceRequest) -> Option<ResourceResponse>,
) -> (f32, f32) {
    let boxes = element_boxes(html, WIDTH, HEIGHT, provide);
    let img = boxes
        .iter()
        .find(|b| b.tag == "img")
        .expect("the img has a box");
    (img.x, img.y)
}

#[test]
fn an_img_takes_its_natural_size_and_is_painted() {
    let html = page(r#"<img src="a.png">"#);
    let mut provide = serving("a.png", png(4, 2, |_, _| RED), "image/png");
    assert_eq!(img_box(&html, &mut provide), (4.0, 2.0));
    let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
    // An inline img stands on the line's baseline, 17px down a 22px line.
    let (x, y) = img_origin(&html, &mut provide);
    assert_eq!(y, 15.0);
    assert_eq!(rgb(&frame, x as usize + 1, y as usize + 1), RED);
    assert_eq!(rgb(&frame, x as usize + 6, y as usize + 1), [255, 255, 255]);
    assert!(
        frame.display_list().contains("image 4x2"),
        "{}",
        frame.display_list()
    );
}

#[test]
fn one_given_dimension_keeps_the_natural_ratio() {
    let image = png(4, 2, |_, _| RED);
    let mut provide = serving("a.png", image.clone(), "image/png");
    assert_eq!(
        img_box(&page(r#"<img src="a.png" width="40">"#), &mut provide),
        (40.0, 20.0)
    );
    let mut provide = serving("a.png", image.clone(), "image/png");
    assert_eq!(
        img_box(
            &page(r#"<img src="a.png" style="height: 10px">"#),
            &mut provide
        ),
        (20.0, 10.0)
    );
    // A block-level image in block flow, which Taffy's block layout would
    // otherwise stretch to the container's width.
    let mut provide = serving("a.png", image, "image/png");
    assert_eq!(
        img_box(
            &page(r#"<img src="a.png" style="display: block; height: 10px">"#),
            &mut provide
        ),
        (20.0, 10.0)
    );
}

#[test]
fn both_given_dimensions_win_over_the_ratio() {
    // Taffy's leaf layout keeps a box with an aspect ratio at least
    // width / ratio high, which turned a 4 × 2 image sized 40 × 1 into 40 × 20.
    let image = png(4, 2, |_, _| RED);
    for (img, size) in [
        (
            r#"<img src="a.png" style="width: 40px; height: 1px">"#,
            (40.0, 1.0),
        ),
        (r#"<img src="a.png" width="40" height="1">"#, (40.0, 1.0)),
        (
            r#"<img src="a.png" style="display: block; width: 40px; height: 1px">"#,
            (40.0, 1.0),
        ),
        (
            r#"<img src="a.png" style="width: 100%; height: 3px">"#,
            (120.0, 3.0),
        ),
        (
            r#"<img src="a.png" style="width: 10px; height: 30px">"#,
            (10.0, 30.0),
        ),
    ] {
        let mut provide = serving("a.png", image.clone(), "image/png");
        assert_eq!(img_box(&page(img), &mut provide), size, "{img}");
    }
}

#[test]
fn an_image_that_never_arrives_has_no_size_and_paints_nothing() {
    let html = page(r#"<img src="missing.png">"#);
    assert_eq!(img_box(&html, &mut |_| None), (0.0, 0.0));
    let frame = render_html(&html, WIDTH, HEIGHT);
    assert!(
        !frame.display_list().contains("image"),
        "{}",
        frame.display_list()
    );
}

#[test]
fn a_response_of_the_wrong_type_is_refused() {
    // PNG bytes, but sent as a stylesheet.
    let html = page(r#"<img src="a.png" width="20" height="20">"#);
    let mut provide = serving("a.png", png(4, 2, |_, _| RED), "text/css");
    let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
    assert_eq!(rgb(&frame, 5, 5), [255, 255, 255]);
}

#[test]
fn a_background_image_repeats_from_the_padding_box() {
    // 2 × 1: red, blue. Repeated across a 20px box with a 3px border:
    // the pattern starts at the padding box, 3px in.
    let html = page(
        r#"<div style="width: 20px; height: 6px; border: 3px solid transparent; background-image: url(stripes.png)"></div>"#,
    );
    let mut provide = serving(
        "stripes.png",
        png(2, 1, |x, _| if x == 0 { RED } else { BLUE }),
        "image/png",
    );
    let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
    assert_eq!(rgb(&frame, 3, 4), RED);
    assert_eq!(rgb(&frame, 4, 4), BLUE);
    assert_eq!(rgb(&frame, 5, 4), RED);
    // It also fills the border area, behind the transparent border.
    assert_eq!(rgb(&frame, 1, 4), RED);
}

#[test]
fn background_position_and_no_repeat_place_one_copy() {
    // A 4 × 4 image centred in a 20 × 20 box: pixels 8..12.
    let html = page(
        r#"<div style="width: 20px; height: 20px; background: url(dot.png) no-repeat 50% 50%"></div>"#,
    );
    let mut provide = serving("dot.png", png(4, 4, |_, _| BLUE), "");
    let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
    assert_eq!(rgb(&frame, 9, 9), BLUE);
    assert_eq!(rgb(&frame, 5, 9), [255, 255, 255]);
    assert_eq!(rgb(&frame, 14, 9), [255, 255, 255]);
}

#[test]
fn background_size_cover_fills_the_box() {
    // A 2 × 1 image covering a 40 × 40 box: scaled to 80 × 40, left half red.
    let html = page(
        r#"<div style="width: 40px; height: 40px; background: url(stripes.png) no-repeat; background-size: cover"></div>"#,
    );
    let mut provide = serving(
        "stripes.png",
        png(2, 1, |x, _| if x == 0 { RED } else { BLUE }),
        "image/png",
    );
    let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
    assert!(
        frame.display_list().contains("80x40"),
        "{}",
        frame.display_list()
    );
    assert_eq!(rgb(&frame, 10, 39), RED);
    assert_eq!(rgb(&frame, 45, 10), [255, 255, 255]);
}

#[test]
fn the_renderer_thread_asks_the_host_and_repaints() {
    let (to, from, renderer) = spawn();
    to.send(ToRenderer::Load {
        html: page(
            r#"<img src="a.png"><div style="background-image: url('b.png'); height: 4px"></div>"#,
        ),
    })
    .unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: HEIGHT,
    })
    .unwrap();

    // The requests come first, with the URLs as written and their kind.
    let Ok(FromRenderer::Resources(requests)) = from.recv_timeout(PATIENCE) else {
        panic!("expected resource requests");
    };
    let urls: Vec<(&str, ResourceKind)> =
        requests.iter().map(|r| (r.url.as_str(), r.kind)).collect();
    assert_eq!(
        urls,
        [
            ("a.png", ResourceKind::Image),
            ("b.png", ResourceKind::Image)
        ]
    );
    // The frame painted without them says so.
    let Ok(FromRenderer::Frame(first)) = from.recv_timeout(PATIENCE) else {
        panic!("expected a frame");
    };
    assert!(first.resources_pending());

    let a = requests.iter().find(|r| r.url == "a.png").unwrap();
    let b = requests.iter().find(|r| r.url == "b.png").unwrap();
    to.send(ToRenderer::Resource(ResourceResponse {
        id: a.id,
        mime: "image/png".to_owned(),
        data: png(4, 2, |_, _| RED),
    }))
    .unwrap();
    to.send(ToRenderer::ResourceMissing { id: b.id }).unwrap();

    let last = loop {
        match from.recv_timeout(PATIENCE) {
            Ok(FromRenderer::Frame(frame)) if !frame.resources_pending() => break frame,
            Ok(FromRenderer::Frame(_)) => {}
            Ok(FromRenderer::Resources(more)) => panic!("asked again: {more:?}"),
            Ok(_) => panic!("nothing else was asked"),
            Err(RecvTimeoutError::Timeout) => panic!("no final frame"),
            Err(RecvTimeoutError::Disconnected) => panic!("the renderer thread died"),
        }
    };
    assert_eq!(rgb(&last, 1, 16), RED);

    // A resize repaints without asking again.
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: 60,
    })
    .unwrap();
    let Ok(FromRenderer::Frame(resized)) = from.recv_timeout(PATIENCE) else {
        panic!("a resize repaints, it does not ask again");
    };
    assert_eq!(rgb(&resized, 1, 16), RED);
    to.send(ToRenderer::Shutdown).unwrap();
    renderer.join().unwrap();
}

/// The next frame `height` pixels tall, skipping earlier ones; a frame of a
/// given height is painted after every message sent before the resize.
fn frame_of_height(from: &std::sync::mpsc::Receiver<FromRenderer>, height: u16) -> Frame {
    loop {
        match from.recv_timeout(PATIENCE) {
            Ok(FromRenderer::Frame(frame)) if frame.height() == height => break frame,
            Ok(_) => {}
            Err(error) => panic!("no frame {height} pixels tall: {error:?}"),
        }
    }
}

fn requested(from: &std::sync::mpsc::Receiver<FromRenderer>) -> Vec<ResourceRequest> {
    loop {
        match from.recv_timeout(PATIENCE) {
            Ok(FromRenderer::Resources(requests)) => break requests,
            Ok(_) => {}
            Err(error) => panic!("no resource requests: {error:?}"),
        }
    }
}

#[test]
fn an_answer_for_the_previous_document_is_not_taken_for_this_one() {
    let (to, from, renderer) = spawn();
    let load = |src: &str| ToRenderer::Load {
        html: page(&format!(r#"<img src="{src}" width="20" height="20">"#)),
    };
    to.send(load("old.png")).unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: HEIGHT,
    })
    .unwrap();
    let old = requested(&from);
    to.send(load("new.png")).unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: 60,
    })
    .unwrap();
    let new = requested(&from);
    assert_ne!(old[0].id, new[0].id, "ids are not reused across documents");
    frame_of_height(&from, 60);

    // The old document's answer arrives late.
    to.send(ToRenderer::Resource(ResourceResponse {
        id: old[0].id,
        mime: "image/png".to_owned(),
        data: png(4, 4, |_, _| RED),
    }))
    .unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: 50,
    })
    .unwrap();
    let frame = frame_of_height(&from, 50);
    assert_eq!(rgb(&frame, 5, 5), [255, 255, 255]);
    assert!(frame.resources_pending(), "new.png is still unanswered");
    to.send(ToRenderer::Shutdown).unwrap();
    renderer.join().unwrap();
}

#[test]
fn a_request_is_answered_once_and_unknown_ids_are_ignored() {
    let (to, from, renderer) = spawn();
    to.send(ToRenderer::Load {
        html: page(r#"<img src="a.png" width="20" height="20"><img src="a.png">"#),
    })
    .unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: HEIGHT,
    })
    .unwrap();
    let requests = requested(&from);
    assert_eq!(requests.len(), 1, "one URL, one request: {requests:?}");
    let id = requests[0].id;
    to.send(ToRenderer::Resource(ResourceResponse {
        id,
        mime: "image/png".to_owned(),
        data: png(4, 4, |_, _| RED),
    }))
    .unwrap();
    // A second answer, and answers to requests never made.
    to.send(ToRenderer::ResourceMissing { id }).unwrap();
    to.send(ToRenderer::ResourceMissing { id: id + 100 })
        .unwrap();
    to.send(ToRenderer::Resource(ResourceResponse {
        id: u64::MAX,
        mime: "image/png".to_owned(),
        data: Vec::new(),
    }))
    .unwrap();
    to.send(ToRenderer::Resize {
        width: WIDTH,
        height: 60,
    })
    .unwrap();
    let frame = frame_of_height(&from, 60);
    assert_eq!(rgb(&frame, 5, 5), RED);
    assert!(!frame.resources_pending());
    to.send(ToRenderer::Shutdown).unwrap();
    renderer.join().unwrap();
}

#[test]
fn extreme_image_sizes_and_positions_render_without_panicking() {
    let image = png(2, 2, |x, _| if x == 0 { RED } else { BLUE });
    let mut provide = |request: &ResourceRequest| {
        Some(ResourceResponse {
            id: request.id,
            mime: String::new(),
            data: image.clone(),
        })
    };
    for declarations in [
        "background-size: 0.0000001px",
        "background-size: 1e30px 1e30px",
        "background-size: 0 10px",
        "background-size: cover; width: 0",
        "background-size: contain; height: 0",
        "background-position: -1e30px 1e30px",
        "background-position: 100000% -100000%; background-repeat: no-repeat",
        "background-size: 1px; border: 1e6px solid transparent",
        "border-radius: 1e30px; background-size: 3px",
        "background-image: url(a.png), url(b.png), url(a.png); background-size: 1px, cover",
    ] {
        let html = page(&format!(
            r#"<div style="width: 50px; height: 50px; background-image: url(a.png); {declarations}"></div>
            <img src="a.png" style="{declarations}; width: 1e30px">
            <img src="a.png" style="height: 0.0000001px">"#
        ));
        let frame = render_html_with_resources(&html, WIDTH, HEIGHT, &mut provide);
        assert_eq!(frame.width(), WIDTH, "{declarations}");
    }
}
