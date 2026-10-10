//! Formula evaluation for the spreadsheet MVP.
//!
//! A tiny recursive-descent parser: tokenise, then expression/term/factor. Supported: numbers,
//! `+ - * /`, parentheses, cell references `A1`-`Z99`, and `SUM`/`AVG`/`MIN`/`MAX` over a
//! rectangular range like `A1:B3`.
//!
//! MVP limit: there is no recursion into other formulas. A referenced cell that itself holds a
//! formula is read as its raw text, and if that text is not a number the reference is `#REF!`.

/// Evaluate `input`, which may or may not start with `=`.
///
/// `cell` reads the raw text of the cell at `(row, col)` (both zero-based), or `None` when the
/// reference is out of range.
pub fn eval_formula(
    input: &str,
    cell: impl Fn(i32, i32) -> Option<String>,
) -> Result<String, String> {
    let input = input.strip_prefix('=').unwrap_or(input);
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, pos: 0, cell: &cell };
    let value = parser.expression()?;
    if parser.pos != parser.tokens.len() {
        return Err("#REF!".into());
    }
    Ok(format_number(value))
}

fn format_number(value: f64) -> String {
    if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    LParen,
    RParen,
    Comma,
    Colon,
    Ident(String),
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {}
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '*' => tokens.push(Token::Star),
            '/' => tokens.push(Token::Slash),
            '(' => tokens.push(Token::LParen),
            ')' => tokens.push(Token::RParen),
            ',' => tokens.push(Token::Comma),
            ':' => tokens.push(Token::Colon),
            c if c.is_ascii_digit() || c == '.' => {
                let mut num = String::new();
                num.push(c);
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() || n == '.' {
                        num.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value: f64 = num.parse().map_err(|_| "#REF!".to_string())?;
                tokens.push(Token::Number(value));
            }
            c if c.is_ascii_alphabetic() => {
                let mut ident = String::new();
                ident.push(c);
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() {
                        ident.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Ident(ident));
            }
            _ => return Err("#REF!".into()),
        }
    }
    Ok(tokens)
}

struct Parser<'a, F> {
    tokens: Vec<Token>,
    pos: usize,
    cell: &'a F,
}

impl<F: Fn(i32, i32) -> Option<String>> Parser<'_, F> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.next();
                    value += self.term()?;
                }
                Some(Token::Minus) => {
                    self.next();
                    value -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.factor()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.next();
                    value *= self.factor()?;
                }
                Some(Token::Slash) => {
                    self.next();
                    let divisor = self.factor()?;
                    if divisor == 0.0 {
                        return Err("#DIV/0!".into());
                    }
                    value /= divisor;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn factor(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Token::Number(n)) => Ok(n),
            Some(Token::Minus) => Ok(-self.factor()?),
            Some(Token::Plus) => self.factor(),
            Some(Token::LParen) => {
                let value = self.expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(value),
                    _ => Err("#REF!".into()),
                }
            }
            Some(Token::Ident(name)) => {
                let upper = name.to_ascii_uppercase();
                if matches!(upper.as_str(), "SUM" | "AVG" | "MIN" | "MAX") {
                    self.function(&upper)
                } else {
                    self.reference(&upper)
                }
            }
            _ => Err("#REF!".into()),
        }
    }

    fn function(&mut self, name: &str) -> Result<f64, String> {
        // Expect `(A1:B3)`.
        if self.next() != Some(Token::LParen) {
            return Err("#REF!".into());
        }
        let (r1, c1) = self.cell_ref()?;
        if self.next() != Some(Token::Colon) {
            return Err("#REF!".into());
        }
        let (r2, c2) = self.cell_ref()?;
        if self.next() != Some(Token::RParen) {
            return Err("#REF!".into());
        }

        let (top, bottom) = if r1 <= r2 { (r1, r2) } else { (r2, r1) };
        let (left, right) = if c1 <= c2 { (c1, c2) } else { (c2, c1) };

        let mut values = Vec::new();
        for row in top..=bottom {
            for col in left..=right {
                let raw = (self.cell)(row, col).ok_or("#REF!")?;
                let value: f64 = raw.trim().parse().map_err(|_| "#REF!")?;
                values.push(value);
            }
        }

        match name {
            "SUM" => Ok(values.iter().sum()),
            "AVG" => Ok(values.iter().sum::<f64>() / values.len() as f64),
            "MIN" => Ok(values.iter().copied().fold(f64::INFINITY, f64::min)),
            "MAX" => Ok(values.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
            _ => Err("#REF!".into()),
        }
    }

    fn reference(&mut self, name: &str) -> Result<f64, String> {
        let (row, col) = parse_ref(name)?;
        let raw = (self.cell)(row, col).ok_or("#REF!")?;
        raw.trim().parse::<f64>().map_err(|_| "#REF!".into())
    }

    fn cell_ref(&mut self) -> Result<(i32, i32), String> {
        match self.next() {
            Some(Token::Ident(name)) => parse_ref(&name.to_ascii_uppercase()),
            _ => Err("#REF!".into()),
        }
    }
}

/// Parse a reference like `A1` into zero-based `(row, col)`. Columns are letters `A`-`Z`, rows
/// are digits `1`-`99`.
fn parse_ref(name: &str) -> Result<(i32, i32), String> {
    let bytes = name.as_bytes();
    if bytes.len() < 2 {
        return Err("#REF!".into());
    }
    let col_letter = bytes[0];
    if !col_letter.is_ascii_uppercase() {
        return Err("#REF!".into());
    }
    let col = (col_letter - b'A') as i32;
    let row: i32 = name[1..].parse().map_err(|_| "#REF!")?;
    if !(1..=99).contains(&row) {
        return Err("#REF!".into());
    }
    Ok((row - 1, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid<'a>(rows: &'a [&'a [&'a str]]) -> impl Fn(i32, i32) -> Option<String> + 'a {
        move |row, col| {
            rows.get(row as usize)
                .and_then(|r| r.get(col as usize))
                .map(|s| s.to_string())
        }
    }

    #[test]
    fn arithmetic_and_parentheses() {
        assert_eq!(eval_formula("=1+2*3", |_, _| None).unwrap(), "7");
        assert_eq!(eval_formula("=(1+2)*3", |_, _| None).unwrap(), "9");
    }

    #[test]
    fn sum_over_range() {
        let cell = grid(&[&["1", "2"], &["3", "4"]]);
        assert_eq!(eval_formula("=SUM(A1:B2)", cell).unwrap(), "10");
    }

    #[test]
    fn division_by_zero() {
        let cell = grid(&[&["1", "0"]]);
        assert_eq!(eval_formula("=A1/B1", cell).unwrap_err(), "#DIV/0!");
    }

    #[test]
    fn text_reference_is_ref() {
        let cell = grid(&[&["hello"]]);
        assert_eq!(eval_formula("=A1+1", cell).unwrap_err(), "#REF!");
    }
}
