//! Find and replace over the document's text.

use crate::limits::{validate, MAX_BYTES};

/// Every match of `query`, case-insensitively, as byte ranges into `text`.
///
/// Through `regex` rather than by lowercasing a copy: case folding changes byte lengths for some
/// characters, so offsets taken from a lowercased string do not point where they claim to in the
/// original, and a replace built on them cuts a document in the wrong place.
pub fn matches(text: &str, query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(true)
        .build()
        .map(|r| r.find_iter(text).map(|m| (m.start(), m.end())).collect())
        .unwrap_or_default()
}

/// Replace the given ranges, checking the resulting size before allocating for it.
pub fn replace(text: &str, ranges: &[(usize, usize)], replacement: &str) -> Result<String, String> {
    let removed: usize = ranges.iter().map(|(a, z)| z - a).sum();
    let added = replacement
        .len()
        .checked_mul(ranges.len())
        .ok_or("That replacement is too large.")?;
    let size = text
        .len()
        .checked_sub(removed)
        .and_then(|n| n.checked_add(added))
        .ok_or("That replacement is too large.")?;
    if size > MAX_BYTES {
        return Err("That replacement would push the document past the 1 MiB limit.".into());
    }
    let mut out = String::with_capacity(size);
    let mut end = 0;
    for &(a, z) in ranges {
        out.push_str(&text[end..a]);
        out.push_str(replacement);
        end = z;
    }
    out.push_str(&text[end..]);
    validate(&out)?;
    Ok(out)
}

/// Replace every match, and say how many there were. Zero is not a failure; it is a count.
pub fn replace_all(text: &str, query: &str, with: &str) -> Result<(String, usize), String> {
    if query.is_empty() {
        return Err("Say what to find.".into());
    }
    let ranges = matches(text, query);
    let count = ranges.len();
    Ok((replace(text, &ranges, with)?, count))
}
