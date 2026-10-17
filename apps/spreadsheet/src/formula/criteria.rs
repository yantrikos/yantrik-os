//! The condition COUNTIF and SUMIF test each cell against: `">5"`, `"<>0"`, `"apples"`.
//!
//! Ported from the old `matches_criteria` (5c6a55aa^), which already had the operator set; it
//! now takes worked-out values rather than display strings, so a blank cell is blank and not "".

use super::value::{parse_number, Value};

pub fn matches(cell: &Value, criteria: &Value) -> bool {
    let text = match criteria {
        Value::Number(n) => return cell_number(cell) == Some(*n),
        Value::Bool(b) => return matches!(cell, Value::Bool(c) if c == b),
        Value::Empty => return matches!(cell, Value::Empty),
        Value::Error(_) => return false,
        Value::Text(t) => t.trim().to_string(),
    };
    for op in [">=", "<=", "<>", ">", "<", "="] {
        let Some(rest) = text.strip_prefix(op) else { continue };
        let rest = rest.trim();
        if let (Some(want), Some(have)) = (parse_number(rest), cell_number(cell)) {
            return match op {
                ">=" => have >= want,
                "<=" => have <= want,
                "<>" => have != want,
                ">" => have > want,
                "<" => have < want,
                _ => have == want,
            };
        }
        let shown = cell_text(cell);
        return match op {
            "=" => shown.eq_ignore_ascii_case(rest),
            "<>" => !shown.eq_ignore_ascii_case(rest),
            _ => false,
        };
    }
    if let (Some(want), Some(have)) = (parse_number(&text), cell_number(cell)) {
        return want == have;
    }
    cell_text(cell).eq_ignore_ascii_case(&text)
}

fn cell_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Text(t) => t.clone(),
        Value::Empty => String::new(),
        other => other.display(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn comparisons_and_words() {
        assert!(matches(&Value::Number(6.0), &t(">5")));
        assert!(!matches(&Value::Number(5.0), &t(">5")));
        assert!(matches(&Value::Number(5.0), &t(">=5")));
        assert!(matches(&Value::Number(1.0), &t("<>0")));
        assert!(matches(&t("Apples"), &t("apples")));
        assert!(matches(&t("pears"), &t("<>apples")));
        assert!(matches(&Value::Number(3.0), &Value::Number(3.0)));
        assert!(matches(&Value::Number(3.0), &t("3")));
        assert!(!matches(&Value::Empty, &t(">0")));
    }
}
