//! Cell addresses: `A1`, `$AB$12`, and the ranges between two of them.
//!
//! Ported from the shell's old in-process spreadsheet (`parse_cell_ref`, `parse_range`,
//! `col_to_letter`, deleted with the shell's copy of the apps in 5c6a55aa), which already read
//! multi-letter columns and the `$` of an absolute reference. What changed: a column is counted
//! the way every spreadsheet counts it (`Z` is 25, `AA` is 26 — the old fold made `AA` 0), and a
//! reference too large to be a cell is refused rather than overflowing.

/// The largest row and column an address may name. Far beyond what the window draws; the limit
/// is there so `ZZZZZZZZ999999999999` is "not a cell" rather than an overflow.
pub const MAX_ROWS: usize = 1_000_000;
pub const MAX_COLS: usize = 18_278; // ZZZ

/// One cell, zero-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Cell {
    pub row: usize,
    pub col: usize,
}

impl Cell {
    pub fn new(row: usize, col: usize) -> Self {
        Self { row, col }
    }

    /// `A1`-style name of this cell.
    pub fn name(self) -> String {
        format!("{}{}", col_name(self.col), self.row + 1)
    }
}

/// A rectangle of cells, corners included, top-left first whichever way it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub first: Cell,
    pub last: Cell,
}

impl Range {
    pub fn new(a: Cell, b: Cell) -> Self {
        Self {
            first: Cell::new(a.row.min(b.row), a.col.min(b.col)),
            last: Cell::new(a.row.max(b.row), a.col.max(b.col)),
        }
    }

    pub fn rows(&self) -> usize {
        self.last.row - self.first.row + 1
    }

    pub fn cols(&self) -> usize {
        self.last.col - self.first.col + 1
    }

    pub fn len(&self) -> usize {
        self.rows() * self.cols()
    }

    /// Every cell, row by row.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (self.first.row..=self.last.row)
            .flat_map(move |r| (self.first.col..=self.last.col).map(move |c| Cell::new(r, c)))
    }

    pub fn name(&self) -> String {
        if self.first == self.last {
            self.first.name()
        } else {
            format!("{}:{}", self.first.name(), self.last.name())
        }
    }
}

/// `0 → A`, `25 → Z`, `26 → AA`, `701 → ZZ`.
pub fn col_name(col: usize) -> String {
    let mut n = col + 1;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n - 1) % 26;
        out.push(b'A' + rem as u8);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Read `A1`, `b7` or `$C$3`. `None` for anything that is not exactly one cell.
pub fn parse_cell(text: &str) -> Option<Cell> {
    let s = text.trim().as_bytes();
    let mut i = 0;
    if s.get(i) == Some(&b'$') {
        i += 1;
    }
    let mut col = 0usize;
    let letters = i;
    while let Some(b) = s.get(i).filter(|b| b.is_ascii_alphabetic()) {
        col = col.checked_mul(26)?.checked_add((b.to_ascii_uppercase() - b'A') as usize + 1)?;
        if col > MAX_COLS {
            return None;
        }
        i += 1;
    }
    if i == letters {
        return None;
    }
    if s.get(i) == Some(&b'$') {
        i += 1;
    }
    let digits = &s[i..];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) || digits.len() > 7 {
        return None;
    }
    let row: usize = std::str::from_utf8(digits).ok()?.parse().ok()?;
    if row == 0 || row > MAX_ROWS {
        return None;
    }
    Some(Cell::new(row - 1, col - 1))
}

/// Read `A1:B5`, or a single cell as a one-cell range.
pub fn parse_range(text: &str) -> Option<Range> {
    match text.split_once(':') {
        Some((a, b)) => Some(Range::new(parse_cell(a)?, parse_cell(b)?)),
        None => parse_cell(text).map(|c| Range::new(c, c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_count_like_every_spreadsheet() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(701), "ZZ");
        assert_eq!(col_name(702), "AAA");
        for c in [0, 25, 26, 51, 700, 701, 702, 18_277] {
            assert_eq!(parse_cell(&format!("{}1", col_name(c))), Some(Cell::new(0, c)));
        }
    }

    #[test]
    fn absolute_and_lowercase_references_read() {
        assert_eq!(parse_cell("$B$3"), Some(Cell::new(2, 1)));
        assert_eq!(parse_cell("b3"), Some(Cell::new(2, 1)));
        assert_eq!(parse_cell(" AA10 "), Some(Cell::new(9, 26)));
    }

    #[test]
    fn what_is_not_a_cell_is_refused() {
        for bad in ["", "A", "1", "A0", "1A", "A1B", "A-1", "ZZZZ1", "A99999999", "A1:"] {
            assert_eq!(parse_cell(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn a_range_is_normalised_whichever_way_it_is_written() {
        let r = parse_range("B3:A1").unwrap();
        assert_eq!(r.name(), "A1:B3");
        assert_eq!((r.rows(), r.cols(), r.len()), (3, 2, 6));
        assert_eq!(parse_range("C2").unwrap().name(), "C2");
        assert_eq!(parse_range("A1:nope"), None);
    }
}
