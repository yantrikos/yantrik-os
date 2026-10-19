//! A formula's text, cut into tokens.

use super::value::{Error, Kind};

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(f64),
    Text(String),
    /// A name: a function, a cell reference, `TRUE`/`FALSE`. Kept as written; the parser decides.
    Word(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Amp,
    Percent,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LParen,
    RParen,
    Comma,
    Colon,
}

pub fn tokenize(input: &str) -> Result<Vec<Token>, Error> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        let token = match c {
            ' ' | '\t' | '\n' | '\r' => continue,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '^' => Token::Caret,
            '&' => Token::Amp,
            '%' => Token::Percent,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' | ';' => Token::Comma,
            ':' => Token::Colon,
            '=' => Token::Eq,
            '<' => match chars.get(i) {
                Some('=') => { i += 1; Token::Le }
                Some('>') => { i += 1; Token::Ne }
                _ => Token::Lt,
            },
            '>' => match chars.get(i) {
                Some('=') => { i += 1; Token::Ge }
                _ => Token::Gt,
            },
            '"' => {
                let mut text = String::new();
                loop {
                    match chars.get(i) {
                        None => {
                            return Err(Error::new(
                                Kind::Syntax,
                                "a quoted piece of text is never closed — add the missing \"",
                            ))
                        }
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            text.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&ch) => {
                            text.push(ch);
                            i += 1;
                        }
                    }
                }
                Token::Text(text)
            }
            c if c.is_ascii_digit() || (c == '.' && chars.get(i).is_some_and(char::is_ascii_digit)) => {
                let start = i - 1;
                while chars.get(i).is_some_and(|ch| ch.is_ascii_digit() || *ch == '.') {
                    i += 1;
                }
                // An exponent: 1e3, 2.5E-4.
                if matches!(chars.get(i), Some('e' | 'E'))
                    && (chars.get(i + 1).is_some_and(char::is_ascii_digit)
                        || (matches!(chars.get(i + 1), Some('+' | '-'))
                            && chars.get(i + 2).is_some_and(char::is_ascii_digit)))
                {
                    i += 2;
                    while chars.get(i).is_some_and(char::is_ascii_digit) {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                // A number run straight into letters — `12abc` — is neither.
                if chars.get(i).is_some_and(|ch| ch.is_ascii_alphabetic()) {
                    return Err(Error::new(Kind::Syntax, format!("`{text}` runs straight into letters")));
                }
                let n = text
                    .parse::<f64>()
                    .map_err(|_| Error::new(Kind::Syntax, format!("`{text}` is not a number")))?;
                Token::Number(n)
            }
            c if c.is_alphabetic() || c == '$' || c == '_' => {
                let start = i - 1;
                while chars.get(i).is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '$' | '_' | '.')) {
                    i += 1;
                }
                Token::Word(chars[start..i].iter().collect())
            }
            other => {
                return Err(Error::new(
                    Kind::Syntax,
                    format!("the character `{other}` has no meaning in a formula"),
                ))
            }
        };
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_a_formula_into_its_parts() {
        let t = tokenize(r#"SUM(A1:$B$2) <> "a""b" & 1.5e2%"#).unwrap();
        assert_eq!(
            t,
            vec![
                Token::Word("SUM".into()),
                Token::LParen,
                Token::Word("A1".into()),
                Token::Colon,
                Token::Word("$B$2".into()),
                Token::RParen,
                Token::Ne,
                Token::Text("a\"b".into()),
                Token::Amp,
                Token::Number(150.0),
                Token::Percent,
            ]
        );
    }

    #[test]
    fn an_unclosed_quote_says_so() {
        let e = tokenize("\"abc").unwrap_err();
        assert_eq!(e.kind, Kind::Syntax);
        assert!(e.message.contains("never closed"));
    }
}
