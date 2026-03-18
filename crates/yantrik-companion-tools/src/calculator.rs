//! Calculator tool — evaluate mathematical expressions.
//! Uses `bc` with safety guards.

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
                            "description": "Math expression (e.g. '15% of 2347', '2^10', 'sqrt(144)', '3.14 * 5^2')"
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

        // Block dangerous characters (only allow math-safe chars)
        if expr.contains(|c: char| {
            !c.is_alphanumeric() && c != '+' && c != '-' && c != '*' && c != '/'
                && c != '^' && c != '(' && c != ')' && c != '.' && c != ' '
                && c != '%' && c != '_'
        }) {
            return "Error: expression contains invalid characters".to_string();
        }

        // Pre-process common patterns
        let processed = preprocess_expr(expr);

        // Feed to bc with scale
        let bc_input = format!("scale={}; {}", precision, processed);

        let mut child = match std::process::Command::new("bc")
            .arg("-l")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return format!("Error (bc not available?): {e}"),
        };

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(bc_input.as_bytes());
            let _ = stdin.write_all(b"\n");
        }

        match child.wait_with_output() {
            Ok(o) if o.status.success() => {
                let result = String::from_utf8_lossy(&o.stdout);
                let trimmed = result.trim();
                if trimmed.is_empty() {
                    let err = String::from_utf8_lossy(&o.stderr);
                    format!("Error: {}", err.trim())
                } else {
                    // Clean up trailing zeros
                    let cleaned = clean_number(trimmed);
                    format!("{expr} = {cleaned}")
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Calculation error: {}", err.trim())
            }
            Err(e) => format!("Error: {e}"),
        }
    }
}

/// Pre-process natural-language math into bc syntax.
fn preprocess_expr(expr: &str) -> String {
    let mut s = expr.to_lowercase();

    // "15% of 2347" → "2347 * 15 / 100"
    if s.contains("% of ") {
        let parts: Vec<&str> = s.split("% of ").collect();
        if parts.len() == 2 {
            let pct = parts[0].trim();
            let base = parts[1].trim();
            return format!("{base} * {pct} / 100");
        }
    }

    // "X% " at end → "* X / 100" doesn't apply cleanly, skip

    // sqrt(x) → bc uses sqrt()
    // ^ → bc uses ^ for power
    s = s.replace("**", "^");

    s
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
