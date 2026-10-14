//! Form controls' values and checkedness, read and set by the host
//! (M5.7, decision 8 of the M5 plan).

use std::cell::Cell;
use std::rc::Rc;

use erk::{App, Config, EventKind, Node, Status};

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
