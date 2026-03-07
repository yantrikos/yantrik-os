//! Time tools — date_calc, timer.

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(DateCalcTool));
    reg.register(Box::new(TimerTool));
}

// ── Date Calc ──

pub struct DateCalcTool;

impl Tool for DateCalcTool {
    fn name(&self) -> &'static str { "date_calc" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "time" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "date_calc",
                "description": "Add or subtract days from a date. Returns the resulting date.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "date": {"type": "string", "description": "Start date (YYYY-MM-DD). Use 'today' for current date."},
                        "days": {"type": "integer", "description": "Days to add (negative to subtract)"}
                    },
                    "required": ["date", "days"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let date_str = args.get("date").and_then(|v| v.as_str()).unwrap_or_default();
        let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(0);

        if date_str.is_empty() {
            return "Error: date is required".to_string();
        }

        let base_date = if date_str == "today" {
            chrono::Local::now().date_naive()
        } else {
            match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => d,
                Err(_) => return format!("Error: invalid date format '{date_str}', use YYYY-MM-DD"),
            }
        };

        let result = base_date + chrono::Duration::days(days);
        let day_name = result.format("%A").to_string();

        if days >= 0 {
            format!("{date_str} + {days} days = {} ({})", result, day_name)
        } else {
            format!("{date_str} - {} days = {} ({})", -days, result, day_name)
        }
    }
}

// ── Timer ──

pub struct TimerTool;

impl Tool for TimerTool {
    fn name(&self) -> &'static str { "timer" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Standard }
    fn category(&self) -> &'static str { "time" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "timer",
                "description": "Set a countdown timer. Sends a notification when done.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "seconds": {"type": "integer", "description": "Countdown in seconds (max 3600)"},
                        "label": {"type": "string", "description": "Timer label (shown in notification)"}
                    },
                    "required": ["seconds"]
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let seconds = args.get("seconds").and_then(|v| v.as_u64()).unwrap_or(0);
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("Timer");

        if seconds == 0 {
            return "Error: seconds must be > 0".to_string();
        }
        if seconds > 3600 {
            return "Error: max timer duration is 3600 seconds (1 hour)".to_string();
        }

        let label_owned = label.to_string();
        let secs = seconds;

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            let _ = std::process::Command::new("notify-send")
                .args(["-u", "normal", "Timer Complete", &label_owned])
                .spawn();
        });

        let display = if seconds >= 60 {
            format!("{}m {}s", seconds / 60, seconds % 60)
        } else {
            format!("{seconds}s")
        };
        format!("Timer set: {display} — \"{label}\"")
    }
}
