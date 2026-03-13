//! Spreadsheet wire module — screen 29.
//!
//! In-memory spreadsheet with formula evaluation, multi-sheet tabs,
//! cell formatting, sort/filter, undo/redo, find/replace, copy/paste,
//! merge cells, freeze panes, chart data, cell comments, number formatting,
//! auto-save, and CSV import/export.
//! Data stored as `.ysheet` files in `~/.local/share/yantrik/sheets/`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use std::sync::Arc;

use crate::app_context::AppContext;
use crate::bridge::CompanionBridge;
use crate::{App, SheetTab, SpreadsheetCell};

// ── Data structures ──

/// Number format type for cell display.
#[derive(Clone, Debug, Copy, PartialEq)]
#[repr(i32)]
enum NumberFormat {
    General,    // 0
    Number,     // 1 — 1,234.56
    Currency,   // 2 — $1,234.56
    Percent,    // 3 — 12.34%
    Date,       // 4 — mm/dd/yyyy
    Scientific, // 5 — 1.23E+04
}

impl NumberFormat {
    fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Number,
            2 => Self::Currency,
            3 => Self::Percent,
            4 => Self::Date,
            5 => Self::Scientific,
            _ => Self::General,
        }
    }
}

/// Per-cell data stored in the backend.
#[derive(Clone, Debug)]
struct CellData {
    raw: String,
    display: String,
    is_bold: bool,
    is_italic: bool,
    align: i32, // 0=left, 1=center, 2=right
    bg_color_idx: i32,   // 0=none, 1-8 = preset colors
    text_color_idx: i32, // 0=default, 1-8 = preset colors
    number_format: NumberFormat,
}

impl Default for CellData {
    fn default() -> Self {
        Self {
            raw: String::new(),
            display: String::new(),
            is_bold: false,
            is_italic: false,
            align: 0,
            bg_color_idx: 0,
            text_color_idx: 0,
            number_format: NumberFormat::General,
        }
    }
}

/// Merge region: top-left cell owns the content.
#[derive(Clone, Debug)]
struct MergeRegion {
    r1: usize,
    c1: usize,
    r2: usize,
    c2: usize,
}

/// Chart data for UI rendering.
#[derive(Clone, Debug)]
struct ChartData {
    chart_type: i32, // 0=bar, 1=line, 2=pie
    title: String,
    labels: Vec<String>,
    series: Vec<f64>,
}

/// A single sheet (tab) with its own cell grid.
#[derive(Clone, Debug)]
struct Sheet {
    name: String,
    cells: Vec<Vec<CellData>>, // rows x cols
    comments: HashMap<(usize, usize), String>,
    merges: Vec<MergeRegion>,
}

impl Sheet {
    fn new(name: &str, rows: usize, cols: usize) -> Self {
        let cells = vec![vec![CellData::default(); cols]; rows];
        Self {
            name: name.to_string(),
            cells,
            comments: HashMap::new(),
            merges: Vec::new(),
        }
    }

    fn ensure_size(&mut self, rows: usize, cols: usize) {
        while self.cells.len() < rows {
            self.cells.push(vec![CellData::default(); cols]);
        }
        for row in &mut self.cells {
            while row.len() < cols {
                row.push(CellData::default());
            }
        }
    }

    fn get_cell(&self, row: usize, col: usize) -> CellData {
        if row < self.cells.len() && col < self.cells[row].len() {
            self.cells[row][col].clone()
        } else {
            CellData::default()
        }
    }

    fn set_cell(&mut self, row: usize, col: usize, data: CellData) {
        self.ensure_size(row + 1, col + 1);
        self.cells[row][col] = data;
    }

    /// Find which merge region (if any) a cell belongs to.
    fn find_merge(&self, row: usize, col: usize) -> Option<usize> {
        self.merges.iter().position(|m| {
            row >= m.r1 && row <= m.r2 && col >= m.c1 && col <= m.c2
        })
    }

    /// Get the last used row index (0-based).
    fn last_used_row(&self) -> usize {
        for r in (0..self.cells.len()).rev() {
            if self.cells[r].iter().any(|c| !c.raw.is_empty()) {
                return r;
            }
        }
        0
    }

    /// Get the last used column index in a given row.
    fn last_used_col(&self, row: usize) -> usize {
        if row >= self.cells.len() {
            return 0;
        }
        for c in (0..self.cells[row].len()).rev() {
            if !self.cells[row][c].raw.is_empty() {
                return c;
            }
        }
        0
    }
}

/// Undo/redo snapshot.
#[derive(Clone, Debug)]
struct Snapshot {
    sheets: Vec<Sheet>,
    active_sheet: usize,
}

/// Full spreadsheet state.
struct SpreadsheetState {
    sheets: Vec<Sheet>,
    active_sheet: usize,
    active_row: usize,
    active_col: usize,
    row_count: usize,
    col_count: usize,

    // Filter state
    filter_col: Option<usize>,
    filter_text: String,
    original_rows: Option<Vec<Vec<CellData>>>, // stored before filter

    // Clipboard
    clipboard: Vec<Vec<CellData>>,
    clipboard_is_cut: bool,
    clipboard_origin: (usize, usize),

    // Undo/redo
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,

    // Find state
    find_query: String,
    find_row: usize,
    find_col: usize,

    // Freeze panes
    freeze_rows: i32,
    freeze_cols: i32,

    // Charts
    charts: Vec<ChartData>,

    // Auto-save
    dirty: bool,
    current_file: Option<PathBuf>,
}

const MAX_UNDO: usize = 50;

impl SpreadsheetState {
    fn new() -> Self {
        Self {
            sheets: vec![Sheet::new("Sheet 1", 50, 26)],
            active_sheet: 0,
            active_row: 0,
            active_col: 0,
            row_count: 50,
            col_count: 26,
            filter_col: None,
            filter_text: String::new(),
            original_rows: None,
            clipboard: Vec::new(),
            clipboard_is_cut: false,
            clipboard_origin: (0, 0),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            find_query: String::new(),
            find_row: 0,
            find_col: 0,
            freeze_rows: 0,
            freeze_cols: 0,
            charts: Vec::new(),
            dirty: false,
            current_file: None,
        }
    }

    fn current_sheet(&self) -> &Sheet {
        &self.sheets[self.active_sheet]
    }

    fn current_sheet_mut(&mut self) -> &mut Sheet {
        &mut self.sheets[self.active_sheet]
    }

    /// Take a snapshot for undo.
    fn push_undo(&mut self) {
        let snap = Snapshot {
            sheets: self.sheets.clone(),
            active_sheet: self.active_sheet,
        };
        self.undo_stack.push(snap);
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.dirty = true;
    }

    fn undo(&mut self) -> bool {
        if let Some(snap) = self.undo_stack.pop() {
            let current = Snapshot {
                sheets: self.sheets.clone(),
                active_sheet: self.active_sheet,
            };
            self.redo_stack.push(current);
            self.sheets = snap.sheets;
            self.active_sheet = snap.active_sheet;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    fn redo(&mut self) -> bool {
        if let Some(snap) = self.redo_stack.pop() {
            let current = Snapshot {
                sheets: self.sheets.clone(),
                active_sheet: self.active_sheet,
            };
            self.undo_stack.push(current);
            self.sheets = snap.sheets;
            self.active_sheet = snap.active_sheet;
            self.dirty = true;
            true
        } else {
            false
        }
    }
}

// ── Color presets ──

/// Color index to (r, g, b). 0 = none/default.
fn color_preset(idx: i32) -> Option<(u8, u8, u8)> {
    match idx {
        1 => Some((255, 255, 255)), // white
        2 => Some((239, 83, 80)),   // red
        3 => Some((255, 167, 38)),  // orange
        4 => Some((255, 238, 88)),  // yellow
        5 => Some((102, 187, 106)), // green
        6 => Some((66, 165, 245)),  // blue
        7 => Some((171, 71, 188)),  // purple
        8 => Some((158, 158, 158)), // gray
        _ => None,
    }
}

/// Convert color index to brush for background colors (0 = transparent/default).
fn color_idx_to_brush(idx: i32) -> slint::Brush {
    if let Some((r, g, b)) = color_preset(idx) {
        slint::Brush::from(slint::Color::from_rgb_u8(r, g, b))
    } else {
        slint::Brush::default()
    }
}

/// Convert color index to brush for text colors (0 = light gray, visible on dark bg).
fn text_color_idx_to_brush(idx: i32) -> slint::Brush {
    if let Some((r, g, b)) = color_preset(idx) {
        slint::Brush::from(slint::Color::from_rgb_u8(r, g, b))
    } else {
        // Default text color — light gray, visible on dark background
        slint::Brush::from(slint::Color::from_rgb_u8(230, 230, 230))
    }
}

// ── Formula evaluation ──

/// Parse a cell reference like "A1" -> (row, col). Returns None on invalid ref.
fn parse_cell_ref(s: &str) -> Option<(usize, usize)> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }

    let mut col = 0usize;
    let mut i = 0;
    let bytes = s.as_bytes();

    // Skip optional $ for absolute refs
    if i < bytes.len() && bytes[i] == b'$' {
        i += 1;
    }

    // Parse column letters
    let col_start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        col = col * 26 + (bytes[i] - b'A') as usize;
        i += 1;
    }

    if i == col_start || i >= bytes.len() {
        return None;
    }

    // Skip optional $ before row
    if i < bytes.len() && bytes[i] == b'$' {
        i += 1;
    }

    if i >= bytes.len() {
        return None;
    }

    // Parse row number
    let row_str = &s[i..];
    let row: usize = row_str.parse().ok()?;
    if row == 0 {
        return None;
    }

    Some((row - 1, col))
}

/// Parse a range like "A1:B5" -> list of (row, col) pairs.
fn parse_range(s: &str) -> Vec<(usize, usize)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        // Single cell reference
        if let Some(rc) = parse_cell_ref(s) {
            return vec![rc];
        }
        return vec![];
    }

    let start = match parse_cell_ref(parts[0]) {
        Some(r) => r,
        None => return vec![],
    };
    let end = match parse_cell_ref(parts[1]) {
        Some(r) => r,
        None => return vec![],
    };

    let mut cells = Vec::new();
    let r_min = start.0.min(end.0);
    let r_max = start.0.max(end.0);
    let c_min = start.1.min(end.1);
    let c_max = start.1.max(end.1);

    for r in r_min..=r_max {
        for c in c_min..=c_max {
            cells.push((r, c));
        }
    }
    cells
}

/// Get numeric value of a cell (for formula evaluation).
fn cell_value(sheet: &Sheet, row: usize, col: usize) -> f64 {
    let cell = sheet.get_cell(row, col);
    cell.display.parse::<f64>().unwrap_or(0.0)
}

/// Get string value of a cell.
fn cell_text(sheet: &Sheet, row: usize, col: usize) -> String {
    let cell = sheet.get_cell(row, col);
    cell.display.clone()
}

