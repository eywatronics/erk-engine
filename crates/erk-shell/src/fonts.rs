//! The demo host's fonts (p1-contract §6.2): the system's, as the platform
//! lists them (DirectWrite, fontconfig, CoreText, through fontique).
//!
//! The renderer gets the catalogue at once: the family names, and what the
//! generic families and each writing system's fallback stand for, which the
//! platform answers without opening a font file. A family's faces are read
//! only when the renderer asks for one (`font:<family>?weight=..&style=..`):
//! listing every face of every family opens every font file and took more
//! than a second at startup.

use std::path::PathBuf;
use std::sync::Mutex;

use erk_renderer::{FontCatalog, GenericFamilies, ScriptFallback};
use fontique::{
    Collection, CollectionOptions, FallbackKey, FamilyId, FontStyle, FontWeight, GenericFamily,
    Language, Script, SourceKind,
};

/// The generic families a page may name, as the catalogue names them.
const GENERICS: [(&str, GenericFamily); 7] = [
    ("serif", GenericFamily::Serif),
    ("sans-serif", GenericFamily::SansSerif),
    ("monospace", GenericFamily::Monospace),
    ("cursive", GenericFamily::Cursive),
    ("fantasy", GenericFamily::Fantasy),
    ("system-ui", GenericFamily::SystemUi),
    ("emoji", GenericFamily::Emoji),
];

/// The writing systems the catalogue lists fallback families for, and the
/// languages that draw some of them with other fonts (Han characters look
/// different in Japanese, Korean and traditional Chinese).
const FALLBACKS: &[(&str, &[&str])] = &[
    ("Arab", &["fa", "ur"]),
    ("Armn", &[]),
    ("Beng", &[]),
    ("Cyrl", &[]),
    ("Deva", &[]),
    ("Ethi", &[]),
    ("Geor", &[]),
    ("Grek", &[]),
    ("Gujr", &[]),
    ("Guru", &[]),
    ("Hang", &[]),
    ("Hani", &["ja", "ko", "zh-TW", "zh-HK"]),
    ("Hebr", &[]),
    ("Hira", &[]),
    ("Kana", &[]),
    ("Khmr", &[]),
    ("Knda", &[]),
    ("Laoo", &[]),
    ("Latn", &[]),
    ("Mlym", &[]),
    ("Mong", &[]),
    ("Mymr", &[]),
    ("Orya", &[]),
    ("Sinh", &[]),
    ("Taml", &[]),
    ("Telu", &[]),
    ("Thaa", &[]),
    ("Thai", &[]),
    ("Tibt", &[]),
];

/// The system's fonts: the catalogue, and the collection that finds the
/// files behind a request.
pub(crate) struct SystemFonts {
    catalogue: FontCatalog,
    collection: Mutex<Collection>,
}

/// A face request, as the renderer writes it.
#[derive(Debug, PartialEq)]
struct FaceRequest {
    family: String,
    weight: f32,
    italic: bool,
}

impl SystemFonts {
    /// Ask the platform for its font families and fallback lists.
    pub(crate) fn scan() -> Self {
        let mut collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts: true,
        });
        let families: Vec<String> = collection.family_names().map(str::to_owned).collect();
        let mut generic = Vec::new();
        for (name, family) in GENERICS {
            let ids: Vec<_> = collection.generic_families(family).collect();
            generic.push(GenericFamilies {
                generic: name.to_owned(),
                families: family_names(&mut collection, ids),
            });
        }
        let mut fallback = Vec::new();
        for &(script, languages) in FALLBACKS {
            let Ok(code) = script.parse::<Script>() else {
                continue;
            };
            for language in std::iter::once("").chain(languages.iter().copied()) {
                let tag = (!language.is_empty())
                    .then(|| Language::parse(language).ok())
                    .flatten();
                let ids: Vec<_> = collection
                    .fallback_families(FallbackKey::new(code, tag.as_ref()))
                    .collect();
                let families = family_names(&mut collection, ids);
                if !families.is_empty() {
                    fallback.push(ScriptFallback {
                        script: script.to_owned(),
                        language: language.to_owned(),
                        families,
                    });
                }
            }
        }
        Self {
            catalogue: FontCatalog {
                families,
                generic,
                fallback,
            },
            collection: Mutex::new(collection),
        }
    }

    pub(crate) fn catalogue(&self) -> &FontCatalog {
        &self.catalogue
    }

    /// The file of the face a `font:` request asks for: the family's face
    /// nearest its weight and style. `None` for anything else.
    pub(crate) fn data(&self, url: &str) -> Option<Vec<u8>> {
        let request = parse(url)?;
        let mut collection = self.collection.lock().ok()?;
        let family = collection.family_by_name(&request.family)?;
        let face = family
            .fonts()
            .iter()
            .min_by(|a, b| {
                let distance = |font: &fontique::FontInfo| {
                    let style = if (font.style() != FontStyle::Normal) == request.italic {
                        0.0
                    } else {
                        10_000.0
                    };
                    style + (font.weight().value() - FontWeight::new(request.weight).value()).abs()
                };
                distance(a).total_cmp(&distance(b))
            })?
            .clone();
        match &face.source().kind {
            SourceKind::Path(path) => std::fs::read(PathBuf::from(&**path)).ok(),
            SourceKind::Memory(blob) => Some(blob.as_ref().to_vec()),
        }
    }
}

