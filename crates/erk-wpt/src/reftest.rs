//! What a WPT reftest file says about itself: its references and the fuzzy
//! tolerance it allows. Read from the markup with a small attribute
//! scanner; a reftest's `<link>` and `<meta>` elements are plain enough.
//! The rules are WPT's (docs/writing-tests/reftests.md).

/// How a test relates to one of its references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// `rel=match`: must render identically.
    Match,
    /// `rel=mismatch`: must not.
    Mismatch,
}

/// One reference of a test: the relation and the `href` as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub relation: Relation,
    pub href: String,
}

/// A `<meta name=fuzzy>` allowance: inclusive ranges for the largest
/// difference on any channel and for the number of differing pixels,
/// optionally for one reference only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fuzzy {
    pub reference: Option<String>,
    pub max_difference: (u8, u8),
    pub total_pixels: (u64, u64),
}

/// The references of `markup`, in document order. Empty for a file that is
/// not a reftest.
pub fn references(markup: &str) -> Vec<Reference> {
    elements(markup, "link")
        .into_iter()
        .filter_map(|attributes| {
            let rel = attribute(&attributes, "rel")?.to_ascii_lowercase();
            let relation = match rel.trim() {
                "match" => Relation::Match,
                "mismatch" => Relation::Mismatch,
                _ => return None,
            };
            let href = attribute(&attributes, "href")?;
            Some(Reference {
                relation,
                href: href.to_owned(),
            })
        })
        .collect()
}

/// The fuzzy allowances of `markup`.
pub fn fuzzy(markup: &str) -> Vec<Fuzzy> {
    elements(markup, "meta")
        .into_iter()
        .filter(|attributes| {
            attribute(attributes, "name").is_some_and(|name| name.eq_ignore_ascii_case("fuzzy"))
        })
        .filter_map(|attributes| parse_fuzzy(attribute(&attributes, "content")?))
        .collect()
}

/// `[url:]maxDifference=a-b;totalPixels=c-d`, names optional, a single
/// number meaning exactly that number.
fn parse_fuzzy(content: &str) -> Option<Fuzzy> {
    let (reference, ranges) = match content.rsplit_once(':') {
        Some((url, ranges)) => (Some(url.trim().to_owned()), ranges),
        None => (None, content),
    };
    let mut parts = ranges.split(';').map(|part| {
        let value = part.split_once('=').map_or(part, |(_, value)| value);
        range(value.trim())
    });
    let (difference, pixels) = (parts.next()??, parts.next()??);
    Some(Fuzzy {
        reference,
        max_difference: (
            u8::try_from(difference.0.min(255)).ok()?,
            u8::try_from(difference.1.min(255)).ok()?,
        ),
        total_pixels: pixels,
    })
}

fn range(value: &str) -> Option<(u64, u64)> {
    match value.split_once('-') {
        Some((low, high)) => Some((low.trim().parse().ok()?, high.trim().parse().ok()?)),
        None => {
            let exact = value.parse().ok()?;
            Some((exact, exact))
        }
    }
}

/// The attribute text of every `<name ...>` start tag, lowercase names.
fn elements(markup: &str, name: &str) -> Vec<String> {
    let lower = markup.to_ascii_lowercase();
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = lower[from..].find(&open) {
        let start = from + start + open.len();
        // `<linkfoo` is another element.
        if !lower[start..].starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>') {
            from = start;
            continue;
        }
        let Some(end) = tag_end(&markup[start..]) else {
            break;
        };
        found.push(markup[start..start + end].to_owned());
        from = start + end;
    }
    found
}

/// Where the tag that `rest` is inside ends: the first `>` outside a quoted
/// attribute value, so `title="a > b"` does not end it early. A quote opens
/// a value only right after `=` (spaces allowed); one inside an unquoted
/// value (`title=it's`) is part of it.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    let mut after_equals = false;
    for (index, c) in rest.char_indices() {
        match (quote, c) {
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') if after_equals => quote = Some(c),
            (None, '>') => return Some(index),
            _ => {}
        }
        if quote.is_none() && !c.is_whitespace() {
            after_equals = c == '=';
        }
    }
    None
}