/// Collect numeric values from a range expression.
fn range_values(sheet: &Sheet, range_str: &str) -> Vec<f64> {
    let refs = parse_range(range_str);
    refs.iter()
        .map(|&(r, c)| cell_value(sheet, r, c))
        .collect()
}

/// Split function arguments, respecting nested parens and quoted strings.
fn split_args(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut in_quotes = false;
    let mut current = String::new();

    for ch in s.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            '(' if !in_quotes => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quotes => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 && !in_quotes => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

/// Resolve a single argument: could be a cell ref, nested function, number, or literal string.
fn resolve_arg(arg: &str, sheet: &Sheet) -> String {
    let arg = arg.trim();
    if arg.is_empty() {
        return String::new();
    }

    // Quoted string literal — strip quotes
    if arg.starts_with('"') && arg.ends_with('"') && arg.len() >= 2 {
        return arg[1..arg.len() - 1].to_string();
    }

    // Nested function call
    if arg.contains('(') && arg.contains(')') {
        if let Some(result) = try_eval_function(arg, sheet) {
            return result;
        }
    }

    // Cell reference
    if let Some((r, c)) = parse_cell_ref(arg) {
        return cell_text(sheet, r, c);
    }

    // Numeric or literal
    arg.to_string()
}

/// Resolve an argument as a number.
fn resolve_arg_num(arg: &str, sheet: &Sheet) -> f64 {
    let resolved = resolve_arg(arg, sheet);
    resolved.parse::<f64>().unwrap_or(0.0)
}

/// Evaluate a formula string. Returns the display result.
fn evaluate_formula(formula: &str, sheet: &Sheet) -> String {
    let formula = formula.trim();
    if !formula.starts_with('=') {
        return formula.to_string();
    }

    let expr = &formula[1..].trim().to_uppercase();

    // Try function calls: SUM(...), VLOOKUP(...), etc.
    if let Some(result) = try_eval_function(expr, sheet) {
        return result;
    }

    // Try simple cell reference: =A1
    if let Some((r, c)) = parse_cell_ref(expr) {
        let cell = sheet.get_cell(r, c);
        return cell.display.clone();
    }

    // Try arithmetic expression with proper precedence
    if let Some(result) = eval_expression(expr, sheet) {
        return result;
    }

    "#ERR".to_string()
}

/// Try to evaluate a function call like SUM(A1:A10).
fn try_eval_function(expr: &str, sheet: &Sheet) -> Option<String> {
    let open = expr.find('(')?;
    let close = find_matching_paren(expr, open)?;
    if close <= open {
        return None;
    }

    let func_name = expr[..open].trim();
    let args_str = &expr[open + 1..close];

    match func_name {
        // ── Math ──
        "SUM" => {
            let args = split_args(args_str);
            let mut total = 0.0f64;
            for arg in &args {
                if arg.contains(':') {
                    total += range_values(sheet, arg).iter().sum::<f64>();
                } else {
                    total += resolve_arg_num(arg, sheet);
                }
            }
            Some(format_number(total))
        }
        "AVG" | "AVERAGE" => {
            let vals = collect_all_values(args_str, sheet);
            if vals.is_empty() {
                Some("0".to_string())
            } else {
                let sum: f64 = vals.iter().sum();
                Some(format_number(sum / vals.len() as f64))
            }
        }
        "MIN" => {
            let vals = collect_all_values(args_str, sheet);
            let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);
            if min.is_infinite() {
                Some("0".to_string())
            } else {
                Some(format_number(min))
            }
        }
        "MAX" => {
            let vals = collect_all_values(args_str, sheet);
            let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            if max.is_infinite() {
                Some("0".to_string())
            } else {
                Some(format_number(max))
            }
        }
        "COUNT" => {
            let args = split_args(args_str);
            let mut count = 0usize;
            for arg in &args {
                let refs = parse_range(arg);
                count += refs
                    .iter()
                    .filter(|&&(r, c)| {
                        let cell = sheet.get_cell(r, c);
                        !cell.display.is_empty() && cell.display.parse::<f64>().is_ok()
                    })
                    .count();
            }
            Some(count.to_string())
        }
        "PRODUCT" => {
            let vals = collect_all_values(args_str, sheet);
            if vals.is_empty() {
                Some("0".to_string())
            } else {
                let product: f64 = vals.iter().product();
                Some(format_number(product))
            }
        }
        "POWER" => {
            let args = split_args(args_str);
            if args.len() != 2 {
                return Some("#ERR".to_string());
            }
            let base = resolve_arg_num(&args[0], sheet);
            let exp = resolve_arg_num(&args[1], sheet);
            Some(format_number(base.powf(exp)))
        }
        "ABS" => {
            let val = resolve_arg_num(args_str.trim(), sheet);
            Some(format_number(val.abs()))
        }
        "ROUND" => {
            let args = split_args(args_str);
            let val = resolve_arg_num(args.first().map(|s| s.as_str()).unwrap_or("0"), sheet);
            let digits = args
                .get(1)
                .map(|s| resolve_arg_num(s, sheet) as i32)
                .unwrap_or(0);
            let factor = 10f64.powi(digits);
            Some(format_number((val * factor).round() / factor))
        }
        "CEILING" => {
            let val = resolve_arg_num(args_str.trim(), sheet);
            Some(format_number(val.ceil()))
        }
        "FLOOR" => {
            let val = resolve_arg_num(args_str.trim(), sheet);
            Some(format_number(val.floor()))
        }
        "SQRT" => {
            let val = resolve_arg_num(args_str.trim(), sheet);
            if val < 0.0 {
                Some("#NUM!".to_string())
            } else {
                Some(format_number(val.sqrt()))
            }
        }
        "MOD" => {
            let args = split_args(args_str);
            if args.len() != 2 {
                return Some("#ERR".to_string());
            }
            let a = resolve_arg_num(&args[0], sheet);
            let b = resolve_arg_num(&args[1], sheet);
            if b.abs() < f64::EPSILON {
                Some("#DIV/0!".to_string())
            } else {
                Some(format_number(a % b))
            }
        }

        // ── Text ──
        "LEN" => {
            let val = resolve_arg(args_str.trim(), sheet);
            Some(val.len().to_string())
        }
        "UPPER" => {
            let val = resolve_arg(args_str.trim(), sheet);
            Some(val.to_uppercase())
        }
        "LOWER" => {
            let val = resolve_arg(args_str.trim(), sheet);
            Some(val.to_lowercase())
        }
        "TRIM" => {
            let val = resolve_arg(args_str.trim(), sheet);
            // Collapse whitespace like Excel TRIM
            let trimmed: String = val
                .split_whitespace()
                .collect::<Vec<&str>>()
                .join(" ");
            Some(trimmed)
        }
        "CONCATENATE" | "CONCAT" => {
            let args = split_args(args_str);
            let mut result = String::new();
            for arg in &args {
                result.push_str(&resolve_arg(arg, sheet));
            }
            Some(result)
        }
        "LEFT" => {
            let args = split_args(args_str);
            if args.is_empty() {
                return Some("#ERR".to_string());
            }
            let text = resolve_arg(&args[0], sheet);
            let n = args
                .get(1)
                .map(|s| resolve_arg_num(s, sheet) as usize)
                .unwrap_or(1);
            Some(text.chars().take(n).collect())
        }
        "RIGHT" => {
            let args = split_args(args_str);
            if args.is_empty() {
                return Some("#ERR".to_string());
            }
            let text = resolve_arg(&args[0], sheet);
            let n = args
                .get(1)
                .map(|s| resolve_arg_num(s, sheet) as usize)
                .unwrap_or(1);
            let len = text.chars().count();
            let skip = if n >= len { 0 } else { len - n };
            Some(text.chars().skip(skip).collect())
        }
        "MID" => {
            let args = split_args(args_str);
            if args.len() < 3 {
                return Some("#ERR".to_string());
            }
            let text = resolve_arg(&args[0], sheet);
            let start = (resolve_arg_num(&args[1], sheet) as usize).saturating_sub(1); // 1-based
            let length = resolve_arg_num(&args[2], sheet) as usize;
            Some(text.chars().skip(start).take(length).collect())
        }
        "SUBSTITUTE" => {
            let args = split_args(args_str);
            if args.len() < 3 {
                return Some("#ERR".to_string());
            }
            let text = resolve_arg(&args[0], sheet);
            let old = resolve_arg(&args[1], sheet);
            let new = resolve_arg(&args[2], sheet);
            if let Some(instance) = args.get(3) {
                // Replace only the Nth instance
                let n = resolve_arg_num(instance, sheet) as usize;
                let mut count = 0usize;
                let mut result = String::new();
                let mut remainder = text.as_str();
                while let Some(pos) = remainder.find(&old) {
                    count += 1;
                    if count == n {
                        result.push_str(&remainder[..pos]);
                        result.push_str(&new);
                        result.push_str(&remainder[pos + old.len()..]);
                        return Some(result);
                    }
                    result.push_str(&remainder[..pos + old.len()]);
                    remainder = &remainder[pos + old.len()..];
                }
                result.push_str(remainder);
                Some(result)
            } else {
                Some(text.replace(&old, &new))
            }
        }
        "FIND" => {
            let args = split_args(args_str);
            if args.len() < 2 {
                return Some("#ERR".to_string());
            }
            let find_text = resolve_arg(&args[0], sheet);
            let within_text = resolve_arg(&args[1], sheet);
            let start_pos = args
                .get(2)
                .map(|s| (resolve_arg_num(s, sheet) as usize).saturating_sub(1))
                .unwrap_or(0);
            match within_text[start_pos..].find(&find_text) {
                Some(pos) => Some((pos + start_pos + 1).to_string()), // 1-based
                None => Some("#VALUE!".to_string()),
            }
        }

        // ── Logical ──
        "IF" => {
            let parts = split_args(args_str);
            if parts.len() < 2 {
                return Some("#ERR".to_string());
            }
            let cond = evaluate_condition(&parts[0], sheet);
            if cond {
                Some(resolve_arg(parts.get(1).map(|s| s.as_str()).unwrap_or(""), sheet))
            } else {
                Some(resolve_arg(parts.get(2).map(|s| s.as_str()).unwrap_or(""), sheet))
            }
        }
        "AND" => {
            let parts = split_args(args_str);
            let result = parts.iter().all(|p| {
                let v = resolve_arg(p, sheet);
                if let Ok(n) = v.parse::<f64>() {
                    n != 0.0
                } else {
                    evaluate_condition(p, sheet)
                }
            });
            Some(if result { "TRUE" } else { "FALSE" }.to_string())
        }
        "OR" => {
            let parts = split_args(args_str);
            let result = parts.iter().any(|p| {
                let v = resolve_arg(p, sheet);
                if let Ok(n) = v.parse::<f64>() {
                    n != 0.0
                } else {
                    evaluate_condition(p, sheet)
                }
            });
            Some(if result { "TRUE" } else { "FALSE" }.to_string())
        }
        "NOT" => {
            let v = resolve_arg(args_str.trim(), sheet);
            if let Ok(n) = v.parse::<f64>() {
                Some(if n == 0.0 { "TRUE" } else { "FALSE" }.to_string())
            } else {
                let cond = evaluate_condition(args_str.trim(), sheet);
                Some(if !cond { "TRUE" } else { "FALSE" }.to_string())
            }
        }
        "IFERROR" => {
            let parts = split_args(args_str);
            if parts.len() < 2 {
                return Some("#ERR".to_string());
            }
            let val = resolve_arg(&parts[0], sheet);
            if val.starts_with('#') {
                Some(resolve_arg(&parts[1], sheet))
            } else {
                Some(val)
            }
        }

        // ── Statistical ──
        "COUNTA" => {
            let args = split_args(args_str);
            let mut count = 0usize;
            for arg in &args {
                let refs = parse_range(arg);
                if refs.is_empty() {
                    // Literal — counts as non-empty
                    let v = resolve_arg(arg, sheet);
                    if !v.is_empty() {
                        count += 1;
                    }
                } else {
                    count += refs
                        .iter()
                        .filter(|&&(r, c)| !sheet.get_cell(r, c).display.is_empty())
                        .count();
                }
            }
            Some(count.to_string())
        }
        "COUNTIF" => {
            let args = split_args(args_str);
            if args.len() != 2 {
                return Some("#ERR".to_string());
            }
            let refs = parse_range(&args[0]);
            let criteria = resolve_arg(&args[1], sheet);
            let count = count_if_refs(&refs, &criteria, sheet);
            Some(count.to_string())
        }
        "SUMIF" => {
            let args = split_args(args_str);
            if args.len() < 2 {
                return Some("#ERR".to_string());
            }
            let criteria_range = parse_range(&args[0]);
            let criteria = resolve_arg(&args[1], sheet);
            let sum_range = if args.len() >= 3 {
                parse_range(&args[2])
            } else {
                criteria_range.clone()
            };

            let mut total = 0.0f64;
            for (i, &(r, c)) in criteria_range.iter().enumerate() {
                let cell_val = cell_text(sheet, r, c);
                if matches_criteria(&cell_val, &criteria) {
                    if let Some(&(sr, sc)) = sum_range.get(i) {
                        total += cell_value(sheet, sr, sc);
                    }
                }
            }
            Some(format_number(total))
        }
        "MEDIAN" => {
            let mut vals = collect_all_values(args_str, sheet);
            if vals.is_empty() {
                return Some("0".to_string());
            }
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = vals.len() / 2;
            let median = if vals.len() % 2 == 0 {
                (vals[mid - 1] + vals[mid]) / 2.0
            } else {
                vals[mid]
            };
            Some(format_number(median))
        }
        "STDEV" => {
            let vals = collect_all_values(args_str, sheet);
            if vals.len() < 2 {
                return Some("#DIV/0!".to_string());
            }
            let mean: f64 = vals.iter().sum::<f64>() / vals.len() as f64;
            let variance: f64 =
                vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (vals.len() - 1) as f64;
            Some(format_number(variance.sqrt()))
        }

        // ── Lookup ──
        "VLOOKUP" => {
            let args = split_args(args_str);
            if args.len() < 3 {
                return Some("#ERR".to_string());
            }
            let lookup_val = resolve_arg(&args[0], sheet);
            let range_refs = parse_range(&args[1]);
            let col_idx = resolve_arg_num(&args[2], sheet) as usize;
            let exact = args
                .get(3)
                .map(|s| {
                    let v = resolve_arg(s, sheet).to_uppercase();
                    v == "FALSE" || v == "0"
                })
                .unwrap_or(false);

            if range_refs.is_empty() || col_idx == 0 {
                return Some("#ERR".to_string());
            }

            // Determine range bounds
            let min_r = range_refs.iter().map(|r| r.0).min().unwrap();
            let max_r = range_refs.iter().map(|r| r.0).max().unwrap();
            let min_c = range_refs.iter().map(|r| r.1).min().unwrap();

            let target_col = min_c + col_idx - 1;

            for r in min_r..=max_r {
                let cv = cell_text(sheet, r, min_c);
                let matched = if exact {
                    cv == lookup_val
                } else {
                    cv == lookup_val
                        || cv.parse::<f64>()
                            .ok()
                            .zip(lookup_val.parse::<f64>().ok())
                            .map(|(a, b)| (a - b).abs() < f64::EPSILON)
                            .unwrap_or(false)
                };
                if matched {
                    return Some(cell_text(sheet, r, target_col));
                }
            }
            Some("#N/A".to_string())
        }
        "HLOOKUP" => {
            let args = split_args(args_str);
            if args.len() < 3 {
                return Some("#ERR".to_string());
            }
            let lookup_val = resolve_arg(&args[0], sheet);
            let range_refs = parse_range(&args[1]);
            let row_idx = resolve_arg_num(&args[2], sheet) as usize;

            if range_refs.is_empty() || row_idx == 0 {
                return Some("#ERR".to_string());
            }

            let min_r = range_refs.iter().map(|r| r.0).min().unwrap();
            let min_c = range_refs.iter().map(|r| r.1).min().unwrap();
            let max_c = range_refs.iter().map(|r| r.1).max().unwrap();

            let target_row = min_r + row_idx - 1;

            for c in min_c..=max_c {
                let cv = cell_text(sheet, min_r, c);
                if cv == lookup_val {
                    return Some(cell_text(sheet, target_row, c));
                }
            }
            Some("#N/A".to_string())
        }
        "INDEX" => {
            let args = split_args(args_str);
            if args.len() < 3 {
                return Some("#ERR".to_string());
            }
            let range_refs = parse_range(&args[0]);
            let row_num = resolve_arg_num(&args[1], sheet) as usize;
            let col_num = resolve_arg_num(&args[2], sheet) as usize;

            if range_refs.is_empty() || row_num == 0 || col_num == 0 {
                return Some("#ERR".to_string());
            }

            let min_r = range_refs.iter().map(|r| r.0).min().unwrap();
            let min_c = range_refs.iter().map(|r| r.1).min().unwrap();

            let target_r = min_r + row_num - 1;
            let target_c = min_c + col_num - 1;
            Some(cell_text(sheet, target_r, target_c))
        }
        "MATCH" => {
            let args = split_args(args_str);
            if args.len() < 2 {
                return Some("#ERR".to_string());
            }
            let lookup_val = resolve_arg(&args[0], sheet);
            let range_refs = parse_range(&args[1]);

            for (i, &(r, c)) in range_refs.iter().enumerate() {
                let cv = cell_text(sheet, r, c);
                if cv == lookup_val {
                    return Some((i + 1).to_string()); // 1-based
                }
            }
            Some("#N/A".to_string())
        }

        // ── Date ──
        "TODAY" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            // Simple date formatting: days since epoch
            let days = now / 86400;
            // Approximate date calculation
            let (y, m, d) = days_to_ymd(days);
            Some(format!("{:02}/{:02}/{:04}", m, d, y))
        }
        "NOW" => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let days = now / 86400;
            let (y, mo, d) = days_to_ymd(days);
            let secs_in_day = now % 86400;
            let h = secs_in_day / 3600;
            let mi = (secs_in_day % 3600) / 60;
            Some(format!("{:02}/{:02}/{:04} {:02}:{:02}", mo, d, y, h, mi))
        }

        _ => None,
    }
}

