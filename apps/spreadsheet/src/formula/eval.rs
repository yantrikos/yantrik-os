//! Working out one formula against the values of the cells it names.
//!
//! The evaluator never recurses into another cell's formula: the sheet works cells out in
//! dependency order (`sheet::recalc`) and hands this a [`Grid`] whose cells are already done, so
//! a chain of ten thousand running totals costs no stack and a loop is found before anything runs.

use super::budget::{self, Budget};
use super::parser::{Expr, Op, MAX_DEPTH};
use super::value::{parse_number, Error, Kind, Value};
use crate::address::{Cell, Range};

/// The cells a formula can read: worked-out values, and how big the sheet is.
pub trait Grid {
    fn value(&self, cell: Cell) -> Value;
    fn rows(&self) -> usize;
    fn cols(&self) -> usize;
}

/// One value a range or an argument produced, and the cell it came from when it came from one.
pub struct Item {
    pub value: Value,
    pub from: Option<Cell>,
}

pub struct Eval<'a> {
    pub grid: &'a dyn Grid,
    /// Shared by every formula of one recalculation.
    pub budget: &'a Budget,
    /// How many `value` calls are open. The parser already refuses a tree deeper than
    /// [`MAX_DEPTH`]; this holds the evaluator to it too, so no tree can reach the stack's end.
    depth: std::cell::Cell<usize>,
}

impl<'a> Eval<'a> {
    pub fn new(grid: &'a dyn Grid, budget: &'a Budget) -> Self {
        Self { grid, budget, depth: std::cell::Cell::new(0) }
    }

    pub fn run(&self, expr: &Expr) -> Value {
        match self.value(expr) {
            Ok(v) => v,
            Err(e) => Value::Error(e),
        }
    }

    /// A single value. A range here is a mistake unless it is one cell.
    pub fn value(&self, expr: &Expr) -> Result<Value, Error> {
        let depth = self.depth.get();
        if depth >= MAX_DEPTH {
            return Err(Error::new(Kind::Syntax, format!("the formula nests more than {MAX_DEPTH} levels deep")));
        }
        self.depth.set(depth + 1);
        let out = self.value_of(expr);
        self.depth.set(depth);
        out
    }

    fn value_of(&self, expr: &Expr) -> Result<Value, Error> {
        Ok(match expr {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(s) => Value::Text(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Ref(cell) => self.cell(*cell)?,
            Expr::Range(r) if r.len() == 1 => self.cell(r.first)?,
            Expr::Range(r) => {
                return Err(Error::new(
                    Kind::Value,
                    format!("the range {} is many cells where one value was needed — wrap it in SUM or another function", r.name()),
                ))
            }
            Expr::Neg(e) => Value::Number(-self.number(e, "made negative")?),
            Expr::Percent(e) => Value::Number(self.number(e, "used as a percentage")? / 100.0),
            Expr::Binary(op, a, b) => self.binary(*op, a, b)?,
            Expr::Call(name, args) => super::functions::call(self, name, args)?,
        })
    }

    /// A cell's value, with an error in it passed on and marked with where it started.
    pub fn cell(&self, cell: Cell) -> Result<Value, Error> {
        self.budget.work(1)?;
        self.read(cell)
    }

    fn read(&self, cell: Cell) -> Result<Value, Error> {
        match self.grid.value(cell) {
            Value::Error(e) => Err(passed_on(e, cell)),
            v => Ok(v),
        }
    }

    /// Every value an argument stands for: each cell of a range, or the one value.
    pub fn items(&self, expr: &Expr) -> Result<Vec<Item>, Error> {
        match expr {
            Expr::Range(r) => self.range_items(r),
            Expr::Ref(c) => Ok(vec![Item { value: self.cell(*c)?, from: Some(*c) }]),
            other => Ok(vec![Item { value: self.value(other)?, from: None }]),
        }
    }

    /// The cells of a range that lie inside the sheet. Everything outside it is blank, so
    /// skipping it changes no sum, count or average.
    pub fn range_items(&self, r: &Range) -> Result<Vec<Item>, Error> {
        let mut out = Vec::new();
        let last_row = r.last.row.min(self.grid.rows().saturating_sub(1));
        let last_col = r.last.col.min(self.grid.cols().saturating_sub(1));
        if r.first.row >= self.grid.rows() || r.first.col >= self.grid.cols() {
            return Ok(out);
        }
        // Paid for before anything is read, so a range too big to read allocates nothing.
        self.budget.work((last_row + 1 - r.first.row) * (last_col + 1 - r.first.col))?;
        for row in r.first.row..=last_row {
            for col in r.first.col..=last_col {
                let cell = Cell::new(row, col);
                out.push(Item { value: self.read(cell)?, from: Some(cell) });
            }
        }
        Ok(out)
    }

    /// The expression as a number, or a sentence naming what could not be one.
    ///
    /// A blank cell is 0 and a truth is 1 or 0, as in every spreadsheet. Text that reads as a
    /// number is that number; any other text is refused, naming the cell it is in.
    pub fn number(&self, expr: &Expr, doing: &str) -> Result<f64, Error> {
        let from = match expr {
            Expr::Ref(c) => Some(*c),
            Expr::Range(r) if r.len() == 1 => Some(r.first),
            _ => None,
        };
        to_number(&self.value(expr)?, from, doing)
    }

    pub fn text(&self, expr: &Expr) -> Result<String, Error> {
        Ok(match self.value(expr)? {
            Value::Number(n) => super::value::format_number(n),
            other => other.display(),
        })
    }

    pub fn truth(&self, expr: &Expr) -> Result<bool, Error> {
        match self.value(expr)? {
            Value::Bool(b) => Ok(b),
            Value::Empty => Ok(false),
            Value::Number(n) => Ok(n != 0.0),
            Value::Text(t) => match t.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" => Ok(false),
                _ => Err(Error::new(Kind::Value, format!("the text \"{t}\" is neither TRUE nor FALSE"))),
            },
            Value::Error(e) => Err(e),
        }
    }

