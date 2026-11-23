//! Transitions on the host's clock (M5.12): a style change starts one, the
//! value at a time comes from its timing function, and every frame's
//! styles equal a full style's at the same time.

use erk_dom::{Document, NodeId, local_name};
use erk_invalidation::journal::{Journal, connected};
use erk_style::style::servo_arc::Arc;
use erk_style::{ComputedValues, Interaction, Restyler, StyleEngine, Styles, same_style};

const MS: u64 = 1_000_000;

/// A document changed the way the engine's page changes it, restyled once
/// per frame at the time the test sets.
struct Session {
    doc: Document,
    journal: Journal,
    restyler: Restyler,
    interaction: Interaction,
    last: Styles,
}

impl Session {
    fn new(html: &str) -> Self {
        let mut session = Self {
            doc: Document::parse_html(html),
            journal: Journal::new_document(),
            restyler: Restyler::new(StyleEngine::new(800.0, 600.0)),
            interaction: Interaction::default(),
            last: Styles::default(),
        };
        session.frame_at(0);
        session
    }

    fn find(&self, id_attr: &str) -> NodeId {
        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(self.doc.children(id));
            let found = self
                .doc
                .node(id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| element.attr(&local_name!("id")) == Some(id_attr));
            if found {
                return id;
            }
        }
        panic!("no #{id_attr}");
    }

    fn set_attr(&mut self, id: NodeId, name: &str, value: &str) {
        self.journal.touch(&self.doc, id);
        self.doc.set_attr(id, name, value).unwrap();
    }

    fn remove(&mut self, id: NodeId) {
        if let Some(parent) = self.doc.node(id).and_then(|node| node.parent()) {
            self.journal.touch(&self.doc, parent);
        }
        self.doc.remove(id);
    }

    fn create(&mut self, tag: &str) -> NodeId {
        let made = self.doc.create_element(tag).unwrap();
        self.journal.created(made);
        made
    }

    fn append(&mut self, parent: NodeId, child: NodeId) {
        self.journal.touch(&self.doc, parent);
        let joins = !connected(&self.doc, child);
        self.doc.insert(parent, child, None).unwrap();
        if joins && connected(&self.doc, child) {
            self.journal.arrived(&self.doc, child);
        }
    }

    /// Restyle at `ms` milliseconds on the host's clock, and check every
    /// element's style against a full style at the same time.
    fn frame_at(&mut self, ms: u64) -> &Styles {
        self.restyler.set_time(ms * MS);
        let (changes, _) = self.journal.take(&self.doc);
        let styles = self
            .restyler
            .restyle(&self.doc, &self.interaction, &changes);
        let full = StyleEngine::new(800.0, 600.0).style_at(
            &self.doc,
            &self.interaction,
            &self.restyler.transitions(),
        );
        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(self.doc.children(id));
            match (styles.computed(id), full.computed(id)) {
                (Some(restyled), Some(full)) => assert!(
                    same_style(&restyled, &full),
                    "at {ms} ms, {id:?} is styled otherwise than a full style styles it"
                ),
                (None, None) => {}
                (restyled, full) => panic!(
                    "{id:?}: restyled {}, full style {}",
                    restyled.is_some(),
                    full.is_some()
                ),
            }
        }
        self.last = styles;
        &self.last
    }

    fn style(&self, id_attr: &str) -> Arc<ComputedValues> {
        self.last
            .computed(self.find(id_attr))
            .unwrap_or_else(|| panic!("#{id_attr} was not styled"))
    }

    fn red(&self, id_attr: &str) -> f32 {
        self.style(id_attr).clone_color().components.0
    }

    fn background_red(&self, id_attr: &str) -> f32 {
        let style = self.style(id_attr);
        style
            .resolve_color(&style.get_background().background_color)
            .components
            .0
    }
}

fn near(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() < 1e-3
}

#[test]
fn a_colour_goes_from_the_old_value_to_the_new_on_the_hosts_clock() {
    let mut s =
        Session::new("<div id=a style='color: rgb(0, 0, 0); transition: color 1s linear'>a</div>");
    assert!(!s.last.animating(), "nothing moves before a change");
    let a = s.find("a");
    s.set_attr(
        a,
        "style",
        "color: rgb(200, 0, 0); transition: color 1s linear",
    );
    // The change is seen at 100 ms: the transition starts then.
    s.frame_at(100);
    assert!(near(s.red("a"), 0.0), "{}", s.red("a"));
    assert!(s.last.animating());
    s.frame_at(600);
    assert!(near(s.red("a"), 100.0 / 255.0), "{}", s.red("a"));
    assert!(s.last.animating());
    s.frame_at(850);
    assert!(near(s.red("a"), 150.0 / 255.0), "{}", s.red("a"));
    // Not ended yet, a frame before its end.
    s.frame_at(1090);
    assert!(near(s.red("a"), 198.0 / 255.0), "{}", s.red("a"));
    assert!(s.last.animating());
    // Ended: the new value, and nothing moves any more.
    s.frame_at(1100);
    assert!(near(s.red("a"), 200.0 / 255.0), "{}", s.red("a"));
    assert!(!s.last.animating());
}

