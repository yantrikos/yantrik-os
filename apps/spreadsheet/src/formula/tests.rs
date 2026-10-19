//! The evaluator, against a sheet with values typed into it.

use crate::address::parse_cell;
use crate::formula::Kind;
use crate::sheet::Sheet;

/// A sheet with `cells` typed in, and what `formula` works out to in a spare cell.
fn eval(cells: &[(&str, &str)], formula: &str) -> crate::formula::Value {
    let mut s = Sheet::default();
    for (at, text) in cells {
        s.set(parse_cell(at).unwrap(), text).unwrap();
    }
    let out = parse_cell("Z50").unwrap();
    s.set(out, formula).unwrap();
    s.value(out)
}

fn shows(cells: &[(&str, &str)], formula: &str) -> String {
    eval(cells, formula).display()
}

fn problem(cells: &[(&str, &str)], formula: &str) -> (Kind, String) {
    let v = eval(cells, formula);
    let e = v.error().unwrap_or_else(|| panic!("{formula} gave {v:?}, not an error"));
    (e.kind, e.message.clone())
}

#[test]
fn arithmetic_follows_spreadsheet_precedence() {
    assert_eq!(shows(&[], "=1+2*3"), "7");
    assert_eq!(shows(&[], "=(1+2)*3"), "9");
    assert_eq!(shows(&[], "=2^3^2"), "64", "^ is left to right in a spreadsheet");
    assert_eq!(shows(&[], "=-2^2"), "4");
    assert_eq!(shows(&[], "=50%"), "0.5");
    assert_eq!(shows(&[], "=0.1+0.2"), "0.3");
    assert_eq!(shows(&[], "=10/4"), "2.5");
}

#[test]
fn references_read_the_cells_they_name() {
    let cells = [("A1", "2"), ("B1", "3"), ("AA1", "10")];
    assert_eq!(shows(&cells, "=A1*B1"), "6");
    assert_eq!(shows(&cells, "=a1+$B$1"), "5");
    assert_eq!(shows(&cells, "=AA1/A1"), "5");
}

#[test]
fn a_blank_cell_is_zero_in_a_range_and_in_arithmetic() {
    // The review's finding 3: one empty cell in a range failed the whole SUM with #REF!.
    let cells = [("A1", "1"), ("A2", ""), ("A3", "2"), ("A5", "4")];
    assert_eq!(shows(&cells, "=SUM(A1:A10)"), "7");
    assert_eq!(shows(&cells, "=A2+1"), "1");
    assert_eq!(shows(&cells, "=A1:A3+0"), "#VALUE!", "a range is not one value");
    // The statistical functions count only cells with numbers in them, as every sheet does.
    assert_eq!(shows(&cells, "=COUNT(A1:A10)"), "3");
    assert_eq!(shows(&cells, "=AVERAGE(A1:A5)"), "2.333333333");
    assert_eq!(shows(&cells, "=MIN(A1:A5)"), "1");
    assert_eq!(shows(&cells, "=MAX(A1:A5)"), "4");
}

