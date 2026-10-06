//! The Rust API against p1-contract: ids (§2), status codes (§8, §10), the
//! windowless app (M3 plan, decision 2).

use erk::{App, Config, Input, Modifiers, Node, PointerButton, PointerInput, PointerKind, Status};

const PAGE: &str = r#"<body style="margin: 0; background: #123456">
    <p id="a">bir</p><p id="b">iki</p>
    <div id="hover" style="position: absolute; left: 0; top: 40px; width: 100px; height: 20px"></div>
    <style>#hover:hover { background: #fedcba }</style>"#;

fn app() -> App {
    let mut app = App::headless(Config {
        width: 100,
        height: 60,
        ..Config::default()
    })
    .unwrap();
    app.load_html(PAGE);
    app
}

fn node(app: &App, selector: &str) -> Node {
    app.query(None, selector).unwrap().unwrap()
}

fn pixel(app: &App, x: usize, y: usize) -> [u8; 3] {
    let frame = app.frame().expect("a frame was painted");
    let at = (y * usize::from(frame.width()) + x) * 4;
    let rgba = frame.rgba();
    [rgba[at], rgba[at + 1], rgba[at + 2]]
}

#[test]
fn another_apps_ids_name_no_node_here() {
    // Two apps with the same document: the same nodes, made in the same
    // order, have the same engine ids. Mixed with each app's key, an id of
    // one is stale in the other.
    let (one, two) = (app(), app());
    let (a1, a2) = (node(&one, "#a"), node(&two, "#a"));
    assert_ne!(a1, a2);
    assert_eq!(two.text(a1), Err(Status::StaleNode));
    assert_eq!(one.text(a2), Err(Status::StaleNode));
    assert_eq!(one.text(a1).unwrap(), "bir");
    assert_eq!(two.query(Some(a1), "p"), Err(Status::StaleNode));
}

#[test]
fn a_destroyed_apps_ids_name_no_node_in_a_new_one() {
    let old = app();
    let ids: Vec<Node> = ["#a", "#b", "#hover", "body", "html"]
        .iter()
        .map(|selector| node(&old, selector))
        .collect();
    drop(old);
    let mut new = app();
    for id in ids {
        assert_eq!(new.text(id), Err(Status::StaleNode), "{id:?}");
        assert_eq!(new.set_text(id, "x"), Err(Status::StaleNode), "{id:?}");
    }
}

#[test]
fn a_new_documents_ids_replace_the_old_ones() {
    let mut app = app();
    let a = node(&app, "#a");
    let root = app.root();
    app.load_html(PAGE);
    assert_eq!(app.text(a), Err(Status::StaleNode));
    assert_ne!(node(&app, "#a"), a);
    // The document node stays: the arena is the same (p1-contract §2).
    assert_eq!(app.root(), root);
}

#[test]
fn ids_round_trip_through_their_number() {
    let app = app();
    let a = node(&app, "#a");
    assert_ne!(a.to_raw(), 0);
    assert_eq!(Node::from_raw(a.to_raw()), Some(a));
    assert_eq!(Node::from_raw(0), None);
}

#[test]
fn status_codes_are_the_headers_numbers() {
    let codes = [
        (Status::InvalidArgument, 1),
        (Status::StaleNode, 2),
        (Status::WrongThread, 3),
        (Status::BufferTooSmall, 4),
        (Status::NotFound, 5),
        (Status::Reentrant, 6),
        (Status::Panic, 7),
        (Status::Poisoned, 8),
    ];
    for (status, code) in codes {
        assert_eq!(status as i32, code, "{status:?}");
    }
}

#[test]
fn a_selector_that_does_not_parse_is_an_invalid_argument() {
    let app = app();
    assert_eq!(app.query(None, "p >"), Err(Status::InvalidArgument));
    assert_eq!(app.query(None, "#yok"), Ok(None));
}

#[test]
fn a_viewport_out_of_range_is_refused() {
    for config in [
        Config {
            width: 0,
            ..Config::default()
        },
        Config {
            height: 70_000,
            ..Config::default()
        },
        Config {
            width: 40_000,
            scale: 2.0,
            ..Config::default()
        },
        Config {
            scale: f32::NAN,
            ..Config::default()
        },
        Config {
            scale: 0.0,
            ..Config::default()
        },
    ] {
        assert!(
            matches!(App::headless(config.clone()), Err(Status::InvalidArgument)),
            "{config:?}"
        );
    }
}

#[test]
fn a_tick_paints_what_changed() {
    let mut app = app();
    assert!(app.frame().is_none());
    app.tick(1);
    assert_eq!(pixel(&app, 50, 50), [0x12, 0x34, 0x56]);
    // Input on the UI thread restyles; the next tick paints it.
    app.input(Input::Pointer(PointerInput {
        kind: PointerKind::Move,
        x: 50.0,
        y: 50.0,
        button: PointerButton::None,
        modifiers: Modifiers::default(),
    }));
    app.tick(2);
    assert_eq!(pixel(&app, 50, 50), [0xfe, 0xdc, 0xba]);
}

#[test]
fn a_tick_with_nothing_changed_keeps_the_frame() {
    let mut app = app();
    app.tick(1);
    let first = app.frame().unwrap().rgba().to_vec();
    app.tick(2);
    assert_eq!(app.frame().unwrap().rgba(), first.as_slice());
    let a = node(&app, "#a");
    app.set_text(a, "değişti").unwrap();
    assert_eq!(app.text(a).unwrap(), "değişti");
    app.tick(3);
    assert_ne!(app.frame().unwrap().rgba(), first.as_slice());
}
