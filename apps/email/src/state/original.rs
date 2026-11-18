//! The HTML original, as the browser gets it.

use std::path::{Path, PathBuf};

//
// "Open original" hands the sender's HTML to a browser, for the mail whose layout the reading
// pane's text cannot carry. The file it hands over is cleaned first, and every decision about
// that cleaning is here, where `tests/email-core` can exercise it without a window.

/// Take out of `html` every `<img>` that would send a request over the network, and say how
/// many went.
///
/// Opening tracking: notification mail is full of images that show nothing and exist so that
/// the sender learns the message was opened, when, and from which address — and a browser will
/// happily report all of it for a click that only meant "let me see the layout". Pictures the
/// mail carries inline (`cid:`, `data:`) never leave the machine and stay.
///
/// This is a scanner, not an HTML parser: it finds `<img` tags, reads their `src` and `srcset`,
/// and drops the tag when either points at the network. That is enough for the one job this
/// has — no pixel of the sender's may fetch — and a tag too malformed for the scanner to read
/// is a tag a browser will not render either.
pub fn strip_remote_images(html: &str) -> (String, usize) {
    let chars: Vec<char> = html.chars().collect();
    let mut out = String::with_capacity(html.len());
    let mut removed = 0;
    let mut i = 0;
    while i < chars.len() {
        if !is_img_tag_start(&chars, i) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let end = img_tag_end(&chars, i);
        let tag: String = chars[i..end].iter().collect();
        if img_is_remote(&tag) {
            removed += 1;
        } else {
            out.push_str(&tag);
        }
        i = end;
    }
    (out, removed)
}

/// True at the `<` of an `<img` tag, case-insensitively, and not at `<image` or `<imgx`: what
/// follows the name has to end it — a space, the tag's own `>`, or a `/`.
fn is_img_tag_start(chars: &[char], i: usize) -> bool {
    if chars[i] != '<' {
        return false;
    }
    let name = &chars[i + 1..std::cmp::min(i + 4, chars.len())];
    if !name.iter().zip("img".chars()).all(|(a, b)| a.to_ascii_lowercase() == b) || name.len() < 3
    {
        return false;
    }
    match chars.get(i + 4) {
        None => true,
        Some(c) => c.is_ascii_whitespace() || *c == '>' || *c == '/',
    }
}

/// The index just past the `>` that ends the tag started at `i`. A `>` inside a quoted
/// attribute value does not end a tag; a tag that never ends runs to the end of the input.
fn img_tag_end(chars: &[char], i: usize) -> usize {
    let mut quote = None;
    let mut j = i + 1;
    while j < chars.len() {
        let c = chars[j];
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '>' => return j + 1,
                _ => {}
            },
        }
        j += 1;
    }
    chars.len()
}

/// Whether a tag points a picture at the network, through `src` or `srcset`.
fn img_is_remote(tag: &str) -> bool {
    if attr_value(tag, "src").is_some_and(|src| fetches_from_network(&src)) {
        return true;
    }
    // A srcset is a comma-separated list of "url descriptor" candidates; each is a fetch the
    // browser may make, so the tag goes if any candidate is remote.
    attr_value(tag, "srcset").is_some_and(|set| {
        set.split(',')
            .any(|candidate| candidate.split_whitespace().next().is_some_and(fetches_from_network))
    })
}

/// Whether a URL would leave this machine: an http(s) address, or a protocol-relative `//host/…`,
/// which is http(s) too once the page has a scheme. `cid:` and `data:` would not.
fn fetches_from_network(url: &str) -> bool {
    let url = url.trim().to_lowercase();
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("//")
}

/// The value of one attribute of a tag — quoted or bare — or `None` when it does not carry one.
/// Only an attribute beginning at a boundary counts, so a lazy-load `data-src` placeholder,
/// which by itself fetches nothing, does not decide the tag's fate.
fn attr_value(tag: &str, name: &str) -> Option<String> {
    let chars: Vec<char> = tag.chars().collect();
    let needle: Vec<char> = format!("{name}=").chars().collect();
    let mut i = 0;
    while i + needle.len() <= chars.len() {
        let at_boundary = i == 0 || chars[i - 1].is_ascii_whitespace();
        let matches = chars[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b);
        if at_boundary && matches {
            let mut j = i + needle.len();
            while j < chars.len() && chars[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= chars.len() {
                return None;
            }
            return match chars[j] {
                q @ ('"' | '\'') => {
                    let end =
                        (j + 1..chars.len()).find(|k| chars[*k] == q).unwrap_or(chars.len());
                    Some(chars[j + 1..end].iter().collect())
                }
                _ => {
                    let end = (j..chars.len())
                        .find(|k| chars[*k].is_ascii_whitespace() || chars[*k] == '>')
                        .unwrap_or(chars.len());
                    Some(chars[j..end].iter().collect())
                }
            };
        }
        i += 1;
    }
    None
}

/// The file one message's original is written to, inside `dir`.
///
/// The id came off a wire, and what comes off a wire never becomes a filename as it stands:
/// everything but letters, digits, `-` and `_` is dropped, so no path separator and no `..`
/// segment can survive, and an id that was all punctuation leaves the word "message".
pub fn original_html_file(dir: &Path, message_id: &str) -> PathBuf {
    let name: String = message_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let name = if name.is_empty() { "message".to_string() } else { name };
    dir.join(format!("{name}.html"))
}

/// What the notice says after the browser was handed the file, including the part that has to
/// be said: what was taken out of the mail before it went anywhere near a network.
pub fn original_opened_note(removed: usize) -> String {
    match removed {
        0 => "Opened the original HTML in the browser. It had no remote images to remove."
            .to_string(),
        1 => "Opened the original HTML in the browser with 1 remote image removed, so the \
              sender is not told this was opened."
            .to_string(),
        n => format!(
            "Opened the original HTML in the browser with {n} remote images removed, so the \
             sender is not told this was opened."
        ),
    }
}
