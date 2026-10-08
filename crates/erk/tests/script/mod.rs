//! Bytes read as a script of host calls (M4.2): the fuzz target
//! (`fuzz/fuzz_targets/mutations.rs`) and the fixed-seed test
//! (`tests/mutations.rs`) both run it, so a script libFuzzer finds can go
//! into the test as it is.
//!
//! Every call of the host API that changes or reads the document can come
//! up, in any order, on any node the script has seen: nodes it removed,
//! nodes of a document it replaced, nodes into themselves, references to
//! the batch's own nodes that point forward or nowhere, callbacks that
//! remove the node they run on. Every call may fail; none may panic.

use erk::{
    App, EventKind, Input, Key, KeyInput, KeyState, Modifiers, Mutation, Node, PointerButton,
    PointerInput, PointerKind, Ref, Subscription,
};

const TAGS: &[&str] = &[
    "div", "p", "span", "ul", "li", "button", "a", "template", "table", "tr", "td", "img", "br",
    "section", "input", "A b", "", "svg", "-x",
];

const NAMES: &[&str] = &[
    "id", "class", "style", "tabindex", "href", "hidden", "data-x", "x y", "", "1a", "src",
];

const VALUES: &[&str] = &[
    "",
    "a",
    "b c",
    "0",
    "-1",
    "#",
    "display: none",
    "display: flex; width: 50%",
    "overflow: scroll; height: 10px",
    "position: absolute; left: -1e9px",
    "font-size: 1e9px",
    "width: calc(100% - 1e30px)",
    "görev ✓",
    "url(\"file:///etc/passwd\")",
];

const SELECTORS: &[&str] = &[
    "*",
    "div",
    "p > span",
    ".a",
    "#a",
    "[data-x]",
    "li:first-child",
    ":focus",
    ":hover",
    "p >",
    "",
    "::before",
    "body *",
];

const PAGES: &[&str] = &[
    "",
    "<ul id=a><li class=a>bir<li>iki</ul><button>b</button>",
    "<div style='overflow: scroll; height: 30px'><p>1</p><p>2</p><p>3</p></div>",
    "<template><p>t</p></template><a href=#>x</a>",
];

/// Reads the script's bytes; past the end every read is 0, so any prefix of
/// a script is a script.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        match self.0.split_first() {
            Some((first, rest)) => {
                self.0 = rest;
                *first
            }
            None => 0,
        }
    }

    fn below(&mut self, n: usize) -> usize {
        usize::from(self.byte()) % n
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }

    fn coordinate(&mut self) -> f32 {
        f32::from(self.byte()) - 8.0
    }
}

/// What the script has seen.
struct State {
    nodes: Vec<Node>,
    subscriptions: Vec<Subscription>,
}

impl State {
    fn node(&self, bytes: &mut Bytes) -> Node {
        self.nodes[bytes.below(self.nodes.len())]
    }

    /// A node, or a reference to what the batch made, forward or not.
    fn reference(&self, bytes: &mut Bytes) -> Ref {
        if bytes.byte().is_multiple_of(2) {
            Ref::Node(self.node(bytes))
        } else {
            Ref::New(bytes.below(8))
        }
    }

    fn see(&mut self, node: Node) {
        // Bounded, so that a long script keeps picking among a few nodes.
        if self.nodes.len() < 64 {
            self.nodes.push(node);
        } else {
            let at = node.to_raw() as usize % self.nodes.len();
            self.nodes[at] = node;
        }
    }
}

fn mutation(state: &State, bytes: &mut Bytes) -> Mutation {
    match bytes.below(10) {
        0 => Mutation::CreateElement(bytes.pick(TAGS).to_owned()),
        1 => Mutation::CreateText(bytes.pick(VALUES).to_owned()),
        2 => Mutation::Append {
            parent: state.reference(bytes),
            child: state.reference(bytes),
        },
        3 => Mutation::InsertBefore {
            parent: state.reference(bytes),
            child: state.reference(bytes),
            before: bytes
                .byte()
                .is_multiple_of(2)
                .then(|| state.reference(bytes)),
        },
        4 => Mutation::Remove(state.reference(bytes)),
        5 => Mutation::SetText(state.reference(bytes), bytes.pick(VALUES).to_owned()),
        6 => Mutation::SetAttr(
            state.reference(bytes),
            bytes.pick(NAMES).to_owned(),
            bytes.pick(VALUES).to_owned(),
        ),
        7 => Mutation::RemoveAttr(state.reference(bytes), bytes.pick(NAMES).to_owned()),
        8 => Mutation::AddClass(state.reference(bytes), bytes.pick(VALUES).to_owned()),
        _ => Mutation::RemoveClass(state.reference(bytes), bytes.pick(VALUES).to_owned()),
    }
}

const KINDS: &[EventKind] = &[
    EventKind::Click,
    EventKind::KeyDown,
    EventKind::KeyUp,
    EventKind::Focus,
    EventKind::Blur,
];

