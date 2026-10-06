//! Input through the renderer's messages: hit-testing in paint order,
//! clicks with their propagation path, the developer tools' inspection and
//! highlight, and `#id` queries (M2.1); hover, press and focus state, the
//! focus order and keyboard activation (M2.2); the wheel, scrolling and
//! scroll bars (M2.3).

mod support;

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use erk_renderer::{
    Cursor, Event, EventKind, Frame, Key, KeyInput, KeyState, Modifiers, PointerButton,
    PointerInput, PointerKind, Status,
};
use support::protocol::{FromRenderer, ToRenderer, spawn};

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
        Self::open(PAGE)
    }

    fn open(html: &str) -> (Self, Frame) {
        Self::open_sized(html, 200, 150)
    }

    fn open_sized(html: &str, width: u16, height: u16) -> (Self, Frame) {
        let (to, from, renderer) = spawn();
        let session = Self {
            to,
            from,
            renderer: Some(renderer),
            request: 0,
        };
        session.send(ToRenderer::Load {
            html: html.to_owned(),
        });
        session.send(ToRenderer::Resize { width, height });
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
        self.select(None, &format!("#{id}"))
            .unwrap_or_else(|status| panic!("#{id}: {status:?}"))
            .unwrap_or_else(|| panic!("no #{id}"))
    }

    /// The answer to a query for `selector` inside `scope`.
    fn select(&mut self, scope: Option<u64>, selector: &str) -> Result<Option<u64>, Status> {
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::Query {
            request,
            scope,
            selector: selector.to_owned(),
        });
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::QueryResult { request: r, result }) if r == request => {
                    break result;
                }
                Ok(_) => {}
                Err(error) => panic!("no answer to the query: {error:?}"),
            }
        }
    }

    /// The answer to setting `node`'s text.
    fn set_text(&mut self, node: u64, text: &str) -> Result<(), Status> {
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::SetText {
            request,
            node,
            text: text.to_owned(),
        });
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Done { request: r, result }) if r == request => break result,
                Ok(_) => {}
                Err(error) => panic!("no answer to the change: {error:?}"),
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

    /// The events `inputs` make, in order (an inspection marks the end).
    fn events(&mut self, inputs: Vec<ToRenderer>) -> Vec<Event> {
        for input in inputs {
            self.send(input);
        }
        self.request += 1;
        let request = self.request;
        self.send(ToRenderer::InspectAt {
            request,
            x: 0.0,
            y: 0.0,
        });
        let mut events = Vec::new();
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Event(event)) => events.push(event),
                Ok(FromRenderer::Inspected { request: r, .. }) if r == request => break events,
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
fn a_query_finds_the_first_match_or_says_what_is_wrong() {
    let (mut page, _) = Session::new();
    let (a, outer, inner) = (page.query("a"), page.query("outer"), page.query("inner"));
    assert_eq!(
        page.select(None, "div"),
        Ok(Some(a)),
        "the first in document order"
    );
    assert_eq!(page.select(None, "div > div"), Ok(Some(inner)));
    assert_eq!(page.select(None, "p span, #outer"), Ok(Some(outer)));
    assert_eq!(page.select(None, "#missing"), Ok(None));
    // Inside a scope, which is not itself a match.
    assert_eq!(page.select(Some(outer), "div"), Ok(Some(inner)));
    assert_eq!(page.select(Some(inner), "div"), Ok(None));
    assert_eq!(page.select(Some(outer), ":scope > div"), Ok(Some(inner)));
    // What is wrong.
    for selector in ["", "#", "div >", "!!"] {
        assert_eq!(
            page.select(None, selector),
            Err(Status::InvalidArgument),
            "{selector:?}"
        );
    }
    assert_eq!(
        page.select(Some(0), "div"),
        Err(Status::InvalidArgument),
        "no node"
    );
    assert_eq!(
        page.select(Some(outer + (1 << 32)), "div"),
        Err(Status::StaleNode),
        "another generation"
    );
}

