//! `text-transform` changes the case of text in the language it is written
//! in (CSS Text 3 §2.1): the nearest `lang` attribute decides, as in
//! Chrome, so Turkish `i` becomes `İ`.

use erk_renderer::render_html;

/// The painted text, its glyph runs joined.
fn painted(body: &str) -> String {
    let html = format!("<style>body {{ margin: 0; width: 600px }}</style>{body}");
    render_html(&html, 640, 120)
        .display_list()
        .lines()
        .filter(|line| line.starts_with("glyphs"))
        .map(|line| {
            let quoted = &line[line.find('"').expect("the run's text")..];
            quoted.trim_matches('"').to_owned()
        })
        .collect()
}

#[test]
fn uppercase_and_lowercase_follow_the_language() {
    assert_eq!(
        painted(r#"<p style="text-transform: uppercase">istanbul ılık</p>"#),
        "ISTANBUL ILIK"
    );
    assert_eq!(
        painted(r#"<p lang="tr" style="text-transform: uppercase">istanbul ılık</p>"#),
        "İSTANBUL ILIK"
    );
    assert_eq!(
        painted(r#"<p lang="tr-TR" style="text-transform: lowercase">IŞIK İÇİN</p>"#),
        "ışık için"
    );
    // Greek capitals drop the accent in Greek only.
    assert_eq!(
        painted(r#"<p lang="el" style="text-transform: uppercase">Γειά σου</p>"#),
        "ΓΕΙΑ ΣΟΥ"
    );
    assert_eq!(
        painted(r#"<p style="text-transform: uppercase">Γειά</p>"#),
        "ΓΕΙΆ"
    );
    // German sharp s widens to two letters.
    assert_eq!(
        painted(r#"<p style="text-transform: uppercase">straße</p>"#),
        "STRASSE"
    );
}

#[test]
fn the_nearest_lang_attribute_decides() {
    assert_eq!(
        painted(
            r#"<div lang="tr"><p style="text-transform: uppercase">iki <span lang="en">is</span> bir</p></div>"#
        ),
        "İKİ IS BİR"
    );
    // An invalid tag is no language.
    assert_eq!(
        painted(r#"<p lang="!!" style="text-transform: uppercase">i</p>"#),
        "I"
    );
}

#[test]
fn capitalize_titlecases_the_first_letter_of_each_word() {
    assert_eq!(
        painted(r#"<p style="text-transform: capitalize">hello wORLD, don't foo-bar 3rd</p>"#),
        "Hello WORLD, Don't Foo-Bar 3rd"
    );
    assert_eq!(
        painted(r#"<p lang="tr" style="text-transform: capitalize">ilk iş</p>"#),
        "İlk İş"
    );
    assert_eq!(
        painted(r#"<p lang="nl" style="text-transform: capitalize">ijsland</p>"#),
        "IJsland"
    );
}

#[test]
fn a_word_continues_across_inline_elements() {
    // Element boundaries are not word boundaries; spaces are.
    assert_eq!(
        painted(
            r#"<p style="text-transform: capitalize"><span>a</span><b>b</b> <span>c</span></p>"#
        ),
        "Ab C"
    );
}

#[test]
fn only_the_transformed_element_changes() {
    assert_eq!(
        painted(r#"<p>one <span style="text-transform: uppercase">two</span> three</p>"#),
        "one TWO three"
    );
    // none on a child turns an inherited transform off.
    assert_eq!(
        painted(
            r#"<p style="text-transform: uppercase">one <span style="text-transform: none">two</span></p>"#
        ),
        "ONE two"
    );
}
