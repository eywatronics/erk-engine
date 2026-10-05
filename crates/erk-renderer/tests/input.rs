//! Pointer input through the renderer's messages (M2.1): hit-testing in
//! paint order, clicks with their propagation path, the developer tools'
//! inspection and highlight, and `#id` queries.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use erk_renderer::{
    Event, EventKind, Frame, FromRenderer, Modifiers, PointerButton, PointerInput, PointerKind,
    ToRenderer, spawn,
};

const PATIENCE: Duration = Duration::from_secs(60);
/// Long enough for a frame that is not coming to show up if it were.
const QUIET: Duration = Duration::from_millis(500);

/// Everything placed absolutely, so that the points below are exact.
const PAGE: &str = r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px">
  <div id="a" style="position: absolute; left: 0; top: 0; width: 50px; height: 50px; background: #f00"></div>
  <div id="outer" style="position: absolute; left: 60px; top: 0; width: 80px; height: 80px; background: #0f0">
    <div id="inner" style="margin: 10px; width: 30px; height: 30px; background: #00f"></div>
  </div>
  <p id="para" style="position: absolute; left: 0; top: 90px; margin: 0">Bir <span id="word">kelime</span> burada</p>
  <div id="top" style="position: absolute; left: 150px; top: 0; width: 40px; height: 40px; z-index: 2"></div>
  <div id="under" style="position: absolute; left: 150px; top: 0; width: 40px; height: 40px; z-index: 1"></div>
  <div id="ghost" style="position: absolute; left: 150px; top: 50px; width: 40px; height: 40px; z-index: 3; pointer-events: none"></div>
  <div id="base" style="position: absolute; left: 150px; top: 50px; width: 40px; height: 40px; z-index: 2"></div>
  <div id="hidden" style="position: absolute; left: 100px; top: 100px; width: 40px; height: 40px; visibility: hidden">gizli metin</div>
</body>"#;

struct Session {
    to: Sender<ToRenderer>,
    from: Receiver<FromRenderer>,
    renderer: Option<JoinHandle<()>>,
    request: u64,
}

impl Session {
    fn new() -> (Self, Frame) {
        let (to, from, renderer) = spawn();
        let session = Self {
            to,
            from,
            renderer: Some(renderer),
            request: 0,
        };
        session.send(ToRenderer::Load {
            html: PAGE.to_owned(),
        });
        session.send(ToRenderer::Resize {
            width: 200,
            height: 150,
        });
        let frame = session.frame();
        (session, frame)
    }

    fn send(&self, message: ToRenderer) {
        self.to.send(message).expect("the renderer thread runs");
    }

