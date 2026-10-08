//! Batches of mutations, query_all and keyboard events (M4.1).

use std::cell::RefCell;
use std::rc::Rc;

use erk::{
    App, BatchError, Config, EventKind, Input, Key, KeyInput, KeyState, Modifiers, Mutation, Node,
    Phase, Ref, Status,
};

fn app(html: &str) -> App {
    let mut app = App::headless(Config {
        width: 120,
        height: 80,
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
fn a_batch_builds_a_subtree_naming_what_it_made() {
    let mut app = app(r#"<ul id="liste"></ul>"#);
    let list = node(&app, "#liste");
    let made = app
        .apply(&[
            Mutation::CreateElement("li".to_owned()),
            Mutation::CreateText("bir".to_owned()),
            Mutation::Append {
                parent: Ref::New(0),
                child: Ref::New(1),
            },
            Mutation::AddClass(Ref::New(0), "yeni".to_owned()),
            Mutation::SetAttr(Ref::New(0), "data-n".to_owned(), "1".to_owned()),
            Mutation::Append {
                parent: Ref::Node(list),
                child: Ref::New(0),
            },
            Mutation::CreateElement("li".to_owned()),
            Mutation::SetText(Ref::New(6), "sıfır".to_owned()),
            Mutation::InsertBefore {
                parent: Ref::Node(list),
                child: Ref::New(6),
                before: Some(Ref::New(0)),
            },
        ])
        .unwrap();
    assert_eq!(made.len(), 9);
    let (first, second) = (made[0].unwrap(), made[6].unwrap());
    assert!(made[2].is_none() && made[5].is_none());
    assert_eq!(app.query_all(Some(list), "li").unwrap(), [second, first]);
    assert_eq!(app.text(list).unwrap(), "sıfırbir");
    assert!(app.has_class(first, "yeni").unwrap());
    assert_eq!(app.attr(first, "data-n").unwrap().as_deref(), Some("1"));
}

#[test]
fn the_first_failing_mutation_stops_the_batch_and_says_which() {
    let mut app = app(r#"<ul id="liste"></ul>"#);
    let list = node(&app, "#liste");
    let failed = app.apply(&[
        Mutation::CreateElement("li".to_owned()),
        Mutation::Append {
            parent: Ref::Node(list),
            child: Ref::New(0),
        },
        // Into itself.
        Mutation::Append {
            parent: Ref::New(0),
            child: Ref::New(0),
        },
        Mutation::CreateElement("p".to_owned()),
    ]);
    assert_eq!(
        failed,
        Err(BatchError {
            index: 2,
            status: Status::InvalidArgument
        })
    );
    // The ones before it stay; the one after never ran.
    assert_eq!(app.query_all(None, "li").unwrap().len(), 1);
    assert_eq!(app.query_all(None, "p").unwrap().len(), 0);
}

#[test]
fn a_reference_must_name_an_earlier_creation() {
    let mut app = app(r#"<ul id="liste"></ul>"#);
    let list = node(&app, "#liste");
    for (batch, index) in [
        // Forward.
        (
            vec![
                Mutation::Append {
                    parent: Ref::Node(list),
                    child: Ref::New(1),
                },
                Mutation::CreateElement("li".to_owned()),
            ],
            0,
        ),
        // A mutation that created nothing.
        (
            vec![
                Mutation::CreateElement("li".to_owned()),
                Mutation::Append {
                    parent: Ref::Node(list),
                    child: Ref::New(0),
                },
                Mutation::Remove(Ref::New(1)),
            ],
            2,
        ),
    ] {
        assert_eq!(
            app.apply(&batch),
            Err(BatchError {
                index,
                status: Status::InvalidArgument
            })
        );
    }
}

#[test]
fn removing_a_missing_attribute_in_a_batch_is_no_error() {
    let mut app = app(r#"<p id="p">x</p>"#);
    let p = node(&app, "#p");
    assert!(
        app.apply(&[Mutation::RemoveAttr(Ref::Node(p), "yok".to_owned())])
            .is_ok()
    );
}

#[test]
fn query_all_finds_every_match_in_document_order() {
    let app =
        app(r#"<div id="a"><p class="x">1</p><span class="x">2</span></div><p class="x">3</p>"#);
    let all = app.query_all(None, ".x").unwrap();
    let texts: Vec<String> = all.iter().map(|n| app.text(*n).unwrap()).collect();
    assert_eq!(texts, ["1", "2", "3"]);
    let inside = app.query_all(Some(node(&app, "#a")), ".x").unwrap();
    assert_eq!(inside, all[..2]);
    assert_eq!(app.query_all(None, "p >"), Err(Status::InvalidArgument));
    assert!(app.query_all(None, ".yok").unwrap().is_empty());
}

#[test]
fn a_query_inside_a_detached_element_finds_nothing() {
    // Found by the mutation fuzz (M4.2): the scope had no place in the
    // styled tree, and matching it panicked.
    let mut app = app(r#"<p class="x">1</p>"#);
    let detached = app.create_element("div").unwrap();
    let inside = app.create_element("p").unwrap();
    app.append(detached, inside).unwrap();
    app.add_class(inside, "x").unwrap();
    assert_eq!(app.query_all(Some(detached), "*"), Ok(vec![]));
    assert_eq!(app.query(Some(detached), ".x"), Ok(None));
    assert_eq!(
        app.query_all(Some(detached), "p >"),
        Err(Status::InvalidArgument)
    );
}

/// What a key subscription saw: kind, phase, target and key.
type Seen = (EventKind, Phase, Node, Option<Key>);

#[test]
fn keys_reach_the_focused_element_and_bubble() {
    let mut app = app(r#"<body><div id="alan" tabindex="0">alan</div>"#);
    let (body, field) = (node(&app, "body"), node(&app, "#alan"));
    let seen: Rc<RefCell<Vec<Seen>>> = Rc::default();
    let record = seen.clone();
    app.on(body, EventKind::KeyDown, move |_, event| {
        record
            .borrow_mut()
            .push((event.kind, event.phase, event.target, event.key.clone()));
    })
    .unwrap();
    let key = |key: Key| {
        Input::Key(KeyInput {
            key,
            state: KeyState::Down,
            modifiers: Modifiers::default(),
        })
    };
    // Nothing focused: the body gets it.
    app.input(key(Key::Character("a".to_owned())));
    // Tab focuses the field (the key itself still goes to the body first).
    app.input(key(Key::Tab));
    // Now the field gets it, and it bubbles to the body.
    app.input(key(Key::Character("ş".to_owned())));
    assert_eq!(
        *seen.borrow(),
        [
            (
                EventKind::KeyDown,
                Phase::Target,
                body,
                Some(Key::Character("a".to_owned()))
            ),
            (EventKind::KeyDown, Phase::Target, body, Some(Key::Tab)),
            (
                EventKind::KeyDown,
                Phase::Bubble,
                field,
                Some(Key::Character("ş".to_owned()))
            ),
        ]
    );
}

#[test]
fn a_transaction_holds_frames_until_its_outermost_commit() {
    let mut app = app(r#"<p id="p">bir</p>"#);
    let p = node(&app, "#p");
    let frame = |app: &App| app.last_frame_timings().map(|t| t.frame);
    let shown = frame(&app);
    app.begin_transaction();
    app.set_text(p, "iki").unwrap();
    app.tick(1);
    assert_eq!(frame(&app), shown, "no frame inside a transaction");
    // What the host reads is what it wrote, frame or not.
    assert_eq!(app.text(p).unwrap(), "iki");
    app.commit_transaction().unwrap();
    app.tick(2);
    assert_ne!(frame(&app), shown);
    let shown = frame(&app);
    let length = app.transaction(|cx| {
        cx.set_text(p, "üç").unwrap();
        cx.text(p).unwrap().len()
    });
    assert_eq!(length, "üç".len());
    app.tick(3);
    assert_ne!(frame(&app), shown);
    assert_eq!(app.commit_transaction(), Err(Status::InvalidArgument));
}
