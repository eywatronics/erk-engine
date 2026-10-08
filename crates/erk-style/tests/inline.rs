//! One property of an inline style, read and written as CSSOM does
//! (M5.3).

use erk_style::{InvalidProperty, remove_style_property, set_style_property, style_property};

#[test]
fn a_property_is_set_beside_the_others_and_replaces_itself() {
    let style = set_style_property("", "color", "red").unwrap();
    assert_eq!(style, "color: red;");
    let style = set_style_property(&style, "width", "10px").unwrap();
    assert_eq!(style, "color: red; width: 10px;");
    // Set again: one declaration, with the new value.
    let style = set_style_property(&style, "color", "blue").unwrap();
    assert_eq!(style, "color: blue; width: 10px;");
}

#[test]
fn a_value_holding_a_semicolon_survives() {
    let style = r#"background-image: url("a;b.png"); color: blue"#;
    let style = set_style_property(style, "color", "red").unwrap();
    assert!(style.contains(r#"url("a;b.png")"#), "{style}");
    assert_eq!(
        style_property(&style, "color").unwrap().as_deref(),
        Some("red")
    );
}

#[test]
fn a_shorthand_sets_and_removes_its_longhands() {
    let style = set_style_property("", "margin", "1px 2px").unwrap();
    assert_eq!(
        style_property(&style, "margin-left").unwrap().as_deref(),
        Some("2px")
    );
    assert_eq!(
        style_property(&style, "margin").unwrap().as_deref(),
        Some("1px 2px")
    );
    let style = remove_style_property(&style, "margin").unwrap();
    assert_eq!(style, "");
}

#[test]
fn a_property_is_removed_and_an_empty_value_removes_it() {
    let style = "color: red; width: 10px";
    assert_eq!(
        remove_style_property(style, "color").unwrap(),
        "width: 10px;"
    );
    assert_eq!(
        set_style_property(style, "width", "").unwrap(),
        "color: red;"
    );
    // Not there: nothing changes.
    assert_eq!(
        remove_style_property(style, "height").unwrap(),
        "color: red; width: 10px;"
    );
    assert_eq!(style_property(style, "height").unwrap(), None);
}

#[test]
fn custom_properties_are_properties() {
    let style = set_style_property("", "--accent", "#0a0").unwrap();
    assert_eq!(
        style_property(&style, "--accent").unwrap().as_deref(),
        Some("#0a0")
    );
}

#[test]
fn an_unknown_name_or_a_wrong_value_is_an_error_and_changes_nothing() {
    assert_eq!(
        set_style_property("", "colour", "red"),
        Err(InvalidProperty)
    );
    assert_eq!(set_style_property("", "width", "red"), Err(InvalidProperty));
    // `!important` is not part of a value.
    assert_eq!(
        set_style_property("", "width", "1px !important"),
        Err(InvalidProperty)
    );
    assert_eq!(remove_style_property("", "colour"), Err(InvalidProperty));
    assert_eq!(style_property("", "colour"), Err(InvalidProperty));
}