    fn frame(&self) -> Frame {
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Frame(frame)) => break frame,
                Ok(_) => {}
                Err(error) => panic!("no frame: {error:?}"),
            }
        }
    }

    fn query(&mut self, id: &str) -> u64 {
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::Query {
            request,
            selector: format!("#{id}"),
        });
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::QueryResult { request: r, node }) if r == request => {
                    break node.unwrap_or_else(|| panic!("no #{id}"));
                }
                Ok(_) => {}
                Err(error) => panic!("no answer to the query: {error:?}"),
            }
        }
    }

    fn inspect(&mut self, x: f32, y: f32) -> Option<u64> {
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::InspectAt { request, x, y });
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Inspected { request: r, node }) if r == request => break node,
                Ok(_) => {}
                Err(error) => panic!("no answer to the inspection: {error:?}"),
            }
        }
    }

    fn pointer(&self, kind: PointerKind, button: PointerButton, x: f32, y: f32) {
        self.send(ToRenderer::Pointer(PointerInput {
            kind,
            x,
            y,
            button,
            modifiers: Modifiers::default(),
        }));
    }

    /// A press at one point and a release at another, and the event they
    /// make, if any (an inspection marks the end of the answers).
    fn press_release(
        &mut self,
        down: (f32, f32),
        up: (f32, f32),
        button: PointerButton,
    ) -> Option<Event> {
        self.pointer(PointerKind::Down, button, down.0, down.1);
        self.pointer(PointerKind::Up, button, up.0, up.1);
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::InspectAt {
            request,
            x: 0.0,
            y: 0.0,
        });
        let mut event = None;
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Event(e)) => event = Some(e),
                Ok(FromRenderer::Inspected { request: r, .. }) if r == request => break event,
                Ok(_) => {}
                Err(error) => panic!("no answer: {error:?}"),
            }
        }
    }

    fn click(&mut self, x: f32, y: f32) -> Event {
        self.press_release((x, y), (x, y), PointerButton::Primary)
            .unwrap_or_else(|| panic!("no click at {x}, {y}"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.to.send(ToRenderer::Shutdown);
        if let Some(renderer) = self.renderer.take() {
            renderer.join().unwrap();
        }
    }
}

#[test]
fn a_click_reports_the_element_under_the_pointer_and_its_path() {
    let (mut page, _) = Session::new();
    let (a, outer, inner) = (page.query("a"), page.query("outer"), page.query("inner"));

    let click = page.click(85.0, 25.0);
    assert_eq!(click.kind, EventKind::Click);
    assert_eq!(click.target, inner);
    // inner, outer, body, html.
    assert_eq!(click.path.len(), 4, "{:?}", click.path);
    assert_eq!(&click.path[..2], &[inner, outer]);
    assert_eq!((click.x, click.y), (85.0, 25.0));

    let click = page.click(25.0, 25.0);
    assert_eq!(click.target, a);
    assert_eq!(click.path.len(), 3);
    // Body and html are the same for both.
    let other = page.click(85.0, 25.0);
    assert_eq!(&click.path[1..], &other.path[2..]);
}

#[test]
fn a_click_on_text_targets_the_element_the_text_is_in() {
    let (mut page, _) = Session::new();
    let (para, word) = (page.query("para"), page.query("word"));
    // "Bir " is about 30px wide; "kelime" follows it.
    assert_eq!(page.click(55.0, 100.0).target, word);
    assert_eq!(page.click(10.0, 100.0).target, para);
}

#[test]
fn the_topmost_target_wins_and_untargetable_boxes_are_passed_through() {
    let (mut page, _) = Session::new();
    let (top, base, hidden) = (page.query("top"), page.query("base"), page.query("hidden"));
    // z-index 2 over z-index 1.
    assert_eq!(page.click(170.0, 20.0).target, top);
    // pointer-events: none at z-index 3 lets the click through to z-index 2.
    assert_eq!(page.click(170.0, 70.0).target, base);
    // visibility: hidden is no target. The body is empty (its children are
    // all positioned), so the click reaches the root element, which owns
    // the canvas.
    let through = page.click(120.0, 120.0);
    assert_ne!(through.target, hidden);
    assert_eq!(through.path, [through.target], "html alone");
}

#[test]
fn a_press_and_a_release_on_different_elements_click_their_common_ancestor() {
    let (mut page, _) = Session::new();
    let (outer, inner) = (page.query("outer"), page.query("inner"));
    let click = page
        .press_release((85.0, 25.0), (65.0, 5.0), PointerButton::Primary)
        .expect("a click");
    assert_eq!(click.target, outer);
    let click = page
        .press_release((25.0, 25.0), (85.0, 25.0), PointerButton::Primary)
        .expect("a click");
    assert_ne!(click.target, inner);
    assert_eq!(click.path.len(), 2, "body and html");
    // A release on the empty canvas: the root element is the common
    // ancestor.
    let click = page
        .press_release((25.0, 25.0), (120.0, 140.0), PointerButton::Primary)
        .expect("a click");
    assert_eq!(click.path.len(), 1, "html alone");
}

#[test]
fn only_a_press_and_release_of_the_primary_button_click() {
    let (mut page, _) = Session::new();
    // A release without a press.
    page.pointer(PointerKind::Up, PointerButton::Primary, 25.0, 25.0);
    assert_eq!(
        page.press_release((25.0, 25.0), (25.0, 25.0), PointerButton::Secondary),
        None
    );
    // Pressed with one button, released with another.
    page.pointer(PointerKind::Down, PointerButton::Primary, 25.0, 25.0);
    assert_eq!(
        page.press_release((25.0, 25.0), (25.0, 25.0), PointerButton::Secondary),
        None
    );
    // The primary press still waits for its release: that one clicks.
    let a = page.query("a");
    assert_eq!(
        page.press_release((25.0, 25.0), (25.0, 25.0), PointerButton::Primary)
            .map(|click| click.target),
        Some(a)
    );
    // A press released outside the page.
    assert_eq!(
        page.press_release((25.0, 25.0), (500.0, 500.0), PointerButton::Primary),
        None
    );
}

#[test]
fn inspect_at_answers_the_topmost_node_or_none() {
    let (mut page, _) = Session::new();
    let inner = page.query("inner");
    assert_eq!(page.inspect(85.0, 25.0), Some(inner));
    assert_eq!(page.inspect(500.0, 500.0), None);
}

#[test]
fn pointer_input_paints_no_frame() {
    let (page, _) = Session::new();
    page.pointer(PointerKind::Move, PointerButton::None, 25.0, 25.0);
    page.pointer(PointerKind::Down, PointerButton::Primary, 25.0, 25.0);
    page.pointer(PointerKind::Up, PointerButton::Primary, 25.0, 25.0);
    page.pointer(PointerKind::Leave, PointerButton::None, 0.0, 0.0);
    loop {
        match page.from.recv_timeout(QUIET) {
            Ok(FromRenderer::Frame(_)) => panic!("pointer input painted a frame"),
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
}

#[test]
fn the_highlight_is_drawn_over_the_page_and_not_into_it() {
    let (mut page, plain) = Session::new();
    let inner = page.query("inner");
    page.send(ToRenderer::Highlight { node: Some(inner) });
    let lit = page.frame();
    // The display list gains the overlay and nothing else.
    let (overlay, rest): (Vec<&str>, Vec<&str>) = lit
        .display_list()
        .lines()
        .partition(|line| line.starts_with("highlight"));
    assert_eq!(overlay, ["highlight 70 10 30x30"]);
    assert_eq!(rest, plain.display_list().lines().collect::<Vec<_>>());
    // Pixels outside the highlighted box do not change.
    let pixel = |frame: &Frame, x: usize, y: usize| {
        let i = (y * 200 + x) * 4;
        frame.rgba()[i..i + 4].to_vec()
    };
    assert_eq!(pixel(&lit, 25, 25), pixel(&plain, 25, 25));
    assert_ne!(pixel(&lit, 85, 25), pixel(&plain, 85, 25));
    // Removed, or pointed at a node that does not exist: nothing is drawn.
    for node in [None, Some(u64::MAX), Some(inner ^ (1 << 40))] {
        page.send(ToRenderer::Highlight { node });
        let frame = page.frame();
        assert_eq!(frame.display_list(), plain.display_list(), "{node:?}");
    }
}

#[test]
fn a_query_understands_ids_only_for_now() {
    let (mut page, _) = Session::new();
    for selector in ["#missing", "div", "#", ""] {
        page.request += 1;
        let request = page.request;
        page.send(ToRenderer::Query {
            request,
            selector: selector.to_owned(),
        });
        loop {
            match page.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::QueryResult { request: r, node }) if r == request => {
                    assert_eq!(node, None, "{selector:?}");
                    break;
                }
                Ok(_) => {}
                Err(error) => panic!("{error:?}"),
            }
        }
    }
}
