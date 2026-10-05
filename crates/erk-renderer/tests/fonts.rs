//! Fonts from the host (p1-contract §6.2): the host sends a catalogue of the
//! fonts it can provide, the renderer asks for the faces a document uses and
//! draws with them; the embedded Noto Sans remains the last fallback.
//!
//! The served font is tests/fonts/ErkTest.ttf: `x`, `中`, `文` and `א`, each a
//! solid square 0.8 em wide on a 1 em advance. At 20px with no margins the
//! first square covers x 2..18, the second 22..38, the line's top 0..16.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use erk_renderer::{
    FontCatalog, Frame, FromRenderer, GenericFamilies, ResourceKind, ResourceRequest,
    ResourceResponse, ScriptFallback, ToRenderer, spawn,
};

const ERK_TEST: &[u8] = include_bytes!("fonts/ErkTest.ttf");
const PATIENCE: Duration = Duration::from_secs(60);
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];

/// The URL a regular face of `family` is asked for by.
const ERK_TEST_URL: &str = "font:Erk Test?weight=400&style=normal";

fn catalogue() -> FontCatalog {
    FontCatalog {
        families: vec!["Erk Test".to_owned()],
        generic: Vec::new(),
        fallback: Vec::new(),
    }
}

fn page(style: &str, text: &str) -> String {
    format!(
        "<body style=\"margin: 0\"><div style=\"font-size: 20px; line-height: 1; color: black; {style}\">{text}</div>"
    )
}

/// Serves ErkTest.ttf for every font request.
fn erk_test(request: &ResourceRequest) -> Option<ResourceResponse> {
    (request.kind == ResourceKind::Font).then(|| ResourceResponse {
        id: request.id,
        mime: "font/ttf".to_owned(),
        data: ERK_TEST.to_vec(),
    })
}

/// A host session: the catalogue (if any), then each page in turn, every
/// request answered by `serve`. The first complete frame of each page, and
/// every request the renderer made.
struct Host {
    to: std::sync::mpsc::Sender<ToRenderer>,
    from: Receiver<FromRenderer>,
    renderer: Option<std::thread::JoinHandle<()>>,
    requests: Vec<ResourceRequest>,
}

impl Host {
    fn new(fonts: Option<FontCatalog>) -> Self {
        let (to, from, renderer) = spawn();
        if let Some(fonts) = fonts {
            to.send(ToRenderer::Fonts(fonts)).unwrap();
        }
        to.send(ToRenderer::Resize {
            width: 120,
            height: 40,
        })
        .unwrap();
        Self {
            to,
            from,
            renderer: Some(renderer),
            requests: Vec::new(),
        }
    }

    fn show(
        &mut self,
        html: &str,
        serve: &dyn Fn(&ResourceRequest) -> Option<ResourceResponse>,
    ) -> Frame {
        self.to
            .send(ToRenderer::Load {
                html: html.to_owned(),
            })
            .unwrap();
        loop {
            match self.from.recv_timeout(PATIENCE) {
                Ok(FromRenderer::Resources(requests)) => {
                    for request in requests {
                        let answer = match serve(&request) {
                            Some(response) => ToRenderer::Resource(response),
                            None => ToRenderer::ResourceMissing { id: request.id },
                        };
                        self.to.send(answer).unwrap();
                        self.requests.push(request);
                    }
                }
                Ok(FromRenderer::Frame(frame)) if !frame.resources_pending() => break frame,
                Ok(_) => {}
                Err(error) => panic!("no complete frame: {error:?}"),
            }
        }
    }

    fn font_urls(&self) -> Vec<&str> {
        self.requests
            .iter()
            .filter(|request| request.kind == ResourceKind::Font)
            .map(|request| request.url.as_str())
            .collect()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.to.send(ToRenderer::Shutdown);
        if let Some(renderer) = self.renderer.take() {
            renderer.join().unwrap();
        }
    }
}

fn rgb(frame: &Frame, x: usize, y: usize) -> [u8; 3] {
    let i = (y * usize::from(frame.width()) + x) * 4;
    [frame.rgba()[i], frame.rgba()[i + 1], frame.rgba()[i + 2]]
}

/// Whether the text's first two characters were drawn as Erk Test's squares.
fn two_squares(frame: &Frame) -> bool {
    rgb(frame, 10, 8) == BLACK && rgb(frame, 20, 8) == WHITE && rgb(frame, 30, 8) == BLACK
}

/// Whether two of Erk Test's squares (16 × 16 solid pixels each) were drawn
/// anywhere: right-to-left text does not start at the left yet.
fn two_squares_anywhere(frame: &Frame) -> bool {
    let solid = (0..usize::from(frame.height()))
        .flat_map(|y| (0..usize::from(frame.width())).map(move |x| (x, y)))
        .filter(|&(x, y)| rgb(frame, x, y) == BLACK)
        .count();
    solid >= 2 * 15 * 15
}

