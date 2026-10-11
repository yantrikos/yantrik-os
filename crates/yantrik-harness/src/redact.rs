//! Finding forgotten words by their digest (the `redact` event).
//!
//! When the person asks a mind to forget something and answers *Erase* to its Keep/Erase
//! question, the mind erases its own memory. The shell keeps copies of its own — the agent's pane
//! transcript and the run store — and a forget that left those would not be one. So the mind tells
//! the shell what to erase, without ever sending the words: each needle travels as the SHA-256 of
//! its canonical form and its length in Unicode scalar values, and the shell slides a window of
//! that length over the canonical form of its own text, hashing as it goes.
//!
//! This module is that matching, and nothing else: no store, no rule about who may ask. The
//! acceptance rule is the run store's (`run_store`, `RunStore::redact`), the host's
//! (`Host`'s `redact` event) and the shell's transcript (`yantrik-ui`'s agent store).
//!
//! # The canonical form
//!
//! NFC first, then Unicode default lowercasing (`str::to_lowercase`; in Python
//! `unicodedata.normalize('NFC', t).lower()`), as UTF-8. The needle's `sha256` is over that, and
//! its `len` is the canonical form's length in scalar values — after lowercasing, which can change
//! it ('İ' U+0130 lowercases to 'i' + U+0307, two scalars). The shell puts each text it holds in
//! the same form before taking windows. Nothing else is folded: 'ß' is not 'ss'.
//!
//! # How text is matched
//!
//! - Case-insensitive and over NFC, as above: "Priya", "PRIYA" and "priya" are one needle, and so
//!   are "café" typed composed and stored decomposed.
//! - **What is replaced is the original.** Every scalar of the canonical form knows the bytes of
//!   the stored text it came from, and a match replaces the stored span that produced it. Where a
//!   window begins or ends inside what one original character became (the 'i' of a lowercased
//!   'İ', the 'é' two stored characters composed into), the span widens to the whole of it.
//!   Everything outside the spans is handed back byte for byte: its case, its normalisation.
//! - **Pieces are joined before matching.** A reply arrives as chunks, and a name split across two
//!   of them ("Pri" + "ya") is still a name. [`redact_pieces`] matches over the joined pieces and
//!   hands back the same number of pieces, so a store can write each one back where it was: the
//!   piece the match began in carries the marker, and the rest of the match is taken out of the
//!   pieces it ran into.
//! - Left to right, longest needle first at each place, and a match is never matched again.
//!
//! # Searching off the lock, applying under it
//!
//! A needle is only a digest, so every window of every needle length is hashed in full; nothing
//! cheaper can rule a window out. That is bounded instead: a store copies its texts out
//! ([`Prepared`]), the caller adds up [`Search::work`] over everything one `redact` would search,
//! and refuses the whole of it over [`MAX_WORK`] before anything is hashed or touched. Then
//! [`Search::find`] runs on the copies, with no lock held, and the store takes its lock only to
//! apply what was found ([`Found::apply`]), checking each match again against the text as it is by
//! then.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::{is_nfc_quick, IsNormalized, UnicodeNormalization};

/// What is left where the words were.
pub const MARKER: &str = "[erased at your request]";

/// The most needles one `redact` may carry.
pub const MAX_NEEDLES: usize = 16;

/// The longest one needle may be, in Unicode scalar values.
pub const MAX_NEEDLE_CHARS: usize = 4096;

/// The shortest one needle may be, in Unicode scalar values after canonicalisation: a shorter one
/// ("e", "not") would erase too much to be what the person meant.
pub const MIN_NEEDLE_CHARS: usize = 4;

/// The refusal for a needle under [`MIN_NEEDLE_CHARS`].
pub const TOO_SHORT: &str = "a needle is too short to erase safely";

/// Words to erase, as their digest: never the words themselves.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Needle {
    /// SHA-256 of the needle's canonical form ([`canonical`]) as UTF-8, as 64 lowercase hex digits.
    pub sha256: String,
    /// The canonical form's length in Unicode scalar values (after lowercasing).
    pub len: usize,
}

impl Needle {
    /// The needle for `text`, as a harness computes it before sending. Here for tests and for the
    /// shell's own callers; a mind computes its own (`turn.redact` in the Python library).
    pub fn of(text: &str) -> Needle {
        let canon = canonical(text);
        Needle { sha256: digest_hex(&canon), len: canon.chars().count() }
    }

    fn bytes(&self) -> Option<[u8; 32]> {
        if self.sha256.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, pair) in self.sha256.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(pair).ok()?;
            out[i] = u8::from_str_radix(hex, 16).ok()?;
        }
        Some(out)
    }
}

