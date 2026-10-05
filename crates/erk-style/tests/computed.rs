use erk_dom::{Document, LocalName, NodeId};
use erk_style::style::values::computed::Display;
use erk_style::style::values::generics::length::GenericMargin;
use erk_style::{ComputedValues, Interaction, StyleEngine, Styles};

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
fn bgcolor_sets_the_background_colour() {
    let (doc, styles) = style(r##"<table bgcolor="#336699"><tr><td>x</td></tr></table>"##);
    let table = computed(&doc, &styles, "table");
    let colour = table.resolve_color(&table.get_background().background_color);
    let [r, g, b] = [
        colour.components.0,
        colour.components.1,
        colour.components.2,
    ];
    assert_eq!(
        [r, g, b].map(|c| (c * 255.0).round() as u8),
        [0x33, 0x66, 0x99]
    );
}

#[test]
fn align_sets_text_align() {
    use erk_style::style::values::computed::TextAlign;
    let (doc, styles) = style(r#"<p align="center">x</p><div align="right">y</div>"#);
    let align = |tag| {
        computed(&doc, &styles, tag)
            .get_inherited_text()
            .clone_text_align()
    };
    // Styled only: text-align is not laid out until inline layout (M1).
    assert_eq!(align("p"), TextAlign::MozCenter);
    assert_eq!(align("div"), TextAlign::MozRight);
}

#[test]
fn elements_inside_display_none_are_not_styled() {
    let (doc, styles) = style("<div style=\"display: none\"><p>x</p></div>");
    assert!(styles.computed(find(&doc, "div")).is_some());
    assert!(styles.computed(find(&doc, "p")).is_none());
}

#[test]
fn css_custom_properties_resolve() {
    // Utility-first CSS such as Tailwind leans on custom properties.
    let (doc, styles) = style(
        "<style>:root { --vurgu: rgb(0, 51, 255) } p { color: var(--vurgu) }</style><p>x</p>",
    );
    let [r, g, b] = rgb(&computed(&doc, &styles, "p"));
    assert_eq!([r, g, b].map(|c| (c * 255.0).round() as u8), [0, 51, 255]);
}

#[test]
fn resolution_media_queries_see_the_device_scale() {
    let html = "<style>p { color: blue } @media (min-resolution: 2dppx) { p { color: red } }</style><p>x</p>";
    let doc = Document::parse_html(html);
    let at = |scale: f32| {
        let styles = StyleEngine::new(400.0, 300.0)
            .with_device_scale(scale)
            .style(&doc);
        rgb(&computed(&doc, &styles, "p"))
    };
    assert_eq!(at(1.0), [0.0, 0.0, 1.0]);
    assert_eq!(at(2.0), [1.0, 0.0, 0.0]);
}

#[test]
fn the_viewport_stays_in_css_pixels_at_any_scale() {
    // 400 CSS pixels wide, whatever the device pixels.
    let html =
        "<style>p { color: blue } @media (max-width: 400px) { p { color: red } }</style><p>x</p>";
    let doc = Document::parse_html(html);
    let styles = StyleEngine::new(400.0, 300.0)
        .with_device_scale(2.0)
        .style(&doc);
    assert_eq!(rgb(&computed(&doc, &styles, "p")), [1.0, 0.0, 0.0]);
}

const STATES: &str = "<style>
    section:hover, section:active, section:focus-within { color: rgb(0, 0, 255) }
    p:hover { color: rgb(255, 0, 0) }
    p:active { color: rgb(0, 255, 0) }
    p:focus { color: rgb(255, 255, 0) }
    </style><section><article><p>x</p></article><aside>y</aside></section>";

fn style_with(interaction: impl Fn(&Document) -> Interaction) -> (Document, Styles) {
    let doc = Document::parse_html(STATES);
    let styles = StyleEngine::new(800.0, 600.0).style_with(&doc, &interaction(&doc));
    (doc, styles)
}

#[test]
fn hover_active_and_focus_match_the_element_the_user_points_at() {
    let black = [0.0, 0.0, 0.0];
    let (doc, styles) = style_with(|_| Interaction::default());
    assert_eq!(rgb(&computed(&doc, &styles, "p")), black);
    assert_eq!(rgb(&computed(&doc, &styles, "section")), black);

    for (state, colour) in [
        ("hover", [1.0, 0.0, 0.0]),
        ("active", [0.0, 1.0, 0.0]),
        ("focus", [1.0, 1.0, 0.0]),
    ] {
        let (doc, styles) = style_with(|doc| {
            let p = Some(find(doc, "p"));
            match state {
                "hover" => Interaction {
                    hover: p,
                    ..Default::default()
                },
                "active" => Interaction {
                    active: p,
                    ..Default::default()
                },
                _ => Interaction {
                    focus: p,
                    ..Default::default()
                },
            }
        });
        assert_eq!(rgb(&computed(&doc, &styles, "p")), colour, "{state}");
        // The state reaches the ancestors (`:focus-within` for the focus)...
        assert_eq!(
            rgb(&computed(&doc, &styles, "section")),
            [0.0, 0.0, 1.0],
            "{state}"
        );
        // ...and not a sibling.
        assert_eq!(
            rgb(&computed(&doc, &styles, "aside")),
            [0.0, 0.0, 1.0],
            "{state}: inherits from section"
        );
    }

    // The sibling under the pointer: its parent hovers, the paragraph does not.
    let (doc, styles) = style_with(|doc| Interaction {
        hover: Some(find(doc, "aside")),
        ..Default::default()
    });
    assert_eq!(rgb(&computed(&doc, &styles, "section")), [0.0, 0.0, 1.0]);
    assert_eq!(
        rgb(&computed(&doc, &styles, "p")),
        [0.0, 0.0, 1.0],
        "inherits; not :hover"
    );
}

#[test]
fn a_node_that_is_not_in_the_document_puts_no_element_in_a_state() {
    // The paragraph's slot one generation on: a node removed and replaced.
    let stale = |doc: &Document| NodeId::from_bits(find(doc, "p").to_bits() + (1 << 32));
    let (doc, styles) = style_with(|doc| Interaction {
        hover: stale(doc),
        active: stale(doc),
        focus: stale(doc),
    });
    for tag in ["section", "article", "p", "aside"] {
        assert_eq!(rgb(&computed(&doc, &styles, tag)), [0.0, 0.0, 0.0], "{tag}");
    }
}

#[test]
fn styles_say_which_states_their_selectors_depend_on() {
    let doc = Document::parse_html(STATES);
    let p = Some(find(&doc, "p"));
    let none = Interaction::default();
    let styles = StyleEngine::new(800.0, 600.0).style(&doc);
    for changed in [
        Interaction { hover: p, ..none },
        Interaction { active: p, ..none },
        Interaction { focus: p, ..none },
    ] {
        assert!(styles.react_to(&none, &changed), "{changed:?}");
        assert!(!styles.react_to(&changed, &changed), "{changed:?}");
    }

    let doc = Document::parse_html("<style>p:hover { color: red }</style><p>x</p>");
    let p = Some(find(&doc, "p"));
    let styles = StyleEngine::new(800.0, 600.0).style(&doc);
    assert!(styles.react_to(&none, &Interaction { hover: p, ..none }));
    assert!(!styles.react_to(&none, &Interaction { active: p, ..none }));
    assert!(!styles.react_to(&none, &Interaction { focus: p, ..none }));

    let doc = Document::parse_html("<p>x</p>");
    let p = Some(find(&doc, "p"));
    let styles = StyleEngine::new(800.0, 600.0).style(&doc);
    let all = Interaction {
        hover: p,
        active: p,
        focus: p,
    };
    assert!(
        !styles.react_to(&none, &all),
        "the UA stylesheet has no state rules"
    );
}
