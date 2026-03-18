//! Weather tool — get_weather.
//! Uses wttr.in (free, no API key needed).

use super::{Tool, ToolContext, ToolRegistry, PermissionLevel};

pub fn register(reg: &mut ToolRegistry) {
    reg.register(Box::new(GetWeatherTool));
}

// ── Get Weather ──

pub struct GetWeatherTool;

impl Tool for GetWeatherTool {
    fn name(&self) -> &'static str { "get_weather" }
    fn permission(&self) -> PermissionLevel { PermissionLevel::Safe }
    fn category(&self) -> &'static str { "weather" }

    fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "get_weather",
                "description": "Get current weather and forecast for a location",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "City name or coordinates (e.g. 'London', 'New York', '48.8566,2.3522'). Leave empty for auto-detected location."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["brief", "full"],
                            "description": "brief = current conditions only, full = 3-day forecast (default: brief)"
                        }
                    }
                }
            }
        })
    }

    fn execute(&self, _ctx: &ToolContext, args: &serde_json::Value) -> String {
        let location = args.get("location").and_then(|v| v.as_str()).unwrap_or("");
        let format = args.get("format").and_then(|v| v.as_str()).unwrap_or("brief");

        // Validate location — no shell metacharacters
        if location.contains(|c: char| c == '`' || c == '$' || c == ';' || c == '|' || c == '&' || c == '<' || c == '>') {
            return "Error: invalid location characters".to_string();
        }

        let url = if format == "full" {
            if location.is_empty() {
                "https://wttr.in/?format=v2".to_string()
            } else {
                format!("https://wttr.in/{}?format=v2", location.replace(' ', "+"))
            }
        } else {
            // Brief: one-line format
            // Format: location, condition, temp, humidity, wind
            let fmt = "%l:+%C+%t+humidity:%h+wind:%w";
            if location.is_empty() {
                format!("https://wttr.in/?format={}", fmt)
            } else {
                format!("https://wttr.in/{}?format={}", location.replace(' ', "+"), fmt)
            }
        };

        match std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "10", &url])
            .env("LANG", "en_US.UTF-8")
            .output()
        {
            Ok(o) if o.status.success() => {
                let text = String::from_utf8_lossy(&o.stdout);
                if text.trim().is_empty() {
                    "Could not fetch weather data.".to_string()
                } else if text.len() > 3000 {
                    format!("{}", &text[..text.floor_char_boundary(3000)])
                } else {
                    text.to_string()
                }
            }
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr);
                format!("Weather fetch failed: {}", err.trim())
            }
            Err(e) => format!("Error (curl not available?): {e}"),
        }
    }
}