/// The form needles are hashed in and text is matched in: NFC, then Unicode default lowercasing.
pub fn canonical(text: &str) -> String {
    text.nfc().collect::<String>().to_lowercase()
}

/// Whether `needles` may be acted on: between one and [`MAX_NEEDLES`] of them, each a 64-digit
/// lowercase hex digest and a length from [`MIN_NEEDLE_CHARS`] to [`MAX_NEEDLE_CHARS`]. The error says which is wrong
/// and never repeats a digest.
pub fn validate(needles: &[Needle]) -> Result<(), String> {
    if needles.is_empty() {
        return Err("a `redact` needs at least one needle".to_string());
    }
    if needles.len() > MAX_NEEDLES {
        return Err(format!("a `redact` carries at most {MAX_NEEDLES} needles, not {}", needles.len()));
    }
    for (i, needle) in needles.iter().enumerate() {
        let lower_hex = needle.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if needle.sha256.len() != 64 || !lower_hex {
            return Err(format!("needle {i}: `sha256` must be 64 lowercase hex digits"));
        }
        if needle.len < MIN_NEEDLE_CHARS {
            return Err(TOO_SHORT.to_string());
        }
        if needle.len > MAX_NEEDLE_CHARS {
            return Err(format!("needle {i}: `len` must be from {MIN_NEEDLE_CHARS} to {MAX_NEEDLE_CHARS}"));
        }
    }
    Ok(())
}

/// The most searching one `redact` may cost, everywhere it searches together: the bytes hashed,
/// each window's bytes plus [`WINDOW_COST`]. Over it, the whole `redact` is refused before
/// anything is searched or touched ([`TOO_MUCH`]). A needle travels only as a digest, so every
/// window of every needle length has to be hashed in full; this is what bounds that. About a second
/// of SHA-256 on this hardware.
pub const MAX_WORK: u64 = 1 << 30;

/// What every window costs beyond its own bytes: one SHA-256 block of padding and finishing.
pub const WINDOW_COST: u64 = 64;

/// The refusal when a `redact` would search more than [`MAX_WORK`].
pub const TOO_MUCH: &str = "too much to search; ask again with fewer or shorter needles";

/// How much of a question's prompt the person is shown: the question card clips to it.
pub const QUESTION_CHARS: usize = 2000;

/// The part of a question's prompt the person saw: all of it, or, past [`QUESTION_CHARS`], the
/// characters before the card's ellipsis.
pub fn question_shown(prompt: &str) -> &str {
    if prompt.chars().count() <= QUESTION_CHARS {
        return prompt;
    }
    let end = prompt.char_indices().nth(QUESTION_CHARS - 1).map_or(prompt.len(), |(at, _)| at);
    &prompt[..end]
}

/// The quoted spans of a question, as the person was shown it ([`question_shown`]), each in
/// canonical form. See [`quoted_texts`] for the rule.
pub fn quoted_spans(prompt: &str) -> Vec<String> {
    quoted_texts(prompt).into_iter().map(canonical).collect()
}

/// What a straight quote may open after: whitespace, the start of the text, or one of these.
const OPENS_AFTER: [char; 3] = ['(', '[', '{'];

/// What a straight quote may close before: whitespace, the end of the text, or one of these.
const CLOSES_BEFORE: [char; 9] = ['.', ',', ';', ':', '!', '?', ')', ']', '}'];