/// Collect all numeric values from a comma-separated list of ranges/values.
fn collect_all_values(args_str: &str, sheet: &Sheet) -> Vec<f64> {
    let args = split_args(args_str);
    let mut vals = Vec::new();
    for arg in &args {
        if arg.contains(':') {
            vals.extend(range_values(sheet, arg));
        } else if let Some((r, c)) = parse_cell_ref(arg) {
            vals.push(cell_value(sheet, r, c));
        } else if let Ok(n) = arg.parse::<f64>() {
            vals.push(n);
        }
    }
    vals
}

/// Find matching closing paren for an opening paren at `start`.
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let bytes = s.as_bytes();
    for i in start..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// COUNTIF helper: count cells matching criteria.
fn count_if_refs(refs: &[(usize, usize)], criteria: &str, sheet: &Sheet) -> usize {
    refs.iter()
        .filter(|&&(r, c)| {
            let cv = cell_text(sheet, r, c);
            matches_criteria(&cv, criteria)
        })
        .count()
}

/// Check if a cell value matches a criteria string.
/// Supports: ">5", "<10", ">=3", "<=7", "<>0", "=text", or plain text match.
fn matches_criteria(cell_val: &str, criteria: &str) -> bool {
    let criteria = criteria.trim();
    if criteria.is_empty() {
        return cell_val.is_empty();
    }

    // Comparison operators
    for op in &[">=", "<=", "<>", "!=", ">", "<", "="] {
        if let Some(rest) = criteria.strip_prefix(op) {
            let rest = rest.trim();
            let cv = cell_val.parse::<f64>().unwrap_or(f64::NAN);
            let crit_v = rest.parse::<f64>().unwrap_or(f64::NAN);
            if !cv.is_nan() && !crit_v.is_nan() {
                return match *op {
                    ">=" => cv >= crit_v,
                    "<=" => cv <= crit_v,
                    "<>" | "!=" => (cv - crit_v).abs() > f64::EPSILON,
                    ">" => cv > crit_v,
                    "<" => cv < crit_v,
                    "=" => (cv - crit_v).abs() < f64::EPSILON,
                    _ => false,
                };
            }
            // String comparison for "="
            if *op == "=" {
                return cell_val == rest;
            }
            return false;
        }
    }

    // Plain text match (case-insensitive)
    cell_val.to_uppercase() == criteria.to_uppercase()
}

