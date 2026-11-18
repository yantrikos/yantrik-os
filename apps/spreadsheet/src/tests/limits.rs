//! The security review of #727 (5f3f849e), one test per finding, with the reviewer's inputs: text and
//! work have bounds, nesting has a bound, recalculation is not quadratic, parsing stops at the
//! sheet's limits. They ran against the old session; here the same inputs go through the file layer
//! and the framework's dispatch, and the memory ones are measured on the thread that does the work.

use serde_json::json;

use super::{act, cell, scratch, started};
use crate::address::{parse_cell, Cell};
use crate::formula::budget::{MAX_TEXT, MAX_WORK};
use crate::formula::parser::MAX_DEPTH;
use crate::sheet::Sheet;

fn csv_file(name: &str, text: &str) -> std::path::PathBuf {
    let path = scratch(name).join("sheet.csv");
    std::fs::write(&path, text).unwrap();
    path
}

// ── Memory: text and work have bounds ───────────────────────────────────

/// The reviewer's 1.3 KB file: 1,000 × `x`, then `=A1&A1`, `=A2&A2`, … for 40 rows.
#[test]
fn the_doubling_csv_opens_and_stops_at_the_cell_limit() {
    let app = started();
    let mut text = "x".repeat(1000) + "\n";
    for r in 1..40 {
        text.push_str(&format!("=A{r}&A{r}\n"));
    }
    assert!(text.len() < 1400, "{}", text.len());
    let path = csv_file("doubling", &text);
    act(&app, "open", json!({ "path": path.display().to_string() })).unwrap();
    let a6 = act(&app, "get_range", json!({ "range": "A6" })).unwrap();
    assert_eq!(a6["values"][0][0].as_str().unwrap().len(), 32_000, "A6 still fits in a cell");
    let a7 = act(&app, "get_range", json!({ "range": "A7" })).unwrap();
    assert!(a7["values"][0][0]["message"].as_str().unwrap().contains("more than the 32768"), "{a7}");
    let a40 = act(&app, "get_range", json!({ "range": "A40" })).unwrap();
    assert!(a40["values"][0][0]["error"].is_string(), "A40 shows the error passed on");
}

#[test]
fn an_agent_set_cell_chain_is_held_to_the_same_limit() {
    let app = started();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "x".repeat(1000) })).unwrap();
    for r in 1..40 {
        act(&app, "set_cell", json!({ "cell": format!("A{}", r + 1), "text": format!("=A{r}&A{r}") })).unwrap();
    }
    let got = act(&app, "get_range", json!({ "range": "A7" })).unwrap();
    assert_eq!(got["values"][0][0]["error"], "#VALUE!", "{got}");
}

#[test]
fn concat_upper_and_ampersand_refuse_text_longer_than_a_cell() {
    let mut s = Sheet::default();
    s.set(parse_cell("A1").unwrap(), &"y".repeat(MAX_TEXT)).unwrap();
    for f in ["=A1&\"z\"", "=CONCAT(A1,A1)", "=CONCAT(A1:A2,\"z\")"] {
        s.set(parse_cell("B1").unwrap(), f).unwrap();
        assert!(s.value(parse_cell("B1").unwrap()).error().is_some(), "{f}");
    }
    s.set(parse_cell("A1").unwrap(), &"ß".repeat(MAX_TEXT)).unwrap();
    s.set(parse_cell("B1").unwrap(), "=UPPER(A1)").unwrap();
    assert!(s.value(parse_cell("B1").unwrap()).error().is_some(), "UPPER doubles ß to SS");
}

#[test]
fn formulas_copying_a_long_text_stop_at_the_sheets_text_budget() {
    // 3,000 copies of a 32,768-character cell would be 96 MB; the sheet keeps 64 MB.
    let mut rows = vec![vec!["w".repeat(MAX_TEXT)]];
    rows.extend((0..3000).map(|_| vec!["=$A$1".to_string()]));
    let s = Sheet::from_rows(&rows).unwrap();
    assert_eq!(s.value(Cell::new(1, 0)).display().len(), MAX_TEXT);
    let last = s.value(Cell::new(3000, 0));
    assert!(last.error().unwrap().message.contains("MB of text"), "{last:?}");
}