/// The quoted spans of a question as the person was shown it ([`question_shown`]), as written:
/// the text between a pair of double quotes, exactly, where the quotes read as a pair.
///
/// - Only double quotes delimit. Single quotes, apostrophes, `‘` and `’` never open or close a span.
/// - A straight `"` (U+0022) opens only at the start of the text, or after whitespace or one of
///   `( [ {`; it closes at the next `"`, and only when that one is followed by whitespace, one of
///   `. , ; : ! ? ) ] }`, or the end of the text. Inside `"…"`, `“` and `”` are ordinary.
/// - `“` (U+201C) opens and `”` (U+201D) closes, at the next `”`, wherever they are; `”…“` (curly
///   quotes used backwards) is never a span. Inside `“…”`, `"` is ordinary.
/// - A span whose text starts or ends with whitespace is no span.
/// - Left to right; spans do not nest; no escapes. Where an opener makes no span (no closer, a
///   closer in the wrong place, or the whitespace rule), the scan goes on from the character after
///   it. "Whitespace" is Unicode `White_Space` (`char::is_whitespace`).
///
/// Every span is listed, however short; a needle must also be [`MIN_NEEDLE_CHARS`] long. The same
/// rule in Python is `quoted_spans` in `harnesses/lib/yantrik_harness.py`; both are held to
/// `harnesses/tests/fixtures/redact_spans.json`.
pub fn quoted_texts(prompt: &str) -> Vec<&str> {
    let shown = question_shown(prompt);
    let chars: Vec<(usize, char)> = shown.char_indices().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        let close = match c {
            '"' if i == 0 || chars[i - 1].1.is_whitespace() || OPENS_AFTER.contains(&chars[i - 1].1) => Some('"'),
            '\u{201c}' => Some('\u{201d}'),
            _ => None,
        };
        if let Some(close) = close {
            if let Some(j) = (i + 1..chars.len()).find(|&j| chars[j].1 == close) {
                let closes = close != '"'
                    || chars.get(j + 1).map_or(true, |&(_, n)| n.is_whitespace() || CLOSES_BEFORE.contains(&n));
                let text = &shown[at + c.len_utf8()..chars[j].0];
                let trimmed = !text.starts_with(char::is_whitespace) && !text.ends_with(char::is_whitespace);
                if closes && trimmed {
                    spans.push(text);
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    spans
}

/// The needles a `redact` answered by this question may carry: one per quoted span
/// ([`quoted_spans`]) at least [`MIN_NEEDLE_CHARS`] long. A needle must equal one of them exactly.
pub fn quoted_needles(prompt: &str) -> Vec<Needle> {
    quoted_spans(prompt)
        .iter()
        .filter(|span| span.chars().count() >= MIN_NEEDLE_CHARS)
        .map(|span| Needle { sha256: digest_hex(span), len: span.chars().count() })
        .collect()
}

/// The needles, ready to search with.
pub struct Search {
    digests: HashSet<[u8; 32]>,
    /// Each distinct length, longest first.
    lengths: Vec<usize>,
}

impl Search {
    pub fn new(needles: &[Needle]) -> Search {
        let digests: HashSet<[u8; 32]> = needles.iter().filter_map(Needle::bytes).collect();
        let mut lengths: Vec<usize> = needles.iter().map(|n| n.len).filter(|&l| l > 0).collect();
        lengths.sort_unstable_by(|a, b| b.cmp(a));
        lengths.dedup();
        Search { digests, lengths }
    }

    fn is_empty(&self) -> bool {
        self.digests.is_empty() || self.lengths.is_empty()
    }

    /// What searching text of `bytes` raw bytes would cost, about, in [`MAX_WORK`]'s units:
    /// every byte the start of a window of every needle length. Worked out from lengths alone,
    /// before anything is copied.
    pub fn estimate(&self, bytes: u64) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let per_place: u64 = self.lengths.iter().map(|&len| len as u64 + WINDOW_COST).sum();
        bytes.saturating_mul(per_place)
    }

    /// What searching `text` will cost, in [`MAX_WORK`]'s units, worked out without hashing.
    pub fn work(&self, text: &Prepared) -> u64 {
        if self.is_empty() {
            return 0;
        }
        let offsets = &text.canon.offsets;
        let count = offsets.len() - 1;
        // prefix[k] is the sum of offsets[..k], so a run of offsets sums in one subtraction.
        let mut prefix: Vec<u64> = Vec::with_capacity(offsets.len() + 1);
        prefix.push(0);
        for &o in offsets {
            prefix.push(prefix.last().unwrap() + o as u64);
        }
        let mut work: u64 = 0;
        for &len in &self.lengths {
            if len > count {
                continue;
            }
            let windows = (count - len + 1) as u64;
            // Window `at` is offsets[at + len] - offsets[at] bytes; summed over every `at`.
            let ends = prefix[count + 1] - prefix[len];
            let starts = prefix[count - len + 1];
            work = work.saturating_add(ends - starts).saturating_add(windows * WINDOW_COST);
        }
        work
    }

    /// Every match in `text`, left to right, longest needle first at each place, never matching
    /// what was matched. Takes no lock: `text` is a copy.
    pub fn find(&self, text: &Prepared) -> Found {
        let mut hits: Vec<Hit> = Vec::new();
        if self.is_empty() {
            return Found { hits };
        }
        let canon = &text.canon;
        let count = canon.origin.len();
        let mut at = 0;
        'scan: while at < count {
            for &len in &self.lengths {
                if at + len > count {
                    continue;
                }
                let digest: [u8; 32] = Sha256::digest(canon.window(at, len)).into();
                if self.digests.contains(&digest) {
                    hits.push(Hit { at, len, digest });
                    let end = canon.origin[at + len - 1].1;
                    // On past everything the match's span took, including the rest of a character
                    // it widened into: what is erased is not matched again.
                    at += len;
                    while at < count && canon.origin[at].0 < end {
                        at += 1;
                    }
                    continue 'scan;
                }
            }
            at += 1;
        }
        Found { hits }
    }
}

/// One text — its pieces, joined — in canonical form: a copy taken out of a store, so it can be
/// measured and searched without the store's lock.
pub struct Prepared {
    canon: Canonical,
}

impl Prepared {
    pub fn new<S: AsRef<str>>(pieces: &[S]) -> Prepared {
        let joined: String = pieces.iter().map(|p| p.as_ref()).collect();
        Prepared { canon: Canonical::of(&joined) }
    }
}

/// What a search found in one text: where in the canonical form, and which needle. No words.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    hits: Vec<Hit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Hit {
    /// The window's first canonical scalar, and how many.
    at: usize,
    len: usize,
    digest: [u8; 32],
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    /// Erase what was found from `pieces` — the text as it is now, under the store's lock. Each
    /// match is checked again first: the window at the same place must still hash to the same
    /// needle, or it is skipped. So only the needles' words are ever replaced, even if the text
    /// changed since it was searched. `None` when nothing is erased; otherwise the same number of
    /// pieces (one no match reached comes back exactly as it was) and how many places.
    pub fn apply(&self, pieces: &[&str]) -> Option<(Vec<String>, usize)> {
        if self.hits.is_empty() {
            return None;
        }
        let joined: String = pieces.concat();
        let canon = Canonical::of(&joined);
        let count = canon.origin.len();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for hit in &self.hits {
            if hit.at + hit.len > count {
                continue;
            }
            let digest: [u8; 32] = Sha256::digest(canon.window(hit.at, hit.len)).into();
            if digest != hit.digest {
                continue;
            }
            let span = (canon.origin[hit.at].0, canon.origin[hit.at + hit.len - 1].1);
            if spans.last().is_some_and(|&(_, end)| span.0 < end) {
                continue;
            }
            spans.push(span);
        }
        if spans.is_empty() {
            return None;
        }
        Some((write_back(pieces, &joined, &spans), spans.len()))
    }
}

