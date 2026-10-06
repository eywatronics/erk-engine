//! Fonts from the host (p1-contract §6.2).
//!
//! The host scans the system's fonts and sends a catalogue: every face with
//! the URL it serves it under, what the generic families stand for, and the
//! families that draw each writing system. A document's faces are asked for
//! like images, as `ResourceKind::Font` requests, and kept across
//! documents: the font of one page is usually the font of the next.
//!
//! What a document needs is decided here, before shaping, the way Parley
//! will choose when it shapes: for each text, the first family of its
//! `font-family` the catalogue has, in the face nearest its weight and
//! style; and for each character the embedded Noto Sans cannot draw, the
//! fallback family of its script (in the text's language when the
//! catalogue has a list for it), or the emoji family. Without a catalogue
//! nothing is asked for and everything is drawn in Noto Sans.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use erk_dom::{Document, NodeData, NodeId, local_name};
use erk_style::style::values::computed::font::{
    FontStyle as CssFontStyle, GenericFontFamily, SingleFontFamily,
};
use erk_style::{ComputedValues, Styles};
use icu_properties::props::{EmojiPresentation, Script};
use icu_properties::{CodePointMapData, CodePointSetData, PropertyNamesShort};
use parley::fontique::{Blob, FallbackKey, GenericFamily, Language, Script as FontScript};
use skrifa::MetadataProvider;

use crate::messages::{FontCatalog, ScriptFallback};
use crate::text::NOTO_SANS_REGULAR;

/// A font family as CSS names it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Family {
    Named(String),
    Generic(GenericFamily),
}

/// The `font-family` list of `style`.
pub(crate) fn families(style: &ComputedValues) -> Arc<[Family]> {
    style
        .get_font()
        .font_family
        .families
        .iter()
        .filter_map(|family| match family {
            SingleFontFamily::FamilyName(name) => Some(Family::Named(name.name.to_string())),
            SingleFontFamily::Generic(generic) => generic_of(*generic).map(Family::Generic),
        })
        .collect()
}

fn generic_of(generic: GenericFontFamily) -> Option<GenericFamily> {
    Some(match generic {
        GenericFontFamily::Serif => GenericFamily::Serif,
        GenericFontFamily::SansSerif => GenericFamily::SansSerif,
        GenericFontFamily::Monospace => GenericFamily::Monospace,
        GenericFontFamily::Cursive => GenericFamily::Cursive,
        GenericFontFamily::Fantasy => GenericFamily::Fantasy,
        GenericFontFamily::SystemUi => GenericFamily::SystemUi,
        GenericFontFamily::None => return None,
    })
}

/// The generic family a catalogue's name stands for.
pub(crate) fn generic_named(name: &str) -> Option<GenericFamily> {
    Some(match name {
        "serif" => GenericFamily::Serif,
        "sans-serif" => GenericFamily::SansSerif,
        "monospace" => GenericFamily::Monospace,
        "cursive" => GenericFamily::Cursive,
        "fantasy" => GenericFamily::Fantasy,
        "system-ui" => GenericFamily::SystemUi,
        "emoji" => GenericFamily::Emoji,
        _ => return None,
    })
}

/// Whether `style` asks for an italic or oblique face.
pub(crate) fn is_italic(style: &ComputedValues) -> bool {
    style.clone_font_style() != CssFontStyle::NORMAL
}

/// The language a `lang` tag names, as fontique keys fallback lists.
pub(crate) fn language(tag: &str) -> Option<Language> {
    (!tag.is_empty())
        .then(|| Language::parse(tag).ok())
        .flatten()
}

/// The list of `catalogue` that draws `script` in `language`: the list for
/// that language if the catalogue has one (and fontique keys the pair),
/// else the list for any language. The same choice [`crate::text`] makes
/// when it tells Parley the text's language.
pub(crate) fn fallback_for<'a>(
    catalogue: &'a FontCatalog,
    script: &str,
    language: Option<&Language>,
) -> Option<&'a ScriptFallback> {
    let font_script = FontScript::from_str(script).ok()?;
    if let Some(entry) = language_fallback(catalogue, font_script, language) {
        return Some(entry);
    }
    catalogue
        .fallback
        .iter()
        .find(|entry| entry.script == script && entry.language.is_empty())
}