#[test]
fn a_named_family_from_the_catalogue_draws_the_text() {
    let mut host = Host::new(Some(catalogue()));
    let frame = host.show(&page("font-family: 'Erk Test'", "xx"), &erk_test);
    assert_eq!(host.font_urls(), [ERK_TEST_URL]);
    assert!(two_squares(&frame), "{}", frame.display_list());
}

#[test]
fn without_a_catalogue_every_family_is_the_embedded_font() {
    let mut host = Host::new(None);
    let frame = host.show(&page("font-family: 'Erk Test'", "xx"), &erk_test);
    assert!(host.font_urls().is_empty());
    assert!(!two_squares(&frame));
    let embedded = host.show(&page("font-family: 'Noto Sans'", "xx"), &erk_test);
    assert_eq!(frame.rgba(), embedded.rgba());
}

#[test]
fn a_family_the_page_does_not_use_is_not_requested() {
    let mut fonts = catalogue();
    fonts.families.push("Unused".to_owned());
    let mut host = Host::new(Some(fonts));
    // A family the catalogue lacks is skipped, the first it has is used.
    host.show(
        &page("font-family: 'Missing', 'erk test', 'Unused'", "xx"),
        &erk_test,
    );
    assert_eq!(host.font_urls(), [ERK_TEST_URL]);
}

#[test]
fn a_face_is_asked_for_by_family_weight_and_style() {
    let mut fonts = catalogue();
    fonts.families.push("A&B #1 ?%".to_owned());
    let mut host = Host::new(Some(fonts));
    host.show(
        &page(
            "font-family: 'Erk Test'; font-weight: 600",
            "x<i>x</i><span style=\"font-family: 'A&B #1 ?%'\">x</span>",
        ),
        &erk_test,
    );
    assert_eq!(
        host.font_urls(),
        [
            "font:Erk Test?weight=600&style=normal",
            "font:Erk Test?weight=600&style=italic",
            "font:A%26B %231 %3F%25?weight=600&style=normal",
        ]
    );
}

#[test]
fn a_generic_family_maps_through_the_catalogue() {
    let fonts = FontCatalog {
        generic: vec![GenericFamilies {
            generic: "monospace".to_owned(),
            families: vec!["Missing".to_owned(), "Erk Test".to_owned()],
        }],
        ..catalogue()
    };
    let mut host = Host::new(Some(fonts));
    let frame = host.show(&page("font-family: monospace", "xx"), &erk_test);
    assert!(two_squares(&frame), "{}", frame.display_list());
}