/// What a callback does to the document while the event is on its way.
fn subscribe(app: &mut App, state: &mut State, bytes: &mut Bytes) {
    let node = state.node(bytes);
    let kind = KINDS[bytes.below(KINDS.len())];
    let action = bytes.byte();
    let callback = move |cx: &mut erk::Context, event: &erk::Event| match action % 5 {
        0 => {
            let _ = cx.remove(event.target);
        }
        1 => {
            let _ = cx.set_text(event.target, "değişti");
        }
        2 => cx.stop_propagation(),
        3 => cx.load_html("<p>yeni</p>"),
        _ => {
            if let Ok(made) = cx.create_element("p") {
                let _ = cx.append(event.target, made);
            }
        }
    };
    let made = if action & 0x80 == 0 {
        app.on(node, kind, callback)
    } else {
        app.on_capture(node, kind, callback)
    };
    if let Ok(subscription) = made {
        state.subscriptions.push(subscription);
    }
}

fn key(bytes: &mut Bytes) -> Key {
    match bytes.below(6) {
        0 => Key::Tab,
        1 => Key::Enter,
        2 => Key::Space,
        3 => Key::Escape,
        4 => Key::Character(bytes.pick(&["a", "ş", " ", "\n"]).to_owned()),
        _ => Key::Other,
    }
}

/// Run `script` against `app`.
pub fn run(app: &mut App, script: &[u8]) {
    let mut bytes = Bytes(script);
    let mut state = State {
        nodes: vec![app.root()],
        subscriptions: Vec::new(),
    };
    app.set_verifying(true);
    let mut now = 0;
    while !bytes.0.is_empty() {
        match bytes.below(18) {
            0 => {
                if let Ok(node) = app.create_element(bytes.pick(TAGS)) {
                    state.see(node);
                }
            }
            1 => {
                let node = app.create_text(bytes.pick(VALUES));
                state.see(node);
            }
            2 => {
                let (parent, child) = (state.node(&mut bytes), state.node(&mut bytes));
                let _ = app.append(parent, child);
            }
            3 => {
                let (parent, child) = (state.node(&mut bytes), state.node(&mut bytes));
                let before = bytes
                    .byte()
                    .is_multiple_of(2)
                    .then(|| state.node(&mut bytes));
                let _ = app.insert_before(parent, child, before);
            }
            4 => {
                let _ = app.remove(state.node(&mut bytes));
            }
            5 => {
                let _ = app.set_text(state.node(&mut bytes), bytes.pick(VALUES));
            }
            6 => {
                let node = state.node(&mut bytes);
                let _ = app.set_attr(node, bytes.pick(NAMES), bytes.pick(VALUES));
            }
            7 => {
                let _ = app.remove_attr(state.node(&mut bytes), bytes.pick(NAMES));
            }
            8 => {
                let node = state.node(&mut bytes);
                let class = bytes.pick(VALUES);
                let _ = if bytes.byte().is_multiple_of(2) {
                    app.add_class(node, class)
                } else {
                    app.remove_class(node, class)
                };
            }
            9 => {
                let batch: Vec<Mutation> = (0..1 + bytes.below(8))
                    .map(|_| mutation(&state, &mut bytes))
                    .collect();
                if let Ok(made) = app.apply(&batch) {
                    for node in made.into_iter().flatten() {
                        state.see(node);
                    }
                }
            }
            10 => {
                now += 16_000_000;
                app.tick(now);
                check(app);
            }
            11 => {
                let state = if bytes.byte().is_multiple_of(2) {
                    KeyState::Down
                } else {
                    KeyState::Up
                };
                app.input(Input::Key(KeyInput {
                    key: key(&mut bytes),
                    state,
                    modifiers: Modifiers::default(),
                }));
            }
            12 => {
                let kind = [
                    PointerKind::Move,
                    PointerKind::Down,
                    PointerKind::Up,
                    PointerKind::Leave,
                ][bytes.below(4)];
                app.input(Input::Pointer(PointerInput {
                    kind,
                    x: bytes.coordinate(),
                    y: bytes.coordinate(),
                    button: PointerButton::Primary,
                    modifiers: Modifiers::default(),
                }));
            }
            13 => {
                let (x, y) = (bytes.coordinate(), bytes.coordinate());
                app.input(Input::Wheel {
                    dx: bytes.coordinate(),
                    dy: bytes.coordinate(),
                    x,
                    y,
                });
            }
            14 => {
                let scope = bytes
                    .byte()
                    .is_multiple_of(2)
                    .then(|| state.node(&mut bytes));
                if let Ok(found) = app.query_all(scope, bytes.pick(SELECTORS)) {
                    for node in found.into_iter().take(4) {
                        state.see(node);
                    }
                }
            }
            15 => app.load_html(bytes.pick(PAGES)),
            16 => subscribe(app, &mut state, &mut bytes),
            _ => {
                if !state.subscriptions.is_empty() {
                    let at = bytes.below(state.subscriptions.len());
                    let _ = app.off(state.subscriptions.swap_remove(at));
                }
                // And what a host reads back.
                let node = state.node(&mut bytes);
                let _ = (app.text(node), app.node_box(node), app.parent(node));
            }
        }
    }
    app.tick(now + 16_000_000);
    check(app);
}

/// The frame just made agrees with the document recomputed from nothing
/// (M5 plan, decision 9): an incremental path that forgets something
/// shows here first.
fn check(app: &mut App) {
    if let Err(parted) = app.verify_frame() {
        panic!("the frame is not what the document recomputed gives: {parted}");
    }
}