/// The language-specific list for `script` in `language`, if the catalogue
/// has one that fontique would look up for that pair.
pub(crate) fn language_fallback<'a>(
    catalogue: &'a FontCatalog,
    script: FontScript,
    text_language: Option<&Language>,
) -> Option<&'a ScriptFallback> {
    let key = FallbackKey::new(script, text_language);
    if !key.is_tracked() || key.is_default() {
        return None;
    }
    catalogue.fallback.iter().find(|entry| {
        FontScript::from_str(&entry.script).ok() == Some(script)
            && language(&entry.language)
                .is_some_and(|tag| FallbackKey::new(script, Some(&tag)).locale() == key.locale())
    })
}

/// Whether some list of `catalogue` is specific to `language`: then Parley
/// is told the text's language, and fontique looks up those lists.
pub(crate) fn has_language_fallback(catalogue: &FontCatalog, language: &Language) -> bool {
    catalogue.fallback.iter().any(|entry| {
        FontScript::from_str(&entry.script)
            .is_ok_and(|script| language_fallback(catalogue, script, Some(language)).is_some())
    })
}

enum Face {
    Pending,
    Ready(Blob<u8>),
    Missing,
}

/// The host's catalogue and the faces that have arrived.
#[derive(Default)]
pub(crate) struct HostFonts {
    catalogue: Option<FontCatalog>,
    faces: HashMap<String, Face>,
}

impl HostFonts {
    pub(crate) fn set_catalogue(&mut self, catalogue: FontCatalog) {
        self.catalogue = Some(catalogue);
    }

    pub(crate) fn catalogue(&self) -> Option<&FontCatalog> {
        self.catalogue.as_ref()
    }

    /// The faces that have arrived, as Parley registers them.
    pub(crate) fn loaded(&self) -> impl Iterator<Item = &Blob<u8>> {
        self.faces.values().filter_map(|face| match face {
            Face::Ready(blob) => Some(blob),
            _ => None,
        })
    }

    pub(crate) fn is_known(&self, url: &str) -> bool {
        self.faces.contains_key(url)
    }

    pub(crate) fn requested(&mut self, url: &str) {
        self.faces.insert(url.to_owned(), Face::Pending);
    }

    /// The host's answer for `url`: kept if it is a font, missing if not,
    /// and then why.
    pub(crate) fn complete(&mut self, url: &str, mime: &str, data: &[u8]) -> Result<(), String> {
        let (face, result) = match validate(mime) {
            Ok(()) => (Face::Ready(Blob::new(Arc::new(data.to_vec()))), Ok(())),
            Err(reason) => (Face::Missing, Err(reason)),
        };
        self.faces.insert(url.to_owned(), face);
        result
    }

    pub(crate) fn missing(&mut self, url: &str) {
        self.faces.insert(url.to_owned(), Face::Missing);
    }

    pub(crate) fn pending(&self) -> bool {
        self.faces
            .values()
            .any(|face| matches!(face, Face::Pending))
    }

