//! Changing a document after it is parsed: removing nodes, setting text,
//! loading another document into the same arena. A removed node's id goes
//! stale and never names another node (p1-contract §2).

use erk_dom::{Document, NodeData, NodeId, StaleNode};

/// Every node in `doc`, in tree order, the document node first.
fn all(doc: &Document) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        out.push(id);
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children);
    }
    out
}

fn by_id(doc: &Document, value: &str) -> NodeId {
    all(doc)
        .into_iter()
        .find(|id| {
            doc.node(*id)
                .and_then(|node| node.as_element())
                .and_then(|element| element.attr(&erk_dom::local_name!("id")))
                == Some(value)
        })
        .unwrap_or_else(|| panic!("no #{value}"))
}

fn texts(doc: &Document, parent: NodeId) -> Vec<String> {
    doc.children(parent)
        .map(|child| match &doc.node(child).unwrap().data {
            NodeData::Text(text) => text.clone(),
            NodeData::Element(element) => format!("<{}>", element.name.local),
            _ => "?".to_owned(),
        })
        .collect()
}

#[test]
fn a_removed_subtree_is_gone_and_its_ids_stay_stale() {
    let mut doc = Document::parse_html(
        r#"<div id="a"><p id="p">x<b id="b">y</b></p></div><span id="s">z</span>"#,
    );
    let (a, p, b, s) = (
        by_id(&doc, "a"),
        by_id(&doc, "p"),
        by_id(&doc, "b"),
        by_id(&doc, "s"),
    );
    let inside: Vec<NodeId> = all(&doc)
        .into_iter()
        .filter(|id| {
            let mut current = Some(*id);
            while let Some(node) = current {
                if node == a {
                    return true;
                }
                current = doc.node(node).and_then(|n| n.parent());
            }
            false
        })
        .collect();
    assert_eq!(inside.len(), 5, "div, p, text, b, text");

    assert!(doc.remove(a));
    for id in &inside {
        assert!(doc.node(*id).is_none(), "{id:?} is gone");
    }
    assert!(doc.node(s).is_some(), "its sibling stays");
    assert!(!doc.remove(a), "already removed");
    assert!(!doc.remove(doc.root()), "the document node stays");
    assert!(doc.node(doc.root()).is_some());

    // New nodes take the freed slots under new ids.
    let fresh: Vec<NodeId> = (0..10)
        .map(|_| doc.create(NodeData::Text(String::new())))
        .collect();
    for id in [a, p, b] {
        assert!(!fresh.contains(&id), "{id:?} named again");
        assert!(doc.node(id).is_none());
    }
    assert!(
        fresh
            .iter()
            .any(|id| inside.iter().any(|old| old.index() == id.index())),
        "the slots are reused"
    );
}

#[test]
fn a_templates_contents_go_with_it() {
    let mut doc = Document::parse_html(r#"<template id="t"><p>içerik</p></template>"#);
    let template = by_id(&doc, "t");
    let before = doc.capacity_hint();
    assert!(doc.remove(template));
    // The contents' slots are free again: as many new nodes fit without
    // growing the arena.
    for _ in 0..4 {
        doc.create(NodeData::Text(String::new()));
    }
    assert_eq!(doc.capacity_hint(), before);
}

#[test]
fn set_text_works_as_text_content_does() {
    let mut doc = Document::parse_html(r#"<p id="p">a<b>b</b>c</p><i id="i">eski</i>"#);
    let p = by_id(&doc, "p");
    let old: Vec<NodeId> = doc.children(p).collect();
    assert_eq!(texts(&doc, p), ["a", "<b>", "c"]);

    assert_eq!(doc.set_text(p, "yeni metin"), Ok(()));
    assert_eq!(texts(&doc, p), ["yeni metin"]);
    for id in old {
        assert!(doc.node(id).is_none(), "the old children are removed");
    }
    assert_eq!(doc.set_text(p, ""), Ok(()));
    assert!(
        texts(&doc, p).is_empty(),
        "no text node for an empty string"
    );

    // A text node keeps its id; only its data changes.
    let i = by_id(&doc, "i");
    let text = doc.children(i).next().unwrap();
    assert_eq!(doc.set_text(text, "değişti"), Ok(()));
    assert_eq!(doc.children(i).next(), Some(text));
    assert_eq!(texts(&doc, i), ["değişti"]);

    // The document node: nothing happens.
    let root = doc.root();
    let before = all(&doc).len();
    assert_eq!(doc.set_text(root, "x"), Ok(()));
    assert_eq!(all(&doc).len(), before);

    // A removed node is an error, not a panic.
    assert!(doc.remove(i));
    assert_eq!(doc.set_text(i, "x"), Err(StaleNode));
    assert_eq!(doc.set_text(text, "x"), Err(StaleNode));
}

#[test]
fn loading_a_document_makes_every_old_id_stale() {
    let mut doc = Document::parse_html("<p>bir</p><!-- yorum --><p>iki</p>");
    let old = all(&doc);
    let root = doc.root();
    doc.load_html("<p>üç</p><p>dört</p><p>beş</p>");
    assert_eq!(doc.root(), root, "the document node stays");
    for id in &old[1..] {
        assert!(doc.node(*id).is_none(), "{id:?} still names a node");
    }
    let new = all(&doc);
    assert!(new.iter().skip(1).all(|id| !old.contains(id)));
    let body = new
        .iter()
        .copied()
        .find(|id| {
            doc.node(*id)
                .and_then(|node| node.as_element())
                .is_some_and(|element| &*element.name.local == "body")
        })
        .unwrap();
    assert_eq!(texts(&doc, body), ["<p>", "<p>", "<p>"]);
    // The new document reuses the old one's slots.
    assert!(doc.capacity_hint() < old.len() + new.len());
}