#[test]
fn a_formula_that_reads_too_many_cells_is_refused_before_it_reads_them() {
    let mut s = Sheet::new(18_000, 26);
    let range = "A1:Z17000"; // 442,000 cells
    let reads = MAX_WORK / 442_000 + 1;
    let f = format!("=SUM({})", vec![range; reads].join(","));
    s.set(parse_cell("A18000").unwrap(), &f).unwrap();
    let v = s.value(parse_cell("A18000").unwrap());
    assert!(v.error().unwrap().message.contains("cells to work out"), "{v:?}");
}

// ── Stack: nesting has a bound ──────────────────────────────────────────

fn nested(depth: usize) -> String {
    format!("={}1{}", "(".repeat(depth), ")".repeat(depth))
}

/// The reviewer's formula: 16,000 brackets, 32,002 characters, under the `set_cell` cap.
#[test]
fn sixteen_thousand_brackets_are_an_error_not_a_stack_overflow() {
    let f = nested(16_000);
    assert_eq!(f.len(), 32_002);
    let app = started();
    let got = act(&app, "set_cell", json!({ "cell": "A1", "text": f })).unwrap();
    assert_eq!(got["value"]["error"], "#ERROR!", "{got}");
    assert!(got["problem"].as_str().unwrap().contains("nests more than 100 levels"), "{got}");
}

#[test]
fn every_kind_of_deep_nesting_is_refused_and_shallow_nesting_still_works() {
    let mut s = Sheet::default();
    let at = parse_cell("B1").unwrap();
    let deep = [
        nested(MAX_DEPTH + 1),
        format!("={}1", "-".repeat(16_000)),
        format!("={}1", "+".repeat(16_000)),
        format!("=1{}", "+1".repeat(16_000)),
        format!("=1{}", "%".repeat(16_000)),
        format!("={}1{}", "ABS(".repeat(5_000), ")".repeat(5_000)),
        format!("={}1{}", "IF(TRUE,".repeat(3_000), ")".repeat(3_000)),
    ];
    for f in &deep {
        s.set(at, f).unwrap();
        let v = s.value(at);
        assert!(v.error().is_some_and(|e| e.message.contains("levels deep")), "{}…: {v:?}", &f[..20]);
    }
    s.set(at, &nested(50)).unwrap();
    assert_eq!(s.value(at).display(), "1");
    s.set(at, &format!("=1{}", "+1".repeat(60))).unwrap();
    assert_eq!(s.value(at).display(), "61");
}

#[test]
fn a_csv_cell_longer_than_a_cell_holds_is_refused_with_its_line() {
    let app = started();
    let path = csv_file("long-cell", &format!("ok\n\"{}\"\n", nested(20_000)));
    let e = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(e.contains("line 2") && e.contains("32768 characters"), "{e}");
    assert!(crate::csv::parse(&"a".repeat(MAX_TEXT)).is_ok(), "exactly a cell's worth is fine");
}

#[test]
fn the_evaluator_refuses_a_tree_deeper_than_the_parser_allows() {
    use crate::formula::budget::Budget;
    use crate::formula::eval::{Eval, Grid};
    use crate::formula::{Expr, Value};
    struct Blank;
    impl Grid for Blank {
        fn value(&self, _: Cell) -> Value {
            Value::Empty
        }
        fn rows(&self) -> usize {
            1
        }
        fn cols(&self) -> usize {
            1
        }
    }
    let mut e = Expr::Number(1.0);
    for _ in 0..MAX_DEPTH + 5 {
        e = Expr::Neg(Box::new(e));
    }
    let budget = Budget::default();
    let v = Eval::new(&Blank, &budget).run(&e);
    assert!(v.error().is_some_and(|e| e.message.contains("levels deep")), "{v:?}");
}

// ── Time: recalculation is not quadratic ────────────────────────────────

/// `rows` rows of 26 `=SUM(An:An)` cells, as the reviewer's file had them.
fn sums(rows: usize) -> String {
    (1..=rows).map(|r| vec![format!("=SUM(A{r}:A{r})"); 26].join(",") + "\n").collect()
}