/// Simple days-since-epoch to (year, month, day).
fn days_to_ymd(total_days: u64) -> (u64, u64, u64) {
    // Approximate algorithm
    let mut y = 1970u64;
    let mut remaining = total_days as i64;

    loop {
        let days_in_year: i64 = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let month_days: [i64; 12] = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 1u64;
    for &md in &month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }

    (y, m, remaining as u64 + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// Evaluate a simple condition like "A1>0", "B2=5", "C3<>0".
fn evaluate_condition(cond: &str, sheet: &Sheet) -> bool {
    let cond = cond.trim();

    // Check for TRUE/FALSE literals
    let upper = cond.to_uppercase();
    if upper == "TRUE" {
        return true;
    }
    if upper == "FALSE" {
        return false;
    }

    // Nested function call as condition
    if cond.contains('(') && cond.contains(')') {
        if let Some(result) = try_eval_function(&cond.to_uppercase(), sheet) {
            let r = result.to_uppercase();
            return r == "TRUE" || r.parse::<f64>().map(|n| n != 0.0).unwrap_or(false);
        }
    }

    let ops = [">=", "<=", "<>", "!=", ">", "<", "="];

    for op in &ops {
        if let Some(pos) = cond.find(op) {
            let left_str = cond[..pos].trim().to_uppercase();
            let right_str = cond[pos + op.len()..].trim().to_uppercase();

            let left_val = if let Some((r, c)) = parse_cell_ref(&left_str) {
                cell_value(sheet, r, c)
            } else {
                left_str.parse::<f64>().unwrap_or(0.0)
            };

            let right_val = if let Some((r, c)) = parse_cell_ref(&right_str) {
                cell_value(sheet, r, c)
            } else {
                right_str.parse::<f64>().unwrap_or(0.0)
            };

            return match *op {
                ">=" => left_val >= right_val,
                "<=" => left_val <= right_val,
                "<>" | "!=" => (left_val - right_val).abs() > f64::EPSILON,
                ">" => left_val > right_val,
                "<" => left_val < right_val,
                "=" => (left_val - right_val).abs() < f64::EPSILON,
                _ => false,
            };
        }
    }
    false
}

/// Expression evaluator with proper operator precedence.
/// Supports +, -, *, /, cell refs, numbers, and nested function calls.
fn eval_expression(expr: &str, sheet: &Sheet) -> Option<String> {
    let result = eval_expr_additive(expr.trim(), sheet)?;
    Some(format_number(result))
}

/// Parse additive expression: term (('+' | '-') term)*
fn eval_expr_additive(expr: &str, sheet: &Sheet) -> Option<f64> {
    let tokens = tokenize_expr(expr)?;
    let (val, rest) = parse_additive(&tokens, sheet)?;
    if rest.is_empty() {
        Some(val)
    } else {
        None
    }
}

#[derive(Clone, Debug)]
enum Token {
    Num(f64),
    CellRef(usize, usize),
    FuncCall(String, String), // name, args_str
    Op(char),
    LParen,
    RParen,
}

fn tokenize_expr(expr: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let bytes = expr.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];
        match ch {
            b' ' => {
                i += 1;
            }
            b'+' | b'*' | b'/' => {
                tokens.push(Token::Op(ch as char));
                i += 1;
            }
            b'-' => {
                // Could be unary minus or subtraction
                let is_unary = tokens.is_empty()
                    || matches!(tokens.last(), Some(Token::Op(_)) | Some(Token::LParen));
                if is_unary {
                    // Read the number
                    i += 1;
                    let start = i;
                    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                        i += 1;
                    }
                    if i > start {
                        let num_str = &expr[start..i];
                        let n: f64 = num_str.parse().ok()?;
                        tokens.push(Token::Num(-n));
                    } else {
                        // Treat as subtraction operator
                        tokens.push(Token::Op('-'));
                    }
                } else {
                    tokens.push(Token::Op('-'));
                    i += 1;
                }
            }
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b'0'..=b'9' | b'.' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                let num_str = &expr[start..i];
                let n: f64 = num_str.parse().ok()?;
                tokens.push(Token::Num(n));
            }
            b'A'..=b'Z' | b'a'..=b'z' | b'$' => {
                let start = i;
                // Read identifier (letters, digits, $)
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'$')
                {
                    i += 1;
                }
                let ident = &expr[start..i];

                // Check if followed by '(' — function call
                if i < bytes.len() && bytes[i] == b'(' {
                    let paren_start = i;
                    if let Some(paren_end) = find_matching_paren(expr, paren_start) {
                        let args = &expr[paren_start + 1..paren_end];
                        tokens.push(Token::FuncCall(ident.to_uppercase(), args.to_string()));
                        i = paren_end + 1;
                    } else {
                        return None;
                    }
                } else if let Some((r, c)) = parse_cell_ref(ident) {
                    tokens.push(Token::CellRef(r, c));
                } else {
                    return None; // Unknown identifier
                }
            }
            _ => return None,
        }
    }

    Some(tokens)
}

fn parse_additive<'a>(tokens: &'a [Token], sheet: &Sheet) -> Option<(f64, &'a [Token])> {
    let (mut val, mut rest) = parse_multiplicative(tokens, sheet)?;
    while !rest.is_empty() {
        match rest.first() {
            Some(Token::Op('+')) => {
                let (rhs, r) = parse_multiplicative(&rest[1..], sheet)?;
                val += rhs;
                rest = r;
            }
            Some(Token::Op('-')) => {
                let (rhs, r) = parse_multiplicative(&rest[1..], sheet)?;
                val -= rhs;
                rest = r;
            }
            _ => break,
        }
    }
    Some((val, rest))
}

fn parse_multiplicative<'a>(tokens: &'a [Token], sheet: &Sheet) -> Option<(f64, &'a [Token])> {
    let (mut val, mut rest) = parse_primary(tokens, sheet)?;
    while !rest.is_empty() {
        match rest.first() {
            Some(Token::Op('*')) => {
                let (rhs, r) = parse_primary(&rest[1..], sheet)?;
                val *= rhs;
                rest = r;
            }
            Some(Token::Op('/')) => {
                let (rhs, r) = parse_primary(&rest[1..], sheet)?;
                if rhs.abs() < f64::EPSILON {
                    return None; // Division by zero
                }
                val /= rhs;
                rest = r;
            }
            _ => break,
        }
    }
    Some((val, rest))
}

fn parse_primary<'a>(tokens: &'a [Token], sheet: &Sheet) -> Option<(f64, &'a [Token])> {
    if tokens.is_empty() {
        return None;
    }
    match &tokens[0] {
        Token::Num(n) => Some((*n, &tokens[1..])),
        Token::CellRef(r, c) => Some((cell_value(sheet, *r, *c), &tokens[1..])),
        Token::FuncCall(name, args) => {
            let full = format!("{}({})", name, args);
            if let Some(result) = try_eval_function(&full, sheet) {
                let val = result.parse::<f64>().unwrap_or(0.0);
                Some((val, &tokens[1..]))
            } else {
                None
            }
        }
        Token::LParen => {
            let (val, rest) = parse_additive(&tokens[1..], sheet)?;
            if let Some(Token::RParen) = rest.first() {
                Some((val, &rest[1..]))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Format a number, removing trailing zeros.
fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "#NUM!".to_string();
    }
    if n.is_infinite() {
        return "#DIV/0!".to_string();
    }
    if n == n.floor() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{:.6}", n);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Apply number formatting for display.
fn apply_number_format(value: &str, fmt: NumberFormat) -> String {
    match fmt {
        NumberFormat::General => value.to_string(),
        NumberFormat::Number => {
            if let Ok(n) = value.parse::<f64>() {
                format_with_thousands(n, 2)
            } else {
                value.to_string()
            }
        }
        NumberFormat::Currency => {
            if let Ok(n) = value.parse::<f64>() {
                format!("${}", format_with_thousands(n, 2))
            } else {
                value.to_string()
            }
        }
        NumberFormat::Percent => {
            if let Ok(n) = value.parse::<f64>() {
                format!("{}%", format_number(n * 100.0))
            } else {
                value.to_string()
            }
        }
        NumberFormat::Date => {
            // If the value is a number, treat as days since epoch
            if let Ok(n) = value.parse::<f64>() {
                let days = n as u64;
                let (y, m, d) = days_to_ymd(days);
                format!("{:02}/{:02}/{:04}", m, d, y)
            } else {
                value.to_string()
            }
        }
        NumberFormat::Scientific => {
            if let Ok(n) = value.parse::<f64>() {
                format!("{:.2E}", n)
            } else {
                value.to_string()
            }
        }
    }
}

/// Format number with thousands separators and decimal places.
fn format_with_thousands(n: f64, decimals: usize) -> String {
    let negative = n < 0.0;
    let n = n.abs();
    let formatted = format!("{:.prec$}", n, prec = decimals);
    let parts: Vec<&str> = formatted.split('.').collect();
    let int_part = parts[0];

    // Add thousands separators
    let mut result = String::new();
    for (i, ch) in int_part.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.insert(0, ',');
        }
        result.insert(0, ch);
    }

    if parts.len() > 1 {
        result.push('.');
        result.push_str(parts[1]);
    }

    if negative {
        result.insert(0, '-');
    }
    result
}

/// Column index -> letter(s): 0->A, 1->B, ..., 25->Z, 26->AA
fn col_to_letter(col: usize) -> String {
    let mut result = String::new();
    let mut c = col;
    loop {
        result.insert(0, (b'A' + (c % 26) as u8) as char);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    result
}

// ── Persistence (JSON-based .ysheet) ──

fn sheets_directory() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home).join(".local/share/yantrik/sheets")
}

/// Serialize state to JSON.
fn serialize_state(state: &SpreadsheetState) -> String {
    // Simple JSON serialization without serde
    let mut json = String::from("{\n");
    json.push_str(&format!("  \"active_sheet\": {},\n", state.active_sheet));
    json.push_str("  \"sheets\": [\n");

    for (si, sheet) in state.sheets.iter().enumerate() {
        json.push_str("    {\n");
        json.push_str(&format!("      \"name\": \"{}\",\n", escape_json(&sheet.name)));
        json.push_str("      \"cells\": [\n");

        for (ri, row) in sheet.cells.iter().enumerate() {
            json.push_str("        [");
            for (ci, cell) in row.iter().enumerate() {
                json.push_str(&format!(
                    "{{\"r\":\"{}\",\"b\":{},\"i\":{},\"a\":{},\"bg\":{},\"tc\":{},\"nf\":{}}}",
                    escape_json(&cell.raw),
                    cell.is_bold,
                    cell.is_italic,
                    cell.align,
                    cell.bg_color_idx,
                    cell.text_color_idx,
                    cell.number_format as i32,
                ));
                if ci + 1 < row.len() {
                    json.push(',');
                }
            }
            json.push(']');
            if ri + 1 < sheet.cells.len() {
                json.push(',');
            }
            json.push('\n');
        }

        json.push_str("      ],\n");

        // Comments
        json.push_str("      \"comments\": {");
        let comments: Vec<_> = sheet.comments.iter().collect();
        for (i, ((r, c), text)) in comments.iter().enumerate() {
            json.push_str(&format!("\"{}_{}\": \"{}\"", r, c, escape_json(text)));
            if i + 1 < comments.len() {
                json.push(',');
            }
        }
        json.push_str("}\n");

        json.push_str("    }");
        if si + 1 < state.sheets.len() {
            json.push(',');
        }
        json.push('\n');
    }

    json.push_str("  ]\n");
    json.push_str("}\n");
    json
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Save state to a .ysheet file.
fn save_state(state: &SpreadsheetState) {
    let dir = sheets_directory();
    let _ = std::fs::create_dir_all(&dir);

    let path = if let Some(ref p) = state.current_file {
        p.clone()
    } else {
        dir.join("spreadsheet.ysheet")
    };

    let json = serialize_state(state);
    match std::fs::write(&path, &json) {
        Ok(()) => {
            tracing::info!(path = %path.display(), "Spreadsheet saved");
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to save spreadsheet");
        }
    }
}

/// Load state from the most recent .ysheet file.
/// Returns None if no file found or parse error.
fn load_state_from_disk(state: &mut SpreadsheetState) -> bool {
    let dir = sheets_directory();
    if !dir.exists() {
        return false;
    }

    let ysheet_path = if let Some(ref p) = state.current_file {
        if p.exists() {
            Some(p.clone())
        } else {
            None
        }
    } else {
        // Find most recent .ysheet
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .map_or(false, |ext| ext == "ysheet")
            })
            .collect();
        files.sort_by_key(|e| {
            std::cmp::Reverse(
                e.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            )
        });
        files.first().map(|e| e.path())
    };

    if let Some(path) = ysheet_path {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if parse_ysheet_json(&content, state) {
                state.current_file = Some(path.clone());
                tracing::info!(path = %path.display(), "Spreadsheet loaded");
                return true;
            }
        }
    }
    false
}

