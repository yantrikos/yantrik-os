//! Calculator tool — evaluate mathematical expressions in-process.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(CalculateTool));
    reg.register(Box::new(UnitConvertTool));
}

// ── Calculate ──

pub struct CalculateTool;

impl Tool for CalculateTool {
    fn name(&self) -> &'static str { "calculate" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calculator" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "calculate",
                "description": "Evaluate a mathematical expression",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "Math expression over + - * / % ^, parentheses and decimals (e.g. '2^10', '3.14 * 5^2', '(6+7)/2')"
                        },
                        "precision": {
                            "type": "integer",
                            "description": "Decimal places (default: 4)"
                        }
                    },
                    "required": ["expression"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let expr = args.get("expression").and_then(|v| v.as_str()).unwrap_or_default();
        let precision = args.get("precision").and_then(|v| v.as_u64()).unwrap_or(4).min(20);

        if expr.is_empty() {
            return "Error: expression is required".to_string();
        }

        if expr.len() > 500 {
            return "Error: expression too long".to_string();
        }

        match eval_expr(expr, precision as usize) {
            Ok(value) => format!("{expr} = {value}"),
            Err(e) => format!("Error: {e}"),
        }
    }
}

/// Evaluate an arithmetic expression: + - * / % ^, parentheses, unary signs,
/// decimals. Replaces the `bc` subprocess (#543): the image does not ship bc,
/// and a shell tool for plain arithmetic is one more thing to sandbox.
/// Anything else — names, assignment, stray characters — is refused with a
/// short error a model can read.
fn eval_expr(src: &str, precision: usize) -> Result<String, String> {
    let mut p = Parser { src: src.as_bytes(), pos: 0, depth: 0 };
    let value = p.expr()?;
    p.skip_ws();
    if p.pos != p.src.len() {
        return Err(format!("unexpected character '{}'", p.src[p.pos] as char));
    }
    if !value.is_finite() {
        return Err("result is not finite".to_string());
    }
    Ok(clean_number(&format!("{:.*}", precision, value)))
}

/// Recursive-descent parser. ^ is right-associative and binds tighter than a
/// leading minus, as in ordinary notation: -2^2 is -4 (bc says 4), 2^-1 is 0.5.
struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    depth: u32,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    // Nesting depth is bounded so a 500-character "(" storm cannot overflow
    // the stack through the expr/unary recursion.
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > 64 {
            return Err("expression nested too deeply".to_string());
        }
        Ok(())
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Result<f64, String> {
        self.enter()?;
        let mut value = self.term()?;
        loop {
            self.skip_ws();
            if self.eat(b'+') {
                value += self.term()?;
            } else if self.eat(b'-') {
                value -= self.term()?;
            } else {
                self.depth -= 1;
                return Ok(value);
            }
        }
    }

    /// term := unary (('*' | '/' | '%') unary)*
    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        loop {
            self.skip_ws();
            if self.eat(b'*') {
                value *= self.unary()?;
            } else if self.eat(b'/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("division by zero".to_string());
                }
                value /= d;
            } else if self.eat(b'%') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err("division by zero".to_string());
                }
                value %= d;
            } else {
                return Ok(value);
            }
        }
    }

    /// unary := ('+' | '-') unary | power
    fn unary(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.peek() == Some(b'-') || self.peek() == Some(b'+') {
            self.enter()?;
            let neg = self.eat(b'-');
            let _ = self.eat(b'+');
            let value = self.unary()?;
            self.depth -= 1;
            return Ok(if neg { -value } else { value });
        }
        self.power()
    }

    /// power := atom ('^' unary)?
    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        self.skip_ws();
        if self.eat(b'^') {
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    /// atom := number | '(' expr ')'
    fn atom(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.eat(b'(') {
            let value = self.expr()?;
            self.skip_ws();
            if !self.eat(b')') {
                return Err("expected ')'".to_string());
            }
            return Ok(value);
        }
        self.number()
    }

    fn number(&mut self) -> Result<f64, String> {
        if let Some(c) = self.peek() {
            if c == b'_' || c.is_ascii_alphabetic() {
                return Err("names are not supported".to_string());
            }
        }
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9') | Some(b'.')) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err(match self.peek() {
                Some(c) => format!("unexpected character '{}'", c as char),
                None => "unexpected end of expression".to_string(),
            });
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
        text.parse::<f64>().map_err(|_| format!("invalid number '{text}'"))
    }
}

/// Remove trailing zeros from a decimal result.
fn clean_number(s: &str) -> String {
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "-" {
            "0".to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        s.to_string()
    }
}

// ── Unit Converter ──

pub struct UnitConvertTool;

impl Tool for UnitConvertTool {
    fn name(&self) -> &'static str { "unit_convert" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "calculator" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "unit_convert",
                "description": "Convert between units: temperature (C/F/K), distance",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "number", "description": "The numeric value to convert"},
                        "from": {"type": "string", "description": "Source unit (e.g. 'C', 'km', 'kg', 'GB')"},
                        "to": {"type": "string", "description": "Target unit (e.g. 'F', 'mi', 'lb', 'MB')"}
                    },
                    "required": ["value", "from", "to"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let value = args.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let from = args.get("from").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();
        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();

        if from.is_empty() || to.is_empty() {
            return "Error: from and to units are required".to_string();
        }

        let result = convert(value, &from, &to);
        match result {
            Some(r) => format!("{value} {from} = {:.4} {to}", r),
            None => format!("Error: cannot convert from '{from}' to '{to}'"),
        }
    }
}

