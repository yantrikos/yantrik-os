//! Git's C-quoted paths (#1247, review N2).
//!
//! `git status --short`, a stat and a stash's long output wrap a path in double quotes when it
//! holds a space, a `"`, a `\`, a control byte or (by default) a non-ASCII character:
//! `A  "tokens (1).json"`, `"a\"b/tokens.json" | 1 +`, `"\303\251/id_rsa"`. A name rule that sees
//! the quotes sees no secret, so [`unquoted`] gives the line as its paths are really spelled.

/// `line` with every `"…"` span replaced by the path it quotes: `\"`, `\\`, `\a \b \f \n \r \t \v`
/// and `\NNN` octal bytes decoded, the bytes read as UTF-8 (a bad byte becomes U+FFFD). A quote
/// that never closes is left as it is.
pub fn unquoted(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('"') {
        out.push_str(&rest[..open]);
        let body = &rest[open + 1..];
        match close_of(body) {
            Some(close) => {
                out.push_str(&decode(&body[..close]));
                rest = &body[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Where the quote that ends `body` is: the first `"` that no backslash escapes.
fn close_of(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'"' => return Some(at),
            _ => at += 1,
        }
    }
    None
}

fn decode(quoted: &str) -> String {
    let bytes = quoted.as_bytes();
    let mut raw = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' || at + 1 >= bytes.len() {
            raw.push(bytes[at]);
            at += 1;
            continue;
        }
        let octal = bytes[at + 1..].iter().take(3).take_while(|b| (b'0'..=b'7').contains(b)).count();
        if octal == 3 {
            raw.push(bytes[at + 1..at + 4].iter().fold(0u8, |sum, b| sum.wrapping_mul(8).wrapping_add(b - b'0')));
            at += 4;
            continue;
        }
        raw.push(match bytes[at + 1] {
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 11,
            other => other,
        });
        at += 2;
    }
    String::from_utf8_lossy(&raw).into_owned()
}

#[cfg(test)]
mod tests {
    use super::unquoted;

    #[test]
    fn a_quoted_path_is_spelled_as_it_is() {
        assert_eq!(unquoted(r#"A  "tokens (1).json""#), "A  tokens (1).json");
        assert_eq!(unquoted(r#" "a\"b/tokens.json" | 1 +"#), r#" a"b/tokens.json | 1 +"#);
        assert_eq!(unquoted(r#"?? "a\\b/x""#), r"?? a\b/x");
        assert_eq!(unquoted(r#""\303\251/id_rsa""#), "é/id_rsa");
        assert_eq!(unquoted(r#"R  "a b" -> "c\td""#), "R  a b -> c\td");
        assert_eq!(unquoted("plain.txt | 2 +-"), "plain.txt | 2 +-");
        assert_eq!(unquoted(r#"A  "never closed"#), r#"A  "never closed"#);
    }
}
