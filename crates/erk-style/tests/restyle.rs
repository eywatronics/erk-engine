//! Restyling frame after frame (M5.3): every frame's styles equal a full
//! style's, and a change restyles only what it reaches.

use erk_dom::{Document, NodeId, local_name};
use erk_invalidation::journal::{Journal, connected};
use erk_style::{Interaction, Invalidation, Restyler, StyleEngine, Styles, same_style};

/// A document changed the way the engine's page changes it, recording each
/// change in a journal, and restyled once per frame.
struct Session {
    doc: Document,
    journal: Journal,
    restyler: Restyler,
    interaction: Interaction,
}

impl Session {
    fn new(html: &str) -> Self {
        let mut session = Self {
            doc: Document::parse_html(html),
            journal: Journal::new_document(),
            restyler: Restyler::new(StyleEngine::new(800.0, 600.0)),
            interaction: Interaction::default(),
        };
        session.frame();
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

    fn remove_attr(&mut self, id: NodeId, name: &str) {
        self.journal.touch(&self.doc, id);
        self.doc.remove_attr(id, name).unwrap();
    }

    fn create(&mut self, tag: &str) -> NodeId {
        let made = self.doc.create_element(tag).unwrap();
        self.journal.created(made);
        made
    }

    fn insert(&mut self, parent: NodeId, child: NodeId, before: Option<NodeId>) {
        if let Some(old) = self.doc.node(child).and_then(|node| node.parent()) {
            self.journal.touch(&self.doc, old);
        }
        self.journal.touch(&self.doc, parent);
        let joins = !connected(&self.doc, child);
        self.doc.insert(parent, child, before).unwrap();
        if joins && connected(&self.doc, child) {
            self.journal.arrived(&self.doc, child);
        }
    }

    fn detach(&mut self, id: NodeId) {
        if let Some(parent) = self.doc.node(id).and_then(|node| node.parent()) {
            self.journal.touch(&self.doc, parent);
        }
        self.doc.detach(id);
    }

    fn remove(&mut self, id: NodeId) {
        if let Some(parent) = self.doc.node(id).and_then(|node| node.parent()) {
            self.journal.touch(&self.doc, parent);
        }
        self.doc.remove(id);
    }

    fn set_text(&mut self, id: NodeId, text: &str) {
        self.journal.touch(&self.doc, id);
        self.doc.set_text(id, text).unwrap();
        if self.doc.node(id).is_some_and(|n| n.as_element().is_some())
            && let Some(child) = self.doc.children(id).next()
        {
            self.journal.created(child);
        }
    }

    /// Restyle, and check every element's style against a full style.
    fn frame(&mut self) -> Styles {
        let (changes, _) = self.journal.take(&self.doc);
        let styles = self
            .restyler
            .restyle(&self.doc, &self.interaction, &changes);
        let full = StyleEngine::new(800.0, 600.0).style_with(&self.doc, &self.interaction);
        let mut stack = vec![self.doc.root()];
        while let Some(id) = stack.pop() {
            stack.extend(self.doc.children(id));
            match (styles.computed(id), full.computed(id)) {
                (Some(restyled), Some(full)) => assert!(
                    same_style(&restyled, &full),
                    "{id:?} is styled otherwise than a full style styles it"
                ),
                (None, None) => {}
                (restyled, full) => panic!(
                    "{id:?}: restyled {}, full style {}",
                    restyled.is_some(),
                    full.is_some()
                ),
            }
        }
        styles
    }
}

/// `count` items with ids `i0`, `i1`, ...
fn items(tag: &str, count: usize) -> String {
    (0..count)
        .map(|i| format!("<{tag} id=i{i}><span>{i}</span></{tag}>"))
        .collect()
}

fn damage_of(styles: &Styles, id: NodeId) -> Invalidation {
    styles
        .damage()
        .iter()
        .find(|(node, _)| *node == id)
        .map_or(Invalidation::NONE, |(_, bits)| *bits)
}

// A restyled element's children are styled again too: Stylo cascades them
// anew whenever their parent's style changed, however it changed. Their
// own children it leaves alone when the children's styles come out equal.

#[test]
fn a_class_restyles_only_the_element_it_is_on() {
    let html = format!(
        "<style>.on {{ background: red }}</style><div>{}</div>",
        items("p", 200)
    );
    let mut s = Session::new(&html);
    let p = s.find("i7");
    s.set_attr(p, "class", "on");
    let styles = s.frame();
    // The paragraph and its span.
    assert_eq!(styles.styled(), 2);
    assert_eq!(
        damage_of(&styles, p),
        Invalidation::PAINT_SELF | Invalidation::HIT_TEST | Invalidation::A11Y_SELF
    );
    // Nothing changed: nothing is styled.
    assert_eq!(s.frame().styled(), 0);
    s.remove_attr(p, "class");
    assert_eq!(s.frame().styled(), 2);
}

#[test]
fn an_inherited_property_restyles_the_descendants_that_inherit_it() {
    let html = format!(
        "<style>.on {{ color: red }}</style><div>{}</div>",
        items("p", 50)
    );
    let mut s = Session::new(&html);
    let p = s.find("i3");
    s.set_attr(p, "class", "on");
    // The paragraph and its span.
    assert_eq!(s.frame().styled(), 2);
}

#[test]
fn a_sibling_combinator_restyles_the_sibling() {
    let html = format!(
        "<style>.a + p {{ color: red }} .a ~ p.far {{ color: blue }}</style><div>{}<p class=far id=far></p></div>",
        items("p", 20)
    );
    let mut s = Session::new(&html);
    let p = s.find("i4");
    s.set_attr(p, "class", "a");
    let styles = s.frame();
    // The next paragraph and its span; the far one.
    assert!(styles.styled() <= 4, "{}", styles.styled());
    assert!(styles.styled() >= 3);
}

#[test]
fn a_descendant_selector_restyles_the_descendants() {
    let mut s = Session::new(
        "<style>.open span { color: red }</style><div id=box><p><span>a</span></p><p><span>b</span></p></div><div><span>c</span></div>",
    );
    let open = s.find("box");
    s.set_attr(open, "class", "open");
    assert_eq!(s.frame().styled(), 2);
}

#[test]
fn has_is_not_parsed_yet() {
    // Stylo 0.20 does not parse `:has()` in Servo mode, so a rule using it
    // is dropped and nothing depends on it. The day it parses, restyling
    // must restyle its anchors (see `restyle.rs`); this test says when.
    let color = |rule: &str| {
        let s = Session::new(&format!(
            "<style>{rule} {{ color: red }}</style><div class=card id=card><p class=picked>x</p></div>"
        ));
        let styles = StyleEngine::new(800.0, 600.0).style(&s.doc);
        format!(
            "{:?}",
            styles.computed(s.find("card")).unwrap().clone_color()
        )
    };
    // The same rule without `:has()` applies, so the test would see one.
    assert_ne!(color(".card"), color(".none"));
    assert_eq!(
        color(".card:has(.picked)"),
        color(".none"),
        "`:has()` matches now: restyling must restyle its anchors"
    );
}

#[test]
fn hovering_restyles_the_hovered_chain_only() {
    let html = format!(
        "<style>li:hover {{ background: yellow }}</style><ul>{}</ul>",
        items("li", 100)
    );
    let mut s = Session::new(&html);
    let li = s.find("i40");
    s.interaction.hover = Some(li);
    // The item and its span.
    assert_eq!(s.frame().styled(), 2);
    s.interaction.hover = Some(s.find("i41"));
    // The one it left and the one it entered.
    assert_eq!(s.frame().styled(), 4);
}

#[test]
fn appending_to_a_list_styles_only_the_new_item() {
    let html = format!(
        "<style>li {{ color: green }}</style><ul id=list>{}</ul>",
        items("li", 100)
    );
    let mut s = Session::new(&html);
    let list = s.find("list");
    let li = s.create("li");
    s.insert(list, li, None);
    assert_eq!(s.frame().styled(), 1);
}

#[test]
fn positional_selectors_restyle_the_siblings() {
    let html = format!(
        "<style>li:nth-child(odd) {{ color: red }} li:first-child {{ font-weight: bold }} li + li {{ margin: 1px }}</style><ul id=list>{}</ul>",
        items("li", 10)
    );
    let mut s = Session::new(&html);
    let list = s.find("list");
    let first = s.find("i0");
    let li = s.create("li");
    s.insert(list, li, Some(first));
    s.frame();
    s.remove(li);
    s.frame();
    let last = s.find("i9");
    s.remove(last);
    s.frame();
}

#[test]
fn being_empty_restyles_the_element_and_its_siblings() {
    let mut s = Session::new(
        "<style>p:empty { display: none } p:empty + span { color: red }</style><div><p id=p></p><span>x</span></div>",
    );
    let p = s.find("p");
    s.set_text(p, "now with text");
    s.frame();
    let text = s.doc.children(p).next().unwrap();
    s.journal.touch(&s.doc, text);
    s.doc.set_text(text, "").unwrap();
    s.frame();
    s.set_text(p, "");
    s.frame();
}

#[test]
fn a_moved_element_is_styled_where_it_went() {
    let mut s = Session::new(
        "<style>.y b { color: red } .x b { color: blue }</style><div class=x id=x><b id=b>b</b></div><div class=y id=y></div>",
    );
    let (b, y, x) = (s.find("b"), s.find("y"), s.find("x"));
    s.insert(y, b, None);
    s.frame();
    // Out of the document for a frame, then back.
    s.detach(b);
    s.frame();
    s.insert(x, b, None);
    s.frame();
}

#[test]
fn a_node_back_from_outside_the_document_is_styled_anew() {
    let mut s =
        Session::new("<style>.on { color: red }</style><div id=x><b id=b>b</b><i>i</i></div>");
    let (b, x) = (s.find("b"), s.find("x"));
    s.detach(b);
    s.frame();
    // Changed while outside: no frame saw the change.
    s.set_attr(b, "class", "on");
    s.frame();
    // Back where it was.
    s.insert(x, b, None);
    s.frame();
}

#[test]
fn a_new_node_in_a_removed_ones_slot_makes_everything_dirty() {
    let mut s = Session::new("<style>p { color: red }</style><div id=x><p id=p></p></div>");
    let (p, x) = (s.find("p"), s.find("x"));
    s.remove(p);
    let made = s.create("p");
    s.insert(x, made, None);
    // The arena gave the new node the removed one's slot.
    assert_eq!(made.index(), p.index());
    let styles = s.frame();
    assert_eq!(
        damage_of(&styles, made),
        Invalidation::ALL - Invalidation::STYLE_SELF - Invalidation::STYLE_SUBTREE
    );
}

#[test]
fn an_element_outside_the_root_element_has_no_style() {
    // Styling starts at the document's first element; a full style never
    // reaches an element beside it, so one that moved there loses its own.
    let mut s = Session::new("<div id=x><b id=b>b</b></div>");
    let (b, x) = (s.find("b"), s.find("x"));
    let root = s.doc.root();
    s.insert(root, b, None);
    s.frame();
    s.insert(x, b, None);
    s.frame();
}

#[test]
fn the_style_attribute_restyles_the_element() {
    let html = format!("<div>{}</div>", items("p", 50));
    let mut s = Session::new(&html);
    let p = s.find("i9");
    s.set_attr(p, "style", "width: 100px");
    let styles = s.frame();
    assert_eq!(styles.styled(), 2);
    let bits = damage_of(&styles, p);
    assert!(bits.contains(Invalidation::LAYOUT_SELF));
    assert!(!bits.contains(Invalidation::TEXT_SHAPE), "{bits:?}");
    s.set_attr(p, "style", "width: 100px; font-size: 30px");
    let bits = damage_of(&s.frame(), p);
    assert!(bits.contains(Invalidation::TEXT_SHAPE), "{bits:?}");
    s.set_attr(p, "style", "width: 100px; font-size: 30px; color: red");
    let styles = s.frame();
    let bits = damage_of(&styles, p);
    assert!(!bits.contains(Invalidation::LAYOUT_SELF), "{bits:?}");
    assert!(bits.contains(Invalidation::PAINT_SELF));
}

#[test]
fn other_sheets_style_everything_again() {
    let html = format!(
        "<style id=sheet>p {{ color: red }}</style><div>{}</div>",
        items("p", 20)
    );
    let mut s = Session::new(&html);
    let sheet = s.find("sheet");
    s.set_text(sheet, "p { color: blue }");
    let styles = s.frame();
    // Every element with a style: html, head, body, the div, 20 paragraphs
    // and their spans. The `<style>` element is in `<head>`, which is not
    // displayed, so it has none.
    assert_eq!(styles.styled(), 44);
}

#[test]
fn display_none_keeps_attributes_for_when_it_shows() {
    let mut s = Session::new(
        "<style>.hidden { display: none } #t.on { color: red }</style><div id=box class=hidden><p id=t>x</p></div>",
    );
    let (b, t) = (s.find("box"), s.find("t"));
    s.set_attr(t, "class", "on");
    s.set_attr(t, "style", "margin: 3px");
    s.frame();
    s.remove_attr(b, "class");
    s.frame();
}

/// xorshift64*: deterministic inputs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn random_changes_restyle_as_a_full_style_styles() {
    const SHEET: &str = "\
        .a { color: red } .b > .a { color: blue } .a + .b { margin: 1px }\
        .b ~ .c { padding: 2px } .c .a { font-size: 20px } :empty { min-height: 1px }\
        li:nth-child(2n) { background: gray } li:first-child, li:last-child { border: 1px solid }\
        div:hover > .a { color: green }\
        .a:not(.b) .c { display: none } [data-x=\"1\"] { opacity: 0.5 } #k1 .b { line-height: 3 }";
    let classes = ["", "a", "b", "c", "a b", "b c"];
    for seed in 1..=40u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let mut s = Session::new(&format!(
            "<style>{SHEET}</style><div id=root><div><ul><li>1</li><li class=a>2</li></ul></div><p class=b>t</p></div>"
        ));
        let root = s.find("root");
        let mut nodes = vec![root];
        for _ in 0..60 {
            let pick = nodes[rng.below(nodes.len())];
            match rng.below(9) {
                0 | 1 => {
                    let tag = ["div", "p", "li", "ul", "span"][rng.below(5)];
                    let made = s.create(tag);
                    if rng.below(2) == 0 {
                        s.set_attr(made, "class", classes[rng.below(classes.len())]);
                    }
                    let before = s.doc.children(pick).nth(rng.below(3));
                    s.insert(pick, made, before);
                    nodes.push(made);
                }
                2 | 3 if pick != root => {
                    s.set_attr(pick, "class", classes[rng.below(classes.len())])
                }
                4 if pick != root => {
                    s.set_attr(pick, "data-x", ["0", "1"][rng.below(2)]);
                    s.set_attr(pick, "id", ["k1", "k2"][rng.below(2)]);
                }
                5 if pick != root => {
                    // Move it somewhere else, unless that is inside itself.
                    let to = nodes[rng.below(nodes.len())];
                    let mut inside = Some(to);
                    while let Some(id) = inside {
                        if id == pick {
                            break;
                        }
                        inside = s.doc.node(id).and_then(|node| node.parent());
                    }
                    if inside.is_none() {
                        s.insert(to, pick, None);
                    }
                }
                6 if pick != root => {
                    s.remove(pick);
                    nodes.retain(|id| s.doc.node(*id).is_some() && connected(&s.doc, *id));
                }
                7 => s.interaction.hover = Some(pick),
                _ => s.set_text(pick, ["", "x"][rng.below(2)]),
            }
            nodes.retain(|id| s.doc.node(*id).is_some() && connected(&s.doc, *id));
            if !nodes.contains(&root) {
                break;
            }
            if rng.below(3) == 0 {
                s.frame();
            }
        }
        s.frame();
    }
}
