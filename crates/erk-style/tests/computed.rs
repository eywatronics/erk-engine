use erk_dom::{Document, LocalName, NodeId};
use erk_style::style::values::computed::Display;
use erk_style::style::values::generics::length::GenericMargin;
use erk_style::{ComputedValues, StyleEngine, Styles};

fn style(html: &str) -> (Document, Styles) {
    let doc = Document::parse_html(html);
    let styles = StyleEngine::new(800.0, 600.0).style(&doc);
    (doc, styles)
}

/// The first element with the given tag, in tree order.
fn find(doc: &Document, tag: &str) -> NodeId {
    let tag = LocalName::from(tag);
    let mut stack = vec![doc.root()];
    while let Some(id) = stack.pop() {
        if doc
            .node(id)
            .and_then(|node| node.as_element())
            .is_some_and(|element| element.name.local == tag)
        {
            return id;
        }
        let mut children: Vec<_> = doc.children(id).collect();
        children.reverse();
        stack.extend(children);
    }
    panic!("no <{tag}> in the document");
}

fn computed(
    doc: &Document,
    styles: &Styles,
    tag: &str,
) -> erk_style::style::servo_arc::Arc<ComputedValues> {
    styles
        .computed(find(doc, tag))
        .unwrap_or_else(|| panic!("<{tag}> was not styled"))
}

fn rgb(style: &ComputedValues) -> [f32; 3] {
    let color = style.clone_color();
    [color.components.0, color.components.1, color.components.2]
}

#[test]
fn author_stylesheet_applies() {
    let (doc, styles) = style("<style>p { color: red }</style><p>x</p>");
    assert_eq!(rgb(&computed(&doc, &styles, "p")), [1.0, 0.0, 0.0]);
}

#[test]
fn user_agent_stylesheet_makes_headings_blocks_with_larger_text() {
    let (doc, styles) = style("<h1>Başlık</h1>");
    let h1 = computed(&doc, &styles, "h1");

    assert_eq!(h1.get_box().clone_display(), Display::Block);
    assert_eq!(h1.get_font().clone_font_size().computed_size().px(), 32.0);
}

#[test]
fn style_attribute_applies() {
    let (doc, styles) = style(r#"<p style="margin-top: 7px">x</p>"#);
    let margin = computed(&doc, &styles, "p").clone_margin_top();

    let GenericMargin::LengthPercentage(length) = margin else {
        panic!("margin-top should be a length, got {margin:?}");
    };
    assert_eq!(length.to_length().map(|l| l.px()), Some(7.0));
}

#[test]
fn inherited_properties_flow_down() {
    let (doc, styles) = style("<style>body { color: blue }</style><p>x</p>");
    assert_eq!(rgb(&computed(&doc, &styles, "p")), [0.0, 0.0, 1.0]);
}

#[test]
fn class_and_id_selectors_match() {
    let (doc, styles) = style(
        r#"<style>
            .uyari { color: red }
            #ana { color: lime }
        </style>
        <p class="a uyari">x</p><div id="ana">y</div>"#,
    );
    assert_eq!(rgb(&computed(&doc, &styles, "p")), [1.0, 0.0, 0.0]);
    assert_eq!(rgb(&computed(&doc, &styles, "div")), [0.0, 1.0, 0.0]);
}

#[test]
fn elements_inside_display_none_are_not_styled() {
    let (doc, styles) = style("<div style=\"display: none\"><p>x</p></div>");
    assert!(styles.computed(find(&doc, "div")).is_some());
    assert!(styles.computed(find(&doc, "p")).is_none());
}
