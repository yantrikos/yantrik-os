//! The functions a formula can call.
//!
//! The set is the old engine's core (SUM, AVG/AVERAGE, MIN, MAX, COUNT, IF, VLOOKUP, COUNTIF,
//! SUMIF and the text helpers, 5c6a55aa^), rewritten over values instead of re-parsed strings.
//! Ranges follow every spreadsheet: a blank cell adds 0 to a sum and never fails it, text and
//! truths inside a range are skipped by the arithmetic functions, and the statistical ones —
//! AVERAGE, MIN, MAX, COUNT — count only the cells that hold numbers.

use super::budget;
use super::criteria;
use super::eval::{to_number, Eval, Item};
use super::parser::Expr;
use super::value::{Error, Kind, Value};
use crate::address::Cell;

/// Every name `call` answers to, for the help line and for a mind reading `describe`.
pub const NAMES: &[&str] = &[
    "SUM", "AVERAGE", "AVG", "MIN", "MAX", "COUNT", "COUNTA", "PRODUCT", "ABS", "ROUND", "SQRT",
    "MOD", "IF", "AND", "OR", "NOT", "IFERROR", "CONCAT", "CONCATENATE", "LEN", "UPPER", "LOWER",
    "TRIM", "COUNTIF", "SUMIF", "VLOOKUP",
];

pub fn call(ev: &Eval, name: &str, args: &[Expr]) -> Result<Value, Error> {
    let n = |x: f64| Ok(Value::Number(x));
    match name {
        "SUM" => n(numbers(ev, name, args)?.iter().sum()),
        "PRODUCT" => n(numbers(ev, name, args)?.iter().product()),
        "AVERAGE" | "AVG" => {
            let xs = numbers(ev, name, args)?;
            if xs.is_empty() {
                return Err(Error::new(Kind::DivZero, format!("{name} found no numbers to average")));
            }
            n(xs.iter().sum::<f64>() / xs.len() as f64)
        }
        "MIN" => n(numbers(ev, name, args)?.into_iter().reduce(f64::min).unwrap_or(0.0)),
        "MAX" => n(numbers(ev, name, args)?.into_iter().reduce(f64::max).unwrap_or(0.0)),
        "COUNT" => n(numbers(ev, name, args)?.len() as f64),
        "COUNTA" => {
            let mut count = 0;
            for a in args {
                count += ev.items(a)?.iter().filter(|i| i.value != Value::Empty).count();
            }
            n(count as f64)
        }
        "ABS" => {
            arity(name, args, 1, 1)?;
            n(ev.number(&args[0], "made absolute")?.abs())
        }
        "SQRT" => {
            arity(name, args, 1, 1)?;
            let x = ev.number(&args[0], "square-rooted")?;
            if x < 0.0 {
                return Err(Error::new(Kind::Value, "SQRT was given a negative number"));
            }
            n(x.sqrt())
        }
        "ROUND" => {
            arity(name, args, 1, 2)?;
            let x = ev.number(&args[0], "rounded")?;
            let places = match args.get(1) {
                Some(p) => ev.number(p, "used as a number of places")?.trunc(),
                None => 0.0,
            };
            let f = 10f64.powf(places);
            n((x * f).round() / f)
        }
        "MOD" => {
            arity(name, args, 2, 2)?;
            let x = ev.number(&args[0], "divided")?;
            let d = ev.number(&args[1], "divided")?;
            if d == 0.0 {
                return Err(Error::new(Kind::DivZero, "MOD was asked to divide by zero"));
            }
            n(x - d * (x / d).floor())
        }
        "IF" => {
            arity(name, args, 2, 3)?;
            if ev.truth(&args[0])? {
                ev.value(&args[1])
            } else {
                args.get(2).map_or(Ok(Value::Bool(false)), |e| ev.value(e))
            }
        }
        "AND" | "OR" => {
            arity(name, args, 1, usize::MAX)?;
            let mut all = true;
            let mut any = false;
            for a in args {
                for item in ev.items(a)? {
                    let t = match item.value {
                        Value::Empty | Value::Text(_) if item.from.is_some() => continue,
                        Value::Bool(b) => b,
                        Value::Number(x) => x != 0.0,
                        other => return Err(Error::new(Kind::Value, format!("{name} was given {}, which is not TRUE or FALSE", other.display()))),
                    };
                    all &= t;
                    any |= t;
                }
            }
            Ok(Value::Bool(if name == "AND" { all } else { any }))
        }
        "NOT" => {
            arity(name, args, 1, 1)?;
            Ok(Value::Bool(!ev.truth(&args[0])?))
        }
        "IFERROR" => {
            arity(name, args, 2, 2)?;
            match ev.value(&args[0]) {
                Ok(v) => Ok(v),
                Err(_) => ev.value(&args[1]),
            }
        }
        "CONCAT" | "CONCATENATE" => {
            let (mut out, mut chars) = (String::new(), 0);
            for a in args {
                for item in ev.items(a)? {
                    let piece = item.value.display();
                    chars += piece.chars().count();
                    budget::fits(chars)?;
                    out.push_str(&piece);
                }
            }
            Ok(Value::Text(out))
        }
        "LEN" => {
            arity(name, args, 1, 1)?;
            n(ev.text(&args[0])?.chars().count() as f64)
        }
        "UPPER" | "LOWER" | "TRIM" => {
            arity(name, args, 1, 1)?;
            let t = ev.text(&args[0])?;
            let out = match name {
                "UPPER" => t.to_uppercase(),
                "LOWER" => t.to_lowercase(),
                _ => t.split_whitespace().collect::<Vec<_>>().join(" "),
            };
            // A few letters grow when their case changes ("ß" is "SS").
            budget::fits(out.chars().count())?;
            Ok(Value::Text(out))
        }
        "COUNTIF" => {
            arity(name, args, 2, 2)?;
            let want = ev.value(&args[1])?;
            let count = ev.items(&args[0])?.iter().filter(|i| criteria::matches(&i.value, &want)).count();
            n(count as f64)
        }
        "SUMIF" => {
            arity(name, args, 2, 3)?;
            // Every formula nested in the call is worked out before a range is read, so no copy
            // of a range is alive while another SUMIF inside this one runs: nested 95 deep over
            // 2,000 long texts, that was 95 copies and 6 GB (security delta review of #727).
            // Without a third argument the tested values are summed too, read once.
            let want = ev.value(&args[1])?;
            let is_range = |e: &Expr| matches!(e, Expr::Range(_) | Expr::Ref(_));
            let (tested, summed) = match args.get(2) {
                None => (ev.items(&args[0])?, None),
                Some(e) if is_range(&args[0]) => {
                    let summed = ev.items(e)?;
                    (ev.items(&args[0])?, Some(summed))
                }
                Some(e) => {
                    let tested = ev.items(&args[0])?;
                    (tested, Some(ev.items(e)?))
                }
            };
            let summed = summed.as_ref().unwrap_or(&tested);
            let mut total = 0.0;
            for (t, s) in tested.iter().zip(summed.iter()) {
                if criteria::matches(&t.value, &want) {
                    if let Value::Number(x) = s.value {
                        total += x;
                    }
                }
            }
            n(total)
        }
        "VLOOKUP" => vlookup(ev, args),
        _ => Err(Error::new(Kind::Name, format!("there is no function called {name} in this sheet"))),
    }
}

