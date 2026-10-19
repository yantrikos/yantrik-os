//! A URL as it may be drawn on a screen, put in a `describe` field, a notification, a card, an
//! approval sentence, an error a mind reads, or a log line: its scheme, host and path, never the
//! sign-in details, query or fragment that can carry a key.
//!
//! This is the one rule. Every place in the workspace that shows an address it did not choose
//! itself (a saved provider's base URL, an account's address, a backend URL an agent set, a
//! mind's own line as it attached) comes through here, directly or through
//! `yantrik_app_runtime::place`, which re-exports it. It lives in a crate of its own, with no
//! dependencies, because the harness host, `yantrik-ml` and the gateway cannot depend on the app
//! runtime; the review of #856 found three separate copies of "the host of a URL" drawing
//! `user:p` for `https://user:p/w@host/v1`.
//!
//! Decisions (is this address local, which catalogue provider is it, where does a request go)
//! do not use this: they read the address as it is. What is drawn is never a guess that could
//! show part of a key, so for an address whose sign-in details cannot be told from its host the
//! answer is empty, and the caller says so in words.

/// A URL as it may be drawn: `https://user:key@aig.example/v1?api-key=k#x` →
/// "https://aig.example/v1".
///
/// Empty when its sign-in details cannot be told from its host and path:
/// - an `@` past the host (`https://user:p/w@aig.example/v1`, a key holding a `/`, `?` or `#`).
///   An `@` that is really in the path or query (`https://host/a@b`) gives empty too: the two
///   cannot be told apart by reading, and the one that is a key must never be drawn;
/// - a port that is not a number (`https://user:p`, a key that a space cut off from its `@`);
/// - no host at all.
///
/// A `\` ends the host as `/` does, as a browser and the `url` crate read it, so
/// `https://evil.example\@api.x.ai` is never drawn as api.x.ai.
pub fn address_of(url: &str) -> String {
    let url = url.trim();
    let (scheme, whole) = url.split_once("://").unwrap_or(("", url));
    let rest = whole.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = rest.split_at(rest.find(['/', '\\']).unwrap_or(rest.len()));
    let host = authority.rsplit('@').next().unwrap_or("");
    if host.is_empty() || whole.matches('@').count() != authority.matches('@').count() || !port_is_a_number(host) {
        return String::new();
    }
    if scheme.is_empty() {
        format!("{host}{path}")
    } else {
        format!("{scheme}://{host}{path}")
    }
}

/// The host (and port) of `address_of`: "aig.example", "[::1]:11434". Empty exactly when
/// `address_of` is.
pub fn shown_host(url: &str) -> String {
    let shown = address_of(url);
    let rest = shown.split_once("://").map_or(shown.as_str(), |(_, r)| r);
    rest.split(['/', '\\']).next().unwrap_or("").to_string()
}

/// What a line says in place of an address that `address_of` will not draw.
pub const NOT_SHOWN: &str = "(address not shown)";

/// Someone else's line (a mind's own words as it attached) with every address in it drawn as
/// `address_of` draws it, or as [`NOT_SHOWN`] when it gives nothing. Everything else, spacing
/// included, is kept.
///
/// A word is an address when it holds `://` or `@`, or a query or fragment that looks like one
/// (`aig.example/v1?api-key=x`, `?token=x`): what follows a `?` or `#` after a `.`, `/` or `:`,
/// or that holds `=` or `&`. "Ready?" and "issue #12" are words.
///
/// Sign-in details with spaces in them (`https://user:a b c@host`) are cut into several words, so
/// a URL word that ends before its path, with no `@` of its own, starts a run that reaches the
/// next word whose authority holds an `@`: the whole run is one [`NOT_SHOWN`], and that word is
/// drawn as `address_of` draws it. That holds when the cut-off word looks whole
/// (`https://user:1234`, a numeric password read as a port), and it hides an ordinary
/// `http://localhost:8080` that is followed by someone's address on the same line: the two cannot
/// be told apart by reading, and hiding is the side that never shows a key. A URL word that
/// already has a path starts a run only when the very next word holds the `@`.
pub fn without_sign_in(text: &str) -> String {
    // Each word with the run of whitespace that follows it.
    let mut pieces: Vec<(&str, &str)> = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let word_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let (word, after) = rest.split_at(word_end);
        let space_end = after.find(|c: char| !c.is_whitespace()).unwrap_or(after.len());
        let (space, next) = after.split_at(space_end);
        pieces.push((word, space));
        rest = next;
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < pieces.len() {
        if let Some(at) = cut_off_run(&pieces, i) {
            out.push_str(NOT_SHOWN);
            out.push_str(pieces[at - 1].1);
            i = at;
            continue;
        }
        let (word, space) = pieces[i];
        if is_address(word) {
            match address_of(word) {
                shown if shown.is_empty() => out.push_str(NOT_SHOWN),
                shown => out.push_str(&shown),
            }
        } else {
            out.push_str(word);
        }
        out.push_str(space);
        i += 1;
    }
    out
}

