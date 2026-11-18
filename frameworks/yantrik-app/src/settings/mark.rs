//! How a file the framework wrote for an app begins, told from its first bytes alone: the
//! judges [`crate::quiet::read_own_headed`] asks before it reads the rest of a file
//! (the review of #936 at 14fe65bd, findings F and G).
//!
//! The mark is a structure at a fixed place, not a string somewhere: the document is an object
//! whose first two members are `"kind": "yantrik-app-settings"` and `"app": "<id>"`, in either
//! order (a hand edit that sorts the keys keeps them first), with JSON whitespace anywhere
//! between the tokens. A value someone can set inside another component's file (an account
//! label in an array, a field further down an object) cannot make that file begin so, and
//! another app's settings file names another id. Strings are matched byte for byte, with no
//! escapes: the framework never writes one in the mark, so `"\u006bind"` is not `"kind"`.
//!
//! A judge never keeps the bytes it is shown, and says only [`Head::Ours`], [`Head::NotOurs`]
//! or [`Head::More`].

use super::file::KIND;
use crate::quiet::Head;

/// A settings file of `app`'s: it begins with `app`'s mark.
pub(crate) fn settings(app: &str) -> impl Fn(&[u8], bool) -> Head + '_ {
    move |bytes, whole| judged(mark(&mut Walk::new(bytes), app), whole, Head::NotOurs)
}

/// A temp the store or the lane left beside `app`'s settings file: it begins with `app`'s mark,
/// or it is the start of one cut short (empty included), as a write killed early leaves it.
pub(crate) fn temp(app: &str) -> impl Fn(&[u8], bool) -> Head + '_ {
    move |bytes, whole| judged(mark(&mut Walk::new(bytes), app), whole, Head::Ours)
}

/// A file at the old shared place that may be `app`'s settings from before: one of the shapes
/// the app wrote there, by its first member. `app`'s mark; the framework's envelope from before
/// the mark (`{"schema": n, "settings": {`, its first two members); or the app's own object
/// from before the framework, whose first key is one of `legacy` (none: that shape is not
/// looked for). The full shape is checked on the whole file only after this
/// (`former::recognise`).
pub(crate) fn former<'a>(app: &'a str, legacy: &'a [&'a str]) -> impl Fn(&[u8], bool) -> Head + 'a {
    move |bytes, whole| judged(former_head(&mut Walk::new(bytes), app, legacy), whole, Head::NotOurs)
}

/// Why a walk stopped short of telling the head ours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The bytes are not the expected structure.
    Not,
    /// The bytes ran out while they still matched.
    Short,
}

type Step<T> = Result<T, Stop>;

/// The verdict on a walk: matched is ours; a mismatch is not; bytes that ran out while they
/// still matched need more, or are `cut_short` once they are the whole file.
fn judged(step: Step<()>, whole: bool, cut_short: Head) -> Head {
    match step {
        Ok(()) => Head::Ours,
        Err(Stop::Not) => Head::NotOurs,
        Err(Stop::Short) if whole => cut_short,
        Err(Stop::Short) => Head::More,
    }
}

/// The two members of the mark, by key.
const MARK_KEYS: [&[u8]; 2] = [b"kind", b"app"];

fn mark(walk: &mut Walk, app: &str) -> Step<()> {
    walk.byte(b'{')?;
    let first = walk.string_of(&MARK_KEYS)?;
    marked_from(walk, first, app)
}

/// The rest of the mark, once its first key (`MARK_KEYS[first]`) is read.
fn marked_from(walk: &mut Walk, first: usize, app: &str) -> Step<()> {
    let value = |key: usize| [KIND.as_bytes(), app.as_bytes()][key];
    walk.byte(b':')?;
    walk.string_of(&[value(first)])?;
    walk.byte(b',')?;
    let second = 1 - first;
    walk.string_of(&[MARK_KEYS[second]])?;
    walk.byte(b':')?;
    walk.string_of(&[value(second)])?;
    walk.one_of(b",}")
}

fn former_head(walk: &mut Walk, app: &str, legacy: &[&str]) -> Step<()> {
    walk.byte(b'{')?;
    let mut keys: Vec<&[u8]> = vec![MARK_KEYS[0], MARK_KEYS[1], b"schema"];
    keys.extend(legacy.iter().map(|k| k.as_bytes()));
    match walk.string_of(&keys)? {
        first @ (0 | 1) => marked_from(walk, first, app),
        2 => {
            walk.byte(b':')?;
            walk.digits()?;
            walk.byte(b',')?;
            walk.string_of(&[b"settings"])?;
            walk.byte(b':')?;
            walk.byte(b'{')
        }
        _ => walk.byte(b':'),
    }
}

/// A walk over a document's first bytes, a token at a time.
struct Walk<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Walk<'a> {
    fn new(bytes: &'a [u8]) -> Walk<'a> {
        Walk { bytes, at: 0 }
    }

    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    /// The next token is one of `wants`, a single byte.
    fn one_of(&mut self, wants: &[u8]) -> Step<()> {
        self.space();
        match self.bytes.get(self.at) {
            None => Err(Stop::Short),
            Some(b) if wants.contains(b) => {
                self.at += 1;
                Ok(())
            }
            Some(_) => Err(Stop::Not),
        }
    }

    fn byte(&mut self, want: u8) -> Step<()> {
        self.one_of(&[want])
    }

    /// The next token is a string, exactly one of `wants`: which one. Bytes that run out while
    /// they are still the start of one are short; anything else, an escape included, is not.
    fn string_of(&mut self, wants: &[&[u8]]) -> Step<usize> {
        self.byte(b'"')?;
        let start = self.at;
        loop {
            let so_far = &self.bytes[start..self.at];
            match self.bytes.get(self.at) {
                Some(b'"') => {
                    self.at += 1;
                    return wants.iter().position(|w| *w == so_far).ok_or(Stop::Not);
                }
                None if wants.iter().any(|w| w.starts_with(so_far)) => return Err(Stop::Short),
                None => return Err(Stop::Not),
                Some(_) if !wants.iter().any(|w| w.starts_with(&self.bytes[start..=self.at])) => return Err(Stop::Not),
                Some(_) => self.at += 1,
            }
        }
    }

    /// The next token is a whole number of decimal digits.
    fn digits(&mut self) -> Step<()> {
        self.space();
        let start = self.at;
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        match self.bytes.get(self.at) {
            None => Err(Stop::Short),
            Some(_) if self.at == start => Err(Stop::Not),
            Some(_) => Ok(()),
        }
    }
}

#[cfg(test)]
#[path = "mark_tests.rs"]
mod tests;
