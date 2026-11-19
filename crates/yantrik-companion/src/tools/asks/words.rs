//! Plans in words, for the cards that ask before work is planned (#1311): a schedule's timing, a
//! recipe's trigger and each of its steps. What the card says is what the thing will do, in the
//! plain sentence a person reads, and nothing is left out: a step that has no special wording
//! is shown as its own definition, whole.

use crate::recipe::{RecipeStep, TriggerType};

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn number(field: &str, max: u32) -> Option<u32> {
    field.parse::<u32>().ok().filter(|n| *n <= max)
}

/// A five-field cron expression as a sentence. Daily, weekly, monthly and yearly forms with a
/// fixed time read naturally; anything else is spelled out field by field, so it is never
/// summarised into something it is not.
pub fn cron_words(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [minute, hour, day, month, weekday] = fields[..] else {
        return format!("on the schedule `{expr}`");
    };
    let at = match (number(hour, 23), number(minute, 59)) {
        (Some(h), Some(m)) => Some(format!("{h:02}:{m:02}")),
        _ => None,
    };
    if let Some(at) = at {
        match (day, month, weekday) {
            ("*", "*", "*") => return format!("every day at {at}"),
            ("*", "*", w) => {
                let names: Option<Vec<&str>> = w.split(',').map(|d| number(d, 7).map(|d| DAYS[(d % 7) as usize])).collect();
                if let Some(names) = names {
                    return format!("every {} at {at}", names.join(" and "));
                }
            }
            (d, "*", "*") => {
                if let Some(d) = number(d, 31) {
                    return format!("on day {d} of every month at {at}");
                }
            }
            (d, m, "*") => {
                if let (Some(d), Some(m)) = (number(d, 31), number(m, 12)) {
                    if m >= 1 {
                        return format!("every year on {d} {} at {at}", MONTHS[(m - 1) as usize]);
                    }
                }
            }
            _ => {}
        }
    }
    let any = |f: &str| if f == "*" { "any".to_string() } else { f.to_string() };
    format!(
        "when the minute is {}, the hour {}, the day of the month {}, the month {} and the weekday {}",
        any(minute),
        any(hour),
        any(day),
        any(month),
        any(weekday)
    )
}

/// A number of seconds as "every 1 hour", "every 30 minutes".
pub fn interval_words(secs: i64) -> String {
    let (n, unit) = match secs {
        s if s % 86_400 == 0 => (s / 86_400, "day"),
        s if s % 3_600 == 0 => (s / 3_600, "hour"),
        s if s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    format!("every {n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A moment, as the scheduler's UTC clock reads it.
pub fn moment(ts: f64) -> String {
    chrono::DateTime::from_timestamp(ts as i64, 0)
        .map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| "an unreadable time".to_string())
}

/// A recipe's trigger in words.
pub fn trigger_words(trigger: Option<&TriggerType>) -> String {
    match trigger {
        None | Some(TriggerType::Manual) => "only when it is started by hand".to_string(),
        Some(TriggerType::Cron { expression }) => cron_words(expression),
        Some(TriggerType::Event { event_type, filter }) => match filter {
            Some(f) => format!("when `{event_type}` happens and matches {f}"),
            None => format!("when `{event_type}` happens"),
        },
        Some(TriggerType::RecipeComplete { recipe_id }) => format!("after recipe {recipe_id} completes"),
    }
}

/// One recipe step in words. The common kinds read as sentences; every other kind is its own
/// definition, compactly, so nothing about it is hidden.
pub fn step_words(step: &RecipeStep) -> String {
    match step {
        RecipeStep::Tool { tool_name, args, .. } => format!("run {tool_name} with {args}"),
        RecipeStep::Think { prompt, .. } => format!("ask the model: {prompt}"),
        RecipeStep::Notify { message } => format!("tell you: {message}"),
        RecipeStep::AskUser { question, .. } => format!("ask you: {question}"),
        other => format!("step {}", serde_json::to_string(other).unwrap_or_else(|_| "that could not be read".to_string())),
    }
}
