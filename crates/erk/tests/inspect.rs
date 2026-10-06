//! The inspection queries and frame timings of p1-contract §8.1.

use erk::{App, BoxModel, Config, NodeKind, Status};

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

fn node(app: &App, selector: &str) -> erk::Node {
    app.query(None, selector).unwrap().unwrap()
}

const TREE: &str = r#"<!DOCTYPE html><body><div id="a" class="kutu" data-x="1">bir<!-- not --><b>iki</b></div></body>"#;

#[test]
fn the_tree_reads_as_the_document_is() {
    let app = app(TREE);
    let root = app.root();
    assert_eq!(app.kind(root).unwrap(), NodeKind::Document);
    assert_eq!(app.parent(root).unwrap(), None);
    let html = node(&app, "html");
    assert_eq!(app.parent(html).unwrap(), Some(root));
    let a = node(&app, "#a");
    assert_eq!(app.tag(a).unwrap().as_deref(), Some("div"));
    assert_eq!(
        app.attributes(a).unwrap(),
        [
            ("id".to_owned(), "a".to_owned()),
            ("class".to_owned(), "kutu".to_owned()),
            ("data-x".to_owned(), "1".to_owned()),
        ]
    );
    assert_eq!(app.child_count(a).unwrap(), 3);
    let kinds: Vec<NodeKind> = (0..3)
        .map(|i| app.kind(app.child_at(a, i).unwrap().unwrap()).unwrap())
        .collect();
    assert_eq!(
        kinds,
        [NodeKind::Text, NodeKind::Comment, NodeKind::Element]
    );
    assert_eq!(app.child_at(a, 3).unwrap(), None);
    let text = app.child_at(a, 0).unwrap().unwrap();
    assert_eq!(app.tag(text).unwrap(), None);
    assert!(app.attributes(text).unwrap().is_empty());
    assert_eq!(app.parent(text).unwrap(), Some(a));
}

#[test]
fn a_stale_node_is_an_error_in_every_query() {
    let mut app = app(TREE);
    let a = node(&app, "#a");
    app.load_html(TREE);
    assert_eq!(app.parent(a), Err(Status::StaleNode));
    assert_eq!(app.child_at(a, 0), Err(Status::StaleNode));
    assert_eq!(app.child_count(a), Err(Status::StaleNode));
    assert_eq!(app.kind(a), Err(Status::StaleNode));
    assert_eq!(app.tag(a), Err(Status::StaleNode));
    assert_eq!(app.attributes(a), Err(Status::StaleNode));
    assert_eq!(app.node_box(a), Err(Status::StaleNode));
    assert_eq!(app.computed_style(a), Err(Status::StaleNode));
    assert_eq!(app.highlight(Some(a)), Err(Status::StaleNode));
}

const BOXES: &str = r#"<body style="margin: 0">
  <div id="box" style="margin: 10px 20px 30px 40px; border: 2px solid; border-left-width: 4px; padding: 5px 6px 7px 8px; width: 50px; height: 20px"></div>
  <div id="scroller" style="height: 40px; overflow: auto">
    <div style="height: 30px"></div><div id="inside" style="position: relative; left: 3px; top: 4px; height: 30px">x</div>
  </div>
  <span id="inline">satır içi</span><p id="hidden" style="display: none">yok</p>"#;

#[test]
fn a_box_is_its_border_box_and_its_sides() {
    let app = app(BOXES);
    let found = app.node_box(node(&app, "#box")).unwrap();
    assert_eq!(
        found,
        BoxModel {
            x: 40.0,
            y: 10.0,
            // Content, padding and border.
            width: 50.0 + 8.0 + 6.0 + 4.0 + 2.0,
            height: 20.0 + 5.0 + 7.0 + 2.0 + 2.0,
            margin: [10.0, 20.0, 30.0, 40.0],
            border: [2.0, 2.0, 2.0, 4.0],
            padding: [5.0, 6.0, 7.0, 8.0],
        }
    );
}

#[test]
fn a_box_is_where_the_frame_painted_it() {
    let mut app = app(BOXES);
    let inside = node(&app, "#inside");
    let before = app.node_box(inside).unwrap();
    // The scroller starts below the first box and its bottom margin.
    let scroller = app.node_box(node(&app, "#scroller")).unwrap();
    assert_eq!(scroller.y, 10.0 + 36.0 + 30.0);
    // Relatively positioned: moved by its offsets.
    assert_eq!((before.x, before.y), (3.0, scroller.y + 30.0 + 4.0));
    // Scrolled: where the next frame paints it.
    app.input(erk::Input::Wheel {
        dx: 0.0,
        dy: 15.0,
        x: 10.0,
        y: scroller.y + 5.0,
    });
    app.tick(1);
    let after = app.node_box(inside).unwrap();
    assert_eq!(after.y, before.y - 15.0);
}