fn convert(value: f64, from: &str, to: &str) -> Option<f64> {
    // Temperature
    match (from, to) {
        ("c", "f") => return Some(value * 9.0 / 5.0 + 32.0),
        ("f", "c") => return Some((value - 32.0) * 5.0 / 9.0),
        ("c", "k") => return Some(value + 273.15),
        ("k", "c") => return Some(value - 273.15),
        ("f", "k") => return Some((value - 32.0) * 5.0 / 9.0 + 273.15),
        ("k", "f") => return Some((value - 273.15) * 9.0 / 5.0 + 32.0),
        _ => {}
    }

    // Convert both to a base unit, then to target
    let to_base = |unit: &str| -> Option<(f64, &str)> {
        match unit {
            // Distance → meters
            "m" => Some((1.0, "distance")),
            "km" => Some((1000.0, "distance")),
            "mi" | "mile" | "miles" => Some((1609.344, "distance")),
            "ft" | "feet" | "foot" => Some((0.3048, "distance")),
            "in" | "inch" | "inches" => Some((0.0254, "distance")),
            "cm" => Some((0.01, "distance")),
            "mm" => Some((0.001, "distance")),
            "yd" | "yard" | "yards" => Some((0.9144, "distance")),
            // Weight → grams
            "g" | "gram" | "grams" => Some((1.0, "weight")),
            "kg" => Some((1000.0, "weight")),
            "lb" | "lbs" | "pound" | "pounds" => Some((453.592, "weight")),
            "oz" | "ounce" | "ounces" => Some((28.3495, "weight")),
            "mg" => Some((0.001, "weight")),
            "ton" | "tons" => Some((907185.0, "weight")),
            "tonne" | "tonnes" => Some((1_000_000.0, "weight")),
            // Data → bytes
            "b" | "byte" | "bytes" => Some((1.0, "data")),
            "kb" => Some((1024.0, "data")),
            "mb" => Some((1_048_576.0, "data")),
            "gb" => Some((1_073_741_824.0, "data")),
            "tb" => Some((1_099_511_627_776.0, "data")),
            // Time → seconds
            "s" | "sec" | "second" | "seconds" => Some((1.0, "time")),
            "min" | "minute" | "minutes" => Some((60.0, "time")),
            "h" | "hr" | "hour" | "hours" => Some((3600.0, "time")),
            "day" | "days" => Some((86400.0, "time")),
            "week" | "weeks" => Some((604800.0, "time")),
            _ => None,
        }
    };

    let (from_factor, from_type) = to_base(from)?;
    let (to_factor, to_type) = to_base(to)?;

    if from_type != to_type {
        return None;
    }

    Some(value * from_factor / to_factor)
}

#[cfg(test)]
mod tests {
    use super::eval_expr;

    fn eval(src: &str) -> String {
        eval_expr(src, 4).unwrap()
    }

    #[test]
    fn operator_precedence() {
        assert_eq!(eval("2+3*4"), "14");
        assert_eq!(eval("(2+3)*4"), "20");
        assert_eq!(eval("2+3*4^2"), "50");
        assert_eq!(eval("10-2-3"), "5");
        assert_eq!(eval("20/2/5"), "2");
    }

    #[test]
    fn power_is_right_associative() {
        assert_eq!(eval("2^3^2"), "512");
        assert_eq!(eval("2^-1"), "0.5");
    }

    #[test]
    fn unary_minus() {
        assert_eq!(eval("-2^2"), "-4");
        assert_eq!(eval("(-2)^2"), "4");
        assert_eq!(eval("-(3+4)"), "-7");
        assert_eq!(eval("--5"), "5");
        assert_eq!(eval("6*-7"), "-42");
    }

    #[test]
    fn decimals_and_modulo() {
        assert_eq!(eval("6*7"), "42");
        assert_eq!(eval("3.14*2"), "6.28");
        assert_eq!(eval(".5+.25"), "0.75");
        assert_eq!(eval("10%3"), "1");
        assert_eq!(eval("-7%3"), "-1");
    }

    #[test]
    fn division_by_zero() {
        assert_eq!(eval_expr("1/0", 4), Err("division by zero".to_string()));
        assert_eq!(eval_expr("5%(2-2)", 4), Err("division by zero".to_string()));
    }

    #[test]
    fn refuses_input_it_cannot_evaluate() {
        assert!(eval_expr("foo", 4).unwrap_err().contains("names"));
        assert!(eval_expr("x = 3", 4).unwrap_err().contains("names"));
        assert!(eval_expr("sqrt(144)", 4).unwrap_err().contains("names"));
        assert!(eval_expr("2 +", 4).unwrap_err().contains("end of expression"));
        assert!(eval_expr("1;2", 4).is_err());
        assert!(eval_expr("1..2", 4).is_err());
        assert!(eval_expr("(1", 4).is_err());
        assert!(eval_expr("1)", 4).is_err());
        assert!(eval_expr("", 4).is_err());
        assert!(eval_expr(&"(".repeat(100), 4).unwrap_err().contains("nested too deeply"));
        assert!(eval_expr("10^308*10^308", 4).unwrap_err().contains("not finite"));
    }

    #[test]
    fn precision_and_trailing_zeros() {
        assert_eq!(eval_expr("1/3", 4).unwrap(), "0.3333");
        assert_eq!(eval_expr("1/3", 2).unwrap(), "0.33");
        assert_eq!(eval_expr("10", 2).unwrap(), "10");
    }
}