fn arity(name: &str, args: &[Expr], min: usize, max: usize) -> Result<(), Error> {
    if (min..=max).contains(&args.len()) {
        return Ok(());
    }
    let wanted = match (min, max) {
        (a, b) if a == b => format!("{a}"),
        (a, usize::MAX) => format!("at least {a}"),
        (a, b) => format!("{a} to {b}"),
    };
    Err(Error::new(
        Kind::Value,
        format!("{name} takes {wanted} argument{}, and was given {}", if max == 1 { "" } else { "s" }, args.len()),
    ))
}

/// The numbers a SUM-like function reads. Inside a range or a cell reference only numbers count;
/// a value written straight into the call must be one, or the call says which it was.
fn numbers(ev: &Eval, name: &str, args: &[Expr]) -> Result<Vec<f64>, Error> {
    let mut out = Vec::new();
    for a in args {
        for Item { value, from } in ev.items(a)? {
            match (value, from) {
                (Value::Number(x), _) => out.push(x),
                (Value::Error(e), _) => return Err(e),
                // From a cell: blanks, text and truths are skipped, as every spreadsheet does.
                (_, Some(_)) => {}
                (v, None) => out.push(to_number(&v, None, &format!("used by {name}"))?),
            }
        }
    }
    Ok(out)
}

fn vlookup(ev: &Eval, args: &[Expr]) -> Result<Value, Error> {
    arity("VLOOKUP", args, 3, 4)?;
    let key = ev.value(&args[0])?;
    let Expr::Range(table) = &args[1] else {
        return Err(Error::new(Kind::Value, "VLOOKUP's second argument has to be a range, like A1:C20"));
    };
    let index = ev.number(&args[2], "used as a column number")?.trunc();
    if index < 1.0 {
        return Err(Error::new(Kind::Value, "VLOOKUP's column number starts at 1"));
    }
    let index = index as usize;
    if index > table.cols() {
        return Err(Error::new(
            Kind::Ref,
            format!("VLOOKUP asks for column {index} of {}, which has {}", table.name(), table.cols()),
        ));
    }
    let exact = match args.get(3) {
        Some(e) => !ev.truth(e)?,
        None => false,
    };
    let last_row = table.last.row.min(ev.grid.rows().saturating_sub(1));
    let mut best = None;
    for row in table.first.row..=last_row {
        let v = ev.cell(Cell::new(row, table.first.col))?;
        if same(&v, &key) {
            best = Some(row);
            break;
        }
        if !exact && below(&v, &key) {
            best = Some(row);
        }
    }
    match best {
        Some(row) => ev.cell(Cell::new(row, table.first.col + index - 1)),
        None => Err(Error::new(
            Kind::NotFound,
            format!("VLOOKUP found no row in {} whose first column is {}", table.name(), shown(&key)),
        )),
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => x.eq_ignore_ascii_case(y),
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
    }
}

fn below(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x < y,
        (Value::Text(x), Value::Text(y)) => x.to_lowercase() < y.to_lowercase(),
        _ => false,
    }
}

fn shown(v: &Value) -> String {
    match v {
        Value::Text(t) => format!("\"{t}\""),
        other => other.display(),
    }
}
