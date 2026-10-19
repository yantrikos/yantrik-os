//! Working every cell out, each formula after the cells it reads.
//!
//! The old engine (5c6a55aa^) evaluated formulas in one pass in reading order, so `=B1` in A1
//! read B1's value from before the edit, and a loop between two cells was never noticed. Here the
//! formulas are put in dependency order first (Kahn's algorithm over the cells and ranges each
//! one names); whatever cannot be ordered is in a loop or depends on one, and says so.

use std::collections::VecDeque;

use crate::address::Cell;
use crate::formula::{self, budget::Budget, eval::Eval, eval::Grid, Error, Kind, Value};

struct View<'a> {
    rows: usize,
    cols: usize,
    values: &'a [Value],
}

impl Grid for View<'_> {
    fn value(&self, cell: Cell) -> Value {
        if cell.row < self.rows && cell.col < self.cols {
            self.values[cell.row * self.cols + cell.col].clone()
        } else {
            Value::Empty
        }
    }
    fn rows(&self) -> usize {
        self.rows
    }
    fn cols(&self) -> usize {
        self.cols
    }
}

/// The value of every cell of a `rows × cols` grid whose text is `raw`, row by row.
pub fn recalc(rows: usize, cols: usize, raw: &[String]) -> Vec<Value> {
    let mut values: Vec<Value> = Vec::with_capacity(raw.len());
    let mut formulas: Vec<(usize, Result<formula::Expr, Error>)> = Vec::new();
    for (i, text) in raw.iter().enumerate() {
        if formula::is_formula(text) {
            let looped = Error::new(
                Kind::Cycle,
                format!(
                    "{} is part of a loop of references (it ends up reading itself), or reads a cell that is",
                    at(i, cols).name()
                ),
            );
            values.push(Value::Error(looped));
            formulas.push((i, formula::compile(text)));
        } else {
            values.push(Value::from_entry(text));
        }
    }

    // Where each formula sits in `formulas`, by grid index, so a reference finds its formula.
    let mut slot = vec![usize::MAX; raw.len()];
    for (k, (i, _)) in formulas.iter().enumerate() {
        slot[*i] = k;
    }

    // The formulas of each column, by row, so a range visits only the formulas inside it. The
    // first version scanned every formula for every range, O(formulas² × ranges): a 9 MB file of
    // `=SUM(An:An)` did not open in five minutes (security review of #727).
    let mut by_col: Vec<Vec<(usize, usize)>> = vec![Vec::new(); cols];
    for (k, (i, _)) in formulas.iter().enumerate() {
        let c = at(*i, cols);
        by_col[c.col].push((c.row, k));
    }

    // Edges: a formula another formula reads must be worked out first. Every link costs a unit
    // of a budget of its own, so a sheet whose ranges overlap without end is bounded too: a
    // formula past it is not worked out, and says why.
    let links = Budget::default();
    let mut refused: Vec<Option<Error>> = vec![None; formulas.len()];
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); formulas.len()];
    let mut waiting = vec![0usize; formulas.len()];
    for (k, (_, compiled)) in formulas.iter().enumerate() {
        let Ok(expr) = compiled else { continue };
        let (mut cells, mut ranges) = (Vec::new(), Vec::new());
        formula::references(expr, &mut cells, &mut ranges);
        let mut deps: Vec<usize> = cells
            .iter()
            .filter(|c| c.row < rows && c.col < cols)
            .map(|c| slot[c.row * cols + c.col])
            .filter(|&d| d != usize::MAX)
            .collect();
        let found = ranges.iter().try_for_each(|r| {
            let last = r.last.col.min(cols.saturating_sub(1));
            for column in by_col.get(r.first.col..=last).unwrap_or_default() {
                links.work(1)?;
                let start = column.partition_point(|&(row, _)| row < r.first.row);
                for &(row, d) in &column[start..] {
                    if row > r.last.row {
                        break;
                    }
                    links.work(1)?;
                    deps.push(d);
                }
            }
            Ok(())
        });
        if let Err(e) = found {
            refused[k] = Some(e);
            continue;
        }
        deps.sort_unstable();
        deps.dedup();
        waiting[k] = deps.len();
        for d in deps {
            readers[d].push(k);
        }
    }

    let budget = Budget::default();
    let mut ready: VecDeque<usize> = (0..formulas.len()).filter(|&k| waiting[k] == 0).collect();
    while let Some(k) = ready.pop_front() {
        let (i, compiled) = &formulas[k];
        let v = match (compiled, &refused[k]) {
            (_, Some(e)) | (Err(e), _) => Value::Error(e.clone()),
            (Ok(expr), None) => {
                let view = View { rows, cols, values: &values };
                let v = Eval::new(&view, &budget).run(expr);
                match v {
                    Value::Text(t) => match budget.text(t.len()) {
                        Ok(()) => Value::Text(t),
                        Err(e) => Value::Error(e),
                    },
                    v => v,
                }
            }
        };
        values[*i] = v;
        for &r in &readers[k] {
            waiting[r] -= 1;
            if waiting[r] == 0 {
                ready.push_back(r);
            }
        }
    }
    // Anything still waiting keeps the loop error it started with.
    values
}

fn at(i: usize, cols: usize) -> Cell {
    Cell::new(i / cols.max(1), i % cols.max(1))
}