/// When the word at `i` is a URL cut off inside its sign-in details (`without_sign_in`), the
/// index of the word that holds their `@`. Every word before it, from `i`, is part of them.
fn cut_off_run(pieces: &[(&str, &str)], i: usize) -> Option<usize> {
    let (_, rest) = pieces[i].0.split_once("://")?;
    if rest.contains('@') {
        return None;
    }
    let reach = if rest.contains(['/', '\\', '?', '#']) { 1 } else { pieces.len() };
    for (j, (next, _)) in pieces.iter().enumerate().skip(i + 1).take(reach) {
        if next.contains("://") {
            return None;
        }
        if authority_of(next).contains('@') {
            return Some(j);
        }
    }
    None
}

/// Whether a word of someone else's line is, or holds, an address (`without_sign_in`).
fn is_address(word: &str) -> bool {
    if word.contains("://") || word.contains('@') {
        return true;
    }
    match word.find(['?', '#']) {
        Some(i) => {
            let (before, tail) = (&word[..i], &word[i + 1..]);
            !tail.is_empty() && (before.contains(['.', '/', ':']) || tail.contains(['=', '&']))
        }
        None => false,
    }
}

/// The part of a word before its path, query or fragment, after any scheme.
fn authority_of(word: &str) -> &str {
    let rest = word.split_once("://").map_or(word, |(_, r)| r);
    rest.split(['/', '\\', '?', '#']).next().unwrap_or("")
}