#[test]
fn functions_from_the_old_engine_work() {
    let cells = [("A1", "3"), ("A2", "5"), ("A3", "apple"), ("B1", "x"), ("B2", "y"), ("B3", "z")];
    assert_eq!(shows(&cells, "=AVG(A1:A2)"), "4");
    assert_eq!(shows(&cells, "=AVERAGE(A1:A2, 10)"), "6");
    assert_eq!(shows(&cells, "=PRODUCT(A1:A2)"), "15");
    assert_eq!(shows(&cells, "=COUNTA(A1:A3)"), "3");
    assert_eq!(shows(&cells, r#"=IF(A1>2, "big", "small")"#), "big");
    assert_eq!(shows(&cells, r#"=IF(A1>4, "big")"#), "FALSE");
    assert_eq!(shows(&cells, "=ROUND(2.567, 2)"), "2.57");
    assert_eq!(shows(&cells, "=MOD(-1, 3)"), "2");
    assert_eq!(shows(&cells, r#"=COUNTIF(A1:A3, ">4")"#), "1");
    assert_eq!(shows(&cells, r#"=SUMIF(A1:A3, ">=3")"#), "8");
    assert_eq!(shows(&cells, r#"=A3 & "s" & 2"#), "apples2");
    assert_eq!(shows(&cells, "=UPPER(B1)&LEN(A3)"), "X5");
    assert_eq!(shows(&cells, "=AND(A1>1, A2>1)"), "TRUE");
    assert_eq!(shows(&cells, "=OR(A1>9, NOT(A2>9))"), "TRUE");
    assert_eq!(shows(&cells, "=IFERROR(1/0, 7)"), "7");
}

#[test]
fn vlookup_finds_exact_and_sorted_matches() {
    let cells = [("A1", "1"), ("B1", "one"), ("A2", "5"), ("B2", "five"), ("A3", "9"), ("B3", "nine"), ("D1", "pear")];
    assert_eq!(shows(&cells, "=VLOOKUP(5, A1:B3, 2, FALSE)"), "five");
    assert_eq!(shows(&cells, "=VLOOKUP(6, A1:B3, 2)"), "five", "approximate: the largest not above");
    let (kind, message) = problem(&cells, "=VLOOKUP(6, A1:B3, 2, FALSE)");
    assert_eq!(kind, Kind::NotFound);
    assert!(message.contains("no row in A1:B3 whose first column is 6"), "{message}");
    let (kind, message) = problem(&cells, "=VLOOKUP(5, A1:B3, 3, FALSE)");
    assert_eq!(kind, Kind::Ref);
    assert!(message.contains("column 3 of A1:B3, which has 2"), "{message}");
}

#[test]
fn each_failure_has_its_own_code_and_says_what_happened() {
    // The review's finding 5: every failure was `#REF!`, said nowhere in words.
    let cells = [("A1", "1"), ("B3", "hello"), ("C1", "0")];

    let (kind, m) = problem(&cells, "=A1+B3");
    assert_eq!(kind, Kind::Value);
    assert_eq!(m, "B3 holds text (\"hello\"), so it can't be added");

    let (kind, m) = problem(&cells, "=A1/C1");
    assert_eq!(kind, Kind::DivZero);
    assert!(m.contains("dividing by C1, which is 0 or blank"), "{m}");

    let (kind, m) = problem(&cells, "=SUMM(A1:A3)");
    assert_eq!(kind, Kind::Name);
    assert!(m.contains("no function called SUMM"), "{m}");

    let (kind, m) = problem(&cells, "=hello+1");
    assert_eq!(kind, Kind::Name);
    assert!(m.contains("`hello` is not a cell"), "{m}");

    let (kind, m) = problem(&cells, "=(1+2");
    assert_eq!(kind, Kind::Syntax);
    assert!(m.contains("never closed"), "{m}");

    let (kind, m) = problem(&cells, "=IF(1)");
    assert_eq!(kind, Kind::Value);
    assert!(m.contains("IF takes 2 to 3 arguments, and was given 1"), "{m}");

    // The code is what the cell shows.
    assert_eq!(shows(&cells, "=A1/C1"), "#DIV/0!");
    assert_eq!(shows(&cells, "=A1+B3"), "#VALUE!");
    assert_eq!(shows(&cells, "=NOPE()"), "#NAME?");
    assert_eq!(shows(&cells, "=1+"), "#ERROR!");
}

#[test]
fn an_error_passed_on_says_where_it_began() {
    let cells = [("A1", "1"), ("A2", "0"), ("B1", "=A1/A2")];
    let (kind, m) = problem(&cells, "=B1*2");
    assert_eq!(kind, Kind::DivZero);
    assert!(m.starts_with("in B1: dividing by A2"), "{m}");
    // An error inside a range is an error in the sum, not a skipped cell.
    assert_eq!(shows(&cells, "=SUM(A1:B1)"), "#DIV/0!");
}

#[test]
fn text_that_reads_as_a_number_is_one() {
    assert_eq!(shows(&[], r#"="3"+1"#), "4");
    assert_eq!(shows(&[("A1", "TRUE")], "=A1+1"), "2");
}

#[test]
fn comparisons_are_case_blind_for_text() {
    let cells = [("A1", "Apple"), ("A2", "apple"), ("A3", "5")];
    assert_eq!(shows(&cells, "=A1=A2"), "TRUE");
    assert_eq!(shows(&cells, "=A3<10"), "TRUE");
    assert_eq!(shows(&cells, "=A3<>5"), "FALSE");
    assert_eq!(shows(&cells, "=A1>A3"), "TRUE", "text sorts after numbers");
}
