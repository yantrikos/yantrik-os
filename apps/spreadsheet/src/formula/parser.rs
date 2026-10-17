//! Tokens into a tree, with the precedence every spreadsheet uses: comparison, then `&`, then
//! `+ -`, then `* /`, then `^`, then a sign, then `%`.

use super::lexer::Token;
use super::value::{Error, Kind};
use crate::address::{self, Cell, Range};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Concat,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Ref(Cell),
    Range(Range),
    Neg(Box<Expr>),
    Percent(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

/// How deep a formula may nest: brackets, function calls, signs, and operators chained on one
/// another. The parser, `references` and the evaluator all walk the tree by recursion, so a
/// formula 16,000 brackets deep overflowed the window's stack and aborted it (security review of
/// #727); past this it is refused as `#ERROR!` with a sentence instead. No hand-written formula
/// comes near it.
pub const MAX_DEPTH: usize = 100;

pub fn parse(tokens: Vec<Token>) -> Result<Expr, Error> {
    if tokens.is_empty() {
        return Err(Error::new(Kind::Syntax, "the formula is empty after the ="));
    }
    let mut p = Parser { tokens, pos: 0, nesting: 0 };
    let (expr, _) = p.comparison()?;
    match p.peek() {
        None => Ok(expr),
        Some(Token::RParen) => Err(Error::new(Kind::Syntax, "there is a ) with no ( to close")),
        Some(t) => Err(Error::new(
            Kind::Syntax,
            format!("{} follows a complete formula — an operator may be missing", describe(t)),
        )),
    }
}

/// A part of the tree, and how deep it goes.
type Parsed = Result<(Expr, usize), Error>;

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    /// How many of the parser's own calls are open, for brackets and signs that add no node.
    nesting: usize,
}

fn too_deep() -> Error {
    Error::new(Kind::Syntax, format!("the formula nests more than {MAX_DEPTH} levels deep"))
}

/// One node over `children`, refused if it would be deeper than [`MAX_DEPTH`].
fn node(expr: Expr, children: usize) -> Parsed {
    if children >= MAX_DEPTH {
        return Err(too_deep());
    }
    Ok((expr, children + 1))
}

fn binary(op: Op, (a, da): (Expr, usize), (b, db): (Expr, usize)) -> Parsed {
    node(Expr::Binary(op, Box::new(a), Box::new(b)), da.max(db))
}