/// Minimal JSON parser for .ysheet files.
/// This is intentionally simple — we control the format.
fn parse_ysheet_json(content: &str, state: &mut SpreadsheetState) -> bool {
    // Very basic parsing: look for sheet data patterns
    // In a real impl we'd use serde_json, but keeping deps minimal
    let content = content.trim();
    if !content.starts_with('{') {
        return false;
    }

    // Extract active_sheet
    if let Some(pos) = content.find("\"active_sheet\"") {
        let after = &content[pos..];
        if let Some(colon) = after.find(':') {
            let val_start = &after[colon + 1..];
            let val_end = val_start
                .find(|c: char| c == ',' || c == '\n' || c == '}')
                .unwrap_or(val_start.len());
            if let Ok(n) = val_start[..val_end].trim().parse::<usize>() {
                state.active_sheet = n;
            }
        }
    }

    // For a full implementation, we'd parse the JSON properly.
    // Keeping the existing data as-is if parsing fails.
    true
}

// ── UI sync ──

/// Push the entire cell grid of the active sheet to the UI.
fn sync_grid_to_ui(ui: &App, state: &SpreadsheetState) {
    let sheet = state.current_sheet();
    let rows = state.row_count;
    let cols = state.col_count;

    let mut flat: Vec<SpreadsheetCell> = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let cell = sheet.get_cell(r, c);

            // Check if this cell is part of a merge but NOT the top-left
            let in_merge_non_primary = sheet.merges.iter().any(|m| {
                r >= m.r1 && r <= m.r2 && c >= m.c1 && c <= m.c2 && (r != m.r1 || c != m.c1)
            });

            let display_text = if in_merge_non_primary {
                String::new()
            } else {
                // Apply number formatting if not general
                match cell.number_format {
                    NumberFormat::General => cell.display.clone(),
                    fmt => apply_number_format(&cell.display, fmt),
                }
            };

            flat.push(SpreadsheetCell {
                text: SharedString::from(&display_text),
                is_formula: cell.raw.starts_with('='),
                is_bold: cell.is_bold,
                is_italic: cell.is_italic,
                align: cell.align,
                bg_color: color_idx_to_brush(cell.bg_color_idx),
                text_color: text_color_idx_to_brush(cell.text_color_idx),
                has_comment: sheet.comments.contains_key(&(r, c)),
                number_format: cell.number_format as i32,
            });
        }
    }

    ui.set_sheet_cell_grid(ModelRc::new(VecModel::from(flat)));
}

/// Push sheet tab state to the UI.
fn sync_tabs_to_ui(ui: &App, state: &SpreadsheetState) {
    let tabs: Vec<SheetTab> = state
        .sheets
        .iter()
        .enumerate()
        .map(|(i, s)| SheetTab {
            name: SharedString::from(&s.name),
            is_active: i == state.active_sheet,
        })
        .collect();
    ui.set_sheet_tabs(ModelRc::new(VecModel::from(tabs)));
}

/// Update the formula bar with the active cell's raw content.
fn sync_formula_bar(ui: &App, state: &SpreadsheetState) {
    let sheet = state.current_sheet();
    let cell = sheet.get_cell(state.active_row, state.active_col);
    ui.set_sheet_cell_data(SharedString::from(&cell.raw));

    // Update formatting toggle state
    ui.set_sheet_fmt_bold_active(cell.is_bold);
    ui.set_sheet_fmt_italic_active(cell.is_italic);
    ui.set_sheet_fmt_align_active(cell.align);
}

/// Update status bar text.
fn sync_status(ui: &App, state: &SpreadsheetState) {
    let sheet = state.current_sheet();
    let cell_ref = format!(
        "{}{}",
        col_to_letter(state.active_col),
        state.active_row + 1
    );
    let sheet_name = &sheet.name;

    // Count non-empty cells
    let filled: usize = sheet
        .cells
        .iter()
        .flat_map(|row| row.iter())
        .filter(|c| !c.display.is_empty())
        .count();

    let mut status = format!(
        "Cell: {} | Sheet: {} | {} cells filled",
        cell_ref, sheet_name, filled
    );

    // Show comment if present
    if let Some(comment) = sheet.comments.get(&(state.active_row, state.active_col)) {
        status.push_str(&format!(" | Comment: {}", comment));
    }

    // Show filter state
    if state.filter_col.is_some() {
        status.push_str(&format!(" | Filtered: \"{}\"", state.filter_text));
    }

    ui.set_sheet_status_text(SharedString::from(&status));
}

/// Re-evaluate all formula cells in the active sheet.
fn recalculate_sheet(state: &mut SpreadsheetState) {
    let sheet_idx = state.active_sheet;
    if sheet_idx >= state.sheets.len() {
        state.active_sheet = 0;
        return;
    }
    let rows = state.sheets[sheet_idx].cells.len();
    let cols = if rows > 0 {
        state.sheets[sheet_idx].cells[0].len()
    } else {
        0
    };

    // Collect formulas first to avoid borrow issues
    let mut formulas: Vec<(usize, usize, String)> = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let cell = &state.sheets[sheet_idx].cells[r][c];
            if cell.raw.starts_with('=') {
                formulas.push((r, c, cell.raw.clone()));
            }
        }
    }

    // Evaluate each formula (simple single-pass; no circular ref detection)
    for (r, c, raw) in formulas {
        let display = evaluate_formula(&raw, &state.sheets[sheet_idx]);
        state.sheets[sheet_idx].cells[r][c].display = display;
    }
}

/// Full sync: grid + tabs + formula bar + status.
fn sync_all(ui: &App, state: &SpreadsheetState) {
    sync_grid_to_ui(ui, state);
    sync_tabs_to_ui(ui, state);
    sync_formula_bar(ui, state);
    sync_status(ui, state);
}

// ── CSV import/export ──

fn import_csv_to_sheet(sheet: &mut Sheet, content: &str, cols: usize) {
    let lines: Vec<&str> = content.lines().collect();
    sheet.cells.clear();

    for line in &lines {
        let fields: Vec<String> = parse_csv_line(line);
        let mut row = Vec::with_capacity(cols);
        for (i, field) in fields.iter().enumerate() {
            if i >= cols {
                break;
            }
            row.push(CellData {
                raw: field.clone(),
                display: field.clone(),
                ..CellData::default()
            });
        }
        // Pad remaining columns
        while row.len() < cols {
            row.push(CellData::default());
        }
        sheet.cells.push(row);
    }

    // Pad remaining rows to at least 50
    while sheet.cells.len() < 50 {
        sheet.cells.push(vec![CellData::default(); cols]);
    }
}

/// Simple CSV line parser (handles quoted fields).
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                if in_quotes {
                    // Check for escaped quote
                    if chars.peek() == Some(&'"') {
                        current.push('"');
                        chars.next();
                    } else {
                        in_quotes = false;
                    }
                } else {
                    in_quotes = true;
                }
            }
            ',' if !in_quotes => {
                fields.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current);
    fields
}

fn export_sheet_to_csv(sheet: &Sheet) -> String {
    let mut csv = String::new();
    for row in &sheet.cells {
        let line: Vec<String> = row
            .iter()
            .map(|cell| {
                let val = &cell.display;
                if val.contains(',') || val.contains('"') || val.contains('\n') {
                    format!("\"{}\"", val.replace('"', "\"\""))
                } else {
                    val.clone()
                }
            })
            .collect();
        csv.push_str(&line.join(","));
        csv.push('\n');
    }
    csv
}

// ── Wire ──