#[test]
fn set_text_changes_the_page_and_a_stale_node_is_an_error() {
    let (mut page, plain) = Session::new();
    let (para, word) = (page.query("para"), page.query("word"));
    // The word was inside the paragraph; the new text replaces it.
    assert_eq!(page.set_text(para, "Yeni metin"), Ok(()));
    let frame = page.frame();
    assert!(
        frame.display_list().contains("\"Yeni metin\""),
        "{}",
        frame.display_list()
    );
    assert!(!frame.display_list().contains("kelime"));
    assert!(frame.rgba() != plain.rgba());
    assert_eq!(
        page.set_text(word, "x"),
        Err(Status::StaleNode),
        "the word is gone"
    );
    assert_eq!(page.select(Some(word), "*"), Err(Status::StaleNode));
    assert_eq!(page.set_text(0, "x"), Err(Status::InvalidArgument));
    // A failed change paints nothing.
    nothing_comes(&page);
    // The same text again: a frame all the same, and the same pixels.
    assert_eq!(page.set_text(para, "Yeni metin"), Ok(()));
    assert!(page.frame().rgba() == frame.rgba());
}

#[test]
fn loading_a_page_makes_the_old_pages_ids_stale() {
    let (mut page, _) = Session::new();
    let ids: Vec<u64> = ["a", "outer", "inner", "para", "word"]
        .into_iter()
        .map(|id| page.query(id))
        .collect();
    // The same page again: its elements get new ids.
    page.send(ToRenderer::Load {
        html: PAGE.to_owned(),
    });
    page.frame();
    for old in &ids {
        assert_eq!(page.set_text(*old, "x"), Err(Status::StaleNode), "{old:#x}");
        assert_eq!(page.select(Some(*old), "*"), Err(Status::StaleNode));
    }
    let new = page.query("a");
    assert!(!ids.contains(&new));
    assert_eq!(page.set_text(new, "yeni"), Ok(()));
}

/// Elements whose look follows the user's state, and a focus order with
/// every kind of `tabindex`.
const STATES: &str = r#"<style>
  div { position: absolute; width: 40px; height: 40px; background: rgb(200, 200, 200) }
  #h:hover { background: rgb(255, 0, 0) }
  #h:active { background: rgb(0, 0, 255) }
  #f:focus { background: rgb(0, 255, 0) }
  #box:focus-within { background: rgb(255, 255, 0) }
</style>
<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px">
  <div id="h" style="left: 0; top: 0"></div>
  <div id="f" tabindex="0" style="left: 50px; top: 0"></div>
  <div id="box" style="left: 100px; top: 0; width: 60px; height: 60px"><a id="link" href="x">bağ</a></div>
  <div id="second" tabindex=" +2 " style="left: 0; top: 50px"></div>
  <div id="first" tabindex="1" style="left: 50px; top: 50px"></div>
  <div id="pointer-only" tabindex="-1" style="left: 0; top: 100px"></div>
  <div id="gone" tabindex="0" style="display: none"></div>
  <div id="hidden" tabindex="0" style="left: 150px; top: 100px; visibility: hidden"></div>
  <div id="card" tabindex="-1" style="left: 150px; top: 50px"><div id="child" style="position: static; width: 20px; height: 20px"></div></div>
  <input id="secret" type="Hidden">
  <a id="plain">no href</a>
  <button id="button" style="position: absolute; left: 100px; top: 100px">OK</button>
  <button id="disabled" disabled tabindex="0">no</button>
</body>"#;

fn pixel(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    [frame.rgba()[i], frame.rgba()[i + 1], frame.rgba()[i + 2]]
}

fn pointer(kind: PointerKind, button: PointerButton, x: f32, y: f32) -> ToRenderer {
    ToRenderer::Pointer(PointerInput {
        kind,
        x,
        y,
        button,
        modifiers: Modifiers::default(),
    })
}

fn key(key: Key, state: KeyState, shift: bool) -> ToRenderer {
    ToRenderer::Key(KeyInput {
        key,
        state,
        modifiers: Modifiers {
            shift,
            ..Modifiers::default()
        },
    })
}

fn press(x: f32, y: f32) -> Vec<ToRenderer> {
    vec![
        pointer(PointerKind::Down, PointerButton::Primary, x, y),
        pointer(PointerKind::Up, PointerButton::Primary, x, y),
    ]
}

/// Each event as its kind and the name of its target.
fn named(events: &[Event], names: &[(&str, u64)]) -> Vec<(EventKind, String)> {
    events
        .iter()
        .map(|event| {
            let name = names
                .iter()
                .find(|(_, node)| *node == event.target)
                .map_or_else(
                    || format!("{:#x}", event.target),
                    |(name, _)| (*name).to_owned(),
                );
            (event.kind, name)
        })
        .collect()
}