/// The pieces again, each span of `joined` replaced: the marker in the piece the span begins in,
/// and the rest of the span taken out of the pieces it runs into.
fn write_back(pieces: &[&str], joined: &str, spans: &[(usize, usize)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(pieces.len());
    let mut first = 0; // the first span that may still reach the current piece
    let mut start = 0;
    for piece in pieces {
        let end = start + piece.len();
        while first < spans.len() && spans[first].1 <= start {
            first += 1;
        }
        let mut kept = String::new();
        let mut cursor = start;
        for &(from, to) in spans[first..].iter().take_while(|&&(from, _)| from < end) {
            if from >= start {
                kept.push_str(&joined[cursor..from]);
                kept.push_str(MARKER);
            }
            cursor = cursor.max(to.min(end));
        }
        kept.push_str(&joined[cursor..end]);
        out.push(kept);
        start = end;
    }
    out
}

/// Erase every needle from one text, searching and applying at once. `None` when nothing
/// matched. For tests and small texts: a store searches with [`Search`] off its lock and applies
/// with [`Found::apply`] under it, after checking the whole erasure's [`Search::work`].
pub fn redact(text: &str, needles: &[Needle]) -> Option<(String, usize)> {
    let (mut pieces, n) = redact_pieces(&[text], needles)?;
    Some((pieces.remove(0), n))
}

/// [`redact`] over pieces matched as one text (see the module docs).
pub fn redact_pieces(pieces: &[&str], needles: &[Needle]) -> Option<(Vec<String>, usize)> {
    Search::new(needles).find(&Prepared::new(pieces)).apply(pieces)
}

/// [`redact`] over each string inside a JSON value, in place. How many places were erased.
pub fn redact_json(value: &mut serde_json::Value, needles: &[Needle]) -> usize {
    let search = Search::new(needles);
    let found: Vec<Found> = json_strings(value).iter().map(|s| search.find(&Prepared::new(&[s]))).collect();
    apply_json(value, &found)
}

/// The strings inside a JSON value — object values and array items, never object keys — in the
/// order [`apply_json`] walks them.
pub fn json_strings(value: &serde_json::Value) -> Vec<String> {
    json_strings_under(value, None)
}

/// [`json_strings`], only those somewhere under an object key in `keys` when it is given.
pub fn json_strings_under(value: &serde_json::Value, keys: Option<&[&str]>) -> Vec<String> {
    let mut out = Vec::new();
    walk_json(value, keys.is_none(), keys.unwrap_or(&[]), &mut |s| out.push(s.clone()));
    out
}

/// Apply `found[i]` to the `i`th string of [`json_strings`], in place. How many places.
pub fn apply_json(value: &mut serde_json::Value, found: &[Found]) -> usize {
    apply_json_under(value, found, None)
}

/// [`apply_json`] over the strings of [`json_strings_under`] with the same `keys`.
pub fn apply_json_under(value: &mut serde_json::Value, found: &[Found], keys: Option<&[&str]>) -> usize {
    let mut next = 0;
    let mut places = 0;
    walk_json_mut(value, keys.is_none(), keys.unwrap_or(&[]), &mut |s| {
        if let Some((mut text, n)) = found.get(next).and_then(|f| f.apply(&[s.as_str()])) {
            *s = text.remove(0);
            places += n;
        }
        next += 1;
    });
    places
}

fn walk_json(value: &serde_json::Value, inside: bool, keys: &[&str], each: &mut dyn FnMut(&String)) {
    match value {
        serde_json::Value::String(s) if inside => each(s),
        serde_json::Value::Array(items) => items.iter().for_each(|v| walk_json(v, inside, keys, each)),
        serde_json::Value::Object(map) => {
            map.iter().for_each(|(k, v)| walk_json(v, inside || keys.contains(&k.as_str()), keys, each))
        }
        _ => {}
    }
}

fn walk_json_mut(value: &mut serde_json::Value, inside: bool, keys: &[&str], each: &mut dyn FnMut(&mut String)) {
    match value {
        serde_json::Value::String(s) if inside => each(s),
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| walk_json_mut(v, inside, keys, each)),
        serde_json::Value::Object(map) => {
            map.iter_mut().for_each(|(k, v)| walk_json_mut(v, inside || keys.contains(&k.as_str()), keys, each))
        }
        _ => {}
    }
}

