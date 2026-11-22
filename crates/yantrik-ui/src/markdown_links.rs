//! Where a link goes, shown before it is pressed. The links are found by the parser Slint itself
//! reads markdown with (`pulldown-cmark`, the same options), so the destination read here is the
//! one `link-clicked` hands on; the host is read by the `url` parser the opener uses
//! (`open_url::normalized`). Shown host and opened host are one parse of one string.

use pulldown_cmark::{Event, LinkType, Options, Parser, Tag, TagEnd};
use url::Url;

/// A link's destination as a web address: `http` or `https` with a host. Anything else is not
/// one, and is not opened.
fn web_link(dest: &str) -> Option<Url> {
    let url = Url::parse(dest.trim()).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then_some(url)
}

/// What the person is shown of a web address: its host, and its port when it is not the default
/// (an IDN is punycode, an IPv6 host keeps its brackets).
pub(crate) fn host_label(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// Backslash-escape the marks markdown reads, so text put back into markdown is read as itself.
fn escaped(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        if "\\`*_[]()<>&!#~|{}+".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

fn same_address(text: &str, dest: &str) -> bool {
    text.trim().trim_end_matches('/') == dest.trim().trim_end_matches('/')
}

/// `markdown` with each link's real target written after it: `[Guide](https://docs.example.com/x)`
/// reads "Guide (docs.example.com)". A link whose words are its own address, and an autolink to a
/// web address, are left as they are. A link that is not a web address is not a link any more: its
/// words stay as plain text, followed by "(not a web link)". Images and code are untouched.
///
/// The note goes in at the byte where the parser says the link ends, never inside its
/// destination, so a `)` in the address, an escaped `]`, a reference link or `<...>` all hold.
/// Not muted: `StyledText` colours a span only with a literal colour, which the theme tokens rule out.
pub(crate) fn show_link_targets(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH).into_offset_iter();
    let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // The open link: its range, destination, kind, and the words inside it.
    let mut open: Option<(std::ops::Range<usize>, String, LinkType, String)> = None;
    for (event, range) in parser {
        match event {
            Event::Start(Tag::Link { link_type, dest_url, .. }) => {
                open = Some((range, dest_url.to_string(), link_type, String::new()));
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, _, _, words)) = open.as_mut() {
                    words.push_str(&t);
                }
            }
            Event::End(TagEnd::Link) => {
                let Some((range, dest, kind, words)) = open.take() else { continue };
                match web_link(&dest) {
                    Some(url) => {
                        let own = matches!(kind, LinkType::Autolink) || same_address(&words, &dest);
                        if !own {
                            let end = range.end;
                            edits.push((end..end, format!(" ({})", escaped(&host_label(&url)))));
                        }
                    }
                    None => edits.push((range, format!("{} (not a web link)", escaped(&words)))),
                }
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(markdown.len());
    let mut at = 0;
    for (range, text) in edits {
        out.push_str(&markdown[at..range.start]);
        out.push_str(&text);
        at = range.end;
    }
    out.push_str(&markdown[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The destinations the parser reads from `markdown`: what `link-clicked` hands on.
    fn destinations(markdown: &str) -> Vec<String> {
        Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH)
            .filter_map(|e| match e {
                Event::Start(Tag::Link { dest_url, .. }) => Some(dest_url.to_string()),
                _ => None,
            })
            .collect()
    }

    /// What is shown, and that it is the host of what opens: the note equals the host of the
    /// serialized address `open_url::normalized` hands to the opener.
    fn check(markdown: &str, note: &str) {
        let shown = show_link_targets(markdown);
        assert!(shown.contains(&format!(" ({})", escaped(note))), "{markdown:?} -> {shown:?}");
        assert_eq!(destinations(&shown), destinations(markdown), "the link still goes where it did: {shown:?}");
        let opened = crate::open_url::normalized(&destinations(markdown)[0]).expect("a web link opens");
        let opened = Url::parse(&opened).unwrap();
        assert_eq!(host_label(&opened), note, "shown host is the opened host");
    }

    #[test]
    fn the_host_shown_is_the_host_that_opens() {
        check("[Guide](https://docs.example.com/x?a=1)", "docs.example.com");
        // The deceptive link of the review: the text is one place, the target another.
        check("see [https://bank.example](http://other.example) now", "other.example");
        // `\\` in markdown is one backslash, which a browser reads as `/`: the host is evil.example.
        check("[Guide](https://evil.example\\\\@bank.example)", "evil.example");
        // `\@` is an escaped `@`: userinfo, the host is bank.example.
        check("[Guide](https://evil.example\\@bank.example)", "bank.example");
        check("[a\\]b](http://evil.example)", "evil.example");
        check("[a](https://x.com/a_(b))", "x.com");
        check("[t][r]\n\n[r]: https://r.example", "r.example");
        check("[t](<https://a.example/x>)", "a.example");
        check("[idn](http://bücher.example/)", "xn--bcher-kva.example");
        check("[v6](http://[::1]:8080/)", "[::1]:8080");
        check("[p](https://h.example:8443/a)", "h.example:8443");
    }

    #[test]
    fn a_paren_in_the_address_leaves_the_link_whole() {
        assert_eq!(show_link_targets("[a](https://x.com/a_(b))"), "[a](https://x.com/a_(b)) (x.com)");
        assert_eq!(show_link_targets("[a\\]b](http://evil.example) end"), "[a\\]b](http://evil.example) (evil.example) end");
    }

    #[test]
    fn a_link_that_is_its_own_address_has_no_note() {
        for same in [
            "[https://a.example/](https://a.example)",
            "<https://a.example>",
            "https://a.example/x",
            "![pic](https://a.example/p.png)",
            "`[x](https://a.example)`",
            "no link [here] (at all)",
        ] {
            assert_eq!(show_link_targets(same), same);
        }
    }

    #[test]
    fn a_target_that_is_not_a_web_link_is_plain_text_with_a_note() {
        for bad in ["javascript:alert(1)", "file:///etc/passwd"] {
            let shown = show_link_targets(&format!("see [click me]({bad}) now"));
            assert!(shown.contains("(not a web link)"), "{shown}");
            assert!(destinations(&shown).is_empty(), "not clickable: {shown}");
            assert!(crate::open_url::normalized(bad).is_err());
        }
        let auto = show_link_targets("<javascript:alert(1)>");
        assert!(destinations(&auto).is_empty() && auto.contains("(not a web link)"), "{auto}");
    }

    #[test]
    fn two_links_each_get_their_own_note() {
        assert_eq!(
            show_link_targets("[a](https://one.example) and [b](https://two.example:81)"),
            "[a](https://one.example) (one.example) and [b](https://two.example:81) (two.example:81)"
        );
    }
}
