//! Building the document from the host (M4.0).

use std::cell::Cell;
use std::rc::Rc;

use erk::{App, Config, EventKind, Input, KeyInput, KeyState, Modifiers, Node, Status};

const PAGE: &str = r#"<body style="margin: 0; background: #ffffff">
  <style>
    .kutu { width: 40px; height: 20px; background: #0000ff }
    .kutu.kirmizi { background: #ff0000 }
    [data-gizli] { display: none }
  </style>
  <div id="liste"></div>"#;

fn app() -> App {
    let mut app = App::headless(Config {
        width: 100,
        height: 80,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    app.load_html(PAGE);
    app.tick(0);
    app
}

fn node(app: &App, selector: &str) -> Node {
    app.query(None, selector).unwrap().unwrap()
}

fn pixel(app: &App, x: usize, y: usize) -> [u8; 3] {
    let frame = app.frame().unwrap();
    let at = (y * usize::from(frame.width()) + x) * 4;
    let rgba = frame.rgba();
    [rgba[at], rgba[at + 1], rgba[at + 2]]
}

/// A new `.kutu` appended to the list.
fn box_in(app: &mut App, list: Node) -> Node {
    let made = app.create_element("div").unwrap();
    app.add_class(made, "kutu").unwrap();
    app.append(list, made).unwrap();
    made
}

#[test]
fn an_element_made_and_appended_is_painted_and_found() {
    let mut app = app();
    let list = node(&app, "#liste");
    assert_eq!(pixel(&app, 10, 10), [255, 255, 255]);
    let made = box_in(&mut app, list);
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [0, 0, 255]);
    assert_eq!(app.query(None, ".kutu").unwrap(), Some(made));
    assert_eq!(app.parent(made).unwrap(), Some(list));
    let found = app.node_box(made).unwrap();
    assert_eq!((found.width, found.height), (40.0, 20.0));
}

#[test]
fn a_detached_node_is_neither_painted_nor_found() {
    let mut app = app();
    let made = app.create_element("div").unwrap();
    app.add_class(made, "kutu").unwrap();
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [255, 255, 255]);
    assert_eq!(app.query(None, ".kutu").unwrap(), None);
    assert_eq!(app.parent(made).unwrap(), None);
    assert_eq!(app.node_box(made), Err(Status::NotFound));
    // Yet it is a node: it has its tag and its class.
    assert_eq!(app.tag(made).unwrap().as_deref(), Some("div"));
    assert!(app.has_class(made, "kutu").unwrap());
}

#[test]
fn attributes_and_classes_restyle_the_next_frame() {
    let mut app = app();
    let list = node(&app, "#liste");
    let made = box_in(&mut app, list);
    app.add_class(made, "kirmizi").unwrap();
    app.add_class(made, "kirmizi").unwrap();
    assert_eq!(
        app.attr(made, "class").unwrap().as_deref(),
        Some("kutu kirmizi")
    );
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [255, 0, 0]);
    app.remove_class(made, "kirmizi").unwrap();
    app.tick(2);
    assert_eq!(pixel(&app, 10, 10), [0, 0, 255]);
    app.set_attr(made, "data-gizli", "").unwrap();
    app.tick(3);
    assert_eq!(pixel(&app, 10, 10), [255, 255, 255]);
    assert_eq!(app.remove_attr(made, "data-gizli"), Ok(true));
    assert_eq!(app.remove_attr(made, "data-gizli"), Ok(false));
    app.set_attr(made, "style", "background: #00ff00").unwrap();
    app.tick(4);
    assert_eq!(pixel(&app, 10, 10), [0, 255, 0]);
    assert_eq!(app.add_class(made, "a b"), Err(Status::InvalidArgument));
    assert_eq!(app.add_class(made, ""), Err(Status::InvalidArgument));
}

#[test]
fn insert_before_places_and_moves() {
    let mut app = app();
    let list = node(&app, "#liste");
    let first = box_in(&mut app, list);
    let second = box_in(&mut app, list);
    let third = app.create_text("üçüncü");
    app.insert_before(list, third, Some(first)).unwrap();
    let order: Vec<Node> = (0..3)
        .map(|i| app.child_at(list, i).unwrap().unwrap())
        .collect();
    assert_eq!(order, [third, first, second]);
    // Moving a node takes it from where it was.
    app.insert_before(list, second, Some(third)).unwrap();
    assert_eq!(app.child_at(list, 0).unwrap(), Some(second));
    assert_eq!(app.child_count(list).unwrap(), 3);
    // Not into itself, not before a node of another parent.
    assert_eq!(app.append(first, first), Err(Status::InvalidArgument));
    let body = node(&app, "body");
    assert_eq!(
        app.insert_before(body, third, Some(first)),
        Err(Status::InvalidArgument)
    );
    let root = app.root();
    assert_eq!(app.append(list, root), Err(Status::InvalidArgument));
    assert_eq!(app.remove(root), Err(Status::InvalidArgument));
}

/// Counts its drops.
struct Drops(Rc<Cell<u32>>);

impl Drop for Drops {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn removing_a_node_ends_its_subtrees_subscriptions_and_ids() {
    let mut app = app();
    let list = node(&app, "#liste");
    let outer = box_in(&mut app, list);
    let inner = app.create_element("span").unwrap();
    app.append(outer, inner).unwrap();
    let drops = Rc::new(Cell::new(0));
    let guard = Drops(drops.clone());
    app.on(inner, EventKind::Click, move |_, _| {
        let _ = &guard;
    })
    .unwrap();
    app.remove(outer).unwrap();
    assert_eq!(drops.get(), 1);
    for gone in [outer, inner] {
        assert_eq!(app.text(gone), Err(Status::StaleNode));
        assert_eq!(app.append(list, gone), Err(Status::StaleNode));
        assert_eq!(app.remove(gone), Err(Status::StaleNode));
    }
    app.tick(1);
    assert_eq!(pixel(&app, 10, 10), [255, 255, 255]);
}

#[test]
fn a_subscription_on_a_detached_node_fires_once_it_is_in_the_page() {
    let mut app = app();
    let list = node(&app, "#liste");
    let made = app.create_element("div").unwrap();
    app.add_class(made, "kutu").unwrap();
    let clicks = Rc::new(Cell::new(0));
    let counted = clicks.clone();
    app.on(made, EventKind::Click, move |_, _| {
        counted.set(counted.get() + 1)
    })
    .unwrap();
    app.append(list, made).unwrap();
    app.tick(1);
    app.click(10.0, 10.0);
    assert_eq!(clicks.get(), 1);
}

#[test]
fn a_removed_focused_element_leaves_the_focus() {
    let mut app = app();
    let list = node(&app, "#liste");
    let made = box_in(&mut app, list);
    app.set_attr(made, "tabindex", "0").unwrap();
    app.tick(1);
    let tab = |app: &mut App| {
        app.input(Input::Key(KeyInput {
            key: erk::Key::Tab,
            state: KeyState::Down,
            modifiers: Modifiers::default(),
        }));
    };
    let focused = Rc::new(Cell::new(0));
    let seen = focused.clone();
    app.on(made, EventKind::Focus, move |_, _| seen.set(seen.get() + 1))
        .unwrap();
    tab(&mut app);
    assert_eq!(focused.get(), 1);
    app.remove(made).unwrap();
    // A second focusable element: Tab reaches it with nothing stale in the
    // way.
    let other = box_in(&mut app, list);
    app.set_attr(other, "tabindex", "0").unwrap();
    app.tick(2);
    let reached = Rc::new(Cell::new(0));
    let seen = reached.clone();
    app.on(other, EventKind::Focus, move |_, _| {
        seen.set(seen.get() + 1)
    })
    .unwrap();
    tab(&mut app);
    assert_eq!(reached.get(), 1);
}