/// A host's port, when it has one, is a number: "aig.example:8443", "[::1]:4000", "::1". Not
/// "user:p", which is what is left of `https://user:p w@host` once a space cuts the word.
fn port_is_a_number(host: &str) -> bool {
    let digits = |p: &str| p.chars().all(|c| c.is_ascii_digit());
    if let Some(inside) = host.strip_prefix('[') {
        return match inside.split_once(']') {
            Some((_, "")) => true,
            Some((_, after)) => after.strip_prefix(':').is_some_and(digits),
            None => false,
        };
    }
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        return true;
    }
    match host.rsplit_once(':') {
        None => true,
        Some((name, port)) => !name.contains(':') && digits(port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hostile addresses from the review of #856, each with what may be drawn of it.
    pub const HOSTILE: &[(&str, &str)] = &[
        ("https://user:p/w@host/v1", ""),
        ("https://user:pw@host/v1?api-key=x#f", "https://host/v1"),
        ("http://u@127.0.0.1:11434", "http://127.0.0.1:11434"),
        ("http://u:p@[::1]:11434", "http://[::1]:11434"),
        ("http://u:p@[::1]", "http://[::1]"),
        ("user:pw@host", "host"),
        ("user:p/w@host", ""),
        ("user%40x:pw@host", "host"),
        ("https://user:p%2Fw@host/v1", "https://host/v1"),
        ("https://user:p@w@host/v1", "https://host/v1"),
        ("https:/user:pw@host", ""),
        ("https://host/a@b", ""),
        ("https://host/v1?x=a@b", ""),
        ("host/v1?api-key=x", "host/v1"),
        ("https://user:p", ""),
        ("https://evil.example\\@api.x.ai/v1", ""),
        ("https://@/v1", ""),
    ];

    /// Pieces of the sign-in details in `HOSTILE`, none of which may ever be drawn.
    const SECRETS: &[&str] = &["user", "pw", "p/w", "p%2F", "api-key", "u@", "u:p", "#f", "?", "%40"];

    fn assert_clean(input: &str, shown: &str) {
        for secret in SECRETS {
            assert!(!shown.contains(secret), "{input} draws {secret:?}: {shown:?}");
        }
    }

    #[test]
    fn every_hostile_address_is_drawn_without_any_part_of_its_sign_in_query_or_fragment() {
        for (input, want) in HOSTILE {
            let shown = address_of(input);
            assert_eq!(shown, *want, "{input}");
            assert_clean(input, &shown);
            assert_clean(input, &shown_host(input));
        }
    }

    #[test]
    fn an_ordinary_address_is_drawn_as_it_is() {
        for url in ["http://127.0.0.1:8341/v1", "https://api.openai.com/v1", "localhost:11434", "https://[fd00::1]:8443/v1", "::1", "ollama.com"] {
            assert_eq!(address_of(url), url);
        }
        assert_eq!(shown_host("https://user:pw@aig.example:8443/v1?k=x"), "aig.example:8443");
        assert_eq!(shown_host("http://u@[::1]:11434"), "[::1]:11434");
        assert_eq!(shown_host("https://user:p/w@aig.example/v1"), "");
    }

    #[test]
    fn a_line_someone_else_wrote_never_draws_the_sign_in_of_an_address_in_it() {
        assert_eq!(without_sign_in("Hermes 0.21.5 \u{b7} picked"), "Hermes 0.21.5 \u{b7} picked");
        assert_eq!(without_sign_in("Ready? see issue #12, 3.5x faster"), "Ready? see issue #12, 3.5x faster");
        assert_eq!(without_sign_in("m \u{b7} aig.example/v1?api-key=x"), "m \u{b7} aig.example/v1");
        assert_eq!(without_sign_in("m \u{b7} ?api-key=x"), format!("m \u{b7} {NOT_SHOWN}"));
        assert_eq!(without_sign_in("a  https://user:pw@h/v1?api-key=x#f\tb"), "a  https://h/v1\tb");
        assert_eq!(without_sign_in("on https://user:p w@host/v1"), format!("on {NOT_SHOWN} host/v1"));
        assert_eq!(without_sign_in("on https://us er:pw@host/v1"), format!("on {NOT_SHOWN} host/v1"));
        // Review of #856, delta finding B: sign-in details split by two or more spaces.
        assert_eq!(without_sign_in("m \u{b7} https://user:a b c@host/v1 \u{b7} up"), format!("m \u{b7} {NOT_SHOWN} host/v1 \u{b7} up"));
        assert_eq!(without_sign_in("on https://user:1234 a b@host"), format!("on {NOT_SHOWN} host"));
        assert_eq!(without_sign_in("on https://us e r:pw@host/v1"), format!("on {NOT_SHOWN} host/v1"));
        // A URL word with a path is whole unless the very next word holds the `@`.
        assert_eq!(without_sign_in("see https://h/v1 and ask bob@x.org"), "see https://h/v1 and ask x.org");
        assert_eq!(without_sign_in("see https://h/v1 ask"), "see https://h/v1 ask");
        for (input, _) in HOSTILE {
            let line = format!("qwen3.5:27b \u{b7} {input} \u{b7} up");
            let shown = without_sign_in(&line);
            assert!(shown.starts_with("qwen3.5:27b \u{b7} ") && shown.ends_with(" \u{b7} up"), "{line} → {shown}");
            assert_clean(input, shown.trim_start_matches("qwen3.5:27b"));
        }
    }

    #[test]
    fn drawing_a_line_twice_draws_it_as_once() {
        for (input, _) in HOSTILE {
            let once = without_sign_in(&format!("x {input} y"));
            assert_eq!(without_sign_in(&once), once, "{input}");
        }
    }
}