fn open_time(rows: usize) -> (yantrik_testing::TestApp<crate::YSheets>, std::time::Duration) {
    let app = started();
    let path = csv_file(&format!("sums-{rows}"), &sums(rows));
    let t = std::time::Instant::now();
    act(&app, "open", json!({ "path": path.display().to_string() })).unwrap();
    let took = t.elapsed();
    let b = super::shown(&app, &format!("B{rows}"));
    assert_eq!(b, "#CYCLE!");
    (app, took)
}

/// 40,000 formula cells took 6 s in a release build before; a debug build now opens them in
/// well under that, and each edit after it costs the same, not more.
#[test]
fn forty_thousand_range_formulas_open_and_edit_in_linear_time() {
    let (app, took) = open_time(40_000 / 26);
    assert!(took.as_secs_f64() < 5.0, "{took:?}");
    let t = std::time::Instant::now();
    act(&app, "set_cell", json!({ "cell": "A1", "text": "5" })).unwrap();
    assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
    assert_eq!(super::shown(&app, "B1"), "5");
}

/// The reviewer's 494,000 cells, about 9 MB: over 300 s before. Run with
/// `cargo test --release -- --ignored`, which prints the time.
#[test]
#[ignore]
fn the_reviewers_nine_megabyte_sheet_opens() {
    let (_, took) = open_time(19_000);
    eprintln!("494,000 formulas opened in {took:?}");
    assert!(took.as_secs_f64() < 10.0, "{took:?}");
}

/// How much of opening the reviewer's 9 MB sheet (494,000 formula cells) is on the window's thread.
/// The job parses the file and works the sheet out; the command that lands it is what the window
/// waits for. Run with `cargo test --release -- --ignored --nocapture large_open`.
#[test]
#[ignore]
fn large_open_ui_part() {
    use crate::command::{Cause, Command};
    let app = started();
    let path = csv_file("large-open", &sums(19_000));
    let job = std::time::Instant::now();
    let mut opened = crate::files::open::open_sheet(&path, false).unwrap();
    opened.cells = Some(crate::view::render::rows_of(crate::view::render::Mask::Off, &opened.sheet));
    let job = job.elapsed();
    let parts = std::time::Instant::now();
    let (_filled, _errors) = opened.sheet.counts();
    let _first = opened.sheet.first_error();
    let counts = parts.elapsed();
    let parts = std::time::Instant::now();
    let model = crate::model::Model::new(std::env::temp_dir(), false);
    let cells: Vec<_> = opened.sheet.values().iter().map(|v| crate::view::render::shown(&model, v)).collect();
    let strings = parts.elapsed();
    drop(cells);
    let parts = std::time::Instant::now();
    let texts: Vec<String> = opened.sheet.values().iter().map(|v| v.display()).collect();
    let formatting = parts.elapsed();
    let parts = std::time::Instant::now();
    let shared: Vec<slint::SharedString> = texts.iter().map(|t| t.as_str().into()).collect();
    let into_slint = parts.elapsed();
    eprintln!("  formatting only {formatting:?}; into Slint strings {into_slint:?} ({} cells)", shared.len());
    eprintln!("  of which: counts and first error {counts:?}; one display string per cell {strings:?}");
    let rows_for_models = opened.cells.clone().unwrap();
    let models = std::time::Instant::now();
    let lines: Vec<crate::SheetRow> = rows_for_models
        .into_iter()
        .map(|cells| crate::SheetRow { cells: slint::ModelRc::new(slint::VecModel::from(cells)) })
        .collect();
    let list = slint::ModelRc::new(slint::VecModel::from(lines));
    let models = models.elapsed();
    eprintln!("  Slint models for 19,000 rows of 26 cells, nothing else: {models:?}");
    drop(list);
    let ui = std::time::Instant::now();
    app.run(Command::Opened { then: Cause::Window, result: Ok(Box::new(opened)) });
    let ui = ui.elapsed();
    eprintln!("large open: job (read, parse, work out) {job:?}; window's thread (install, first render) {ui:?}");
    assert_eq!(super::shown(&app, "B19000"), "#CYCLE!");
}