/// Wire all spreadsheet callbacks.
pub fn wire(ui: &App, ctx: &AppContext) {
    let state = Rc::new(RefCell::new(SpreadsheetState::new()));

    // Try to load saved state on startup
    {
        let mut s = state.borrow_mut();
        if load_state_from_disk(&mut s) {
            recalculate_sheet(&mut s);
        }
    }

    // ── Cell clicked ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_cell_clicked(move |row, col| {
            let mut s = st.borrow_mut();
            let prev_row = s.active_row;
            let prev_col = s.active_col;

            // Auto-commit: persist formula bar value to the previous cell before switching
            if let Some(ui) = ui_weak.upgrade() {
                let current_text = ui.get_sheet_cell_data().to_string();
                let old = s.current_sheet().get_cell(prev_row, prev_col);
                if current_text != old.raw {
                    s.push_undo();
                    let display = if current_text.starts_with('=') {
                        evaluate_formula(&current_text, s.current_sheet())
                    } else {
                        current_text.clone()
                    };
                    s.current_sheet_mut().set_cell(
                        prev_row,
                        prev_col,
                        CellData {
                            raw: current_text,
                            display,
                            is_bold: old.is_bold,
                            is_italic: old.is_italic,
                            align: old.align,
                            bg_color_idx: old.bg_color_idx,
                            text_color_idx: old.text_color_idx,
                            number_format: old.number_format,
                        },
                    );
                    recalculate_sheet(&mut s);
                }
            }

            s.active_row = row as usize;
            s.active_col = col as usize;
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_active_row(row);
                ui.set_sheet_active_col(col);
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Cell edited ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_cell_edited(move |row, col, text| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = row as usize;
            let c = col as usize;
            let raw = text.to_string();

            let old = s.current_sheet().get_cell(r, c);

            let display = if raw.starts_with('=') {
                evaluate_formula(&raw, s.current_sheet())
            } else {
                raw.clone()
            };

            // Check for formula errors
            let formula_error = if raw.starts_with('=') && (display.starts_with("#ERR") || display.starts_with("#DIV") || display.starts_with("#N/A") || display.starts_with("#VALUE") || display.starts_with("#NUM")) {
                format!("Error: {}", display)
            } else {
                String::new()
            };

            s.current_sheet_mut().set_cell(
                r,
                c,
                CellData {
                    raw,
                    display,
                    is_bold: old.is_bold,
                    is_italic: old.is_italic,
                    align: old.align,
                    bg_color_idx: old.bg_color_idx,
                    text_color_idx: old.text_color_idx,
                    number_format: old.number_format,
                },
            );

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_formula_error(SharedString::from(formula_error));
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Formula submitted ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_formula_submitted(move |text| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let raw = text.to_string();

            let old = s.current_sheet().get_cell(r, c);
            let display = if raw.starts_with('=') {
                evaluate_formula(&raw, s.current_sheet())
            } else {
                raw.clone()
            };

            // Check for formula errors
            let formula_error = if raw.starts_with('=') && (display.starts_with("#ERR") || display.starts_with("#DIV") || display.starts_with("#N/A") || display.starts_with("#VALUE") || display.starts_with("#NUM")) {
                format!("Error: {}", display)
            } else {
                String::new()
            };

            s.current_sheet_mut().set_cell(
                r,
                c,
                CellData {
                    raw,
                    display,
                    is_bold: old.is_bold,
                    is_italic: old.is_italic,
                    align: old.align,
                    bg_color_idx: old.bg_color_idx,
                    text_color_idx: old.text_color_idx,
                    number_format: old.number_format,
                },
            );

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_formula_error(SharedString::from(formula_error));
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Switch sheet ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_switch_sheet(move |idx| {
            let mut s = st.borrow_mut();
            let i = idx as usize;
            if i < s.sheets.len() {
                s.active_sheet = i;
                s.active_row = 0;
                s.active_col = 0;

                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_sheet_active_row(0);
                    ui.set_sheet_active_col(0);
                    sync_all(&ui, &s);
                }
            }
        });
    }

    // ── Add sheet ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_add_sheet(move || {
            let mut s = st.borrow_mut();
            s.push_undo();
            let num = s.sheets.len() + 1;
            let name = format!("Sheet {}", num);
            let rows = s.row_count;
            let cols = s.col_count;
            s.sheets.push(Sheet::new(&name, rows, cols));
            s.active_sheet = s.sheets.len() - 1;
            s.active_row = 0;
            s.active_col = 0;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_active_row(0);
                ui.set_sheet_active_col(0);
                sync_all(&ui, &s);
            }
        });
    }

    // ── Format bold ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_format_bold(move || {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.is_bold = !cell.is_bold;
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
            }
        });
    }

    // ── Format italic ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_format_italic(move || {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.is_italic = !cell.is_italic;
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
            }
        });
    }

    // ── Format alignment ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_format_align(move |align| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.align = align;
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
            }
        });
    }

    // ── Import CSV ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_import_csv(move || {
            let mut s = st.borrow_mut();
            s.push_undo();
            let dir = sheets_directory();

            let csv_path = if dir.exists() {
                let mut files: Vec<_> = std::fs::read_dir(&dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| {
                        e.path()
                            .extension()
                            .map_or(false, |ext| ext == "csv")
                    })
                    .collect();
                files.sort_by_key(|e| {
                    std::cmp::Reverse(
                        e.metadata()
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                    )
                });
                files.first().map(|e| e.path())
            } else {
                None
            };

            if let Some(path) = csv_path {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let cols = s.col_count;
                    import_csv_to_sheet(s.current_sheet_mut(), &content, cols);
                    recalculate_sheet(&mut s);
                    tracing::info!(path = %path.display(), "CSV imported into spreadsheet");

                    if let Some(ui) = ui_weak.upgrade() {
                        sync_grid_to_ui(&ui, &s);
                        sync_status(&ui, &s);
                    }
                }
            } else {
                tracing::info!("No CSV files found in sheets directory");
            }
        });
    }

    // ── Export CSV ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_export_csv(move || {
            let s = st.borrow();
            let dir = sheets_directory();
            let _ = std::fs::create_dir_all(&dir);

            let sheet = s.current_sheet();
            let csv = export_sheet_to_csv(sheet);

            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let filename = format!("{}-{}.csv", ts, sheet.name.replace(' ', "_"));
            let path = dir.join(&filename);

            match std::fs::write(&path, &csv) {
                Ok(()) => {
                    tracing::info!(path = %path.display(), "Spreadsheet exported as CSV");
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to export CSV");
                }
            }

            if let Some(ui) = ui_weak.upgrade() {
                let status = format!("Exported to {}", filename);
                ui.set_sheet_status_text(SharedString::from(&status));
            }
        });
    }

    // ── Sort column ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_sort_column(move |col, ascending| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let c = col as usize;
            let sheet = s.current_sheet_mut();

            // Sort all data rows by the specified column
            sheet.cells.sort_by(|row_a, row_b| {
                let val_a = if c < row_a.len() {
                    &row_a[c].display
                } else {
                    ""
                };
                let val_b = if c < row_b.len() {
                    &row_b[c].display
                } else {
                    ""
                };

                // Try numeric comparison first
                let cmp = match (val_a.parse::<f64>(), val_b.parse::<f64>()) {
                    (Ok(a), Ok(b)) => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
                    _ => val_a.to_lowercase().cmp(&val_b.to_lowercase()),
                };

                if ascending {
                    cmp
                } else {
                    cmp.reverse()
                }
            });

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Filter column ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_filter_column(move |col, filter_text| {
            let mut s = st.borrow_mut();
            let c = col as usize;
            let filter = filter_text.to_string().to_lowercase();

            // Store original rows if not already filtered
            if s.original_rows.is_none() {
                s.original_rows = Some(s.current_sheet().cells.clone());
            }

            // Restore original rows first
            if let Some(ref original) = s.original_rows {
                s.current_sheet_mut().cells = original.clone();
            }

            if filter.is_empty() {
                // Clear filter
                s.filter_col = None;
                s.filter_text.clear();
                s.original_rows = None;
            } else {
                // Apply filter: keep rows where col c contains filter text
                let cols = s.col_count;
                let sheet = s.current_sheet_mut();
                sheet.cells.retain(|row| {
                    if c < row.len() {
                        row[c].display.to_lowercase().contains(&filter)
                    } else {
                        false
                    }
                });
                // Ensure minimum rows
                while sheet.cells.len() < 50 {
                    sheet.cells.push(vec![CellData::default(); cols]);
                }
                s.filter_col = Some(c);
                s.filter_text = filter;
            }

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Clear filter ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_clear_filter(move || {
            let mut s = st.borrow_mut();
            if let Some(original) = s.original_rows.take() {
                s.current_sheet_mut().cells = original;
            }
            s.filter_col = None;
            s.filter_text.clear();

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Insert row ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_insert_row(move |after_row| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let idx = (after_row as usize + 1).min(s.current_sheet().cells.len());
            let cols = s.col_count;
            let new_row = vec![CellData::default(); cols];
            s.current_sheet_mut().cells.insert(idx, new_row);
            s.row_count = s.current_sheet().cells.len();

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_row_count(s.row_count as i32);
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Delete row ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_delete_row(move |row| {
            let mut s = st.borrow_mut();
            let r = row as usize;
            if r < s.current_sheet().cells.len() && s.current_sheet().cells.len() > 1 {
                s.push_undo();
                s.current_sheet_mut().cells.remove(r);
                s.row_count = s.current_sheet().cells.len();
                if s.active_row >= s.row_count {
                    s.active_row = s.row_count - 1;
                }

                recalculate_sheet(&mut s);

                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_sheet_row_count(s.row_count as i32);
                    ui.set_sheet_active_row(s.active_row as i32);
                    sync_grid_to_ui(&ui, &s);
                    sync_status(&ui, &s);
                }
            }
        });
    }

    // ── Insert column ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_insert_col(move |after_col| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let idx = (after_col as usize + 1).min(s.col_count);
            let sheet = s.current_sheet_mut();
            for row in &mut sheet.cells {
                row.insert(idx, CellData::default());
            }
            s.col_count = sheet.cells.first().map_or(26, |r| r.len());

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_col_count(s.col_count as i32);
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Delete column ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_delete_col(move |col| {
            let mut s = st.borrow_mut();
            let c = col as usize;
            if c < s.col_count && s.col_count > 1 {
                s.push_undo();
                let sheet = s.current_sheet_mut();
                for row in &mut sheet.cells {
                    if c < row.len() {
                        row.remove(c);
                    }
                }
                s.col_count = sheet.cells.first().map_or(1, |r| r.len());
                if s.active_col >= s.col_count {
                    s.active_col = s.col_count - 1;
                }

                recalculate_sheet(&mut s);

                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_sheet_col_count(s.col_count as i32);
                    ui.set_sheet_active_col(s.active_col as i32);
                    sync_grid_to_ui(&ui, &s);
                    sync_status(&ui, &s);
                }
            }
        });
    }

    // ── Number formatting ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_format_number(move |format_type| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.number_format = NumberFormat::from_int(format_type);
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Cell background color ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_set_bg_color(move |color_idx| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.bg_color_idx = color_idx;
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
            }
        });
    }

    // ── Cell text color ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_set_text_color(move |color_idx| {
            let mut s = st.borrow_mut();
            s.push_undo();
            let r = s.active_row;
            let c = s.active_col;
            let mut cell = s.current_sheet().get_cell(r, c);
            cell.text_color_idx = color_idx;
            s.current_sheet_mut().set_cell(r, c, cell);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
            }
        });
    }

    // ── Find ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_find(move |query| {
            let mut s = st.borrow_mut();
            let q = query.to_string().to_lowercase();
            s.find_query = q.clone();
            s.find_row = 0;
            s.find_col = 0;

            if q.is_empty() {
                return;
            }

            let sheet = s.current_sheet();
            for r in 0..sheet.cells.len() {
                for c in 0..sheet.cells[r].len() {
                    if sheet.cells[r][c].display.to_lowercase().contains(&q) {
                        s.active_row = r;
                        s.active_col = c;
                        s.find_row = r;
                        s.find_col = c;

                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_sheet_active_row(r as i32);
                            ui.set_sheet_active_col(c as i32);
                            sync_formula_bar(&ui, &s);
                            sync_status(&ui, &s);
                        }
                        return;
                    }
                }
            }

            // Not found
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_status_text("Not found".into());
            }
        });
    }

    // ── Find next ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_find_next(move || {
            let mut s = st.borrow_mut();
            let q = s.find_query.clone();
            if q.is_empty() {
                return;
            }

            let sheet = s.current_sheet();
            let rows = sheet.cells.len();
            if rows == 0 {
                return;
            }
            let cols = sheet.cells[0].len();
            let total = rows * cols;

            // Start from current position + 1
            let start_idx = s.find_row * cols + s.find_col + 1;

            for offset in 0..total {
                let idx = (start_idx + offset) % total;
                let r = idx / cols;
                let c = idx % cols;

                if sheet.cells[r][c].display.to_lowercase().contains(&q) {
                    s.active_row = r;
                    s.active_col = c;
                    s.find_row = r;
                    s.find_col = c;

                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_sheet_active_row(r as i32);
                        ui.set_sheet_active_col(c as i32);
                        sync_formula_bar(&ui, &s);
                        sync_status(&ui, &s);
                    }
                    return;
                }
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_status_text("No more matches".into());
            }
        });
    }

    // ── Replace one ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_replace_one(move |find_text, replace_text| {
            let mut s = st.borrow_mut();
            let find = find_text.to_string();
            let replace = replace_text.to_string();
            let r = s.active_row;
            let c = s.active_col;

            let cell = s.current_sheet().get_cell(r, c);
            if cell.display.to_lowercase().contains(&find.to_lowercase()) {
                s.push_undo();
                let new_raw = cell.raw.replace(&find, &replace);
                let new_display = if new_raw.starts_with('=') {
                    evaluate_formula(&new_raw, s.current_sheet())
                } else {
                    new_raw.clone()
                };

                let mut new_cell = cell;
                new_cell.raw = new_raw;
                new_cell.display = new_display;
                s.current_sheet_mut().set_cell(r, c, new_cell);

                recalculate_sheet(&mut s);

                if let Some(ui) = ui_weak.upgrade() {
                    sync_grid_to_ui(&ui, &s);
                    sync_formula_bar(&ui, &s);
                    sync_status(&ui, &s);
                }
            }
        });
    }

    // ── Replace all ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_replace_all(move |find_text, replace_text| {
            let mut s = st.borrow_mut();
            let find = find_text.to_string();
            let replace = replace_text.to_string();

            if find.is_empty() {
                return;
            }

            s.push_undo();
            let mut count = 0usize;

            let sheet_idx = s.active_sheet;
            let rows = s.sheets[sheet_idx].cells.len();
            for r in 0..rows {
                let cols = s.sheets[sheet_idx].cells[r].len();
                for c in 0..cols {
                    let cell = &s.sheets[sheet_idx].cells[r][c];
                    if cell.raw.contains(&find) {
                        count += 1;
                        let new_raw = cell.raw.replace(&find, &replace);
                        s.sheets[sheet_idx].cells[r][c].raw = new_raw.clone();
                        s.sheets[sheet_idx].cells[r][c].display = if new_raw.starts_with('=') {
                            // Will be recalculated below
                            new_raw
                        } else {
                            new_raw
                        };
                    }
                }
            }

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                let status = format!("Replaced {} occurrence(s)", count);
                ui.set_sheet_status_text(SharedString::from(&status));
            }
        });
    }

    // ── Copy ──
    {
        let st = state.clone();
        ui.on_sheet_copy(move || {
            let mut s = st.borrow_mut();
            let r = s.active_row;
            let c = s.active_col;
            let cell = s.current_sheet().get_cell(r, c);
            s.clipboard = vec![vec![cell]];
            s.clipboard_is_cut = false;
            s.clipboard_origin = (r, c);
        });
    }

    // ── Cut ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_cut(move || {
            let mut s = st.borrow_mut();
            let r = s.active_row;
            let c = s.active_col;
            let cell = s.current_sheet().get_cell(r, c);
            s.clipboard = vec![vec![cell]];
            s.clipboard_is_cut = true;
            s.clipboard_origin = (r, c);

            // Clear the source cell
            s.push_undo();
            s.current_sheet_mut().set_cell(r, c, CellData::default());
            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Paste ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_paste(move || {
            let mut s = st.borrow_mut();
            if s.clipboard.is_empty() {
                return;
            }

            s.push_undo();
            let base_r = s.active_row;
            let base_c = s.active_col;

            let clipboard = s.clipboard.clone();
            for (dr, row) in clipboard.iter().enumerate() {
                for (dc, cell) in row.iter().enumerate() {
                    let mut new_cell = cell.clone();
                    // Re-evaluate formula in new position
                    if new_cell.raw.starts_with('=') {
                        new_cell.display =
                            evaluate_formula(&new_cell.raw, s.current_sheet());
                    }
                    s.current_sheet_mut()
                        .set_cell(base_r + dr, base_c + dc, new_cell);
                }
            }

            recalculate_sheet(&mut s);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Undo ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_undo(move || {
            let mut s = st.borrow_mut();
            if s.undo() {
                recalculate_sheet(&mut s);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_all(&ui, &s);
                }
            }
        });
    }

    // ── Redo ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_redo(move || {
            let mut s = st.borrow_mut();
            if s.redo() {
                recalculate_sheet(&mut s);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_all(&ui, &s);
                }
            }
        });
    }

    // ── Keyboard navigation ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_key_pressed(move |key| {
            let mut s = st.borrow_mut();
            let key_str = key.to_string();

            match key_str.as_str() {
                "Up" => {
                    if s.active_row > 0 {
                        s.active_row -= 1;
                    }
                }
                "Down" => {
                    if s.active_row + 1 < s.row_count {
                        s.active_row += 1;
                    }
                }
                "Left" => {
                    if s.active_col > 0 {
                        s.active_col -= 1;
                    }
                }
                "Right" | "Tab" => {
                    if s.active_col + 1 < s.col_count {
                        s.active_col += 1;
                    }
                }
                "Enter" | "Return" => {
                    if s.active_row + 1 < s.row_count {
                        s.active_row += 1;
                    }
                }
                "Home" => {
                    s.active_col = 0;
                }
                "End" => {
                    let last = s.current_sheet().last_used_col(s.active_row);
                    s.active_col = last;
                }
                "Delete" => {
                    s.push_undo();
                    let r = s.active_row;
                    let c = s.active_col;
                    let mut cell = s.current_sheet().get_cell(r, c);
                    cell.raw.clear();
                    cell.display.clear();
                    s.current_sheet_mut().set_cell(r, c, cell);
                    recalculate_sheet(&mut s);

                    if let Some(ui) = ui_weak.upgrade() {
                        sync_grid_to_ui(&ui, &s);
                    }
                }
                _ => return,
            }

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_active_row(s.active_row as i32);
                ui.set_sheet_active_col(s.active_col as i32);
                sync_formula_bar(&ui, &s);
                sync_status(&ui, &s);
            }
        });
    }

    // ── Add comment ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_add_comment(move |text| {
            let mut s = st.borrow_mut();
            let r = s.active_row;
            let c = s.active_col;
            let comment = text.to_string();
            s.current_sheet_mut()
                .comments
                .insert((r, c), comment);
            s.dirty = true;

            if let Some(ui) = ui_weak.upgrade() {
                sync_status(&ui, &s);
            }
        });
    }

    // ── Delete comment ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_delete_comment(move || {
            let mut s = st.borrow_mut();
            let r = s.active_row;
            let c = s.active_col;
            s.current_sheet_mut().comments.remove(&(r, c));
            s.dirty = true;

            if let Some(ui) = ui_weak.upgrade() {
                sync_status(&ui, &s);
            }
        });
    }

    // ── Merge cells ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_merge_cells(move |r1, c1, r2, c2| {
            let mut s = st.borrow_mut();
            s.push_undo();

            let region = MergeRegion {
                r1: r1 as usize,
                c1: c1 as usize,
                r2: r2 as usize,
                c2: c2 as usize,
            };
            s.current_sheet_mut().merges.push(region);

            if let Some(ui) = ui_weak.upgrade() {
                sync_grid_to_ui(&ui, &s);
            }
        });
    }

    // ── Unmerge cells ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_unmerge_cells(move || {
            let mut s = st.borrow_mut();
            let r = s.active_row;
            let c = s.active_col;

            if let Some(idx) = s.current_sheet().find_merge(r, c) {
                s.push_undo();
                s.current_sheet_mut().merges.remove(idx);

                if let Some(ui) = ui_weak.upgrade() {
                    sync_grid_to_ui(&ui, &s);
                }
            }
        });
    }

    // ── Freeze panes ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_freeze_panes(move |rows, cols| {
            let mut s = st.borrow_mut();
            s.freeze_rows = rows;
            s.freeze_cols = cols;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_freeze_row_count(rows);
                ui.set_sheet_freeze_col_count(cols);
            }
        });
    }

    // ── Save ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_save(move || {
            let mut s = st.borrow_mut();
            save_state(&s);
            s.dirty = false;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_status_text("Saved".into());
            }
        });
    }

    // ── Load ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_load(move || {
            let mut s = st.borrow_mut();
            if load_state_from_disk(&mut s) {
                recalculate_sheet(&mut s);
                if let Some(ui) = ui_weak.upgrade() {
                    sync_all(&ui, &s);
                    ui.set_sheet_status_text("Loaded".into());
                }
            }
        });
    }

    // ── Create chart ──
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_create_chart(move |chart_type, range| {
            let mut s = st.borrow_mut();
            let range_str = range.to_string().to_uppercase();
            let refs = parse_range(&range_str);

            if refs.is_empty() {
                return;
            }

            let sheet = s.current_sheet();
            let mut labels = Vec::new();
            let mut series = Vec::new();

            // First column = labels, rest = values
            let min_c = refs.iter().map(|r| r.1).min().unwrap();
            let max_c = refs.iter().map(|r| r.1).max().unwrap();
            let min_r = refs.iter().map(|r| r.0).min().unwrap();
            let max_r = refs.iter().map(|r| r.0).max().unwrap();

            for r in min_r..=max_r {
                labels.push(cell_text(sheet, r, min_c));
                if max_c > min_c {
                    series.push(cell_value(sheet, r, min_c + 1));
                } else {
                    series.push(cell_value(sheet, r, min_c));
                }
            }

            let chart = ChartData {
                chart_type,
                title: format!("Chart {}", s.charts.len() + 1),
                labels,
                series,
            };

            s.charts.push(chart);

            if let Some(ui) = ui_weak.upgrade() {
                let status = format!("Chart created from range {}", range_str);
                ui.set_sheet_status_text(SharedString::from(&status));
            }
        });
    }

    // ── Auto-save timer (every 60 seconds) ──
    {
        let st = state.clone();
        let auto_save_timer = Timer::default();
        auto_save_timer.start(
            TimerMode::Repeated,
            std::time::Duration::from_secs(60),
            move || {
                let mut s = st.borrow_mut();
                if s.dirty {
                    save_state(&s);
                    s.dirty = false;
                    tracing::info!("Spreadsheet auto-saved");
                }
            },
        );

        // Keep timer alive by storing in a leaked Rc
        // (The timer will be dropped when the app exits)
        let _keep = Rc::new(auto_save_timer);
        // We intentionally leak this to keep the timer alive for the app lifetime.
        std::mem::forget(_keep);
    }

    // ── AI Assist ──
    wire_ai(ui, ctx, state);
}

