//! One property of an element's inline style, as CSSOM's
//! `element.style.setProperty`, `removeProperty` and `getPropertyValue`
//! read and write it (M5.3).
//!
//! Splitting the `style` attribute's text at `;` would be wrong for values
//! that hold one (`url("a;b")`, a string in `content`), so the text is
//! parsed into Stylo's declaration block, changed there and written back
//! as Stylo serializes it. The result is a new `style` attribute: the host
//! sets it like any other, and the next frame restyles the element.

use style::context::QuirksMode;
use style::properties::declaration_block::SourcePropertyDeclarationUpdate;
use style::properties::{
    Importance, PropertyDeclarationBlock, PropertyId, SourcePropertyDeclaration,
    parse_one_declaration_into, parse_style_attribute,
};
use style::stylesheets::{CssRuleType, Origin, UrlExtraData};
use style_traits::ParsingMode;

/// A property name Erk does not know, or a value the property does not
/// take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidProperty;

fn url() -> UrlExtraData {
    UrlExtraData::from(url::Url::parse("about:blank").expect("valid URL"))
}

fn block(style: &str, url: &UrlExtraData) -> PropertyDeclarationBlock {
    parse_style_attribute(style, url, None, QuirksMode::NoQuirks, CssRuleType::Style)
}

fn property(name: &str) -> Result<PropertyId, InvalidProperty> {
    PropertyId::parse_enabled_for_all_content(name).map_err(|_| InvalidProperty)
}

fn serialize(block: &PropertyDeclarationBlock) -> String {
    let mut css = String::new();
    block
        .to_css(&mut css)
        .expect("writing to a String does not fail");
    css
}

/// The inline style `style` with property `name` set to `value`: a
/// shorthand sets its longhands, an empty value removes the property, and
/// a value it does not take changes nothing and is an error.
pub fn set_style_property(style: &str, name: &str, value: &str) -> Result<String, InvalidProperty> {
    if value.trim().is_empty() {
        return remove_style_property(style, name);
    }
    let id = property(name)?;
    let url = url();
    let mut parsed = SourcePropertyDeclaration::default();
    parse_one_declaration_into(
        &mut parsed,
        id,
        value,
        Origin::Author,
        &url,
        None,
        ParsingMode::DEFAULT,
        QuirksMode::NoQuirks,
        CssRuleType::Style,
    )
    .map_err(|_| InvalidProperty)?;
    // As CSSOM's "set a CSS declaration": a declaration already there
    // changes where it is.
    let mut block = block(style, &url);
    let mut updates = SourcePropertyDeclarationUpdate::default();
    if block.prepare_for_update(&parsed, Importance::Normal, &mut updates) {
        block.update(parsed.drain(), Importance::Normal, &mut updates);
    }
    Ok(serialize(&block))
}

/// The inline style `style` without property `name` (a shorthand takes
/// its longhands with it).
pub fn remove_style_property(style: &str, name: &str) -> Result<String, InvalidProperty> {
    let id = property(name)?;
    let mut block = block(style, &url());
    if let Some(first) = block.first_declaration_to_remove(&id) {
        block.remove_property(&id, first);
    }
    Ok(serialize(&block))
}

/// Property `name`'s value in the inline style `style`, serialized; `None`
/// when it is not set (or, for a shorthand, not all its longhands are).
pub fn style_property(style: &str, name: &str) -> Result<Option<String>, InvalidProperty> {
    let id = property(name)?;
    let mut value = String::new();
    block(style, &url())
        .property_value_to_css(&id, &mut value)
        .expect("writing to a String does not fail");
    Ok((!value.is_empty()).then_some(value))
}
