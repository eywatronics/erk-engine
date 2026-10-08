//! TodoMVC end to end (M4.5, M4's acceptance): the example's own code,
//! driven as a user would, without a window. Typing builds the new task
//! from key events, Enter adds it, a click ticks or removes it, the filters
//! hide rows, the counter follows; one frame on the way is a golden image.
//!
//! Bless a changed golden image with `ERK_BLESS=1` (exactly `1`) and commit
//! it with the change that caused it.

#[path = "../examples/todomvc/todos.rs"]
mod todos;

use std::path::PathBuf;

use erk::{App, Config, Key, Node};
use todos::{Filter, key_down, titles};

fn app() -> (App, todos::Shared) {
    let mut app = App::headless(Config {
        width: 520,
        height: 560,
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    let todos = todos::mount(&mut app).unwrap();
    app.tick(0);
    (app, todos)
}

fn node(app: &App, selector: &str) -> Node {
    app.query(None, selector).unwrap().unwrap()
}

/// Click the middle of `node`'s box, and paint the change.
fn click(app: &mut App, node: Node) {
    let found = app.node_box(node).unwrap();
    app.click(found.x + found.width / 2.0, found.y + found.height / 2.0);
    app.tick(0);
}

/// Click the element `selector` finds, inside `scope` if given.
fn click_on(app: &mut App, scope: Option<Node>, selector: &str) {
    let target = app.query(scope, selector).unwrap().unwrap();
    click(app, target);
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let key = if c == ' ' {
            Key::Space
        } else {
            Key::Character(c.to_string())
        };
        app.input(key_down(key));
    }
    app.tick(0);
}

fn press(app: &mut App, key: Key) {
    app.input(key_down(key));
    app.tick(0);
}

fn add(app: &mut App, title: &str) {
    type_text(app, title);
    press(app, Key::Enter);
}

/// The titles of the rows shown: a hidden row has no box.
fn shown(app: &App) -> Vec<String> {
    app.query_all(None, ".todo")
        .unwrap()
        .into_iter()
        .filter(|row| app.node_box(*row).is_ok())
        .map(|row| {
            app.text(app.query(Some(row), ".label").unwrap().unwrap())
                .unwrap()
        })
        .collect()
}

fn row(app: &App, title: &str) -> Node {
    app.query_all(None, ".todo")
        .unwrap()
        .into_iter()
        .find(|row| {
            let label = app.query(Some(*row), ".label").unwrap().unwrap();
            app.text(label).unwrap() == title
        })
        .unwrap_or_else(|| panic!("no row {title:?}"))
}

fn counter(app: &App) -> String {
    app.text(node(app, ".todo-count")).unwrap()
}

/// `app`'s frame against the golden image `name`.
fn check_golden(app: &App, name: &str) {
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    let png = app
        .frame()
        .expect("a headless frame")
        .to_png()
        .expect("pixels");
    if std::env::var("ERK_BLESS").is_ok_and(|value| value == "1") {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &png).unwrap();
        return;
    }
    let expected = std::fs::read(&golden).unwrap_or_else(|_| {
        panic!(
            "no golden image at {}; run with ERK_BLESS=1",
            golden.display()
        )
    });
    if expected != png {
        let actual = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/golden-diff/{name}.actual.png"));
        std::fs::create_dir_all(actual.parent().unwrap()).unwrap();
        std::fs::write(&actual, &png).unwrap();
        panic!(
            "{name} differs from its golden image; this frame is at {}",
            actual.display()
        );
    }
}

#[test]
fn todomvc_adds_ticks_filters_and_removes_tasks() {
    let (mut app, todos) = app();
    let field = node(&app, ".new-todo");
    // Nothing yet: no footer.
    assert!(app.node_box(node(&app, ".footer")).is_err());

    // Typing goes to the field once it has the focus.
    click(&mut app, field);
    add(&mut app, "Süt al");
    add(&mut app, "Ekmek");
    // White space alone is no task.
    type_text(&mut app, "   ");
    press(&mut app, Key::Enter);
    // Backspace takes back what was typed.
    type_text(&mut app, "Kitap okx");
    press(&mut app, Key::Backspace);
    add(&mut app, "u");
    assert_eq!(titles(&todos), ["Süt al", "Ekmek", "Kitap oku"]);
    assert_eq!(shown(&app), ["Süt al", "Ekmek", "Kitap oku"]);
    assert_eq!(counter(&app), "3 görev kaldı");
    // The field is empty again and says so.
    assert_eq!(app.text(field).unwrap(), "Ne yapılacak?");

    // Tick one off; the clear button appears.
    let ekmek = row(&app, "Ekmek");
    click_on(&mut app, Some(ekmek), ".toggle");
    assert!(app.has_class(ekmek, "completed").unwrap());
    assert_eq!(counter(&app), "2 görev kaldı");
    assert!(app.node_box(node(&app, ".clear-completed")).is_ok());

    // The click took the focus from the field: typing goes nowhere until
    // the field has it again.
    type_text(&mut app, "kayıp");
    assert_eq!(app.text(field).unwrap(), "Ne yapılacak?");
    click(&mut app, field);
    // A task half typed, one ticked off: the golden image.
    type_text(&mut app, "Yeni gö");
    assert_eq!(app.text(field).unwrap(), "Yeni gö");
    check_golden(&app, "todomvc");
    press(&mut app, Key::Escape);
    assert_eq!(app.text(field).unwrap(), "Ne yapılacak?");

    // The filters.
    click_on(&mut app, None, "#active");
    assert_eq!(todos.borrow().filter, Filter::Active);
    assert_eq!(shown(&app), ["Süt al", "Kitap oku"]);
    click_on(&mut app, None, "#completed");
    assert_eq!(shown(&app), ["Ekmek"]);
    assert!(app.has_class(node(&app, "#completed"), "selected").unwrap());
    assert!(!app.has_class(node(&app, "#all"), "selected").unwrap());
    click_on(&mut app, None, "#all");
    assert_eq!(shown(&app), ["Süt al", "Ekmek", "Kitap oku"]);

    // Remove one, then clear the completed one.
    let sut = row(&app, "Süt al");
    click_on(&mut app, Some(sut), ".destroy");
    assert_eq!(shown(&app), ["Ekmek", "Kitap oku"]);
    click_on(&mut app, None, ".clear-completed");
    assert_eq!(titles(&todos), ["Kitap oku"]);
    assert_eq!(shown(&app), ["Kitap oku"]);
    assert_eq!(counter(&app), "1 görev kaldı");
    assert!(app.node_box(node(&app, ".clear-completed")).is_err());

    // The last one, and the footer goes with it.
    let last = row(&app, "Kitap oku");
    click_on(&mut app, Some(last), ".destroy");
    assert!(titles(&todos).is_empty());
    assert!(app.node_box(node(&app, ".footer")).is_err());
}