#[test]
fn a_frame_with_nothing_changed_still_moves_a_transition() {
    let mut s = Session::new(
        "<style>#a { background-color: rgb(0, 0, 0); transition: background-color 1s linear }\
         #a.on { background-color: rgb(200, 0, 0) }</style><div id=a>a</div>",
    );
    let a = s.find("a");
    s.set_attr(a, "class", "on");
    s.frame_at(0);
    // Nothing changed in the document since: only the clock moved.
    s.frame_at(250);
    assert!(
        near(s.background_red("a"), 50.0 / 255.0),
        "{}",
        s.background_red("a")
    );
    s.frame_at(500);
    assert!(
        near(s.background_red("a"), 100.0 / 255.0),
        "{}",
        s.background_red("a")
    );
    let styled = s.frame_at(750).styled();
    assert!(styled >= 1, "the transitioning element is styled again");
    assert!(!s.frame_at(1000).animating());
    // Ended and settled: the next frame styles nothing.
    assert_eq!(s.frame_at(1200).styled(), 0);
}

#[test]
fn opacity_and_transform_follow_their_timing_functions() {
    let mut s = Session::new(
        "<style>#o { opacity: 0; transition: opacity 1s steps(4, end) }\
         #t { transform: translateX(0px); transition: transform 2s linear }\
         .on#o { opacity: 1 } .on#t { transform: translateX(100px) }</style>\
         <div id=o>o</div><div id=t>t</div>",
    );
    let (o, t) = (s.find("o"), s.find("t"));
    s.set_attr(o, "class", "on");
    s.set_attr(t, "class", "on");
    s.frame_at(0);
    let opacity = |s: &Session| s.style("o").get_effects().opacity;
    let x = |s: &Session| {
        s.style("t")
            .get_box()
            .transform
            .to_transform_3d_matrix(None)
            .unwrap()
            .0
            .m41
    };
    s.frame_at(240);
    assert_eq!(opacity(&s), 0.0, "steps(4, end) holds the first step");
    s.frame_at(260);
    assert_eq!(opacity(&s), 0.25);
    assert!(near(x(&s), 13.0), "{}", x(&s));
    s.frame_at(1000);
    assert_eq!(opacity(&s), 1.0);
    assert!(near(x(&s), 50.0), "{}", x(&s));
    s.frame_at(2000);
    assert!(near(x(&s), 100.0), "{}", x(&s));
    assert!(!s.last.animating());
}

#[test]
fn the_standard_curves_shape_the_value_between_the_ends() {
    // Halfway through, each curve's output (CSS Easing 1 §2): the value is
    // 200 times it, in red.
    for (curve, half) in [
        ("linear", 0.5),
        ("ease", 0.8024),
        ("ease-in", 0.3153),
        ("ease-out", 0.6847),
        ("ease-in-out", 0.5),
        ("cubic-bezier(0, 1, 1, 0)", 0.5),
    ] {
        let mut s = Session::new(&format!(
            "<style>#a {{ color: rgb(0, 0, 0); transition: color 1s {curve} }}\
             #a.on {{ color: rgb(200, 0, 0) }}</style><div id=a>a</div>"
        ));
        let a = s.find("a");
        s.set_attr(a, "class", "on");
        s.frame_at(0);
        s.frame_at(500);
        let expected = 200.0 * half / 255.0;
        assert!(
            (s.red("a") - expected).abs() < 2.0 / 255.0,
            "{curve}: {} at half, not {expected}",
            s.red("a")
        );
    }
}

#[test]
fn a_delay_holds_the_old_value_until_the_transition_starts() {
    let mut s = Session::new(
        "<style>#a { color: rgb(0, 0, 0); transition: color 1s linear 500ms }\
         #a.on { color: rgb(200, 0, 0) }</style><div id=a>a</div>",
    );
    let a = s.find("a");
    s.set_attr(a, "class", "on");
    s.frame_at(0);
    s.frame_at(400);
    assert!(near(s.red("a"), 0.0), "{}", s.red("a"));
    assert!(
        s.last.animating(),
        "it waits for its start, which is to come"
    );
    s.frame_at(1000);
    assert!(near(s.red("a"), 100.0 / 255.0), "{}", s.red("a"));
}

#[test]
fn going_back_halfway_turns_the_transition_around_from_where_it_is() {
    let mut s = Session::new(
        "<style>#a { color: rgb(0, 0, 0); transition: color 1s linear }\
         #a.on { color: rgb(200, 0, 0) }</style><div id=a>a</div>",
    );
    let a = s.find("a");
    s.set_attr(a, "class", "on");
    s.frame_at(0);
    s.frame_at(500);
    let halfway = s.red("a");
    s.set_attr(a, "class", "");
    // No jump: it goes back from where it was, and as it covered half the
    // way, it takes half the time (CSS Transitions 1 §3.1, reversing).
    s.frame_at(500);
    assert!(near(s.red("a"), halfway), "{} then {}", halfway, s.red("a"));
    s.frame_at(750);
    assert!(near(s.red("a"), 50.0 / 255.0), "{}", s.red("a"));
    s.frame_at(1000);
    assert!(near(s.red("a"), 0.0), "{}", s.red("a"));
    assert!(!s.last.animating());
    // The transition it replaced is gone too: nothing is styled again.
    assert_eq!(s.frame_at(1100).styled(), 0);
}

