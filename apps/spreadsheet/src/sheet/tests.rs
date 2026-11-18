//! The grid: always every cell, worked out in dependency order, loops found.

use super::*;
use crate::address::parse_cell;
use crate::formula::Kind;

fn c(name: &str) -> Cell {
    parse_cell(name).unwrap()
}

#[test]
fn a_new_sheet_is_filled_rows_by_cols() {
    // The disease the shelf was about: a grid drawn with no cells under it.
    let s = Sheet::default();
    assert_eq!((s.rows(), s.cols()), (DEFAULT_ROWS, DEFAULT_COLS));
    assert_eq!(s.snap.raw.len(), DEFAULT_ROWS);
    assert!(s.snap.raw.iter().all(|row| row.len() == DEFAULT_COLS));
    assert_eq!(s.values.len(), DEFAULT_ROWS * DEFAULT_COLS);
    assert_eq!(s.value(c("Z50")), Value::Empty);
}

#[test]
fn a_short_file_still_fills_the_grid_and_a_long_one_grows_it() {
    // The review's finding 4: a three-row import left the grid smaller than rows × cols.
    let short = Sheet::from_rows(&[vec!["1".into(), "2".into()], vec!["3".into()]]).unwrap();
    assert_eq!((short.rows(), short.cols()), (DEFAULT_ROWS, DEFAULT_COLS));
    assert_eq!(short.snap.raw.len(), DEFAULT_ROWS);
    assert!(short.snap.raw.iter().all(|row| row.len() == DEFAULT_COLS));
    assert_eq!(short.raw(c("B2")), "", "a ragged row is padded");

    let long: Vec<Vec<String>> = (0..120).map(|r| (0..30).map(|k| format!("{}", r * 30 + k)).collect()).collect();
    let long = Sheet::from_rows(&long).unwrap();
    assert_eq!((long.rows(), long.cols()), (120, 30));
    assert_eq!(long.value(c("AD120")).display(), "3599");
}

#[test]
fn editing_a_cell_past_the_edge_grows_the_sheet() {
    let mut s = Sheet::default();
    s.set(c("AB60"), "7").unwrap();
    assert_eq!((s.rows(), s.cols()), (60, 28));
    assert_eq!(s.snap.raw.len(), 60);
    assert!(s.snap.raw.iter().all(|row| row.len() == 28));
    assert_eq!(s.values.len(), 60 * 28);
    assert_eq!(s.value(c("AB60")).display(), "7");
}

#[test]
fn formulas_recalculate_when_what_they_read_changes() {
    let mut s = Sheet::default();
    s.set(c("A1"), "=B1*2").unwrap(); // reads a cell to its right and below it in order
    s.set(c("B1"), "=C1+1").unwrap();
    s.set(c("C1"), "4").unwrap();
    assert_eq!(s.value(c("A1")).display(), "10");
    s.set(c("C1"), "0").unwrap();
    assert_eq!(s.value(c("A1")).display(), "2");
    s.set(c("D1"), "=SUM(A1:C1)").unwrap();
    assert_eq!(s.value(c("D1")).display(), "3");
}

#[test]
fn a_long_chain_costs_no_stack() {
    let mut rows = vec![vec!["1".to_string()]];
    for r in 2..=5000 {
        rows.push(vec![format!("=A{}+1", r - 1)]);
    }
    let s = Sheet::from_rows(&rows).unwrap();
    assert_eq!(s.value(c("A5000")).display(), "5000");
}

#[test]
fn a_loop_is_named_and_what_reads_it_says_so() {
    let mut s = Sheet::default();
    s.set(c("A1"), "=B1").unwrap();
    s.set(c("B1"), "=A1+1").unwrap();
    s.set(c("C1"), "=A1*2").unwrap();
    s.set(c("D1"), "=D1").unwrap();
    for cell in ["A1", "B1", "C1", "D1"] {
        let v = s.value(c(cell));
        assert_eq!(v.error().map(|e| e.kind), Some(Kind::Cycle), "{cell}: {v:?}");
    }
    assert!(s.problem(c("A1")).unwrap().contains("loop of references"));
    // Breaking the loop clears it.
    s.set(c("B1"), "5").unwrap();
    assert_eq!(s.value(c("C1")).display(), "10");
}

