//! Events, callbacks, posted work and resources against p1-contract §4-§6.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use erk::{
    App, Config, Context, Event, EventKind, Input, KeyInput, KeyState, LogLevel, Modifiers, Node,
    Phase, ResourceKind, Status,
};

/// A box at the top left inside a box inside the body: a click at 10, 10
/// has the path inner, outer, body, html.
const NESTED: &str = r#"<body style="margin: 0; background: #123456">
    <div id="outer" style="width: 60px; height: 40px; padding: 5px">
      <div id="inner" style="width: 30px; height: 20px"></div>
    </div>"#;

fn app(html: &str) -> App {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.load_html(html);
    app.tick(0);
    app
}

fn node(app: &App, selector: &str) -> Node {
    app.query(None, selector).unwrap().unwrap()
}

type Log = Rc<RefCell<Vec<String>>>;

/// A callback that writes `name`, the phase and whether the event's target
/// is right into `log`.
fn record(
    log: &Log,
    name: &'static str,
    target: Node,
) -> impl FnMut(&mut Context, &Event) + 'static {
    let log = log.clone();
    move |_, event| {
        assert_eq!(event.target, target);
        assert_eq!(event.kind, EventKind::Click);
        log.borrow_mut().push(format!("{name} {:?}", event.phase));
    }
}

#[test]
fn a_click_goes_down_to_its_target_and_back_up() {
    let mut app = app(NESTED);
    let (html, body) = (node(&app, "html"), node(&app, "body"));
    let (outer, inner) = (node(&app, "#outer"), node(&app, "#inner"));
    let log = Log::default();
    // Made in an order unlike the dispatch order.
    app.on(body, EventKind::Click, record(&log, "body", inner))
        .unwrap();
    app.on(inner, EventKind::Click, record(&log, "inner", inner))
        .unwrap();
    app.on_capture(html, EventKind::Click, record(&log, "html", inner))
        .unwrap();
    app.on_capture(
        inner,
        EventKind::Click,
        record(&log, "inner-capture", inner),
    )
    .unwrap();
    app.on(html, EventKind::Click, record(&log, "html", inner))
        .unwrap();
    app.on_capture(outer, EventKind::Click, record(&log, "outer", inner))
        .unwrap();
    app.click(10.0, 10.0);
    assert_eq!(
        *log.borrow(),
        [
            "html Capture",
            "outer Capture",
            "inner-capture Target",
            "inner Target",
            "body Bubble",
            "html Bubble",
        ]
    );
}

#[test]
fn stopping_propagation_finishes_the_node_it_is_at() {
    let mut app = app(NESTED);
    let (html, body, inner) = (node(&app, "html"), node(&app, "body"), node(&app, "#inner"));
    let log = Log::default();
    let stopper = log.clone();
    app.on(body, EventKind::Click, move |cx, _| {
        stopper.borrow_mut().push("body stops".to_owned());
        cx.stop_propagation();
    })
    .unwrap();
    app.on(body, EventKind::Click, record(&log, "body", inner))
        .unwrap();
    app.on(html, EventKind::Click, record(&log, "html", inner))
        .unwrap();
    app.click(10.0, 10.0);
    assert_eq!(*log.borrow(), ["body stops", "body Bubble"]);
    // The next event starts afresh.
    log.borrow_mut().clear();
    app.click(10.0, 10.0);
    assert_eq!(*log.borrow(), ["body stops", "body Bubble"]);
}