#[test]
fn characters_the_font_lacks_fall_back_by_script() {
    let fonts = FontCatalog {
        fallback: vec![
            ScriptFallback {
                script: "Hani".to_owned(),
                language: String::new(),
                families: vec!["Erk Test".to_owned()],
            },
            ScriptFallback {
                script: "Hebr".to_owned(),
                language: String::new(),
                families: vec!["Erk Test".to_owned()],
            },
            // Noto Sans draws Latin: this list is never needed.
            ScriptFallback {
                script: "Latn".to_owned(),
                language: String::new(),
                families: vec!["Latin Fallback".to_owned()],
            },
        ],
        families: vec!["Erk Test".to_owned(), "Latin Fallback".to_owned()],
        ..catalogue()
    };
    let mut host = Host::new(Some(fonts));
    // Noto Sans has neither glyph: without the fallback they are its hollow
    // missing-glyph boxes.
    let han = host.show(&page("font-family: 'Noto Sans'", "中文"), &erk_test);
    assert!(two_squares(&han), "{}", han.display_list());
    let hebrew = host.show(&page("", "אא"), &erk_test);
    assert!(two_squares_anywhere(&hebrew), "{}", hebrew.display_list());
    // Japanese text with no Japanese list uses the list for any language.
    let japanese = host.show(&page("", r#"<span lang="ja">中文</span> ab"#), &erk_test);
    assert!(two_squares(&japanese), "{}", japanese.display_list());
    // One face, asked for once although two scripts and three pages use it.
    assert_eq!(host.font_urls(), [ERK_TEST_URL]);

    let mut without = Host::new(Some(catalogue()));
    let tofu = without.show(&page("", "中文"), &erk_test);
    assert!(!two_squares_anywhere(&tofu));
    let tofu = without.show(&page("", "אא"), &erk_test);
    assert!(!two_squares_anywhere(&tofu));
    assert!(without.font_urls().is_empty());
}

#[test]
fn the_text_language_selects_its_fallback() {
    // Han characters in Japanese text use the Japanese list.
    let fonts = FontCatalog {
        fallback: vec![
            ScriptFallback {
                script: "Hani".to_owned(),
                language: String::new(),
                families: vec!["Missing".to_owned()],
            },
            ScriptFallback {
                script: "Hani".to_owned(),
                language: "ja".to_owned(),
                families: vec!["Erk Test".to_owned()],
            },
        ],
        ..catalogue()
    };
    let mut host = Host::new(Some(fonts));
    let japanese = host.show(&page("", r#"<span lang="ja">中文</span>"#), &erk_test);
    assert!(two_squares(&japanese), "{}", japanese.display_list());
    let other = host.show(&page("", r#"<span lang="tr">中文</span>"#), &erk_test);
    assert!(!two_squares(&other));
}

#[test]
fn a_response_that_is_not_a_font_is_refused() {
    let mut host = Host::new(Some(catalogue()));
    let garbage = |request: &ResourceRequest| {
        Some(ResourceResponse {
            id: request.id,
            mime: "font/ttf".to_owned(),
            data: b"\x00\x01\x00\x00 not really a font".to_vec(),
        })
    };
    let frame = host.show(&page("font-family: 'Erk Test'", "xx"), &garbage);
    assert!(!two_squares(&frame));
    // A font sent as an image is refused too.
    let mut host = Host::new(Some(catalogue()));
    let as_image = |request: &ResourceRequest| {
        Some(ResourceResponse {
            mime: "image/png".to_owned(),
            ..erk_test(request)?
        })
    };
    let frame = host.show(&page("font-family: 'Erk Test'", "xx"), &as_image);
    assert!(!two_squares(&frame));
}

#[test]
fn emoji_are_drawn_with_the_emoji_family() {
    let fonts = FontCatalog {
        generic: vec![GenericFamilies {
            generic: "emoji".to_owned(),
            families: vec!["Erk Test".to_owned()],
        }],
        ..catalogue()
    };
    let mut host = Host::new(Some(fonts));
    let frame = host.show(&page("font-family: 'Noto Sans'", "😀😀"), &erk_test);
    assert!(two_squares(&frame), "{}", frame.display_list());
    assert_eq!(host.font_urls(), [ERK_TEST_URL]);
}

#[test]
fn a_nonsense_catalogue_renders_without_panicking() {
    let fallback = |script: &str, language: &str, families: &[&str]| ScriptFallback {
        script: script.to_owned(),
        language: language.to_owned(),
        families: families.iter().map(|family| (*family).to_owned()).collect(),
    };
    let fonts = FontCatalog {
        families: vec![String::new(), "Erk Test".to_owned(), "\u{0}".repeat(3)],
        generic: vec![
            GenericFamilies {
                generic: "no-such-generic".to_owned(),
                families: vec!["Erk Test".to_owned()],
            },
            GenericFamilies {
                generic: "sans-serif".to_owned(),
                families: Vec::new(),
            },
        ],
        fallback: vec![
            fallback("Hani", "", &[]),
            fallback("Xxxxxxxx", "", &["Erk Test"]),
            fallback("", "", &["Erk Test"]),
            fallback("Hani", "!!", &["Erk Test"]),
            fallback("Arab", "fa-IR-x-private", &["Erk Test"]),
            fallback("Hebr", "", &["Missing", "", "Erk Test"]),
        ],
    };
    let mut host = Host::new(Some(fonts));
    let text = "中文 مرحبا אא 😀 ñ x\u{301} \u{10FFFF} \u{E000}";
    for style in [
        "",
        "font-family: sans-serif",
        "font-family: ''",
        "font-style: italic; font-weight: 1",
    ] {
        host.show(&page(style, text), &erk_test);
        host.show(
            &format!("<p lang=\"fa\">{text}</p><p lang=\"zh-Hant-TW\">{text}</p>"),
            &erk_test,
        );
    }
}

#[test]
fn a_language_without_its_own_list_uses_the_list_for_any_language() {
    // Yiddish is a language fontique keys Hebrew fallback by; told it with
    // no Yiddish list in the catalogue, fontique would find nothing.
    let fonts = FontCatalog {
        fallback: vec![ScriptFallback {
            script: "Hebr".to_owned(),
            language: String::new(),
            families: vec!["Erk Test".to_owned()],
        }],
        ..catalogue()
    };
    let mut host = Host::new(Some(fonts));
    let frame = host.show(&page("", r#"<span lang="yi">אא</span>"#), &erk_test);
    assert!(two_squares_anywhere(&frame), "{}", frame.display_list());
}
