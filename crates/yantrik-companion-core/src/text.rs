//! Small text helpers shared by the companion crates and the shell.

/// The word with its first letter in capitals and the rest as it was.
pub fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A source file's code: no comment lines, and nothing from its first `#[cfg(test)]` on. What
/// the source-reading tests look at when they ask whether a file can name a thing.
pub fn code_only(source: &str) -> String {
    let own = source.split("#[cfg(test)]").next().unwrap_or_default();
    own.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::{capitalize, code_only};

    #[test]
    fn code_only_drops_comments_and_tests() {
        let src = "// max_permission\nfn a() {}\n#[cfg(test)]\nmod t { /* ceiling */ }";
        assert_eq!(code_only(src), "fn a() {}");
    }

    #[test]
    fn the_first_letter_only() {
        assert_eq!(capitalize("notes"), "Notes");
        assert_eq!(capitalize("éclair"), "Éclair");
        assert_eq!(capitalize("iOS"), "IOS");
        assert_eq!(capitalize(""), "");
    }
}