fn digest_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A text in canonical form, each scalar knowing where in the original it came from.
struct Canonical {
    /// `canonical(original)`, exactly.
    text: String,
    /// The byte offset in `text` of each scalar, and `text.len()` after the last.
    offsets: Vec<usize>,
    /// For each scalar, the byte range of the original it came from: the original character, or
    /// all the original characters NFC made it from.
    origin: Vec<(usize, usize)>,
}

impl Canonical {
    fn of(original: &str) -> Canonical {
        // NFC, one segment at a time: a segment begins at a character nothing before it can
        // compose or reorder with, so NFC of the segments one by one is NFC of the whole. Where a
        // segment comes out as it went in, each character is its own origin; where NFC changed it,
        // every character it became comes from the whole segment.
        let mut nfc = String::with_capacity(original.len());
        let mut nfc_origin: Vec<(usize, usize)> = Vec::with_capacity(original.len());
        for (from, to) in segments(original) {
            let segment = &original[from..to];
            let before = nfc.len();
            nfc.extend(segment.nfc());
            if &nfc[before..] == segment {
                nfc_origin.extend(segment.char_indices().map(|(i, c)| (from + i, from + i + c.len_utf8())));
            } else {
                let made = nfc[before..].chars().count();
                nfc_origin.extend(std::iter::repeat((from, to)).take(made));
            }
        }

        // Then lowercase the whole (so a final sigma is decided as `str::to_lowercase` decides
        // it), and give each lowercase scalar the origin of the NFC character it came from. Each
        // character lowercases to as many scalars on its own as it does in the whole; the final
        // sigma is the one context-dependent mapping, and it is one scalar either way.
        let text = nfc.to_lowercase();
        let mut origin: Vec<(usize, usize)> = Vec::with_capacity(nfc_origin.len());
        for (c, from) in nfc.chars().zip(nfc_origin) {
            origin.extend(std::iter::repeat(from).take(c.to_lowercase().count()));
        }
        let offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect();
        debug_assert_eq!(origin.len() + 1, offsets.len(), "every canonical scalar has an origin");
        // Never index past the text, even if a future lowercasing broke the count above.
        origin.truncate(offsets.len() - 1);
        Canonical { text, offsets, origin }
    }

    /// The UTF-8 of `len` canonical scalars from `at`.
    fn window(&self, at: usize, len: usize) -> &[u8] {
        &self.text.as_bytes()[self.offsets[at]..self.offsets[at + len]]
    }
}

/// The byte ranges of `text`, cut where nothing before can compose or reorder with what follows.
fn segments(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    for (i, c) in text.char_indices() {
        if i > from && starts_segment(c) {
            out.push((from, i));
            from = i;
        }
    }
    if from < text.len() {
        out.push((from, text.len()));
    }
    out
}