#[test]
fn focus_does_not_bubble() {
    let mut app = app(r#"<body style="margin: 0"><button id="b">tamam</button>"#);
    let (body, button) = (node(&app, "body"), node(&app, "#b"));
    let seen = Log::default();
    let (on_button, on_body) = (seen.clone(), seen.clone());
    app.on(button, EventKind::Focus, move |_, event| {
        on_button
            .borrow_mut()
            .push(format!("button {:?}", event.phase));
    })
    .unwrap();
    app.on(body, EventKind::Focus, move |_, _| {
        on_body.borrow_mut().push("body".to_owned());
    })
    .unwrap();
    app.input(Input::Key(KeyInput {
        key: erk::Key::Tab,
        state: KeyState::Down,
        modifiers: Modifiers::default(),
    }));
    assert_eq!(*seen.borrow(), ["button Target"]);
}

/// Counts its drops: the closure's `destroy`.
struct Drops(Rc<Cell<u32>>);

impl Drop for Drops {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn counted(drops: &Rc<Cell<u32>>) -> impl FnMut(&mut Context, &Event) + 'static {
    let guard = Drops(drops.clone());
    move |_, _| {
        let _ = &guard;
    }
}

#[test]
fn a_callback_is_dropped_once_when_its_subscription_ends() {
    // By `off`.
    let mut app = app(NESTED);
    let inner = node(&app, "#inner");
    let drops = Rc::new(Cell::new(0));
    let subscription = app.on(inner, EventKind::Click, counted(&drops)).unwrap();
    assert_eq!(drops.get(), 0);
    app.off(subscription).unwrap();
    assert_eq!(drops.get(), 1);
    assert_eq!(app.off(subscription), Err(Status::NotFound));
    drop(app);
    assert_eq!(drops.get(), 1, "once");

    // By its node leaving the document: a new document, and a set_text that
    // replaces an element's children.
    let mut app = self::app(NESTED);
    let (inner, outer) = (node(&app, "#inner"), node(&app, "#outer"));
    let (replaced, reloaded) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    app.on(inner, EventKind::Click, counted(&replaced)).unwrap();
    app.on(outer, EventKind::Click, counted(&reloaded)).unwrap();
    app.set_text(outer, "metin").unwrap();
    assert_eq!((replaced.get(), reloaded.get()), (1, 0));
    app.load_html(NESTED);
    assert_eq!((replaced.get(), reloaded.get()), (1, 1));
    assert_eq!(
        app.on(inner, EventKind::Click, |_, _| {}),
        Err(Status::StaleNode)
    );

    // With the app.
    let mut app = self::app(NESTED);
    let inner = node(&app, "#inner");
    let drops = Rc::new(Cell::new(0));
    app.on(inner, EventKind::Click, counted(&drops)).unwrap();
    app.on_capture(inner, EventKind::Click, counted(&drops))
        .unwrap();
    drop(app);
    assert_eq!(drops.get(), 2);
}

#[test]
fn a_callback_that_ends_its_own_subscription_is_dropped_after_it_returns() {
    let mut app = app(NESTED);
    let inner = node(&app, "#inner");
    let drops = Rc::new(Cell::new(0));
    let me: Rc<Cell<Option<erk::Subscription>>> = Rc::default();
    let (guard, seen, mine) = (Drops(drops.clone()), drops.clone(), me.clone());
    let subscription = app
        .on(inner, EventKind::Click, move |cx, _| {
            let _ = &guard;
            cx.off(mine.get().unwrap()).unwrap();
            assert_eq!(seen.get(), 0, "not while it runs");
        })
        .unwrap();
    me.set(Some(subscription));
    app.click(10.0, 10.0);
    assert_eq!(drops.get(), 1);
    // Ended: the next click calls nothing.
    app.click(10.0, 10.0);
}

#[test]
fn a_subscription_ended_by_an_earlier_callback_is_not_called() {
    let mut app = app(NESTED);
    let inner = node(&app, "#inner");
    let later: Rc<Cell<Option<erk::Subscription>>> = Rc::default();
    let called = Rc::new(Cell::new(false));
    let ends = later.clone();
    app.on(inner, EventKind::Click, move |cx, _| {
        cx.off(ends.get().unwrap()).unwrap();
    })
    .unwrap();
    let flag = called.clone();
    later.set(Some(
        app.on(inner, EventKind::Click, move |_, _| flag.set(true))
            .unwrap(),
    ));
    app.click(10.0, 10.0);
    assert!(!called.get());
}

#[test]
fn callbacks_change_the_document_and_the_next_tick_shows_it() {
    let mut app = app(NESTED);
    let inner = node(&app, "#inner");
    app.on(inner, EventKind::Click, |cx, event| {
        assert_eq!(event.phase, Phase::Target);
        let outer = cx.query(None, "#outer").unwrap().unwrap();
        cx.set_text(outer, "tıklandı").unwrap();
    })
    .unwrap();
    let before = app.frame().unwrap().rgba().to_vec();
    app.click(10.0, 10.0);
    let outer = node(&app, "#outer");
    assert_eq!(app.text(outer).unwrap(), "tıklandı");
    app.tick(1);
    assert_ne!(app.frame().unwrap().rgba(), before.as_slice());
}

#[test]
fn a_panic_in_a_callback_reaches_the_caller_and_the_app_goes_on() {
    let mut app = app(NESTED);
    let inner = node(&app, "#inner");
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    app.on(inner, EventKind::Click, move |_, _| {
        counter.set(counter.get() + 1);
        if counter.get() == 1 {
            panic!("the host's bug");
        }
    })
    .unwrap();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.click(10.0, 10.0);
    }));
    let message = caught.expect_err("the panic comes out of the input call");
    assert_eq!(message.downcast_ref::<&str>(), Some(&"the host's bug"));
    // The subscription is back in place.
    app.click(10.0, 10.0);
    assert_eq!(calls.get(), 2);
}

