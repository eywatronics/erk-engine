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
        let Some(end) = markup[start..].find('>') else {
            break;
        };
        found.push(markup[start..start + end].to_owned());
        from = start + end;
    }
    found
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
/// drop `<![CDATA[` and `]]>` markers (an HTML parser keeps them inside
/// `<style>` as text, and the first rule is lost to them), and close
/// self-closing elements that are not void (`<div/>` is an empty div in
/// XML, an open one in HTML). Erk has no XML parser; this lets XHTML tests
/// test layout rather than parsing.
pub fn xhtml_as_html(markup: &str) -> String {
    const VOID: &[&str] = &[
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source",
        "track", "wbr",
    ];
    let markup = markup.replace("<![CDATA[", "").replace("]]>", "");
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup.as_str();
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let Some(close) = rest.find('>') else {
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
    fn xhtml_cdata_and_self_closing_tags_become_html() {
        let xhtml = "<style><![CDATA[ p { color: red } ]]></style><div/><br/><p class=\"a\"/>";
        assert_eq!(
            xhtml_as_html(xhtml),
            "<style> p { color: red } </style><div></div><br/><p class=\"a\"></p>"
        );
    }
}