fn names(page: &mut Session, ids: &[&'static str]) -> Vec<(&'static str, u64)> {
    ids.iter().map(|id| (*id, page.query(id))).collect()
}

#[test]
fn hover_and_active_restyle_the_element_under_the_pointer() {
    let (page, plain) = Session::open(STATES);
    let grey = [200, 200, 200];
    assert_eq!(pixel(&plain, 20, 20), grey);

    page.send(pointer(PointerKind::Move, PointerButton::None, 20.0, 20.0));
    assert_eq!(pixel(&page.frame(), 20, 20), [255, 0, 0], "hover");
    page.send(pointer(
        PointerKind::Down,
        PointerButton::Primary,
        20.0,
        20.0,
    ));
    assert_eq!(pixel(&page.frame(), 20, 20), [0, 0, 255], "active");
    page.send(pointer(PointerKind::Up, PointerButton::Primary, 20.0, 20.0));
    assert_eq!(
        pixel(&page.frame(), 20, 20),
        [255, 0, 0],
        "released, still hovered"
    );
    page.send(pointer(PointerKind::Leave, PointerButton::None, 20.0, 20.0));
    assert_eq!(pixel(&page.frame(), 20, 20), grey, "left the page");
}

#[test]
fn a_move_within_the_hovered_element_paints_nothing() {
    let (page, _) = Session::open(STATES);
    page.send(pointer(PointerKind::Move, PointerButton::None, 20.0, 20.0));
    page.frame();
    page.send(pointer(PointerKind::Move, PointerButton::None, 30.0, 10.0));
    match page.from.recv_timeout(QUIET) {
        Err(RecvTimeoutError::Timeout) => {}
        other => panic!("expected nothing, got {:?}", other.map(|_| "a message")),
    }
}

#[test]
fn a_press_moves_the_focus_and_reports_blur_then_focus() {
    let (mut page, _) = Session::open(STATES);
    let ids = names(
        &mut page,
        &["h", "f", "box", "link", "pointer-only", "card", "child"],
    );
    let (click, focus, blur) = (EventKind::Click, EventKind::Focus, EventKind::Blur);
    let s = |name: &str| name.to_owned();

    let events = page.events(press(70.0, 20.0));
    assert_eq!(named(&events, &ids), [(focus, s("f")), (click, s("f"))]);
    assert_eq!(pixel(&page.frame(), 70, 20), [0, 255, 0], ":focus");

    // On the link's text: the link takes the focus, its block matches
    // :focus-within.
    let events = page.events(press(108.0, 10.0));
    assert_eq!(
        named(&events, &ids),
        [(blur, s("f")), (focus, s("link")), (click, s("link"))]
    );
    let frame = page.frame();
    assert_eq!(pixel(&frame, 70, 20), [200, 200, 200], "blurred");
    assert_eq!(pixel(&frame, 110, 50), [255, 255, 0], ":focus-within");

    // A negative tabindex takes the focus from the pointer.
    let events = page.events(press(20.0, 120.0));
    assert_eq!(
        named(&events, &ids),
        [
            (blur, s("link")),
            (focus, s("pointer-only")),
            (click, s("pointer-only"))
        ]
    );

    // Pressing inside a focusable element focuses it.
    let events = page.events(press(155.0, 55.0));
    assert_eq!(
        named(&events, &ids),
        [
            (blur, s("pointer-only")),
            (focus, s("card")),
            (click, s("child"))
        ]
    );

    // An element that cannot take the focus takes it away.
    let events = page.events(press(20.0, 20.0));
    assert_eq!(named(&events, &ids), [(blur, s("card")), (click, s("h"))]);
    assert!(
        events
            .iter()
            .all(|event| event.path.last() == events[0].path.last())
    );
}

#[test]
fn tab_walks_the_focus_order_and_shift_tab_walks_it_back() {
    let (mut page, _) = Session::open(STATES);
    let ids = names(&mut page, &["first", "second", "f", "link", "button"]);
    let focused = |events: Vec<Event>| -> Vec<String> {
        named(&events, &ids)
            .into_iter()
            .filter(|(kind, _)| *kind == EventKind::Focus)
            .map(|(_, name)| name)
            .collect()
    };
    let tab = |shift| {
        vec![
            key(Key::Tab, KeyState::Down, shift),
            key(Key::Tab, KeyState::Up, shift),
        ]
    };

    // Positive tabindex values first, from the lowest; then the rest in
    // document order; negative, hidden, undisplayed, disabled and
    // href-less elements are skipped; the end wraps around.
    let mut forward = Vec::new();
    for _ in 0..6 {
        forward.extend(focused(page.events(tab(false))));
    }
    assert_eq!(forward, ["first", "second", "f", "link", "button", "first"]);

    let mut back = Vec::new();
    for _ in 0..3 {
        back.extend(focused(page.events(tab(true))));
    }
    assert_eq!(back, ["button", "link", "f"]);

    // With nothing focused, Shift+Tab starts from the end.
    let (mut fresh, _) = Session::open(STATES);
    let button = fresh.query("button");
    let events = fresh.events(tab(true));
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(
        (events[0].kind, events[0].target),
        (EventKind::Focus, button)
    );
}

#[test]
fn enter_and_space_click_the_focused_link_or_button_as_browsers_do() {
    let (mut page, _) = Session::open(STATES);
    let ids = names(&mut page, &["first", "second", "f", "link", "button"]);
    let tab = || vec![key(Key::Tab, KeyState::Down, false)];
    let clicks = |events: Vec<Event>| -> Vec<String> {
        assert!(events.iter().all(|event| (event.x, event.y) == (0.0, 0.0)));
        named(&events, &ids)
            .into_iter()
            .filter(|(kind, _)| *kind == EventKind::Click)
            .map(|(_, name)| name)
            .collect()
    };
    let enter = || {
        vec![
            key(Key::Enter, KeyState::Down, false),
            key(Key::Enter, KeyState::Up, false),
        ]
    };
    let space_down = || vec![key(Key::Space, KeyState::Down, false)];
    let space_up = || vec![key(Key::Space, KeyState::Up, false)];

    // first, second, f: a focusable div does not activate.
    for _ in 0..3 {
        page.events(tab());
    }
    assert!(clicks(page.events(enter())).is_empty());
    assert!(clicks(page.events(space_down().into_iter().chain(space_up()).collect())).is_empty());
    // link: Enter follows it, Space does not.
    page.events(tab());
    assert_eq!(clicks(page.events(enter())), ["link"]);
    assert!(clicks(page.events(space_down().into_iter().chain(space_up()).collect())).is_empty());
    // button: Enter on the press, Space on the release.
    page.events(tab());
    assert_eq!(clicks(page.events(enter())), ["button"]);
    assert!(clicks(page.events(space_down())).is_empty());
    assert_eq!(clicks(page.events(space_up())), ["button"]);
    // Other keys do nothing.
    let other = vec![
        key(Key::Escape, KeyState::Down, false),
        key(Key::Character("a".to_owned()), KeyState::Down, false),
        key(Key::Other, KeyState::Down, false),
    ];
    assert!(page.events(other).is_empty());
}

#[test]
fn keys_with_nothing_to_focus_do_nothing() {
    let (mut page, _) = Session::new();
    let keys = vec![
        key(Key::Tab, KeyState::Down, false),
        key(Key::Tab, KeyState::Down, true),
        key(Key::Enter, KeyState::Down, false),
        key(Key::Space, KeyState::Up, false),
    ];
    assert!(page.events(keys).is_empty());
}

/// The pixels of `frame` outside `x`, `y`, `width` × `height` (the boxes a
/// state may restyle), and the same for `other`, differ nowhere.
fn same_outside(frame: &Frame, other: &Frame, boxes: &[(usize, usize, usize, usize)]) -> bool {
    let width = usize::from(frame.width());
    frame
        .rgba()
        .chunks(4)
        .zip(other.rgba().chunks(4))
        .enumerate()
        .all(|(i, (a, b))| {
            let (x, y) = (i % width, i / width);
            a == b
                || boxes
                    .iter()
                    .any(|(bx, by, bw, bh)| x >= *bx && y >= *by && x < bx + bw && y < by + bh)
        })
}

/// The `states` reference page, whose stateless look Chrome's screenshot
/// checks, in each state its styles name: only the element in the state
/// changes, to the colours its rules give (boxes from Chrome's geometry).
#[test]
fn the_states_reference_page_follows_the_pointer_and_the_focus() {
    let html = include_str!("reference/pages/states.html");
    let (page, plain) = Session::open_sized(html, 800, 600);
    let rgb = |hex: u32| [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8];
    // "Vazgeç": its padding and its border; "Gönder"'s border; the form's
    // border and inside; the "Ad" field's border; the second list item.
    // Regions are a pixel larger than the boxes: Erk rounds their edges.
    let cancel = (115, 15, 89, 43);
    let (cancel_fill, cancel_edge) = ((125, 22), (117, 30));
    let send_edge = (214, 30);
    let form = (19, 91, 398, 174);
    let (form_edge, form_fill) = ((21, 150), (30, 100));
    let name = (37, 137, 362, 36);
    let name_edge = (38, 150);
    let item = (20, 321, 366, 39);
    let item_fill = (380, 330);
    let at = |frame: &Frame, (x, y): (usize, usize)| pixel(frame, x, y);

    assert_eq!(at(&plain, cancel_fill), rgb(0xe2e8f0));
    assert_eq!(at(&plain, cancel_edge), rgb(0x94a3b8));

    let step = |input: ToRenderer| {
        page.send(input);
        page.frame()
    };
    let hovered = step(pointer(PointerKind::Move, PointerButton::None, 125.0, 22.0));
    assert_eq!(at(&hovered, cancel_fill), rgb(0xbfdbfe), ":hover");
    assert_eq!(at(&hovered, cancel_edge), rgb(0x3b82f6), ":hover");
    assert!(same_outside(&hovered, &plain, &[cancel]));

    let pressed = step(pointer(
        PointerKind::Down,
        PointerButton::Primary,
        125.0,
        22.0,
    ));
    assert_eq!(at(&pressed, cancel_fill), rgb(0x1d4ed8), ":active");
    assert_eq!(
        at(&pressed, cancel_edge),
        rgb(0xf59e0b),
        ":focus over :hover"
    );
    assert!(same_outside(&pressed, &plain, &[cancel]));

    let released = step(pointer(
        PointerKind::Up,
        PointerButton::Primary,
        125.0,
        22.0,
    ));
    assert_eq!(at(&released, cancel_fill), rgb(0xbfdbfe));
    let left = step(pointer(PointerKind::Leave, PointerButton::None, 0.0, 0.0));
    assert_eq!(at(&left, cancel_fill), rgb(0xe2e8f0));
    assert_eq!(at(&left, cancel_edge), rgb(0xf59e0b), "still focused");

    let next = step(key(Key::Tab, KeyState::Down, false));
    assert_eq!(at(&next, cancel_edge), rgb(0x94a3b8), "blurred");
    assert_eq!(at(&next, send_edge), rgb(0xf59e0b), ":focus over .birincil");

    let in_form = step(key(Key::Tab, KeyState::Down, false));
    assert_eq!(at(&in_form, form_edge), rgb(0x22c55e), ":focus-within");
    assert_eq!(at(&in_form, form_fill), rgb(0xf0fdf4), ":focus-within");
    assert_eq!(at(&in_form, name_edge), rgb(0x22c55e), ":focus");
    assert!(same_outside(&in_form, &plain, &[form, name]));

    let over_item = step(pointer(
        PointerKind::Move,
        PointerButton::None,
        200.0,
        340.0,
    ));
    assert_eq!(at(&over_item, item_fill), rgb(0xf1f5f9), "list item :hover");
    assert!(same_outside(&over_item, &in_form, &[item]));
}

/// A scroll container (`auto`), one that only clips (`hidden`), and a
/// document taller than the viewport: 50 + 10 + 50 + 400 = 510 pixels in a
/// 150 pixel viewport.
const SCROLLING: &str = r#"<body id="body" style="margin: 0">
  <div id="scroller" style="overflow: auto; width: 100px; height: 50px">
    <div id="s1" style="height: 50px; background: rgb(255, 0, 0)"></div>
    <div id="s2" style="height: 50px; background: rgb(0, 255, 0)"></div>
    <div id="s3" style="height: 50px; background: rgb(0, 0, 255)"></div>
  </div>
  <div id="hidden" style="overflow: hidden; width: 100px; height: 50px; margin-top: 10px">
    <div style="height: 50px; background: rgb(255, 255, 0)"></div>
    <div style="height: 50px; background: rgb(0, 255, 255)"></div>
  </div>
  <div id="tall" style="height: 400px; background: rgb(204, 204, 204)"></div>
</body>"#;

fn wheel(dy: f32, x: f32, y: f32) -> ToRenderer {
    ToRenderer::Wheel { dx: 0.0, dy, x, y }
}

fn nothing_comes(page: &Session) {
    match page.from.recv_timeout(QUIET) {
        Err(RecvTimeoutError::Timeout) => {}
        other => panic!("expected nothing, got {:?}", other.map(|_| "a message")),
    }
}

#[test]
fn the_wheel_scrolls_the_innermost_container_then_the_ones_around_it() {
    let (page, plain) = Session::open(SCROLLING);
    let (red, green, blue) = ([255, 0, 0], [0, 255, 0], [0, 0, 255]);
    let (yellow, cyan) = ([255, 255, 0], [0, 255, 255]);
    assert_eq!(pixel(&plain, 50, 30), red);

    let step = |input: ToRenderer| {
        page.send(input);
        page.frame()
    };
    // 30 pixels into the container.
    let frame = step(wheel(30.0, 50.0, 25.0));
    assert_eq!(pixel(&frame, 50, 10), red);
    assert_eq!(pixel(&frame, 50, 30), green);
    assert_eq!(pixel(&frame, 50, 80), yellow, "the document did not move");

    // The container takes 70 more, to its end; the document the other 30.
    let frame = step(wheel(100.0, 50.0, 25.0));
    assert_eq!(
        pixel(&frame, 50, 10),
        blue,
        "the container's end, 30 pixels up"
    );
    assert_eq!(
        pixel(&frame, 50, 40),
        yellow,
        "the clipping box, 30 pixels up"
    );

    // A box that only clips does not scroll: the document does.
    let frame = step(wheel(20.0, 50.0, 40.0));
    assert_eq!(pixel(&frame, 50, 15), yellow, "50 pixels up");
    assert_ne!(
        pixel(&frame, 50, 55),
        cyan,
        "its hidden content stays hidden"
    );

    // Back to the top of the document; the container keeps its offset.
    let frame = step(wheel(-1000.0, 150.0, 100.0));
    assert_eq!(pixel(&frame, 50, 10), blue);
    assert_eq!(pixel(&frame, 50, 80), yellow);

    // Nothing can move further up: no frame.
    page.send(wheel(-10.0, 150.0, 100.0));
    nothing_comes(&page);
}

#[test]
fn hit_regions_move_with_the_content_and_are_clipped_with_it() {
    let (mut page, _) = Session::open(SCROLLING);
    let ids = names(&mut page, &["body", "s1", "s2", "s3", "scroller"]);
    let at = |page: &mut Session, x, y| {
        let node = page.inspect(x, y).expect("a node");
        ids.iter()
            .find(|(_, id)| *id == node)
            .map_or("other", |(name, _)| *name)
    };
    // Below the container its second block is clipped away: the body is
    // there.
    assert_eq!(at(&mut page, 50.0, 55.0), "body");
    assert_eq!(at(&mut page, 50.0, 10.0), "s1");

    page.send(wheel(100.0, 50.0, 25.0));
    page.frame();
    assert_eq!(at(&mut page, 50.0, 10.0), "s3");
    assert_eq!(at(&mut page, 50.0, 55.0), "body");
    let click = page.click(50.0, 10.0);
    assert_eq!(click.target, ids[3].1, "s3");
}

#[test]
fn scroll_bars_show_while_the_pointer_is_over_what_they_scroll() {
    let (page, plain) = Session::open(SCROLLING);
    // The container's thumb along its right edge, the document's along the
    // viewport's.
    let (thumb, page_thumb) = ((95, 10), (195, 10));
    assert_eq!(pixel(&plain, thumb.0, thumb.1), [255, 0, 0]);
    assert_eq!(pixel(&plain, page_thumb.0, page_thumb.1), [255, 255, 255]);

    page.send(pointer(PointerKind::Move, PointerButton::None, 50.0, 25.0));
    let over = page.frame();
    assert!(
        pixel(&over, thumb.0, thumb.1)[0] < 200,
        "the container's thumb"
    );
    assert!(
        pixel(&over, page_thumb.0, page_thumb.1)[0] < 180,
        "the document's thumb"
    );

    // Over the document only: the container's thumb goes.
    page.send(pointer(
        PointerKind::Move,
        PointerButton::None,
        150.0,
        100.0,
    ));
    let elsewhere = page.frame();
    assert_eq!(pixel(&elsewhere, thumb.0, thumb.1), [255, 0, 0]);
    assert!(pixel(&elsewhere, page_thumb.0, page_thumb.1)[0] < 180);

    page.send(pointer(PointerKind::Leave, PointerButton::None, 0.0, 0.0));
    let left = page.frame();
    assert!(
        left.rgba() == plain.rgba(),
        "no thumbs once the pointer has left"
    );
}

#[test]
fn a_larger_viewport_takes_back_what_the_document_no_longer_scrolls() {
    let (page, _) = Session::open(SCROLLING);
    page.send(wheel(1000.0, 150.0, 100.0));
    let scrolled = page.frame();
    assert_eq!(
        pixel(&scrolled, 50, 10),
        [204, 204, 204],
        "scrolled to the end"
    );
    page.send(ToRenderer::Resize {
        width: 200,
        height: 600,
    });
    let frame = page.frame();
    assert_eq!(pixel(&frame, 50, 10), [255, 0, 0], "back at the top");
}

/// The acceptance item: a long page scrolls with the wheel and shows other
/// content.
#[test]
fn a_long_page_scrolls_with_the_wheel() {
    let html = include_str!("../../../examples/perf/long-page.html");
    let (page, top) = Session::open_sized(html, 800, 600);
    // Each glyph run's position and text.
    let runs = |frame: &Frame| -> Vec<(f32, f32, String)> {
        frame
            .display_list()
            .lines()
            .filter_map(|line| {
                let mut words = line.strip_prefix("glyphs ")?.splitn(3, ' ');
                let x = words.next()?.parse().ok()?;
                let y = words.next()?.parse().ok()?;
                Some((x, y, words.next()?.to_owned()))
            })
            .collect()
    };
    page.send(wheel(1500.0, 400.0, 300.0));
    let down = page.frame();
    assert!(down.rgba() != top.rgba(), "other content shows");
    let (before, after) = (runs(&top), runs(&down));
    assert!(before.len() > 50, "{}", before.len());
    assert_eq!(before.len(), after.len());
    for ((x, y, text), (x2, y2, text2)) in before.iter().zip(&after) {
        assert_eq!((x, text), (x2, text2));
        assert!((y - 1500.0 - y2).abs() < 0.01, "{text}: {y} -> {y2}");
    }
    page.send(wheel(-1500.0, 400.0, 300.0));
    let back = page.frame();
    assert!(back.rgba() == top.rgba(), "back at the top, as it was");
}

#[test]
fn the_cursor_follows_what_the_pointer_is_over() {
    let html = r#"<style>.h:hover { cursor: pointer }</style>
    <body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px">
      <div style="height: 30px"></div>
      <div style="height: 30px">Metin burada</div>
      <a href="x" style="display: block; height: 30px">bağlantı</a>
      <div style="cursor: move; height: 30px"><span style="cursor: auto">yazı</span> boş</div>
      <div style="cursor: none; height: 30px"></div>
      <div class="h" style="height: 30px"></div>
    </body>"#;
    let (page, _) = Session::open_sized(html, 200, 200);
    // The cursor the renderer tells after `input`, if it changes.
    let after = |input: ToRenderer| {
        page.send(input);
        loop {
            match page.from.recv_timeout(QUIET) {
                Ok(FromRenderer::Cursor(cursor)) => break Some(cursor),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => break None,
                Err(error) => panic!("{error:?}"),
            }
        }
    };
    let over = |x, y| pointer(PointerKind::Move, PointerButton::None, x, y);
    assert_eq!(after(over(50.0, 15.0)), None, "the arrow, as before");
    assert_eq!(
        after(over(10.0, 45.0)),
        Some(Cursor::Text),
        "auto over text"
    );
    assert_eq!(
        after(over(150.0, 45.0)),
        Some(Cursor::Default),
        "auto beside it"
    );
    assert_eq!(after(over(10.0, 75.0)), Some(Cursor::Pointer), "a link");
    assert_eq!(
        after(over(5.0, 105.0)),
        Some(Cursor::Text),
        "auto inside move"
    );
    assert_eq!(after(over(150.0, 105.0)), Some(Cursor::Move));
    assert_eq!(after(over(50.0, 135.0)), Some(Cursor::None));
    assert_eq!(
        after(over(50.0, 165.0)),
        Some(Cursor::Pointer),
        "set by :hover"
    );
    let leave = pointer(PointerKind::Leave, PointerButton::None, 0.0, 0.0);
    assert_eq!(after(leave), Some(Cursor::Default), "off the page");
}

/// The `overflow` reference page, whose unscrolled look Chrome's screenshot
/// checks, scrolled: only the scroll container under the wheel changes.
#[test]
fn the_overflow_reference_page_scrolls_inside_its_clips() {
    let html = include_str!("reference/pages/overflow.html");
    let (page, plain) = Session::open_sized(html, 800, 600);
    // The list's and the wide row's border boxes, from Chrome's geometry.
    let (list, row) = ((20, 20, 222, 262), (262, 326, 300, 50));

    page.send(ToRenderer::Wheel {
        dx: 0.0,
        dy: 100.0,
        x: 130.0,
        y: 150.0,
    });
    let down = page.frame();
    assert!(down.rgba() != plain.rgba());
    assert!(same_outside(&down, &plain, &[list]));

    // Sideways in the wide row; the document has nothing to scroll, so
    // turning further down there changes nothing.
    page.send(ToRenderer::Wheel {
        dx: 100.0,
        dy: 0.0,
        x: 400.0,
        y: 350.0,
    });
    let across = page.frame();
    assert!(same_outside(&across, &down, &[row]));
    assert!(across.rgba() != down.rgba());
    page.send(ToRenderer::Wheel {
        dx: 0.0,
        dy: 100.0,
        x: 400.0,
        y: 350.0,
    });
    nothing_comes(&page);
}

#[test]
fn text_directly_in_a_scroll_container_scrolls_with_it() {
    // A paragraph of its own, and text beside a block (anonymous boxes).
    for content in [
        "bir iki üç dört beş",
        "bir<div>iki</div>üç<div>dört</div>beş",
    ] {
        // Nothing else scrolls: the container must take the wheel itself.
        let html = format!(
            r#"<body style="margin: 0; font-family: 'Noto Sans'; font-size: 16px">
            <div style="overflow: auto; width: 40px; height: 40px">{content}</div></body>"#
        );
        let (page, top) = Session::open(&html);
        page.send(wheel(30.0, 20.0, 20.0));
        let down = page.frame();
        let ys = |frame: &Frame| -> Vec<f32> {
            frame
                .display_list()
                .lines()
                .filter_map(|line| {
                    line.strip_prefix("glyphs ")?
                        .split(' ')
                        .nth(1)?
                        .parse()
                        .ok()
                })
                .collect()
        };
        let (before, after) = (ys(&top), ys(&down));
        assert!(before.len() >= 3, "{content}: {before:?}");
        assert_eq!(before.len(), after.len(), "{content}");
        for (y, y2) in before.iter().zip(&after) {
            assert!((y - 30.0 - y2).abs() < 0.01, "{content}: {y} -> {y2}");
        }
        // To the end: the lowest line's baseline is inside the 40 pixel box,
        // in its lower half.
        page.send(wheel(1000.0, 20.0, 20.0));
        let end = page.frame();
        let last = ys(&end).into_iter().fold(f32::MIN, f32::max);
        assert!((20.0..40.0).contains(&last), "{content}: {last}");
    }
}

#[test]
fn a_wide_flex_strip_scrolls_sideways_in_its_scroller() {
    // The M2 demo's strip: a scroller only as tall as its content, so a
    // vertical turn leaves it alone and a sideways one moves it.
    let html = r#"<body style="margin: 0"><div style="width: 300px; overflow: auto">
        <div style="display: flex; width: 900px; height: 40px">
        <div style="flex: 0 0 450px; background: rgb(255, 0, 0)"></div>
        <div style="flex: 0 0 450px; background: rgb(0, 0, 255)"></div></div></div></body>"#;
    let (page, plain) = Session::open(html);
    assert_eq!(pixel(&plain, 10, 20), [255, 0, 0]);
    page.send(wheel(100.0, 100.0, 20.0));
    nothing_comes(&page);
    page.send(ToRenderer::Wheel {
        dx: 500.0,
        dy: 0.0,
        x: 100.0,
        y: 20.0,
    });
    assert_eq!(
        pixel(&page.frame(), 10, 20),
        [0, 0, 255],
        "scrolled sideways"
    );
}

#[test]
fn a_wider_viewport_takes_back_what_the_document_no_longer_scrolls_sideways() {
    let html = r#"<body style="margin: 0"><div style="display: flex; width: 600px; height: 50px">
        <div style="flex: 1; background: rgb(255, 0, 0)"></div>
        <div style="flex: 1; background: rgb(0, 0, 255)"></div></div></body>"#;
    let (page, _) = Session::open(html);
    page.send(ToRenderer::Wheel {
        dx: 1000.0,
        dy: 0.0,
        x: 100.0,
        y: 25.0,
    });
    let scrolled = page.frame();
    assert_eq!(
        pixel(&scrolled, 10, 25),
        [0, 0, 255],
        "scrolled to the right end"
    );
    page.send(ToRenderer::Resize {
        width: 800,
        height: 150,
    });
    assert_eq!(
        pixel(&page.frame(), 10, 25),
        [255, 0, 0],
        "back at the left"
    );
}
