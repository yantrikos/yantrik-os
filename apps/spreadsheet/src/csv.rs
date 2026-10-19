//! CSV, read and written the way RFC 4180 and every spreadsheet do it.
//!
//! The first version split the text on `\n` before it looked at quotes, so a cell with a line
//! break in it was written correctly and read back as broken rows; it did not quote `\r` and
//! kept Excel's byte-order mark as part of the first cell (review of #727, finding 2). This reads
//! character by character with the quote state in hand, so a quoted field may hold commas,
//! doubled quotes, `\n`, `\r` and `\r\n`, and the mark is dropped.
//!
//! A blank line in the middle of a file is a row of one empty cell, not nothing: skipping it
//! moved every row below it up by one, and with it every reference into them.

use crate::formula::budget::MAX_TEXT;
use crate::sheet::{MAX_CELLS, MAX_SHEET_COLS};

/// Rows of fields, or a sentence saying where the file stops making sense. A field longer than
/// the [`MAX_TEXT`] characters a cell holds is refused as it is read: a formula in it could
/// otherwise be any length, and nest as deep as it likes.
///
/// The sheet's own limits are kept while reading, not after: a row of more than
/// [`MAX_SHEET_COLS`] fields, or more than [`MAX_CELLS`] fields in all, stops the read there. A
/// 16 MB file of newlines built 16 million rows, 2.1 GB, before the sheet refused it (security
/// review of #727). Every row holds at least one field, so the rows are bounded too.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // Whether anything of the current row has been read, so a final newline ends the last row
    // instead of starting an empty one.
    let mut started = false;
    let mut line = 1usize;
    let mut quote_line = 0usize;
    let mut chars = text.chars().peekable();
    // Characters in `field`, and the line it began on.
    let (mut field_chars, mut field_line) = (0usize, 1usize);
    let mut fields = 0usize;
    // Keep one more field, refusing the file if it is one too many.
    let mut keep = |row: &mut Vec<String>, field: String, line: usize| -> Result<(), String> {
        fields += 1;
        if fields > MAX_CELLS {
            return Err(format!("the file has more than the {MAX_CELLS} cells one sheet holds (line {line})"));
        }
        if row.len() >= MAX_SHEET_COLS {
            return Err(format!("line {line} has more than the {MAX_SHEET_COLS} columns one sheet holds"));
        }
        row.push(field);
        Ok(())
    };

    while let Some(c) = chars.next() {
        if field_chars == 0 {
            field_line = line;
        }
        if field_chars > MAX_TEXT {
            return Err(too_long(field_line));
        }
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                    field_chars += 1;
                }
                '"' => quoted = false,
                other => {
                    if other == '\n' {
                        line += 1;
                    }
                    field.push(other);
                    field_chars += 1;
                }
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => {
                quoted = true;
                quote_line = line;
                started = true;
            }
            ',' => {
                keep(&mut row, std::mem::take(&mut field), line)?;
                field_chars = 0;
                started = true;
            }
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                keep(&mut row, std::mem::take(&mut field), line)?;
                line += 1;
                rows.push(std::mem::take(&mut row));
                field_chars = 0;
                started = false;
            }
            other => {
                field.push(other);
                field_chars += 1;
                started = true;
            }
        }
    }
    if field_chars > MAX_TEXT {
        return Err(too_long(field_line));
    }
    if quoted {
        return Err(format!(
            "the file ends inside a quoted cell that starts on line {quote_line} — a closing \" is missing"
        ));
    }
    if started {
        keep(&mut row, field, line)?;
        rows.push(row);
    }
    Ok(rows)
}

fn too_long(line: usize) -> String {
    format!("the cell starting on line {line} is longer than the {MAX_TEXT} characters one cell holds")
}

/// Rows as CSV text: the inverse of [`parse`]. Lines end in `\r\n`, as RFC 4180 has them.
pub fn write(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for row in rows {
        for (c, field) in row.iter().enumerate() {
            if c > 0 {
                out.push(',');
            }
            if field.contains([',', '"', '\n', '\r']) {
                out.push('"');
                out.push_str(&field.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(field);
            }
        }
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
        r.iter().map(|row| row.iter().map(|s| s.to_string()).collect()).collect()
    }

    #[test]
    fn quoted_commas_quotes_and_line_breaks_survive_a_round_trip() {
        let data = rows(&[
            &["name", "note", "amount"],
            &["Smith, Jo", "said \"hi\"", "3"],
            &["multi", "line one\nline two", "4"],
            &["cr", "old\rmac", "5"],
            &["crlf", "a\r\nb", "6"],
            &["", "", ""],
        ]);
        let text = write(&data);
        assert_eq!(parse(&text).unwrap(), data);
    }

    #[test]
    fn excel_byte_order_mark_and_line_endings_are_read() {
        let text = "\u{feff}a,b\r\n1,2\r\n";
        assert_eq!(parse(text).unwrap(), rows(&[&["a", "b"], &["1", "2"]]));
        assert_eq!(parse("a,b\r1,2").unwrap(), rows(&[&["a", "b"], &["1", "2"]]));
        assert_eq!(parse("a,b\n1,2").unwrap(), rows(&[&["a", "b"], &["1", "2"]]));
    }

    #[test]
    fn a_blank_line_keeps_its_place() {
        assert_eq!(parse("a\n\nb\n").unwrap(), rows(&[&["a"], &[""], &["b"]]));
    }

    #[test]
    fn empty_text_is_no_rows_and_a_trailing_comma_is_an_empty_cell() {
        assert!(parse("").unwrap().is_empty());
        assert_eq!(parse("a,\n").unwrap(), rows(&[&["a", ""]]));
    }

    #[test]
    fn an_unclosed_quote_says_where_it_began() {
        let e = parse("a,b\n\"never\nclosed").unwrap_err();
        assert!(e.contains("line 2"), "{e}");
    }
}
