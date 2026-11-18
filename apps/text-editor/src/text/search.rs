//! Finding text in a document, and replacing what was found.

use super::limits::{validate, MAX_BYTES};

/// Every match of `query` in `text`, as byte ranges. Regex does the Unicode case folding, so the
/// offsets are the document's own, never offsets into a lowercased copy.
pub fn matches(text: &str, query: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return vec![];
    }
    regex::RegexBuilder::new(&regex::escape(query))
        .case_insensitive(!case_sensitive)
        .build()
        .map(|r| r.find_iter(text).map(|m| (m.start(), m.end())).collect())
        .unwrap_or_default()
}

/// `text` with each of `ranges` replaced by `replacement`, or why not: the result must still be a
/// document ([`validate`]).
pub fn replace(text: &str, ranges: &[(usize, usize)], replacement: &str) -> Result<String, String> {
    let removed: usize = ranges.iter().map(|(a, z)| z - a).sum();
    let added = replacement.len().checked_mul(ranges.len()).ok_or("Replacement is too large")?;
    let size = text
        .len()
        .checked_sub(removed)
        .and_then(|n| n.checked_add(added))
        .ok_or("Replacement is too large")?;
    if size > MAX_BYTES {
        return Err("Replacement would exceed the 1 MiB document limit.".into());
    }
    let mut result = String::with_capacity(size);
    let mut end = 0;
    for &(a, z) in ranges {
        result.push_str(&text[end..a]);
        result.push_str(replacement);
        end = z;
    }
    result.push_str(&text[end..]);
    validate(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_inputs_and_unicode_search() {
        let text = "a".repeat(20000);
        let ranges = matches(&text, "a", true);
        assert_eq!(ranges.len(), 20000);
        assert_eq!(replace(&text, &ranges, "b").unwrap(), "b".repeat(20000));
        assert!(replace(&text, &ranges, &"x".repeat(1024)).is_err());
        // The middle one is the Kelvin sign, U+212A, which folds to `k`.
        let text = "İ α Kelvin \u{212A} kelvin 🦀";
        let ranges = matches(text, "k", false);
        assert_eq!(ranges.iter().map(|&(a, z)| &text[a..z]).collect::<Vec<_>>(), ["K", "\u{212A}", "k"]);
        assert_eq!(matches("one ONE", "one", true), [(0, 3)]);
        assert!(matches("anything", "", false).is_empty());
    }
}