/// Whether NFC leaves everything before `c` alone: `c`, and the first character of its canonical
/// decomposition, are starters that never compose with a character before them (NFC_Quick_Check is
/// not Maybe).
fn starts_segment(c: char) -> bool {
    if c.is_ascii() {
        return true;
    }
    let first = std::iter::once(c).nfd().next().unwrap_or(c);
    canonical_combining_class(c) == 0
        && canonical_combining_class(first) == 0
        && is_nfc_quick(std::iter::once(first)) != IsNormalized::Maybe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_needle_is_found_and_marked_and_nothing_else_moves() {
        let (text, n) = redact("My sister Priya lives in Pune. Priya!", &[Needle::of("Priya")]).unwrap();
        assert_eq!(n, 2);
        assert_eq!(text, format!("My sister {MARKER} lives in Pune. {MARKER}!"));
        assert_eq!(redact("nobody named here", &[Needle::of("Priya")]), None);
    }

    #[test]
    fn matching_is_case_insensitive_and_the_text_around_keeps_its_case() {
        let needle = Needle::of("throwaway-erase2");
        assert_eq!(needle, Needle::of("THROWAWAY-ERASE2"));
        let (text, n) =
            redact("Code THROWAWAY-ERASE2, then Throwaway-Erase2 and throwaway-erase2 — DONE.", &[needle]).unwrap();
        assert_eq!(n, 3);
        assert_eq!(text, format!("Code {MARKER}, then {MARKER} and {MARKER} — DONE."));
    }

    #[test]
    fn composed_and_decomposed_are_the_same_words() {
        let composed = "caf\u{e9}";
        let decomposed = "cafe\u{301}";
        assert_eq!(Needle::of(composed), Needle::of(decomposed));
        assert_eq!(Needle::of(composed).len, 4);
        let (text, n) = redact(&format!("at the {decomposed} on Elm"), &[Needle::of(composed)]).unwrap();
        assert_eq!((text.as_str(), n), (format!("at the {MARKER} on Elm").as_str(), 1));
        let (text, _) = redact(&format!("at the {composed}"), &[Needle::of(decomposed)]).unwrap();
        assert_eq!(text, format!("at the {MARKER}"));
    }

    #[test]
    fn composed_and_decomposed_in_either_case_are_the_same_words() {
        let forms = ["caf\u{e9}", "cafe\u{301}", "CAF\u{c9}", "CAFE\u{301}", "Caf\u{e9}"];
        for needle in &forms[..4] {
            assert_eq!(Needle::of(needle), Needle::of("caf\u{e9}"), "{needle:?}");
            for stored in forms {
                let (text, n) = redact(&format!("At the {stored}, Bob."), &[Needle::of(needle)]).unwrap();
                assert_eq!((text, n), (format!("At the {MARKER}, Bob."), 1), "{needle:?} in {stored:?}");
            }
        }
        // Untouched text keeps its own normalisation, decomposed or not.
        let (text, _) = redact("CAFE\u{301} and Cafe\u{301} Lune", &[Needle::of("CAF\u{c9} LUNE")]).unwrap();
        assert_eq!(text, format!("CAFE\u{301} and {MARKER}"));
    }

    #[test]
    fn a_dotted_capital_i_inside_a_match_is_erased_whole() {
        let needle = Needle::of("\u{130}stanbul");
        assert_eq!(needle.len, 9, "'İ' lowercases to two scalars");
        let (text, n) = redact("Fly to \u{130}STANBUL, then \u{130}stanbul.", &[needle.clone()]).unwrap();
        assert_eq!((text, n), (format!("Fly to {MARKER}, then {MARKER}."), 2));
        // Stored decomposed, 'I' + U+0307 composes to 'İ' first: still the same words.
        let (text, _) = redact("Visit I\u{307}stanbul now", &[needle]).unwrap();
        assert_eq!(text, format!("Visit {MARKER} now"));
    }

    #[test]
    fn a_dotted_capital_i_next_to_a_match_is_not_touched() {
        let (text, n) = redact("\u{130}PRIYA\u{130} x \u{130}priya", &[Needle::of("Priya")]).unwrap();
        assert_eq!((text, n), (format!("\u{130}{MARKER}\u{130} x \u{130}{MARKER}"), 2));
    }

    #[test]
    fn a_window_that_ends_or_begins_inside_a_dotted_capital_i_takes_all_of_it_and_no_more() {
        // "abci" matches the first scalar of the 'İ' in "ABCİ": the span widens to the whole 'İ',
        // and the next character is left as it was.
        let (text, n) = redact("xABC\u{130}y ABC\u{130}", &[Needle::of("abci")]).unwrap();
        assert_eq!((text, n), (format!("x{MARKER}y {MARKER}"), 2));
        // A needle that begins with the dot above matches the second scalar: widened back to the
        // 'İ', and nothing before it.
        let (text, n) = redact("x\u{130}STANBUL", &[Needle::of("\u{307}stan")]).unwrap();
        assert_eq!((text, n), (format!("x{MARKER}BUL"), 1));
        // The dot left over after a widened match is not matched again.
        let (text, n) = redact("ABC\u{130}\u{307}x", &[Needle::of("abci"), Needle::of("\u{307}")]).unwrap();
        assert_eq!((text, n), (format!("{MARKER}{MARKER}x"), 2), "the second dot is its own character");
    }

    #[test]
    fn a_needle_split_across_pieces_is_found_and_written_back_piece_by_piece() {
        let (pieces, n) = redact_pieces(&["Her name is Pri", "ya, and she", " lives here."], &[Needle::of("Priya")]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(pieces, vec![format!("Her name is {MARKER}"), ", and she".to_string(), " lives here.".to_string()]);
        assert_eq!(pieces.concat(), format!("Her name is {MARKER}, and she lives here."));
        let (pieces, _) = redact_pieces(&["Her name is PRI", "ya, and she"], &[Needle::of("priya")]).unwrap();
        assert_eq!(pieces, vec![format!("Her name is {MARKER}"), ", and she".to_string()]);
    }

    #[test]
    fn a_combining_mark_at_the_start_of_a_piece_still_composes() {
        let (pieces, n) = redact_pieces(&["the CAFE", "\u{301} closed"], &[Needle::of("caf\u{e9}")]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(pieces.concat(), format!("the {MARKER} closed"));
        assert_eq!(pieces.len(), 2);
    }

    #[test]
    fn a_piece_no_match_reached_comes_back_as_it_was() {
        let (pieces, _) = redact_pieces(&["Cafe\u{301} ", "Priya", " Cafe\u{301}"], &[Needle::of("priya")]).unwrap();
        assert_eq!(pieces, vec!["Cafe\u{301} ".to_string(), MARKER.to_string(), " Cafe\u{301}".to_string()]);
    }

    #[test]
    fn the_longest_needle_wins_where_two_begin() {
        let needles = [Needle::of("12 Elm"), Needle::of("12 Elm Street")];
        let (text, n) = redact("at 12 ELM STREET now", &needles).unwrap();
        assert_eq!((text, n), (format!("at {MARKER} now"), 1));
    }

    #[test]
    fn strings_inside_json_are_erased_and_keys_are_not() {
        let mut v = serde_json::json!({"Priya": "call PRIYA", "n": 3, "list": ["Priya", "x"]});
        assert_eq!(redact_json(&mut v, &[Needle::of("Priya")]), 2);
        assert_eq!(v, serde_json::json!({"Priya": format!("call {MARKER}"), "n": 3, "list": [MARKER, "x"]}));
    }

    /// The needles both sides must compute, byte for byte: `harnesses/tests/fixtures/
    /// redact_needles.json`, asserted here and by the Python harness library's tests.
    #[test]
    fn needles_match_the_shared_fixtures() {
        let fixtures: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../harnesses/tests/fixtures/redact_needles.json")).unwrap();
        let texts: Vec<&str> = fixtures.iter().map(|f| f["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["Stra\u{df}e", "\u{130}stanbul", "\u{c9}", "E\u{301}", "throwaway-erase2"]);
        for f in &fixtures {
            let text = f["text"].as_str().unwrap();
            let needle = Needle::of(text);
            assert_eq!(needle.sha256, f["sha256"].as_str().unwrap(), "sha256 of {text:?}");
            assert_eq!(needle.len as u64, f["len"].as_u64().unwrap(), "len of {text:?}");
            // 'É' is one scalar: a needle, but too short to send.
            assert_eq!(validate(&[needle.clone()]).is_ok(), needle.len >= MIN_NEEDLE_CHARS, "{text:?}");
            // And the shell finds each one in its own text.
            assert_eq!(redact(&format!("<{text}>"), &[needle]), Some((format!("<{MARKER}>"), 1)), "{text:?}");
        }
    }

    #[test]
    fn needles_are_checked_before_anything_is_touched() {
        assert!(validate(&[Needle::of("xxxx")]).is_ok());
        assert!(validate(&[]).is_err());
        // "e" or "not" would erase far more than anyone meant.
        for short in ["e", "not", "\u{c9}"] {
            assert_eq!(validate(&[Needle::of(short)]), Err(TOO_SHORT.to_string()), "{short:?}");
        }
        assert!(validate(&vec![Needle::of("x"); MAX_NEEDLES + 1]).is_err());
        let upper = Needle { sha256: Needle::of("x").sha256.to_uppercase(), len: 1 };
        assert!(validate(&[upper]).is_err());
        assert!(validate(&[Needle { sha256: "ab".into(), len: 1 }]).is_err());
        assert!(validate(&[Needle { sha256: Needle::of("x").sha256, len: 0 }]).is_err());
        assert!(validate(&[Needle { sha256: Needle::of("x").sha256, len: MAX_NEEDLE_CHARS + 1 }]).is_err());
        // The refusal never repeats the digest it refused.
        let bad = Needle { sha256: format!("{}zz", &Needle::of("x").sha256[..62]), len: 1 };
        assert!(!validate(&[bad.clone()]).unwrap_err().contains(&bad.sha256));
    }

    #[test]
    fn the_work_is_every_window_hashed_counted_before_any_is() {
        let search = Search::new(&[Needle::of("abc"), Needle::of("abcde")]);
        // 10 ASCII scalars: 8 windows of 3 bytes and 6 of 5.
        assert_eq!(search.work(&Prepared::new(&["0123456789"])), 8 * (3 + WINDOW_COST) + 6 * (5 + WINDOW_COST));
        // Counted in bytes: each 'é' is two, so the one window of three is six.
        assert_eq!(search.work(&Prepared::new(&["\u{e9}\u{e9}\u{e9}"])), 6 + WINDOW_COST);
        assert_eq!(search.work(&Prepared::new(&["ab"])), 0, "shorter than every needle");
        // 16 needles of about 4096 over 100 000 scalars is far over the limit.
        let long: Vec<Needle> = (0..16).map(|i| Needle::of(&"x".repeat(4081 + i))).collect();
        assert!(Search::new(&long).work(&Prepared::new(&["y".repeat(100_000)])) > MAX_WORK);
    }

    #[test]
    fn a_match_is_checked_again_against_the_text_as_it_is_when_applied() {
        let search = Search::new(&[Needle::of("Priya")]);
        let found = search.find(&Prepared::new(&["call Priya now"]));
        // Unchanged, or with more after it: applied.
        assert_eq!(found.apply(&["call Priya now"]), Some((vec![format!("call {MARKER} now")], 1)));
        assert_eq!(found.apply(&["call Priya now", ", Priya"]), Some((vec![format!("call {MARKER} now"), ", Priya".to_string()], 1)));
        // Changed under it: the window no longer hashes to the needle, so nothing is replaced.
        assert_eq!(found.apply(&["call Maria now"]), None);
        assert_eq!(found.apply(&["call"]), None);
    }

    /// The quoted-span rule both sides must follow, byte for byte:
    /// `harnesses/tests/fixtures/redact_spans.json`, asserted here and by the Python harness
    /// library's tests.
    #[test]
    fn quoted_spans_match_the_shared_fixtures() {
        let fixtures: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../harnesses/tests/fixtures/redact_spans.json")).unwrap();
        assert!(fixtures.len() >= 9);
        for f in &fixtures {
            let question = f["question"].as_str().unwrap();
            let spans: Vec<String> = f["spans"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            assert_eq!(quoted_spans(question), spans, "{}", f["case"]);
            let short: Vec<&String> = spans.iter().filter(|s| s.chars().count() < MIN_NEEDLE_CHARS).collect();
            let listed: Vec<String> = f["too_short"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
            assert_eq!(short.into_iter().cloned().collect::<Vec<_>>(), listed, "{}", f["case"]);
            // A question may be answered for exactly its long-enough spans, each as one needle.
            let needles = quoted_needles(question);
            assert_eq!(needles.len(), spans.len() - listed.len(), "{}", f["case"]);
            for span in spans.iter().filter(|s| !listed.contains(s)) {
                assert!(needles.contains(&Needle::of(span)), "{}: {span:?}", f["case"]);
            }
        }
    }

    #[test]
    fn a_needle_must_be_a_whole_quoted_span_not_a_piece_of_one_or_of_the_rest() {
        let question = "Forget that you said \u{201c}you will not delete ~/Photos\u{201d}, and \"Priya\"?";
        let allowed = quoted_needles(question);
        // Whole spans, in any case, as the person saw them quoted.
        assert!(allowed.contains(&Needle::of("You will NOT delete ~/Photos")));
        assert!(allowed.contains(&Needle::of("priya")));
        // A piece of a span, or words outside every span, are not.
        for not_quoted in ["not delete", "~/Photos", "Forget that", "you will not"] {
            assert!(!allowed.contains(&Needle::of(not_quoted)), "{not_quoted:?}");
        }
    }
}
