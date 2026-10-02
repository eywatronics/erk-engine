//! `text-transform`'s case mapping (CSS Text 3 §2.1), in the language the
//! text is written in: Turkish and Azerbaijani `i` uppercases to `İ`, Dutch
//! `ij` titlecases to `IJ`, Greek loses its accents in capitals. The mapping
//! is ICU4X's, as Chrome's is ICU's.
//!
//! A text without a language is mapped with the root rules. Chrome falls back
//! to the browser's own locale there; Erk does not read the machine's
//! locale, so the same page transforms the same everywhere.

use std::borrow::Cow;

use erk_style::style::values::specified::text::TextTransformCase;
use icu_casemap::CaseMapper;
use icu_casemap::options::{LeadingAdjustment, TitlecaseOptions, TrailingCase};
use icu_locale_core::LanguageIdentifier;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup};

/// The language of a `lang` attribute's value; the root language for an
/// empty or invalid one (HTML: an invalid tag is an unknown language).
pub(crate) fn language(tag: &str) -> LanguageIdentifier {
    LanguageIdentifier::try_from_str(tag.trim()).unwrap_or(LanguageIdentifier::UNKNOWN)
}

/// `text` with its case transformed. `previous` is the character before it
/// in the paragraph, if any: a word that started there continues here, and
/// is not capitalized again.
pub(crate) fn transform<'a>(
    text: &'a str,
    case: TextTransformCase,
    lang: &LanguageIdentifier,
    previous: Option<char>,
) -> Cow<'a, str> {
    let mapper = CaseMapper::new();
    match case {
        TextTransformCase::None => Cow::Borrowed(text),
        TextTransformCase::Uppercase => mapper.uppercase_to_string(text, lang),
        TextTransformCase::Lowercase => mapper.lowercase_to_string(text, lang),
        TextTransformCase::Capitalize => capitalize(text, lang, previous),
    }
}

/// The first letter of each word titlecased, the rest unchanged. Words
/// follow the Unicode word boundaries closely enough for text: letters,
/// marks, digits and connectors, joined across an apostrophe or a full
/// stop between letters (`don't`, `e.g`), and split by anything else
/// (`foo-bar` is two words). A word starting with a digit keeps it: `3rd`.
fn capitalize<'a>(
    text: &'a str,
    lang: &LanguageIdentifier,
    previous: Option<char>,
) -> Cow<'a, str> {
    let mapper = CaseMapper::new();
    let mut options = TitlecaseOptions::default();
    options.trailing_case = Some(TrailingCase::Unchanged);
    options.leading_adjustment = Some(LeadingAdjustment::None);

    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len());
    let mut in_word = previous.is_some_and(is_word_char);
    let mut before = previous;
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if !continues_word(before, c, chars.get(i + 1).map(|&(_, next)| next), in_word) {
            out.push(c);
            in_word = false;
            before = Some(c);
            i += 1;
            continue;
        }
        if in_word {
            out.push(c);
            before = Some(c);
            i += 1;
            continue;
        }
        // A word starts here: titlecase it as a whole, so that a digraph
        // (Dutch `ij`) is seen together.
        let mut end = i + 1;
        let mut last = c;
        while end < chars.len()
            && continues_word(
                Some(last),
                chars[end].1,
                chars.get(end + 1).map(|&(_, n)| n),
                true,
            )
        {
            last = chars[end].1;
            end += 1;
        }
        let stop = chars.get(end).map_or(text.len(), |&(offset, _)| offset);
        out.push_str(&mapper.titlecase_segment_with_only_case_data_to_string(
            &text[start..stop],
            lang,
            options,
        ));
        in_word = true;
        before = Some(last);
        i = end;
    }
    Cow::Owned(out)
}

/// Whether `c` belongs to a word: to the current one when `in_word`.
fn continues_word(before: Option<char>, c: char, next: Option<char>, in_word: bool) -> bool {
    is_word_char(c)
        || (in_word
            && matches!(c, '\'' | '\u{2019}' | '.' | ':' | '\u{b7}')
            && before.is_some_and(is_letter)
            && next.is_some_and(is_letter))
}

fn is_word_char(c: char) -> bool {
    let category = CodePointMapData::<GeneralCategory>::new().get(c);
    [
        GeneralCategoryGroup::Letter,
        GeneralCategoryGroup::Mark,
        GeneralCategoryGroup::Number,
        GeneralCategoryGroup::ConnectorPunctuation,
    ]
    .iter()
    .any(|group| group.contains(category))
}

fn is_letter(c: char) -> bool {
    GeneralCategoryGroup::Letter.contains(CodePointMapData::<GeneralCategory>::new().get(c))
}
