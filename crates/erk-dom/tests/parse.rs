use erk_dom::{Document, NodeData, NodeId, local_name};

/// Render a subtree compactly: `name(child,child)` for elements, `"text"` for
/// text, comments and doctypes skipped.
fn tree(doc: &Document, id: NodeId) -> String {
    let node = doc.node(id).unwrap();
    let children: Vec<String> = doc
        .children(id)
        .filter_map(|child| match &doc.node(child).unwrap().data {
            NodeData::Comment(_) | NodeData::Doctype { .. } => None,
            _ => Some(tree(doc, child)),
        })
        .collect();
    let label = match &node.data {
        NodeData::Text(text) => return format!("{text:?}"),
        NodeData::Element(element) => element.name.local.to_string(),
        NodeData::Document => "#document".to_owned(),
        NodeData::DocumentFragment => "#fragment".to_owned(),
        _ => "?".to_owned(),
    };
    if children.is_empty() {
        label
    } else {
        format!("{label}({})", children.join(","))
    }
}

fn body(html: &str) -> String {
    let doc = Document::parse_html(html);
    let root = tree(&doc, doc.root());
    let start = root.find("body").expect("parsed document has a body");
    root[start..root.len() - 2].to_owned()
}

#[test]
fn implied_html_head_and_body_are_created() {
    let doc = Document::parse_html("<p>Merhaba <b>dünya</b></p>");
    assert_eq!(
        tree(&doc, doc.root()),
        r#"#document(html(head,body(p("Merhaba ",b("dünya")))))"#
    );
}

#[test]
fn adjacent_text_is_merged_into_one_node() {
    let doc = Document::parse_html("<p>a&amp;b</p>");
    let html = doc.children(doc.root()).last().unwrap();
    let body = doc.children(html).last().unwrap();
    let p = doc.children(body).next().unwrap();
    let texts: Vec<_> = doc.children(p).collect();

    assert_eq!(texts.len(), 1);
    assert_eq!(doc.node(texts[0]).unwrap().as_text(), Some("a&b"));
}

#[test]
fn turkish_text_survives_byte_for_byte() {
    let text = "İıŞşĞğÜüÖöÇç";
    assert_eq!(
        body(&format!("<p>{text}</p>")),
        format!("body(p({text:?}))")
    );
}

#[test]
fn table_gets_an_implied_tbody() {
    assert_eq!(
        body("<table><tr><td>x</td></tr></table>"),
        r#"body(table(tbody(tr(td("x")))))"#
    );
}

#[test]
fn misnested_formatting_is_rebuilt_by_the_adoption_agency() {
    // html5lib tree-construction: <b><p>x</b>y</p>
    assert_eq!(body("<b><p>x</b>y</p>"), r#"body(b,p(b("x"),"y"))"#);
}

#[test]
fn foster_parented_text_is_merged_before_the_table() {
    // Text inside <table> but outside a cell is moved in front of the table;
    // both pieces must end up in one text node.
    assert_eq!(
        body("<table>a<tr><td>x</td></tr>b</table>"),
        r#"body("ab",table(tbody(tr(td("x")))))"#
    );
}

#[test]
fn template_contents_live_in_a_separate_fragment() {
    let doc = Document::parse_html("<template><p>x</p></template>");
    let html = doc.children(doc.root()).last().unwrap();
    let head = doc.children(html).next().unwrap();
    let template = doc.children(head).next().unwrap();
    let element = doc.node(template).unwrap().as_element().unwrap();

    assert_eq!(element.name.local, local_name!("template"));
    assert_eq!(
        doc.children(template).count(),
        0,
        "contents are not children"
    );
    let contents = element.template_contents().unwrap();
    assert_eq!(tree(&doc, contents), r#"#fragment(p("x"))"#);
}

#[test]
fn attributes_are_kept() {
    let doc = Document::parse_html(r#"<p id="giriş" class="a b">x</p>"#);
    let html = doc.children(doc.root()).last().unwrap();
    let body = doc.children(html).last().unwrap();
    let p = doc.children(body).next().unwrap();
    let element = doc.node(p).unwrap().as_element().unwrap();

    assert_eq!(element.attr(&local_name!("id")), Some("giriş"));
    assert_eq!(element.attr(&local_name!("class")), Some("a b"));
}
