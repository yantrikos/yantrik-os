//! The formula language: `=SUM(A1:A5)*2`, `=IF(B2>0, "yes", "no")`, `=VLOOKUP(...)`.
//!
//! Text → [`lexer`] → [`parser`] → a tree that [`eval`] works out against a [`eval::Grid`].

pub mod budget;
pub mod criteria;
pub mod eval;
pub mod functions;
pub mod lexer;
pub mod parser;
pub mod value;

#[cfg(test)]
mod tests;

use crate::address::{Cell, Range};
pub use parser::Expr;
pub use value::{Error, Kind, Value};

/// Whether a cell's text is a formula rather than a value.
pub fn is_formula(raw: &str) -> bool {
    raw.trim_start().starts_with('=') && raw.trim().len() > 1
}

/// Read a formula, with or without its leading `=`.
pub fn compile(raw: &str) -> Result<Expr, Error> {
    let body = raw.trim_start();
    let body = body.strip_prefix('=').unwrap_or(body);
    parser::parse(lexer::tokenize(body)?)
}

/// Every cell and range a formula reads, for working cells out in the right order.
pub fn references(expr: &Expr, cells: &mut Vec<Cell>, ranges: &mut Vec<Range>) {
    match expr {
        Expr::Ref(c) => cells.push(*c),
        Expr::Range(r) => ranges.push(*r),
        Expr::Neg(e) | Expr::Percent(e) => references(e, cells, ranges),
        Expr::Binary(_, a, b) => {
            references(a, cells, ranges);
            references(b, cells, ranges);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| references(a, cells, ranges)),
        Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) => {}
    }
}
