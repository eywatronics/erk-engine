//! Form controls' values and checkedness, read and set by the host
//! (M5.7, decision 8 of the M5 plan).

use std::cell::Cell;
use std::rc::Rc;

use erk::{App, Config, EventKind, Input, Key, KeyInput, KeyState, Modifiers, Node, Status};

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

#[test]
fn the_host_reads_and_sets_what_controls_hold() {
    let mut app = app("<input id=t value=eski><textarea id=a>bir</textarea>\
         <select id=s><option>a<option value=b>B</select>\
         <input type=checkbox id=k><p id=p>metin</p>");
    let (t, a, s, k, p) = (
        node(&app, "#t"),
        node(&app, "#a"),
        node(&app, "#s"),
        node(&app, "#k"),
        node(&app, "#p"),
    );
    assert_eq!(app.value(t), Ok(Some("eski".to_owned())));
    assert_eq!(app.value(a), Ok(Some("bir".to_owned())));
    assert_eq!(app.value(s), Ok(Some("a".to_owned())));
    app.set_value(t, "yeni").unwrap();
    app.set_value(a, "iki").unwrap();
    app.set_value(s, "b").unwrap();
    assert_eq!(app.value(t), Ok(Some("yeni".to_owned())));
    assert_eq!(app.value(a), Ok(Some("iki".to_owned())));
    assert_eq!(app.value(s), Ok(Some("b".to_owned())));
    // The attribute is what the markup said.
    assert_eq!(app.attr(t, "value"), Ok(Some("eski".to_owned())));
    assert_eq!(app.checked(k), Ok(Some(false)));
    app.set_checked(k, true).unwrap();
    assert_eq!(app.checked(k), Ok(Some(true)));
    // An element that has neither.
    assert_eq!(app.value(p), Ok(None));
    assert_eq!(app.checked(t), Ok(None));
    assert_eq!(app.set_value(p, "x"), Err(Status::InvalidArgument));
    assert_eq!(app.set_checked(t, true), Err(Status::InvalidArgument));
    // A removed control is stale.
    app.remove(t).unwrap();
    assert_eq!(app.value(t), Err(Status::StaleNode));
    assert_eq!(app.set_value(t, "x"), Err(Status::StaleNode));
}

#[test]
fn a_value_the_host_sets_raises_no_event() {
    let mut app = app("<input id=t><input type=checkbox id=k>");
    let (t, k) = (node(&app, "#t"), node(&app, "#k"));
    let raised = Rc::new(Cell::new(0));
    for target in [t, k] {
        for kind in [EventKind::Input, EventKind::Change] {
            let raised = raised.clone();
            app.on(target, kind, move |_, _| raised.set(raised.get() + 1))
                .unwrap();
        }
    }
    app.set_value(t, "yeni").unwrap();
    app.set_checked(k, true).unwrap();
    app.tick(1);
    assert_eq!(raised.get(), 0);
}

/// Press `key`, down then up, as the window would deliver it.
fn press(app: &mut App, key: Key) {
    for state in [KeyState::Down, KeyState::Up] {
        app.input(Input::Key(KeyInput {
            key: key.clone(),
            state,
            modifiers: Modifiers::default(),
        }));
    }
}

#[test]
fn what_the_user_types_is_an_input_event_with_the_new_value() {
    let mut app = app("<input id=t value=a><button id=b>b</button>");
    let t = node(&app, "#t");
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
    for kind in [EventKind::Input, EventKind::Change] {
        let seen = seen.clone();
        app.on(t, kind, move |cx, event| {
            let value = cx.value(event.target).unwrap().unwrap();
            seen.borrow_mut().push(format!("{:?} {value}", event.kind));
        })
        .unwrap();
    }
    press(&mut app, Key::Tab);
    press(&mut app, Key::Character("b".to_owned()));
    press(&mut app, Key::Backspace);
    press(&mut app, Key::Character("ş".to_owned()));
    press(&mut app, Key::Tab);
    assert_eq!(
        *seen.borrow(),
        ["Input ab", "Input a", "Input aş", "Change aş"]
    );
}

#[test]
fn a_focused_field_asks_for_the_next_tick_at_its_caret_blink() {
    let mut app = app("<input id=t value=a><button>b</button>");
    assert_eq!(app.next_tick(), None);
    app.tick(1_000);
    press(&mut app, Key::Tab);
    // Ticked at 1000 ns, the caret hides half a second later.
    assert_eq!(app.next_tick(), Some(1_000 + 500_000_000));
    press(&mut app, Key::Tab);
    assert_eq!(app.next_tick(), None);
}

/// Press Control with `letter`.
fn command(app: &mut App, letter: &str) {
    for state in [KeyState::Down, KeyState::Up] {
        app.input(Input::Key(KeyInput {
            key: Key::Character(letter.to_owned()),
            state,
            modifiers: Modifiers {
                control: true,
                ..Modifiers::default()
            },
        }));
    }
}

#[test]
fn what_is_copied_in_one_field_pastes_in_another() {
    let mut app = app("<input id=a value=bir><input id=b value=iki>");
    let b = node(&app, "#b");
    let inputs = Rc::new(Cell::new(0));
    {
        let inputs = inputs.clone();
        app.on(b, EventKind::Input, move |_, _| {
            inputs.set(inputs.get() + 1)
        })
        .unwrap();
    }
    press(&mut app, Key::Tab);
    command(&mut app, "a");
    command(&mut app, "c");
    press(&mut app, Key::Tab);
    command(&mut app, "v");
    // The app's own clipboard: pasted at the end of "iki".
    assert_eq!(app.value(b), Ok(Some("ikibir".to_owned())));
    assert_eq!(inputs.get(), 1);
}

#[test]
fn the_hosts_clipboard_is_written_and_read() {
    use std::cell::RefCell;

    struct Shared(Rc<RefCell<Vec<String>>>);
    impl erk::Clipboard for Shared {
        fn read(&mut self) -> Option<String> {
            Some("dış".to_owned())
        }
        fn write(&mut self, text: &str) {
            self.0.borrow_mut().push(text.to_owned());
        }
    }

    let mut app = app("<input id=a value=metin>");
    let a = node(&app, "#a");
    let written = Rc::new(RefCell::new(Vec::new()));
    app.set_clipboard(Shared(written.clone()));
    press(&mut app, Key::Tab);
    command(&mut app, "a");
    command(&mut app, "x");
    assert_eq!(*written.borrow(), ["metin"]);
    assert_eq!(app.value(a), Ok(Some(String::new())));
    command(&mut app, "v");
    assert_eq!(app.value(a), Ok(Some("dış".to_owned())));
}

#[test]
fn a_prevented_key_types_nothing() {
    let mut app = app("<input id=t value=a>");
    let t = node(&app, "#t");
    let html = node(&app, "html");
    // The host takes the letter x for itself.
    app.on_capture(html, EventKind::KeyDown, |cx, event| {
        if event.key == Some(Key::Character("x".to_owned())) {
            cx.prevent_default();
        }
    })
    .unwrap();
    press(&mut app, Key::Tab);
    press(&mut app, Key::Character("x".to_owned()));
    press(&mut app, Key::Character("y".to_owned()));
    assert_eq!(app.value(t), Ok(Some("ay".to_owned())));
}