    fn binary(&self, op: Op, a: &Expr, b: &Expr) -> Result<Value, Error> {
        let verb = match op {
            Op::Add => "added",
            Op::Sub => "subtracted",
            Op::Mul => "multiplied",
            Op::Div => "divided",
            Op::Pow => "raised to a power",
            Op::Concat => {
                let (x, y) = (self.text(a)?, self.text(b)?);
                budget::fits(x.chars().count() + y.chars().count())?;
                return Ok(Value::Text(x + &y));
            }
            _ => return self.compare(op, a, b),
        };
        let x = self.number(a, verb)?;
        let y = self.number(b, verb)?;
        let n = match op {
            Op::Add => x + y,
            Op::Sub => x - y,
            Op::Mul => x * y,
            Op::Div if y == 0.0 => {
                return Err(Error::new(Kind::DivZero, divisor_is_zero(b)));
            }
            Op::Div => x / y,
            _ => x.powf(y),
        };
        if n.is_finite() {
            Ok(Value::Number(n))
        } else {
            Err(Error::new(Kind::Value, "the result is too large, or not a number at all"))
        }
    }

    fn compare(&self, op: Op, a: &Expr, b: &Expr) -> Result<Value, Error> {
        use std::cmp::Ordering;
        let x = self.value(a)?;
        let y = self.value(b)?;
        // Blank compares as 0 against a number and as "" against text.
        let order = match (&x, &y) {
            (Value::Text(s), Value::Text(t)) => s.to_lowercase().cmp(&t.to_lowercase()),
            (Value::Text(s), Value::Empty) => s.as_str().cmp(""),
            (Value::Empty, Value::Text(t)) => "".cmp(t.as_str()),
            // Text sorts after every number, as it does in every spreadsheet.
            (Value::Text(_), _) => Ordering::Greater,
            (_, Value::Text(_)) => Ordering::Less,
            _ => {
                let p = to_number(&x, None, "compared")?;
                let q = to_number(&y, None, "compared")?;
                p.partial_cmp(&q).unwrap_or(Ordering::Equal)
            }
        };
        Ok(Value::Bool(match op {
            Op::Eq => order == Ordering::Equal,
            Op::Ne => order != Ordering::Equal,
            Op::Lt => order == Ordering::Less,
            Op::Le => order != Ordering::Greater,
            Op::Gt => order == Ordering::Greater,
            _ => order != Ordering::Less,
        }))
    }
}

/// A value as a number, naming the cell when a cell is what held the text.
pub fn to_number(v: &Value, from: Option<Cell>, doing: &str) -> Result<f64, Error> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Number(n) => Ok(*n),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Text(t) => parse_number(t).ok_or_else(|| {
            let shown: String = t.chars().take(24).collect();
            let what = match from {
                Some(c) => format!("{} holds text (\"{shown}\")", c.name()),
                None => format!("\"{shown}\" is text"),
            };
            Error::new(Kind::Value, format!("{what}, so it can't be {doing}"))
        }),
        Value::Error(e) => Err(e.clone()),
    }
}

fn divisor_is_zero(divisor: &Expr) -> String {
    match divisor {
        Expr::Ref(c) => format!("dividing by {}, which is 0 or blank", c.name()),
        _ => "dividing by zero".into(),
    }
}

/// An error that came from another cell, said once, from where it began.
fn passed_on(mut e: Error, cell: Cell) -> Error {
    if !e.message.starts_with("in ") {
        e.message = format!("in {}: {}", cell.name(), e.message);
    }
    e
}