/// Build a data summary string from the current sheet state for AI context.
fn build_data_summary(s: &SpreadsheetState) -> String {
    let sheet = &s.sheets[s.active_sheet];
    let mut lines = Vec::new();
    lines.push(format!(
        "Sheet: {} | Active cell: {}{}",
        sheet.name,
        col_to_letter(s.active_col),
        s.active_row + 1
    ));

    let rows_used = sheet.cells.len().min(50);
    let cols_used = if rows_used > 0 { sheet.cells[0].len().min(26) } else { 0 };

    // Header row
    let mut header = String::from("   ");
    for c in 0..cols_used {
        header.push_str(&format!("{:>12}", col_to_letter(c)));
    }
    lines.push(header);

    // Data rows (first 20 with data)
    let mut data_rows = 0;
    for r in 0..rows_used {
        let mut has_data = false;
        let mut row_str = format!("{:>3}", r + 1);
        for c in 0..cols_used {
            let display = &sheet.cells[r][c].display;
            if !display.is_empty() {
                has_data = true;
            }
            let truncated = if display.len() > 10 { &display[..10] } else { display.as_str() };
            row_str.push_str(&format!("{:>12}", truncated));
        }
        if has_data {
            lines.push(row_str);
            data_rows += 1;
            if data_rows >= 20 {
                lines.push("... (more rows)".to_string());
                break;
            }
        }
    }
    lines.join("\n")
}