/// `font:<family>?weight=<n>&style=<normal|italic>`, the family with `%`,
/// `?`, `&` and `#` percent-encoded.
fn parse(url: &str) -> Option<FaceRequest> {
    let (family, query) = url.strip_prefix("font:")?.split_once('?')?;
    let (mut weight, mut italic) = (None, None);
    for pair in query.split('&') {
        match pair.split_once('=')? {
            ("weight", value) => weight = value.parse::<f32>().ok().filter(|w| w.is_finite()),
            ("style", "normal") => italic = Some(false),
            ("style", "italic") => italic = Some(true),
            _ => return None,
        }
    }
    let family = family
        .replace("%3F", "?")
        .replace("%26", "&")
        .replace("%23", "#")
        .replace("%25", "%");
    (!family.is_empty()).then_some(FaceRequest {
        family,
        weight: weight?,
        italic: italic?,
    })
}

fn family_names(collection: &mut Collection, ids: Vec<FamilyId>) -> Vec<String> {
    ids.into_iter()
        .filter_map(|id| collection.family_name(id).map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_request_is_read_as_the_renderer_writes_it() {
        assert_eq!(
            parse("font:A%26B %231 %3F%25?weight=600&style=italic"),
            Some(FaceRequest {
                family: "A&B #1 ?%".to_owned(),
                weight: 600.0,
                italic: true,
            })
        );
        for url in [
            "font:Arial",
            "font:?weight=400&style=normal",
            "font:Arial?weight=400",
            "font:Arial?weight=heavy&style=normal",
            "font:Arial?weight=NaN&style=normal",
            "font:Arial?weight=400&style=bold",
            "font:Arial?weight=400&style=normal&path=/etc/passwd",
            "file:///etc/passwd",
        ] {
            assert_eq!(parse(url), None, "{url}");
        }
    }

    #[test]
    fn the_system_has_fonts_and_serves_their_faces() {
        let fonts = SystemFonts::scan();
        let catalogue = fonts.catalogue();
        assert!(!catalogue.families.is_empty(), "no system fonts found");
        // Generic families name families the catalogue has.
        for generic in &catalogue.generic {
            for family in &generic.families {
                assert!(
                    catalogue.families.contains(family),
                    "{}: {family} is not in the catalogue",
                    generic.generic
                );
            }
        }
        let family = catalogue
            .generic
            .iter()
            .find(|generic| generic.generic == "sans-serif")
            .and_then(|generic| generic.families.first())
            .unwrap_or(&catalogue.families[0]);
        let url = format!("font:{family}?weight=400&style=normal");
        let data = fonts.data(&url).expect("a family's face is served");
        assert!(data.len() > 12, "{url} has no font data");
        assert!(
            fonts
                .data("font:No Such Family Anywhere?weight=400&style=normal")
                .is_none()
        );
    }

    #[test]
    fn the_face_nearest_the_weight_and_style_is_served() {
        // Some family has separate regular, italic and bold files (Arial on
        // Windows, DejaVu Sans on Linux): each request gets its own.
        let fonts = SystemFonts::scan();
        let face = |family: &str, weight: u16, style: &str| {
            fonts.data(&format!("font:{family}?weight={weight}&style={style}"))
        };
        let found = fonts.catalogue().families.iter().any(|family| {
            let regular = face(family, 400, "normal");
            let italic = face(family, 400, "italic");
            let bold = face(family, 700, "normal");
            regular.is_some() && regular != italic && regular != bold && italic != bold
                // Weight 600 is nearer 700 than 400.
                && face(family, 600, "normal") == bold
        });
        assert!(
            found,
            "no family with separate regular, italic and bold faces"
        );
    }
}