    /// The URLs of the faces `doc` uses, in document order, each once.
    pub(crate) fn wanted(&self, doc: &Document, styles: &Styles) -> Vec<String> {
        let Some(catalogue) = &self.catalogue else {
            return Vec::new();
        };
        let embedded = skrifa::FontRef::new(NOTO_SANS_REGULAR)
            .expect("the embedded font parses")
            .charmap();
        let scripts = CodePointMapData::<Script>::new();
        let script_names = PropertyNamesShort::<Script>::new();
        let emoji = CodePointSetData::new::<EmojiPresentation>();
        let mut urls: Vec<String> = Vec::new();
        let mut add = |url: Option<String>| {
            if let Some(url) = url
                && !urls.contains(&url)
            {
                urls.push(url);
            }
        };

        let mut stack = vec![doc.root()];
        while let Some(id) = stack.pop() {
            let mut children: Vec<_> = doc.children(id).collect();
            children.reverse();
            stack.extend(children);
            let Some(NodeData::Text(text)) = doc.node(id).map(|node| &node.data) else {
                continue;
            };
            let Some(parent) = doc.node(id).and_then(|node| node.parent()) else {
                continue;
            };
            let Some(style) = styles.computed(parent) else {
                continue;
            };
            if text.chars().all(char::is_whitespace) {
                continue;
            }
            let (weight, italic) = (style.clone_font_weight().value(), is_italic(&style));
            let style_families = families(&style);
            let primary = style_families
                .iter()
                .find_map(|family| catalogue_family(catalogue, family));
            add(primary.map(|family| face_url(family, weight, italic)));

            let language = language(&language_of(doc, parent));
            for c in text.chars() {
                if embedded.map(c).is_some() || c.is_whitespace() {
                    continue;
                }
                let fallback: Option<&[String]> = if emoji.contains(c) {
                    catalogue
                        .generic
                        .iter()
                        .find(|generic| generic.generic == "emoji")
                        .map(|generic| generic.families.as_slice())
                } else {
                    script_names
                        .get_locale_script(scripts.get(c))
                        .and_then(|script| {
                            fallback_for(catalogue, script.as_str(), language.as_ref())
                        })
                        .map(|entry| entry.families.as_slice())
                };
                let family = fallback
                    .into_iter()
                    .flatten()
                    .find(|name| has_family(catalogue, name));
                add(family.map(|family| face_url(family, weight, italic)));
            }
        }
        urls
    }
}

/// The catalogue's family for a CSS family: a named family it has, or the
/// first family it has that a generic family stands for.
fn catalogue_family<'a>(catalogue: &'a FontCatalog, family: &'a Family) -> Option<&'a str> {
    match family {
        Family::Named(name) => catalogue
            .families
            .iter()
            .find(|known| known.eq_ignore_ascii_case(name))
            .map(String::as_str),
        Family::Generic(generic) => catalogue
            .generic
            .iter()
            .filter(|entry| generic_named(&entry.generic) == Some(*generic))
            .flat_map(|entry| &entry.families)
            .find(|name| has_family(catalogue, name))
            .map(String::as_str),
    }
}

fn has_family(catalogue: &FontCatalog, name: &str) -> bool {
    catalogue
        .families
        .iter()
        .any(|known| known.eq_ignore_ascii_case(name))
}

/// The URL a face is asked for by (see [`FontCatalog`]): the host picks
/// the family's face nearest the weight and style.
pub(crate) fn face_url(family: &str, weight: f32, italic: bool) -> String {
    let mut encoded = String::with_capacity(family.len());
    for c in family.chars() {
        match c {
            '%' => encoded.push_str("%25"),
            '?' => encoded.push_str("%3F"),
            '&' => encoded.push_str("%26"),
            '#' => encoded.push_str("%23"),
            c => encoded.push(c),
        }
    }
    let style = if italic { "italic" } else { "normal" };
    format!("font:{encoded}?weight={weight}&style={style}")
}

/// The value of the nearest `lang` attribute at or above `id`.
pub(crate) fn language_of(doc: &Document, id: NodeId) -> String {
    let mut node = Some(id);
    while let Some(current) = node {
        let lang = doc
            .node(current)
            .and_then(|node| node.as_element())
            .and_then(|element| element.attr(&local_name!("lang")));
        if let Some(lang) = lang {
            return lang.trim().to_owned();
        }
        node = doc.node(current).and_then(|node| node.parent());
    }
    String::new()
}

/// Whether a response may carry a font: an empty MIME type or a font one.
/// Bytes that are no font are not refused here: fontique registers nothing
/// from them, and the text falls back as if the font had never come.
fn validate(mime: &str) -> Result<(), String> {
    let mime = mime.split(';').next().unwrap_or("").trim();
    if !matches!(
        mime,
        "" | "font/ttf" | "font/otf" | "font/collection" | "font/sfnt" | "application/font-sfnt"
    ) {
        return Err(format!("{mime} is not a font type"));
    }
    Ok(())
}
