//! Transforms from the host's side (M4.4): the pointer finds what is drawn
//! where it is, through the transform; the box query keeps the layout box.

use std::cell::Cell;
use std::rc::Rc;

use erk::{
    App, Config, EventKind, Input, Modifiers, Node, PointerButton, PointerInput, PointerKind,
};

fn app(html: &str) -> App {
    let mut app = App::headless(Config {
        width: 200,
        height: 120,
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

/// A 40×20 box at (40, 40), turned a quarter about its centre (60, 50):
/// drawn 20 wide and 40 tall, from (50, 30) to (70, 70).
const TURNED: &str = r#"<body style="margin: 0"><div id="kutu" style="margin: 40px 0 0 40px; width: 40px; height: 20px; background: #f00; transform: rotate(90deg)"></div>"#;

#[test]
fn the_pointer_finds_a_turned_box_where_it_is_drawn() {
    let app = app(TURNED);
    let turned = node(&app, "#kutu");
    // Drawn here, though the layout box is not.
    assert_eq!(app.inspect_at(60.0, 65.0), Some(turned));
    assert_eq!(app.inspect_at(60.0, 35.0), Some(turned));
    // In the layout box, where nothing of it is drawn.
    assert_ne!(app.inspect_at(45.0, 45.0), Some(turned));
    assert_ne!(app.inspect_at(75.0, 55.0), Some(turned));
}

#[test]
fn a_click_on_a_turned_box_reaches_it() {
    let mut app = app(TURNED);
    let turned = node(&app, "#kutu");
    let clicks = Rc::new(Cell::new(0));
    let count = clicks.clone();
    app.on(turned, EventKind::Click, move |_, _| {
        count.set(count.get() + 1)
    })
    .unwrap();
    for (x, y) in [(60.0, 65.0), (45.0, 45.0)] {
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.input(Input::Pointer(PointerInput {
                kind,
                x,
                y,
                button: PointerButton::Primary,
                modifiers: Modifiers::default(),
            }));
        }
    }
    assert_eq!(clicks.get(), 1, "only the click where it is drawn");
}

#[test]
fn the_box_query_gives_the_layout_box_not_the_transformed_one() {
    let app = app(TURNED);
    let found = app.node_box(node(&app, "#kutu")).unwrap();
    assert_eq!(
        (found.x, found.y, found.width, found.height),
        (40.0, 40.0, 40.0, 20.0)
    );
}

#[test]
fn a_clip_inside_a_transform_cuts_hits_where_it_is_drawn() {
    // The child is 200px tall, clipped to its turned 40×20 parent.
    let app = app(
        r#"<body style="margin: 0"><div style="margin: 40px 0 0 40px; width: 40px; height: 20px; overflow: hidden; transform: rotate(90deg)"><div id="ic" style="height: 200px"></div></div>"#,
    );
    let inner = node(&app, "#ic");
    assert_eq!(app.inspect_at(60.0, 65.0), Some(inner));
    // Inside the child's own box and the unturned clip, outside the turned
    // clip.
    assert_ne!(app.inspect_at(45.0, 45.0), Some(inner));
}

#[test]
fn a_clip_outside_a_transform_cuts_hits_in_its_own_coordinates() {
    // A 40×20 box at (40, 5), turned a quarter about (60, 15): drawn from
    // (50, -5) to (70, 35), inside a 30px tall clip that does not turn.
    let app = app(
        r#"<body style="margin: 0"><div style="overflow: hidden; width: 100px; height: 30px"><div id="kutu" style="margin: 5px 0 0 40px; width: 40px; height: 20px; transform: rotate(90deg)"></div></div>"#,
    );
    let turned = node(&app, "#kutu");
    assert_eq!(app.inspect_at(60.0, 20.0), Some(turned));
    // Drawn there, inside the clip, above the layout box.
    assert_eq!(app.inspect_at(60.0, 2.0), Some(turned));
    // In the layout box and the clip, where nothing of it is drawn.
    assert_ne!(app.inspect_at(45.0, 15.0), Some(turned));
    // Drawn there, but below the clip.
    assert_ne!(app.inspect_at(60.0, 33.0), Some(turned));
}