#[test]
fn ranges_that_overlap_without_end_are_held_to_the_budget() {
    // Running totals over everything above: 7,000 formulas link to 24.5 million others.
    let mut rows = vec![vec!["1".to_string()]];
    rows.extend((2..=7_000).map(|r| vec![format!("=SUM(A1:A{})", r - 1)]));
    let s = Sheet::from_rows(&rows).unwrap();
    assert_eq!(s.value(Cell::new(2, 0)).display(), "2", "the first totals are worked out");
    let last = s.value(Cell::new(6_999, 0));
    assert!(last.error().unwrap().message.contains("cells to work out"), "{last:?}");
}

// ── Parsing stops at the sheet's limits ─────────────────────────────────

/// The reviewer's two 16 MB files: newlines (16 M rows, 2.1 GB before), and commas (16 M empty
/// fields, about 400 MB). Both now stop after at most one sheet's worth of fields.
#[test]
fn sixteen_megabytes_of_newlines_or_commas_stop_at_the_sheets_limits() {
    let app = started();
    let size = crate::files::MAX_FILE_BYTES as usize;
    for (name, byte, says) in [("lines.csv", b'\n', "500000 cells"), ("commas.csv", b',', "256 columns")] {
        let path = scratch("huge").join(name);
        std::fs::write(&path, vec![byte; size]).unwrap();
        let t = std::time::Instant::now();
        let e = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
        assert!(e.contains(says), "{name}: {e}");
        assert!(t.elapsed().as_secs_f64() < 5.0, "{name}: {:?}", t.elapsed());
    }
}

#[test]
fn a_file_one_byte_over_the_size_limit_is_refused_without_being_read_whole() {
    let app = started();
    let path = scratch("one-over").join("big.csv");
    std::fs::write(&path, vec![b'1'; crate::files::MAX_FILE_BYTES as usize + 1]).unwrap();
    let e = act(&app, "open", json!({ "path": path.display().to_string() })).unwrap_err();
    assert!(e.contains("is more than the 16 MB this sheet opens"), "{e}");
}

#[test]
fn the_parser_keeps_the_limits_exactly() {
    use crate::sheet::{MAX_CELLS, MAX_SHEET_COLS};
    let wide = vec!["a"; MAX_SHEET_COLS].join(",");
    assert_eq!(crate::csv::parse(&wide).unwrap()[0].len(), MAX_SHEET_COLS);
    let e = crate::csv::parse(&format!("x\n{wide},b")).unwrap_err();
    assert!(e.contains("line 2 has more than"), "{e}");
    let full = "1,1\n".repeat(MAX_CELLS / 2);
    assert_eq!(crate::csv::parse(&full).unwrap().len(), MAX_CELLS / 2);
    assert!(crate::csv::parse(&(full + "1\n")).unwrap_err().contains("more than the 500000 cells"));
}

// ── Memory: a nested SUMIF holds no copy of its range while the next one runs ──
// The security delta review of #727 (5f3f849e..b2906136): SUMIF kept the range it tests while
// working out its third argument, so 95 SUMIFs nested over one range of long texts held 95
// copies of it: 1.5 GB from a 16 MB CSV, 6.3 GB from one agent `set_cell`.

use std::alloc::{GlobalAlloc, Layout, System};

/// Counts what this thread allocates, so a test can say how much memory one step took at most.
struct Counting;

