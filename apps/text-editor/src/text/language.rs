//! The language a file's name says it is in, and the editor's small lexical colouring of it.

use std::path::Path;

/// The language a path's extension names. A tab with no file yet is plain text.
pub fn language(path: Option<&Path>) -> &'static str {
    match path.and_then(|p| p.extension()).and_then(|s| s.to_str()).unwrap_or("") {
        "rs" => "Rust",
        "py" => "Python",
        "js" | "ts" | "tsx" | "jsx" => "JavaScript / TypeScript",
        "json" => "JSON",
        "md" => "Markdown",
        "toml" => "TOML",
        "sh" => "Shell",
        "c" | "h" | "cpp" => "C / C++",
        _ => "Plain text",
    }
}

/// The three colour layers drawn over the text: keywords, strings, comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layers {
    pub keywords: String,
    pub strings: String,
    pub comments: String,
}

impl Layers {
    pub fn is_empty(&self) -> bool {
        self.keywords.is_empty()
    }
}

const WORDS: [&str; 39] = [
    "fn", "let", "mut", "pub", "use", "mod", "struct", "impl", "enum", "match", "if", "else", "return",
    "for", "in", "while", "loop", "true", "false", "None", "Some", "const", "def", "class", "import",
    "from", "as", "with", "try", "except", "async", "await", "function", "export", "default", "var",
    "null", "new", "self",
];

/// A small lexical highlighter: no parser, no background work, no node per token. Each layer is
/// the text with everything but its own characters blanked, so it lines up with the text exactly
/// in the same font. Lines that are not plain ASCII, or hold a tab, are left uncoloured.
pub fn highlight(text: &str, language: &str) -> Layers {
    if text.len() > 65536 || language == "Plain text" {
        return Layers::default();
    }
    let mut layers = [String::new(), String::new(), String::new()];
    for line in text.split_inclusive('\n') {
        let mut masks = [vec![b' '; line.len()], vec![b' '; line.len()], vec![b' '; line.len()]];
        for (i, b) in line.bytes().enumerate() {
            if b == b'\n' || b == b'\r' {
                for m in &mut masks {
                    m[i] = b;
                }
            }
        }
        if line.is_ascii() && !line.contains('\t') {
            colour(line.as_bytes(), language, &mut masks);
        }
        for (out, mask) in layers.iter_mut().zip(masks) {
            // Every byte is ASCII: the line's own, a space, or a line break.
            out.push_str(&String::from_utf8(mask).unwrap_or_default());
        }
    }
    let [keywords, strings, comments] = layers;
    Layers { keywords, strings, comments }
}

/// One ASCII line's keywords, strings and comment, into their masks.
fn colour(bytes: &[u8], language: &str, masks: &mut [Vec<u8>; 3]) {
    let mut i = 0;
    while i < bytes.len() {
        let hash_comment = bytes[i] == b'#' && matches!(language, "Python" | "Shell" | "TOML" | "Markdown");
        if hash_comment || (bytes[i..].starts_with(b"//") && language != "JSON") {
            masks[2][i..].copy_from_slice(&bytes[i..]);
            return;
        }
        if matches!(bytes[i], b'\'' | b'"') {
            let (start, quote) = (i, bytes[i]);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                i += 1;
                if bytes[i - 1] == quote {
                    break;
                }
            }
            masks[1][start..i].copy_from_slice(&bytes[start..i]);
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1
            }
            if WORDS.iter().any(|w| w.as_bytes() == &bytes[start..i]) {
                masks[0][start..i].copy_from_slice(&bytes[start..i]);
            }
            continue;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_line_up_with_the_text() {
        let text = "fn main() { let s = \"hi\"; } // done\n";
        let l = highlight(text, "Rust");
        assert_eq!(l.keywords.len(), text.len());
        assert!(l.keywords.starts_with("fn") && l.keywords.contains("let"));
        assert!(l.strings.contains("\"hi\""));
        assert!(l.comments.trim_end().ends_with("// done"));
        assert!(highlight(text, "Plain text").is_empty());
        assert_eq!(language(Some(Path::new("/a/b.rs"))), "Rust");
        assert_eq!(language(None), "Plain text");
    }
}