#[test]
fn a_node_without_a_box_has_none() {
    let fresh = App::headless(Config {
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    // No frame yet.
    assert_eq!(fresh.node_box(fresh.root()), Err(Status::NotFound));
    let app = app(BOXES);
    for selector in ["#inline", "#hidden"] {
        assert_eq!(
            app.node_box(node(&app, selector)),
            Err(Status::NotFound),
            "{selector}"
        );
    }
    let text = app.child_at(node(&app, "#inline"), 0).unwrap().unwrap();
    assert_eq!(app.node_box(text), Err(Status::NotFound));
    // Text in a flex container is laid out as an anonymous item; the box
    // is not the text node's.
    let app = self::app(r#"<div id="flex" style="display: flex">metin<b>kalın</b></div>"#);
    let text = app.child_at(node(&app, "#flex"), 0).unwrap().unwrap();
    assert_eq!(app.node_box(text), Err(Status::NotFound));
}

#[test]
fn the_computed_style_reads_as_get_computed_style() {
    let app = app(
        r#"<div id="a" style="color: red; display: flex; padding-left: 1em; font-size: 20px">x</div>"#,
    );
    let style = app.computed_style(node(&app, "#a")).unwrap();
    for line in [
        "color: rgb(255, 0, 0);",
        "display: flex;",
        "padding-left: 20px;",
        "font-size: 20px;",
        "position: static;",
    ] {
        assert!(style.lines().any(|l| l == line), "{line} in\n{style}");
    }
    // One line per property, in the order of the names.
    let names: Vec<&str> = style
        .lines()
        .map(|line| line.split(':').next().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted);
    assert!(names.len() > 50, "{}", names.len());
    // A text node has no style of its own.
    let text = app.child_at(node(&app, "#a"), 0).unwrap().unwrap();
    assert_eq!(app.computed_style(text), Err(Status::NotFound));
}

#[test]
fn inspect_at_finds_what_a_click_would() {
    let app = app(BOXES);
    let found = app.inspect_at(45.0, 15.0).unwrap();
    assert_eq!(found, node(&app, "#box"));
    assert_eq!(app.inspect_at(-5.0, 15.0), None);
}

#[test]
fn the_highlight_changes_the_frame_and_nothing_in_the_document() {
    // p1-contract §11: with and without the highlight, the document and its
    // computed styles are the same; only the frame differs.
    let mut app = app(BOXES);
    let target = node(&app, "#box");
    let snapshot = |app: &App| {
        (
            app.computed_style(target).unwrap(),
            app.text(app.root()).unwrap(),
            app.child_count(node(app, "body")).unwrap(),
            app.node_box(target).unwrap(),
        )
    };
    let (before, plain) = (snapshot(&app), app.frame().unwrap().rgba().to_vec());
    app.highlight(Some(target)).unwrap();
    app.tick(1);
    assert_eq!(snapshot(&app), before);
    assert_ne!(app.frame().unwrap().rgba(), plain.as_slice());
    app.highlight(None).unwrap();
    app.tick(2);
    assert_eq!(snapshot(&app), before);
    assert_eq!(app.frame().unwrap().rgba(), plain.as_slice());
}

#[test]
fn each_painted_frame_has_its_stage_timings() {
    let mut app = App::headless(Config {
        system_fonts: false,
        ..Config::default()
    })
    .unwrap();
    assert_eq!(app.last_frame_timings(), None);
    app.load_html(BOXES);
    let start = std::time::Instant::now();
    app.tick(0);
    let wall = start.elapsed().as_nanos() as u64;
    let first = app.last_frame_timings().expect("a frame was painted");
    assert_eq!(first.frame, 1);
    for (stage, ns) in [
        ("style", first.style_ns),
        ("layout", first.layout_ns),
        ("display list", first.display_list_ns),
        ("raster", first.raster_ns),
    ] {
        assert!(ns > 0, "{stage} took no time: {first:?}");
    }
    let total = first.style_ns + first.layout_ns + first.display_list_ns + first.raster_ns;
    assert!(total <= wall, "{total} ns of stages in a {wall} ns tick");
    // Nothing changed: no frame, the same timings.
    app.tick(1);
    assert_eq!(app.last_frame_timings(), Some(first));
    let text = node(&app, "#inline");
    app.set_text(text, "değişti").unwrap();
    app.tick(2);
    assert_eq!(app.last_frame_timings().unwrap().frame, 2);
}
