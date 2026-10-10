//! Minimal CSV reading and writing for the spreadsheet MVP.
//!
//! Deliberately small: one dialect only. Fields are separated by commas, a field may be
//! wrapped in double quotes (so it can hold a comma), and a quote inside a quoted field is
//! escaped by doubling it. There are no multi-line quoted fields.

/// Split `text` into rows of fields.
///
/// Lines are split on `\n` (a trailing `\r` is dropped), each line is split on commas, and a
/// quoted field may contain a comma. A trailing empty line is skipped.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            // A blank line is not a row; the final newline of a file leaves one here.
            continue;
        }
        rows.push(parse_line(line));
    }
    rows
}

fn parse_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                // A doubled quote inside a quoted field is one literal quote.
                chars.next();
                field.push('"');
            }
            '"' => {
                in_quotes = !in_quotes;
            }
            ',' if !in_quotes => {
                fields.push(std::mem::take(&mut field));
            }
            _ => field.push(c),
        }
    }
    fields.push(field);
    fields
}

/// Render rows as CSV text: the inverse of [`parse_csv`].
///
/// A field is quoted only when it contains a comma, a quote or a newline; quotes inside are
/// doubled.
pub fn to_csv(rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            out.push('\n');
        }
        for (c, field) in row.iter().enumerate() {
            if c > 0 {
                out.push(',');
            }
            out.push_str(&quote_field(field));
        }
    }
    // A trailing newline, so `to_csv(parse_csv(x)) == x` for a file that ends in one.
    if !rows.is_empty() {
        out.push('\n');
    }
    out
}

fn quote_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        let mut quoted = String::with_capacity(field.len() + 2);
        quoted.push('"');
        for ch in field.chars() {
            if ch == '"' {
                quoted.push('"');
            }
            quoted.push(ch);
        }
        quoted.push('"');
        quoted
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_quoted_comma_doubled_quote_and_trailing_newline() {
        let input = "a,\"b,c\",\"say \"\"hi\"\"\"\n1,2,3\n";
        let parsed = parse_csv(input);
        assert_eq!(
            parsed,
            vec![
                vec!["a".to_string(), "b,c".to_string(), "say \"hi\"".to_string()],
                vec!["1".to_string(), "2".to_string(), "3".to_string()],
            ]
        );
        assert_eq!(to_csv(&parsed), input);
    }
}