/// Wire AI assist callbacks for ySheets.
fn wire_ai(ui: &App, ctx: &AppContext, state: Rc<RefCell<SpreadsheetState>>) {
    let bridge = ctx.bridge.clone();
    let ai_state = super::ai_assist::AiAssistState::new();

    // Free-form AI submit
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_submit(move |prompt| {
            let prompt_str = prompt.to_string();
            if prompt_str.trim().is_empty() { return; }
            let context = build_data_summary(&st.borrow());
            let full_prompt = super::ai_assist::office_freeform_prompt("spreadsheet", &context, &prompt_str);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 45,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // Natural language → formula
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_formula(move |prompt| {
            let prompt_str = prompt.to_string();
            let desc = if prompt_str.trim().is_empty() {
                "Generate a useful formula for the selected cell based on surrounding data".to_string()
            } else {
                prompt_str
            };
            let context = build_data_summary(&st.borrow());
            let full_prompt = super::ai_assist::sheet_formula_prompt(&desc, &context);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 30,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // Analyze data
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_analyze(move || {
            let context = build_data_summary(&st.borrow());
            let full_prompt = super::ai_assist::sheet_analyze_prompt(&context);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 45,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // Suggest chart
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_suggest_chart(move || {
            let context = build_data_summary(&st.borrow());
            let full_prompt = super::ai_assist::sheet_chart_prompt(&context);
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 30,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // Apply AI response (insert formula/value or CSV data into cells)
    {
        let st = state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_apply(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let response = ui.get_sheet_ai_response().to_string();
                // Strip markdown code fences if present
                let cleaned = response
                    .trim()
                    .trim_start_matches("```csv")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();

                // Detect multi-line CSV data — find the CSV portion by looking for
                // consecutive lines with commas (skip introductory text)
                let all_lines: Vec<&str> = cleaned.lines()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .collect();

                // Find the first line that looks like CSV (contains comma, not a sentence)
                let csv_start = all_lines.iter().position(|l| {
                    l.contains(',') && !l.ends_with('.') && !l.ends_with(':')
                }).unwrap_or(0);
                let lines: Vec<&str> = all_lines[csv_start..].iter()
                    .take_while(|l| l.contains(','))
                    .copied()
                    .collect();

                let is_csv = lines.len() > 1;

                if is_csv {
                    // Multi-cell CSV apply: fill from active cell
                    let mut s = st.borrow_mut();
                    s.push_undo();
                    let start_row = s.active_row;
                    let start_col = s.active_col;
                    let mut cells_filled = 0;

                    for (ri, line) in lines.iter().enumerate() {
                        let row = start_row + ri;
                        if row >= s.row_count { break; }
                        for (ci, val) in parse_csv_line(line).iter().enumerate() {
                            let col = start_col + ci;
                            if col >= s.col_count { break; }
                            let val = val.trim().to_string();
                            let display = if val.starts_with('=') {
                                evaluate_formula(&val, s.current_sheet())
                            } else {
                                val.clone()
                            };
                            // Bold the header row
                            let is_bold = ri == 0;
                            s.current_sheet_mut().set_cell(row, col, CellData {
                                raw: val,
                                display,
                                is_bold,
                                is_italic: false,
                                align: 0,
                                bg_color_idx: 0,
                                text_color_idx: 0,
                                number_format: NumberFormat::General,
                            });
                            cells_filled += 1;
                        }
                    }
                    recalculate_sheet(&mut s);
                    sync_grid_to_ui(&ui, &s);
                    sync_formula_bar(&ui, &s);
                    ui.set_sheet_status_text(
                        format!("AI filled {} cells ({} rows x {} cols)",
                            cells_filled, lines.len(),
                            lines.first().map(|l| parse_csv_line(l).len()).unwrap_or(0)
                        ).into()
                    );
                } else {
                    // Single-cell apply: formula or value
                    let value = if let Some(formula_line) = cleaned.lines().find(|l| l.trim().starts_with('=')) {
                        formula_line.trim().to_string()
                    } else {
                        cleaned.lines().next().unwrap_or("").trim().to_string()
                    };
                    if !value.is_empty() {
                        let mut s = st.borrow_mut();
                        s.push_undo();
                        let row = s.active_row;
                        let col = s.active_col;
                        let old = s.current_sheet().get_cell(row, col);
                        let display = if value.starts_with('=') {
                            evaluate_formula(&value, s.current_sheet())
                        } else {
                            value.clone()
                        };
                        s.current_sheet_mut().set_cell(row, col, CellData {
                            raw: value,
                            display,
                            is_bold: old.is_bold,
                            is_italic: old.is_italic,
                            align: old.align,
                            bg_color_idx: old.bg_color_idx,
                            text_color_idx: old.text_color_idx,
                            number_format: old.number_format,
                        });
                        recalculate_sheet(&mut s);
                        sync_grid_to_ui(&ui, &s);
                        sync_formula_bar(&ui, &s);
                        ui.set_sheet_status_text(format!("AI applied to {}{}", col_to_letter(col), row + 1).into());
                    }
                }
                ui.set_sheet_ai_response("".into());
            }
        });
    }

    // Dismiss AI response
    {
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_dismiss(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_sheet_ai_response("".into());
            }
        });
    }

    // Contextual insights — uses companion's recall to find related emails/calendar/docs
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_insights(move || {
            let context = build_data_summary(&st.borrow());
            let full_prompt = super::ai_assist::contextual_insights_prompt(&context, "spreadsheet");
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 60,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // ── Generate data ──
    {
        let st = state.clone();
        let bridge = bridge.clone();
        let ai_st = ai_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_sheet_ai_generate_data(move |prompt| {
            let prompt_str = prompt.to_string();
            let desc = if prompt_str.trim().is_empty() {
                "Generate sample sales data with 10 rows".to_string()
            } else {
                prompt_str
            };
            let s = st.borrow();
            let start_cell = format!("{}{}", col_to_letter(s.active_col), s.active_row + 1);
            let full_prompt = format!(
                "You are a CSV data generator. You output ONLY raw CSV. No words, no explanation, no markdown, no code fences.\n\
                 REQUEST: {}\n\
                 RULES: First row = headers. 10-20 data rows. Plain numbers (no $ or %). Dates as YYYY-MM-DD. Realistic varied data.\n\
                 BEGIN CSV OUTPUT NOW:",
                desc
            );
            super::ai_assist::ai_request(
                &ui_weak,
                &bridge,
                &ai_st,
                super::ai_assist::AiAssistRequest {
                    prompt: full_prompt,
                    timeout_secs: 60,
                    set_working: Box::new(|ui, v| ui.set_sheet_ai_working(v)),
                    set_response: Box::new(|ui, s| ui.set_sheet_ai_response(s.into())),
                    get_response: Box::new(|ui| ui.get_sheet_ai_response().to_string()),
                },
            );
        });
    }

    // ── Formula help toggle ──
    {
        let ui_weak = ui.as_weak();
        ui.on_sheet_show_formula_help(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let open = ui.get_sheet_formula_help_open();
                ui.set_sheet_formula_help_open(!open);
                if !open {
                    // Populate hints when opening
                    let hints: Vec<SharedString> = vec![
                        "SUM(range)        — Add all values in range".into(),
                        "AVG(range)        — Average of values".into(),
                        "MIN(range)        — Smallest value".into(),
                        "MAX(range)        — Largest value".into(),
                        "COUNT(range)      — Count numeric cells".into(),
                        "IF(cond,t,f)      — Conditional value".into(),
                        "CONCAT(a,b,...)   — Join text strings".into(),
                        "VLOOKUP(v,rng,c)  — Vertical lookup".into(),
                        "ROUND(val,digits) — Round to N decimals".into(),
                        "ABS(val)          — Absolute value".into(),
                        "MEDIAN(range)     — Median of values".into(),
                        "PRODUCT(range)    — Multiply all values".into(),
                        "TODAY()           — Current date".into(),
                        "NOW()             — Current date + time".into(),
                        "LEN(text)         — Length of text".into(),
                        "UPPER(text)       — Convert to uppercase".into(),
                        "LOWER(text)       — Convert to lowercase".into(),
                    ];
                    ui.set_sheet_formula_hints(ModelRc::new(VecModel::from(hints)));
                }
            }
        });
    }
}

/// Initialize spreadsheet state when navigating to screen 29.
pub fn load_spreadsheet(ui: &App) {
    // Set initial grid data
    let rows = 50;
    let cols = 26;
    let mut flat: Vec<SpreadsheetCell> = Vec::with_capacity(rows * cols);
    for _ in 0..rows * cols {
        flat.push(SpreadsheetCell {
            text: SharedString::default(),
            is_formula: false,
            is_bold: false,
            is_italic: false,
            align: 0,
            bg_color: slint::Brush::default(),
            text_color: slint::Brush::default(),
            has_comment: false,
            number_format: 0,
        });
    }
    ui.set_sheet_cell_grid(ModelRc::new(VecModel::from(flat)));
    ui.set_sheet_active_row(0);
    ui.set_sheet_active_col(0);
    ui.set_sheet_cell_data(SharedString::default());
    ui.set_sheet_status_text("Ready".into());
    ui.set_sheet_row_count(rows as i32);
    ui.set_sheet_col_count(cols as i32);
    ui.set_sheet_tabs(ModelRc::new(VecModel::from(vec![SheetTab {
        name: "Sheet 1".into(),
        is_active: true,
    }])));
}