thread_local! {
    static LIVE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static PEAK: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

fn count(bytes: isize) {
    let _ = LIVE.try_with(|live| {
        live.set(live.get() + bytes);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(live.get())));
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            count(layout.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        count(-(layout.size() as isize));
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let q = System.realloc(p, layout, size);
        if !q.is_null() {
            count(size as isize - layout.size() as isize);
        }
        q
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most bytes `step` had allocated at once on this thread, beyond what was live before it.
fn peak_of<T>(step: impl FnOnce() -> T) -> (T, usize) {
    let start = LIVE.with(std::cell::Cell::get);
    PEAK.with(|p| p.set(start));
    let out = step();
    (out, (PEAK.with(std::cell::Cell::get) - start).max(0) as usize)
}

/// The reviewer's formula: `SUMIF(<range>,"y",…)` nested 95 deep around a 1.
fn nested_sumif(range: &str) -> String {
    format!("={}1{}", format!("SUMIF({range},\"y\",").repeat(95), ")".repeat(95))
}

const MB: usize = 1024 * 1024;

/// The CSV route: 480 rows of 32,768 × `x` and the nested formula, 15.7 MB, under the 16 MB cap.
/// The file layer's own function is what the open job runs, so it is measured on this thread.
#[test]
fn a_nested_sumif_in_a_csv_reads_its_range_once_at_a_time() {
    let mut text = format!("{}\n", "x".repeat(MAX_TEXT)).repeat(480);
    text.push_str(&format!("\"{}\"\n", nested_sumif("A1:A480").replace('"', "\"\"")));
    assert!(text.len() < crate::files::MAX_FILE_BYTES as usize, "{}", text.len());
    let path = csv_file("sumif-csv", &text);
    let file = text.len();
    drop(text);

    let (opened, peak) = peak_of(|| crate::files::open::open_sheet(&path, false));
    let opened = opened.unwrap();
    assert_eq!(opened.sheet.value(Cell::new(480, 0)).display(), "0", "no text is \"y\", so nothing is summed");
    // The file read, parsed and kept, and one copy of the range: a few times the file, not 95.
    assert!(peak < 8 * file, "open took {} MB at once for a {} MB file", peak / MB, file / MB);
}

/// The agent route: 2,000 cells of 32,768 characters, then one `set_cell` of the nested formula,
/// worked out as the recalculation job works it.
#[test]
fn a_nested_sumif_from_an_agent_reads_its_range_once_at_a_time() {
    let mut s = Sheet::from_rows(&vec![vec!["x".repeat(MAX_TEXT)]; 2000]).unwrap();
    let data = 2000 * MAX_TEXT;
    s.put(cell("B1"), &nested_sumif("A1:A2000")).unwrap();
    let snapshot = s.snapshot();
    let (solved, peak) = peak_of(|| snapshot.solve());
    assert_eq!(solved.value(cell("B1")).to_json(), json!(0.0));
    // The recalculation's new values (one copy of every text) and one copy of the range: twice
    // the text, where 95 copies at once was 6.3 GB.
    assert!(peak < 3 * data, "the recalculation took {} MB at once over {} MB of text", peak / MB, data / MB);
}

#[test]
fn sumif_still_sums_with_any_mix_of_ranges_and_nested_formulas() {
    let cells = [("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "10"), ("B2", "20"), ("B3", "30")];
    let mut s = Sheet::default();
    for (at, text) in cells {
        s.set(parse_cell(at).unwrap(), text).unwrap();
    }
    for (f, want) in [
        ("=SUMIF(A1:A3,\">1\")", "5"),
        ("=SUMIF(A1:A3,\">1\",B1:B3)", "50"),
        ("=SUMIF(A1:A3,\">1\",SUMIF(A1:A3,\">0\"))", "0"),
        ("=SUMIF(A1:A3,1,SUMIF(A1:A3,\">0\"))", "6"),
        ("=SUMIF(SUMIF(A1:A3,\">0\"),6,B1)", "10"),
        ("=SUMIF(A2,2,B2)", "20"),
    ] {
        s.set(parse_cell("D1").unwrap(), f).unwrap();
        assert_eq!(s.value(parse_cell("D1").unwrap()).display(), want, "{f}");
    }
}

/// Without a third argument SUMIF reads its first one once, not twice: twice was 2^95 work for
/// 95 nested SUMIFs that read no cell, which the work budget never sees.
#[test]
fn a_nested_sumif_without_a_sum_range_works_its_argument_out_once() {
    let mut s = Sheet::default();
    let f = format!("={}1{}", "SUMIF(".repeat(95), ",\">0\")".repeat(95));
    let t = std::time::Instant::now();
    s.set(parse_cell("A1").unwrap(), &f).unwrap();
    assert_eq!(s.value(parse_cell("A1").unwrap()).display(), "1");
    assert!(t.elapsed().as_secs_f64() < 1.0, "{:?}", t.elapsed());
}