/// The value of attribute `name` in a start tag's attribute text: quoted
/// with either quote, or unquoted.
fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let bytes = attributes.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && (bytes[index].is_ascii_whitespace() || bytes[index] == b'/') {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && bytes[index] != b'='
            && bytes[index] != b'/'
        {
            index += 1;
        }
        let key = &attributes[key_start..index];
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let mut value = "";
        if index < bytes.len() && bytes[index] == b'=' {
            index += 1;
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && (bytes[index] == b'"' || bytes[index] == b'\'') {
                let quote = bytes[index];
                let value_start = index + 1;
                index = value_start;
                while index < bytes.len() && bytes[index] != quote {
                    index += 1;
                }
                value = &attributes[value_start..index];
                index += 1;
            } else {
                let value_start = index;
                while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                    index += 1;
                }
                value = &attributes[value_start..index];
            }
        }
        if key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
        if key.is_empty() {
            index += 1;
        }
    }
    None
}

/// What XML parsing would do to an XHTML test that an HTML parser does not:
/// read CDATA sections (see `resolve_cdata`; inside `<style>` an HTML parser
/// would keep the markers as text and lose the first rule to them), and
/// close self-closing elements that are not void (`<div/>` is an empty div
/// in XML, an open one in HTML). Erk has no XML parser; this lets XHTML
/// tests test layout rather than parsing.
pub fn xhtml_as_html(markup: &str) -> String {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source",
        "track", "wbr",
    ];
    let markup = resolve_cdata(markup);
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup.as_str();
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        // A comment is copied whole: an apostrophe in it is not a quote.
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(rest.len(), |end| end + 3);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        // Only an element tag can self-close; a declaration, a processing
        // instruction or a stray `<` in text is copied as it is.
        if !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '/') {
            out.push('<');
            rest = &rest[1..];
            continue;
        }
        let Some(close) = tag_end(rest) else {
            break;
        };
        let tag = &rest[..=close];
        let name: String = tag[1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
            .collect();
        if tag.ends_with("/>")
            && !name.is_empty()
            && !VOID.contains(&name.to_ascii_lowercase().as_str())
        {
            out.push_str(&tag[..tag.len() - 2]);
            out.push_str("></");
            out.push_str(&name);
            out.push('>');
        } else {
            out.push_str(tag);
        }
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The next `token` in `lower` from `from`. A tag name must end there
/// (`<styles>` is not `<style`); a CDATA marker needs no boundary.
fn find_token(lower: &str, from: usize, token: &str) -> Option<usize> {
    let mut start = from;
    while let Some(at) = lower[start..].find(token) {
        let at = start + at;
        let after = lower[at + token.len()..].chars().next();
        if token.starts_with("<![")
            || after.is_none_or(|c| c.is_whitespace() || c == '>' || c == '/')
        {
            return Some(at);
        }
        start = at + token.len();
    }
    None
}

/// CDATA sections as XML reads them. Inside `<style>` or `<script>`, whose
/// content an HTML parser keeps as raw text, only the markers go. Anywhere
/// else the section is text, so its content is escaped: dropping the
/// markers there would turn `<![CDATA[<b>]]>` into an element. A CDATA
/// marker written inside a CSS string is not told apart (not seen in the
/// suites Erk runs).
fn resolve_cdata(markup: &str) -> String {
    let lower = markup.to_ascii_lowercase();
    let mut out = String::with_capacity(markup.len());
    let mut raw_text: Option<&str> = None;
    let mut index = 0;
    while index < markup.len() {
        // The next thing that changes what a CDATA section means.
        let next = ["<![cdata[", "<style", "</style", "<script", "</script"]
            .iter()
            .filter_map(|token| find_token(&lower, index, token).map(|at| (at, *token)))
            .min_by_key(|(at, _)| *at);
        let Some((at, token)) = next else {
            break;
        };
        out.push_str(&markup[index..at]);
        match token {
            "<![cdata[" => {
                let content_start = at + token.len();
                let end = lower[content_start..]
                    .find("]]>")
                    .map_or(markup.len(), |end| content_start + end);
                let content = &markup[content_start..end];
                if raw_text.is_some() {
                    out.push_str(content);
                } else {
                    out.push_str(
                        &content
                            .replace('&', "&amp;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;"),
                    );
                }
                index = (end + 3).min(markup.len());
                continue;
            }
            "<style" => raw_text = Some("style"),
            "<script" => raw_text = Some("script"),
            _ => raw_text = None,
        }
        out.push_str(&markup[at..at + token.len()]);
        index = at + token.len();
    }
    if index < markup.len() {
        out.push_str(&markup[index..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_read_in_any_quoting() {
        let markup = r#"<link rel="match" href="a-ref.html"><LINK REL=mismatch HREF='b.html'/>
            <link rel="stylesheet" href="x.css"><link rel="help" href="h">"#;
        assert_eq!(
            references(markup),
            vec![
                Reference {
                    relation: Relation::Match,
                    href: "a-ref.html".to_owned()
                },
                Reference {
                    relation: Relation::Mismatch,
                    href: "b.html".to_owned()
                },
            ]
        );
    }

    #[test]
    fn fuzzy_takes_names_ranges_and_references() {
        let markup = r#"<meta name="fuzzy" content="maxDifference=0-5; totalPixels=0-80">
            <meta name=fuzzy content="15;300"><meta name="fuzzy" content="option-ref.html:10-15;200-300">"#;
        assert_eq!(
            fuzzy(markup),
            vec![
                Fuzzy {
                    reference: None,
                    max_difference: (0, 5),
                    total_pixels: (0, 80)
                },
                Fuzzy {
                    reference: None,
                    max_difference: (15, 15),
                    total_pixels: (300, 300)
                },
                Fuzzy {
                    reference: Some("option-ref.html".to_owned()),
                    max_difference: (10, 15),
                    total_pixels: (200, 300)
                },
            ]
        );
    }

    #[test]
    fn a_quoted_greater_than_does_not_end_a_tag() {
        let markup = r#"<link title="a > b" rel="match" href="ref.html"><meta name="fuzzy" content="x>y:0-1;0-2">"#;
        assert_eq!(
            references(markup),
            vec![Reference {
                relation: Relation::Match,
                href: "ref.html".to_owned()
            }]
        );
        assert_eq!(fuzzy(markup).len(), 1);
        assert_eq!(
            xhtml_as_html(r#"<div title="a > b"/>"#),
            r#"<div title="a > b"></div>"#
        );
    }

    #[test]
    fn a_quote_inside_an_unquoted_value_opens_nothing() {
        let markup = "<link title=it's rel=match href=ref.html><p>x</p>";
        assert_eq!(references(markup).len(), 1);
        assert_eq!(
            xhtml_as_html("<div title=it's/><p class='a'/>"),
            "<div title=it's></div><p class='a'></p>"
        );
    }

    #[test]
    fn a_longer_tag_name_is_not_style() {
        // `<styles>` must not start raw text: the CDATA after it is body text.
        assert_eq!(
            xhtml_as_html("<styles><![CDATA[<b>]]></styles>"),
            "<styles>&lt;b&gt;</styles>"
        );
    }

    #[test]
    fn an_apostrophe_in_a_comment_is_not_a_quote() {
        assert_eq!(
            xhtml_as_html("<!-- don't --><div/><p>it's</p><span/>"),
            "<!-- don't --><div></div><p>it's</p><span></span>"
        );
    }

    #[test]
    fn cdata_outside_style_and_script_is_text() {
        assert_eq!(
            xhtml_as_html("<p><![CDATA[<b> & </b>]]></p><script><![CDATA[a<b]]></script>"),
            "<p>&lt;b&gt; &amp; &lt;/b&gt;</p><script>a<b</script>"
        );
    }

    #[test]
    fn xhtml_cdata_and_self_closing_tags_become_html() {
        let xhtml = "<style><![CDATA[ p { color: red } ]]></style><div/><br/><p class=\"a\"/>";
        assert_eq!(
            xhtml_as_html(xhtml),
            "<style> p { color: red } </style><div></div><br/><p class=\"a\"></p>"
        );
    }
}