#[test]
fn a_new_document_starts_without_the_old_ones_transitions() {
    let page = "<style>.x { color: rgb(0, 0, 0); transition: color 1s linear }\
                .x.on { color: rgb(200, 0, 0) }</style><div id=a class=x>a</div>";
    let mut s = Session::new(page);
    let a = s.find("a");
    s.set_attr(a, "class", "x on");
    s.frame_at(0);
    assert!(s.frame_at(300).animating());
    // The same page again, already on: its element sits in the same arena
    // slot, but it is another document's.
    s.doc = Document::parse_html(&page.replace("class=x", "class='x on'"));
    s.journal = Journal::new_document();
    s.frame_at(400);
    assert!(near(s.red("a"), 200.0 / 255.0), "{}", s.red("a"));
    assert!(!s.last.animating());
}

#[test]
fn a_transition_ends_when_its_element_stops_being_shown() {
    let mut s = Session::new(
        "<style>#a { color: rgb(0, 0, 0); transition: color 1s linear }\
         #a.on { color: rgb(200, 0, 0) } .hidden { display: none }</style>\
         <div id=box><div id=a>a</div></div>",
    );
    let (a, outer) = (s.find("a"), s.find("box"));
    s.set_attr(a, "class", "on");
    s.frame_at(0);
    assert!(s.frame_at(300).animating());
    // Its parent hides it: it is not styled, and nothing moves.
    s.set_attr(outer, "class", "hidden");
    assert!(!s.frame_at(400).animating());
    // Shown again, it has its new value, without a transition.
    s.set_attr(outer, "class", "");
    s.frame_at(500);
    assert!(near(s.red("a"), 200.0 / 255.0), "{}", s.red("a"));
    assert!(!s.last.animating());
}

#[test]
fn a_removed_elements_transition_does_not_pass_to_the_next_in_its_place() {
    let mut s = Session::new(
        "<style>.x { color: rgb(0, 0, 0); transition: color 1s linear }\
         .x.on { color: rgb(200, 0, 0) }</style><div id=box><div id=a class=x>a</div></div>",
    );
    let (a, outer) = (s.find("a"), s.find("box"));
    s.set_attr(a, "class", "x on");
    s.frame_at(0);
    assert!(s.frame_at(300).animating());
    s.remove(a);
    assert!(!s.frame_at(400).animating());
    // A new element may take the removed one's arena slot.
    let b = s.create("div");
    s.set_attr(b, "class", "x on");
    s.set_attr(b, "id", "b");
    s.append(outer, b);
    s.frame_at(500);
    assert!(near(s.red("b"), 200.0 / 255.0), "{}", s.red("b"));
    assert!(!s.last.animating());
}

#[test]
fn an_element_put_in_a_removed_ones_place_in_the_same_frame_starts_afresh() {
    let mut s = Session::new(
        "<style>.x { color: rgb(0, 0, 0); transition: color 1s linear }\
         .x.on { color: rgb(200, 0, 0) }</style><div id=box><div id=a class=x></div></div>",
    );
    let (a, outer) = (s.find("a"), s.find("box"));
    s.set_attr(a, "class", "x on");
    s.frame_at(0);
    assert!(s.frame_at(300).animating());
    // Removed and replaced before the next frame: the new element is
    // styled the same frame the old one goes, perhaps in its slot.
    s.remove(a);
    let b = s.create("div");
    assert_eq!(b.index(), a.index(), "the arena gives the slot again");
    s.set_attr(b, "class", "x on");
    s.set_attr(b, "id", "b");
    s.append(outer, b);
    s.frame_at(400);
    assert!(near(s.red("b"), 200.0 / 255.0), "{}", s.red("b"));
    assert!(!s.last.animating());
}

#[test]
fn keyframe_animations_do_not_run_yet() {
    // `@keyframes` animations are for later (p1-contract §7). Until Erk
    // moves them on, an element with one is styled as if it had none, and
    // asks for no frames.
    let mut s = Session::new(
        "<style>@keyframes glow { from { color: rgb(0, 200, 0) } to { color: rgb(0, 0, 200) } }\
         #a { color: rgb(200, 0, 0); animation: glow 1s linear infinite }</style>\
         <div id=a>a</div>",
    );
    assert!(near(s.red("a"), 200.0 / 255.0), "{}", s.red("a"));
    assert!(!s.last.animating());
    s.frame_at(500);
    assert!(near(s.red("a"), 200.0 / 255.0), "{}", s.red("a"));
    assert!(!s.last.animating());
}
