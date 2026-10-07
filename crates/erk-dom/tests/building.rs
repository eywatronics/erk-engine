//! Building a document from outside (M4.0): creating elements and text,
//! inserting them where DOM's pre-insert validity allows, and changing
//! attributes.

use erk_dom::{Document, LocalName, MutationError, NodeData, NodeId};

fn body(doc: &Document) -> NodeId {
    let html = doc.children(doc.root()).last().unwrap();
    doc.children(html).last().unwrap()
}

fn tag(doc: &Document, id: NodeId) -> String {
    doc.node(id)
        .and_then(|node| node.as_element())
        .map(|element| element.name.local.to_string())
        .unwrap()
}

#[test]
fn an_element_is_created_detached_with_its_name_lowercased() {
    let mut doc = Document::parse_html("<p>x</p>");
    let div = doc.create_element("DiV").unwrap();
    assert_eq!(tag(&doc, div), "div");
    assert_eq!(doc.node(div).unwrap().parent(), None);
    for name in ["", "1a", "a b", "a>b", "a=b", "a/b", "-a"] {
        assert_eq!(
            doc.create_element(name),
            Err(MutationError::InvalidName),
            "{name:?}"
        );
    }
    // Names HTML allows beyond ASCII letters.
    for name in ["my-widget", "x_1", "ns:tag", "çizim"] {
        assert!(doc.create_element(name).is_ok(), "{name:?}");
    }
}

#[test]
fn a_template_gets_its_contents_fragment() {
    let mut doc = Document::parse_html("");
    let template = doc.create_element("template").unwrap();
    let contents = doc
        .node(template)
        .and_then(|node| node.as_element())
        .and_then(|element| element.template_contents())
        .expect("a template has contents");
    assert!(matches!(
        doc.node(contents).map(|node| &node.data),
        Some(NodeData::DocumentFragment)
    ));
}

#[test]
fn insertion_follows_pre_insert_validity() {
    let mut doc = Document::parse_html("<div id=a><div id=b></div></div>");
    let body = body(&doc);
    let a = doc.children(body).next().unwrap();
    let b = doc.children(a).next().unwrap();
    let text = doc.create_text("metin");
    // Into itself or its own descendant.
    assert_eq!(doc.insert(a, a, None), Err(MutationError::Hierarchy));
    assert_eq!(doc.insert(b, a, None), Err(MutationError::Hierarchy));
    // The document node moves nowhere; text does not go into it, nor
    // anything into text.
    let root = doc.root();
    assert_eq!(doc.insert(body, root, None), Err(MutationError::Hierarchy));
    assert_eq!(doc.insert(root, text, None), Err(MutationError::Hierarchy));
    assert_eq!(doc.insert(text, a, None), Err(MutationError::Hierarchy));
    // `before` must be a child of the parent.
    assert_eq!(
        doc.insert(body, text, Some(b)),
        Err(MutationError::Hierarchy)
    );
    // Valid moves: before a child, and appended (no `before`).
    doc.insert(body, text, Some(a)).unwrap();
    assert_eq!(doc.children(body).collect::<Vec<_>>(), [text, a]);
    doc.insert(body, b, None).unwrap();
    assert_eq!(doc.children(body).collect::<Vec<_>>(), [text, a, b]);
    assert_eq!(doc.children(a).count(), 0, "moved, not copied");
    // A node inserted before itself stays where it is.
    doc.insert(body, a, Some(a)).unwrap();
    assert_eq!(doc.children(body).collect::<Vec<_>>(), [text, a, b]);
}

#[test]
fn a_removed_nodes_id_is_stale_everywhere() {
    let mut doc = Document::parse_html("<p>x</p>");
    let body = body(&doc);
    let gone = doc.create_element("span").unwrap();
    assert!(doc.remove(gone), "a detached node can be removed");
    assert_eq!(doc.insert(body, gone, None), Err(MutationError::Stale));
    assert_eq!(doc.insert(gone, body, None), Err(MutationError::Stale));
    assert_eq!(doc.set_attr(gone, "id", "x"), Err(MutationError::Stale));
    assert_eq!(doc.remove_attr(gone, "id"), Err(MutationError::Stale));
    // Its slot goes to the next node, with a new generation.
    let next = doc.create_element("b").unwrap();
    assert_eq!(next.index(), gone.index());
    assert_ne!(next, gone);
}

#[test]
fn attributes_are_set_replaced_and_removed() {
    let mut doc = Document::parse_html(r#"<p id="a" class="x">y</p>"#);
    let body = body(&doc);
    let p = doc.children(body).next().unwrap();
    let attrs = |doc: &Document| -> Vec<(String, String)> {
        doc.node(p)
            .and_then(|node| node.as_element())
            .unwrap()
            .attrs
            .iter()
            .map(|attr| (attr.name.local.to_string(), attr.value.clone()))
            .collect()
    };
    doc.set_attr(p, "CLASS", "x y").unwrap();
    doc.set_attr(p, "data-n", "1").unwrap();
    assert_eq!(
        attrs(&doc),
        [
            ("id".to_owned(), "a".to_owned()),
            ("class".to_owned(), "x y".to_owned()),
            ("data-n".to_owned(), "1".to_owned()),
        ]
    );
    assert_eq!(doc.remove_attr(p, "id"), Ok(true));
    assert_eq!(doc.remove_attr(p, "id"), Ok(false));
    let element = doc.node(p).and_then(|node| node.as_element()).unwrap();
    assert_eq!(element.attr(&LocalName::from("class")), Some("x y"));
    assert_eq!(element.attr(&LocalName::from("id")), None);
    for name in ["", "a b", "a=b", "a>b"] {
        assert_eq!(
            doc.set_attr(p, name, "v"),
            Err(MutationError::InvalidName),
            "{name:?}"
        );
    }
    // Text has no attributes.
    let text = doc.children(p).next().unwrap();
    assert_eq!(doc.set_attr(text, "id", "t"), Err(MutationError::Hierarchy));
}