#[test]
fn the_used_block_is_what_a_file_is_written_from() {
    let mut s = Sheet::default();
    assert_eq!(s.used(), (0, 0));
    s.set(c("B3"), "x").unwrap();
    s.set(c("A1"), "=1").unwrap();
    assert_eq!(s.used(), (3, 2));
    assert_eq!(s.used_rows(), vec![vec!["=1".to_string(), String::new()], vec![String::new(), String::new()], vec![String::new(), "x".into()]]);
}

#[test]
fn problems_are_sentences_with_the_code() {
    let mut s = Sheet::default();
    s.set(c("B3"), "hello").unwrap();
    s.set(c("C1"), "=B3+1").unwrap();
    assert_eq!(
        s.problem(c("C1")).unwrap(),
        "C1 shows #VALUE!: B3 holds text (\"hello\"), so it can't be added."
    );
    assert_eq!(s.value(c("C1")).display(), "#VALUE!", "the code stays in the cell");
    assert_eq!(s.counts(), (2, 1));
    assert_eq!(s.first_error(), Some(c("C1")));
}

#[test]
fn a_sheet_too_large_to_draw_is_refused_with_its_size() {
    let wide = vec![vec![String::new(); MAX_SHEET_COLS + 1]];
    assert!(Sheet::from_rows(&wide).unwrap_err().contains("columns"));
    let mut s = Sheet::default();
    assert!(s.set(Cell::new(MAX_CELLS, 0), "x").is_err());
    assert_eq!(s.rows(), DEFAULT_ROWS, "a refused edit leaves the size alone");
}

#[test]
fn a_cell_typed_shows_a_stand_in_until_the_sheet_is_worked_out_and_a_formula_keeps_what_it_showed() {
    let mut s = Sheet::default();
    s.set(c("A1"), "2").unwrap();
    s.set(c("B1"), "=A1*3").unwrap();
    assert_eq!(s.value(c("B1")).display(), "6");
    s.put(c("A1"), "5").unwrap();
    assert!(!s.is_solved());
    assert_eq!(s.value(c("A1")).display(), "5", "text that is not a formula shows as itself");
    assert_eq!(s.value(c("B1")).display(), "6", "a formula shows what it showed until it is worked out");
    let solved = s.snapshot().solve();
    assert!(s.install(&solved));
    assert!(s.is_solved());
    assert_eq!(s.value(c("B1")).display(), "15");
}

#[test]
fn a_result_is_taken_only_while_the_sheet_is_at_the_revision_it_was_made_for() {
    let mut s = Sheet::default();
    s.put(c("A1"), "=1+1").unwrap();
    let old = s.snapshot().solve();
    s.put(c("A2"), "=A1+1").unwrap();
    assert!(!s.install(&old), "typed into since");
    assert!(!s.is_solved());
    let new = s.snapshot().solve();
    assert!(s.install(&new));
    assert_eq!(s.value(c("A2")).display(), "3");
    // A size change is a revision too.
    let before = s.snapshot().solve();
    s.resize(60, 26).unwrap();
    assert!(!s.install(&before));
}

#[test]
fn a_snapshot_shares_the_rows_and_an_edit_copies_only_the_row_it_touches() {
    let mut s = Sheet::default();
    let snap = s.snapshot();
    s.put(c("C3"), "x").unwrap();
    let after = s.snapshot();
    let same = snap.raw.iter().zip(&after.raw).filter(|(a, b)| std::sync::Arc::ptr_eq(a, b)).count();
    assert_eq!(same, DEFAULT_ROWS - 1, "every row but the one typed into is still shared");
    assert_eq!(snap.raw(c("C3")), "", "and the snapshot did not change under the edit");
}

#[test]
fn resizing_keeps_the_text_and_the_values_of_the_cells_that_stay() {
    let mut s = Sheet::default();
    s.set(c("B2"), "7").unwrap();
    s.set(c("C3"), "=B2*2").unwrap();
    s.resize(60, 30).unwrap();
    assert_eq!((s.rows(), s.cols()), (60, 30));
    assert_eq!((s.raw(c("B2")), s.raw(c("C3"))), ("7", "=B2*2"));
    assert_eq!(s.value(c("C3")).display(), "14", "kept until the next result is installed");
    assert_eq!(s.values().len(), 60 * 30);
    s.resize(10, 5).unwrap();
    assert_eq!(s.values().len(), 50);
    assert_eq!(s.raw(c("C3")), "=B2*2");
    s.settle();
    assert_eq!(s.value(c("C3")).display(), "14");
}