fn describe(t: &Token) -> String {
    match t {
        Token::Number(n) => format!("the number {n}"),
        Token::Text(s) => format!("the text \"{s}\""),
        Token::Word(w) => format!("`{w}`"),
        Token::Comma => "a comma outside any function".into(),
        Token::Colon => "a `:` that is not between two cells".into(),
        Token::LParen => "a (".into(),
        Token::RParen => "a )".into(),
        _ => "an operator".into(),
    }
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += usize::from(t.is_some());
        t
    }

    fn eat(&mut self, want: &Token) -> bool {
        if self.peek() == Some(want) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Run one recursive step, refusing it past [`MAX_DEPTH`] open calls.
    fn nested(&mut self, step: impl FnOnce(&mut Self) -> Parsed) -> Parsed {
        if self.nesting >= MAX_DEPTH {
            return Err(too_deep());
        }
        self.nesting += 1;
        let out = step(self);
        self.nesting -= 1;
        out
    }

    fn comparison(&mut self) -> Parsed {
        self.nested(|p| {
            let mut left = p.concat()?;
            loop {
                let op = match p.peek() {
                    Some(Token::Eq) => Op::Eq,
                    Some(Token::Ne) => Op::Ne,
                    Some(Token::Lt) => Op::Lt,
                    Some(Token::Le) => Op::Le,
                    Some(Token::Gt) => Op::Gt,
                    Some(Token::Ge) => Op::Ge,
                    _ => return Ok(left),
                };
                p.pos += 1;
                left = binary(op, left, p.concat()?)?;
            }
        })
    }

    fn concat(&mut self) -> Parsed {
        let mut left = self.additive()?;
        while self.eat(&Token::Amp) {
            left = binary(Op::Concat, left, self.additive()?)?;
        }
        Ok(left)
    }

    fn additive(&mut self) -> Parsed {
        let mut left = self.term()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => Op::Add,
                Some(Token::Minus) => Op::Sub,
                _ => return Ok(left),
            };
            self.pos += 1;
            left = binary(op, left, self.term()?)?;
        }
    }

    fn term(&mut self) -> Parsed {
        let mut left = self.power()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => Op::Mul,
                Some(Token::Slash) => Op::Div,
                _ => return Ok(left),
            };
            self.pos += 1;
            left = binary(op, left, self.power()?)?;
        }
    }

    fn power(&mut self) -> Parsed {
        let mut left = self.unary()?;
        while self.eat(&Token::Caret) {
            left = binary(Op::Pow, left, self.unary()?)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Parsed {
        if self.eat(&Token::Minus) {
            let (e, d) = self.nested(Self::unary)?;
            return node(Expr::Neg(Box::new(e)), d);
        }
        if self.eat(&Token::Plus) {
            return self.nested(Self::unary);
        }
        let mut e = self.primary()?;
        while self.eat(&Token::Percent) {
            e = node(Expr::Percent(Box::new(e.0)), e.1)?;
        }
        Ok(e)
    }

    fn primary(&mut self) -> Parsed {
        match self.bump() {
            None => Err(Error::new(Kind::Syntax, "the formula ends where a value was expected")),
            Some(Token::Number(n)) => Ok((Expr::Number(n), 1)),
            Some(Token::Text(s)) => Ok((Expr::Text(s), 1)),
            Some(Token::LParen) => {
                let e = self.comparison()?;
                if !self.eat(&Token::RParen) {
                    return Err(Error::new(Kind::Syntax, "a ( is never closed — add the missing )"));
                }
                Ok(e)
            }
            Some(Token::Word(w)) => self.word(w),
            Some(t) => Err(Error::new(Kind::Syntax, format!("{} is where a value was expected", describe(&t)))),
        }
    }

    fn word(&mut self, w: String) -> Parsed {
        if self.eat(&Token::LParen) {
            let name = w.to_ascii_uppercase();
            let (mut args, mut deepest) = (Vec::new(), 0);
            if !self.eat(&Token::RParen) {
                loop {
                    let (arg, d) = self.comparison()?;
                    args.push(arg);
                    deepest = deepest.max(d);
                    if self.eat(&Token::RParen) {
                        break;
                    }
                    if !self.eat(&Token::Comma) {
                        return Err(Error::new(
                            Kind::Syntax,
                            format!("{name}( is never closed — add the missing )"),
                        ));
                    }
                }
            }
            return node(Expr::Call(name, args), deepest);
        }
        match w.to_ascii_uppercase().as_str() {
            "TRUE" => return Ok((Expr::Bool(true), 1)),
            "FALSE" => return Ok((Expr::Bool(false), 1)),
            _ => {}
        }
        let Some(first) = address::parse_cell(&w) else {
            return Err(Error::new(
                Kind::Name,
                format!("`{w}` is not a cell or a function this sheet knows"),
            ));
        };
        if self.eat(&Token::Colon) {
            let Some(Token::Word(end)) = self.bump() else {
                return Err(Error::new(Kind::Syntax, format!("the range starting at {} has no end cell", first.name())));
            };
            let Some(last) = address::parse_cell(&end) else {
                return Err(Error::new(Kind::Name, format!("`{end}` is not a cell, so {w}:{end} is not a range")));
            };
            return Ok((Expr::Range(Range::new(first, last)), 1));
        }
        Ok((Expr::Ref(first), 1))
    }
}

#[cfg(test)]
mod tests {
    use super::super::lexer::tokenize;
    use super::*;

    fn p(s: &str) -> Result<Expr, Error> {
        parse(tokenize(s)?)
    }

    #[test]
    fn precedence_is_the_spreadsheet_one() {
        // 1 + (2 * 3), and a sign binds tighter than ^ as it does everywhere else in a sheet.
        assert_eq!(
            p("1+2*3").unwrap(),
            Expr::Binary(
                Op::Add,
                Box::new(Expr::Number(1.0)),
                Box::new(Expr::Binary(Op::Mul, Box::new(Expr::Number(2.0)), Box::new(Expr::Number(3.0))))
            )
        );
        assert_eq!(
            p("-2^2").unwrap(),
            Expr::Binary(Op::Pow, Box::new(Expr::Neg(Box::new(Expr::Number(2.0)))), Box::new(Expr::Number(2.0)))
        );
    }

    #[test]
    fn each_kind_of_mistake_is_named() {
        assert!(p("(1+2").unwrap_err().message.contains("never closed"));
        assert!(p("1+2)").unwrap_err().message.contains("no ( to close"));
        assert!(p("1 2").unwrap_err().message.contains("operator may be missing"));
        assert!(p("1+").unwrap_err().message.contains("ends where a value"));
        assert_eq!(p("hello").unwrap_err().kind, Kind::Name);
        assert!(p("SUM(A1,").unwrap_err().message.contains("ends where a value"));
        assert!(p("").unwrap_err().message.contains("empty"));
    }
}