#[test]
fn posted_work_runs_on_the_ui_thread_before_the_next_frame() {
    let mut app = app(NESTED);
    let outer = node(&app, "#outer");
    let ui = std::thread::current().id();
    let handle = app.handle();
    std::thread::spawn(move || {
        handle
            .post(move |cx| {
                assert_eq!(std::thread::current().id(), ui);
                cx.set_text(outer, "arka plandan").unwrap();
            })
            .unwrap();
    })
    .join()
    .unwrap();
    assert_ne!(
        app.text(outer).unwrap(),
        "arka plandan",
        "not before the tick"
    );
    app.tick(1);
    assert_eq!(app.text(outer).unwrap(), "arka plandan");
}

#[test]
fn posting_to_a_gone_app_drops_the_work_unrun() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Counts its drops, from any thread.
    struct Dropped(Arc<AtomicU32>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let app = app(NESTED);
    let handle = app.handle();
    drop(app);
    let (drops, ran) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
    let (guard, runs) = (Dropped(drops.clone()), ran.clone());
    let posted = handle.post(move |_| {
        let _ = &guard;
        runs.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(posted, Err(Status::NotFound));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(ran.load(Ordering::SeqCst), 0);
}

/// A 2 × 2 opaque red PNG.
fn red_png() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, 2, 2);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255].repeat(4))
            .unwrap();
    }
    out
}

const IMAGE: &str = r#"<body style="margin: 0; background: #123456"><img src="kirmizi.png" style="display: block; width: 20px; height: 20px">"#;

fn pixel(app: &App, x: usize, y: usize) -> [u8; 3] {
    let frame = app.frame().unwrap();
    let at = (y * usize::from(frame.width()) + x) * 4;
    let rgba = frame.rgba();
    [rgba[at], rgba[at + 1], rgba[at + 2]]
}

#[test]
fn a_resource_answered_at_once_is_in_the_same_tick() {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let seen = asked.clone();
    app.set_resource_provider(move |_, request, responder| {
        seen.borrow_mut().push((request.url.clone(), request.kind));
        responder.respond("image/png", red_png());
    });
    app.load_html(IMAGE);
    app.tick(0);
    assert_eq!(
        *asked.borrow(),
        [("kirmizi.png".to_owned(), ResourceKind::Image)]
    );
    assert_eq!(pixel(&app, 10, 10), [255, 0, 0]);
    assert!(!app.frame().unwrap().resources_pending());
}

#[test]
fn a_resource_answered_from_another_thread_shows_on_a_later_tick() {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    let waiting = Rc::new(RefCell::new(Vec::new()));
    let queue = waiting.clone();
    app.set_resource_provider(move |_, _, responder| queue.borrow_mut().push(responder));
    app.load_html(IMAGE);
    app.tick(0);
    assert_eq!(pixel(&app, 10, 10), [0x12, 0x34, 0x56]);
    assert!(app.frame().unwrap().resources_pending());
    let responder = waiting.borrow_mut().pop().unwrap();
    std::thread::spawn(move || responder.respond("", red_png()))
        .join()
        .unwrap();
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [255, 0, 0]);
}

#[test]
fn without_a_provider_no_resource_loads() {
    let mut app = app(IMAGE);
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [0x12, 0x34, 0x56]);
    assert!(!app.frame().unwrap().resources_pending());
}

#[test]
fn a_response_of_the_wrong_kind_is_refused_and_logged() {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.set_resource_provider(|_, _, responder| {
        responder.respond("text/css", b"body { color: red }".to_vec());
    });
    let logged = Rc::new(RefCell::new(Vec::new()));
    let log = logged.clone();
    app.set_log(move |level, message| log.borrow_mut().push((level, message.to_owned())));
    app.load_html(IMAGE);
    app.tick(0);
    assert_eq!(pixel(&app, 10, 10), [0x12, 0x34, 0x56]);
    let logged = logged.borrow();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0].0, LogLevel::Warning);
    assert!(logged[0].1.contains("kirmizi.png"), "{}", logged[0].1);
}

#[test]
fn a_quieter_log_level_hides_warnings() {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        log_level: LogLevel::Error,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.set_resource_provider(|_, _, responder| responder.respond("text/css", Vec::new()));
    let logged = Rc::new(Cell::new(0));
    let log = logged.clone();
    app.set_log(move |_, _| log.set(log.get() + 1));
    app.load_html(IMAGE);
    app.tick(0);
    assert_eq!(logged.get(), 0);
}
