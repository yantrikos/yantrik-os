//! The parity pin: the action table and `describe` exactly as Calendar published them before the
//! framework. The table below was extracted mechanically from the old `apps/calendar/src/main.rs`
//! (`publish_control`, `update_door_args`, `events_between_action`), with the string literals
//! evaluated as rustc reads them, not copied by hand from the new intents, so a word changed in
//! `intents/` fails here.

use serde_json::{json, Value};
use yantrik_app::app::App as _;
use yantrik_app::intents::Intents;

use super::{a_week, open};
use crate::Calendar;

struct Golden {
    name: &'static str,
    risk: &'static str,
    description: &'static str,
    /// Name, JSON type, required, description.
    params: &'static [(&'static str, &'static str, bool, &'static str)],
}

const BEFORE: &[Golden] = &[
    Golden {
        name: "select_day",
        risk: "standard",
        description: r#"Show what is on one day of the month shown"#,
        params: &[
            ("day", "integer", true, r#"Day of the month, 1-31"#),
        ],
    },
    Golden {
        name: "show_date",
        risk: "standard",
        description: r#"Show what is on one day, in whatever month it is"#,
        params: &[
            ("date", "string", true, r#"YYYY-MM-DD"#),
        ],
    },
    Golden {
        name: "events_between",
        risk: "safe",
        description: r#"List the events from one date to another, inclusive, up to 62 days, read from the calendar store whatever the window is showing. Each has date, start, end, title and location"#,
        params: &[
            ("from", "string", true, r#"First day, YYYY-MM-DD"#),
            ("to", "string", true, r#"Last day, YYYY-MM-DD, at most 62 days after `from`"#),
        ],
    },
    Golden {
        name: "show_month",
        risk: "standard",
        description: r#"Move to the next or previous month"#,
        params: &[
            ("direction", "string", true, r#"next | previous"#),
        ],
    },
    Golden {
        name: "go_to_today",
        risk: "standard",
        description: r#"Jump back to the current month and day"#,
        params: &[],
    },
    Golden {
        name: "add_event",
        risk: "standard",
        description: r#"Put something on the calendar"#,
        params: &[
            ("title", "string", true, r#""#),
            ("date", "string", true, r#"YYYY-MM-DD"#),
            ("time", "string", false, r#"HH:MM, 24-hour; needed unless `all_day` is set"#),
            ("notes", "string", false, r#""#),
            ("duration_min", "integer", false, r#"How long it runs, in minutes; an hour when not given"#),
            ("all_day", "boolean", false, r#"A whole day rather than a time; `time` and `duration_min` are not used with it"#),
            ("reminder_minutes", "integer", false, r#"How many minutes before it starts to announce it; ten when not given. All-day events are not announced"#),
        ],
    },
    Golden {
        name: "delete_event",
        risk: "sensitive",
        description: r#"Take an event off the calendar. It is not recoverable"#,
        params: &[
            ("id", "string", false, r#"The id the store gave the event — `add_event` answers with it and `describe` lists it for every event it shows"#),
            ("title", "string", false, r#"The event's exact title, given with `date`, when the id is not known"#),
            ("date", "string", false, r#"YYYY-MM-DD, given with `title`"#),
        ],
    },
    Golden {
        name: "delete_own_event",
        risk: "standard",
        description: r#"Take an event this caller created itself off the calendar. Anything else is `delete_event`, which asks a person first"#,
        params: &[
            ("id", "string", false, r#"The id the store gave the event — `add_event` answers with it and `describe` lists it, marked with whether this caller may delete it unasked"#),
            ("title", "string", false, r#"The event's exact title, given with `date`, when the id is not known"#),
            ("date", "string", false, r#"YYYY-MM-DD, given with `title`"#),
        ],
    },
    Golden {
        name: "update_event",
        risk: "sensitive",
        description: r#"Move or rename any event that is already on the calendar"#,
        params: &[
            ("id", "string", true, r#"The id the store gave the event"#),
            ("title", "string", false, r#"A new title"#),
            ("date", "string", false, r#"Move it to this day, YYYY-MM-DD"#),
            ("time", "string", false, r#"Move it to this time, HH:MM"#),
            ("duration_min", "integer", false, r#"How long it runs, in minutes; unchanged when not given"#),
            ("notes", "string", false, r#""#),
            ("reminder_minutes", "integer", false, r#"How many minutes before it starts to announce it; unchanged when not given"#),
        ],
    },
    Golden {
        name: "update_own_event",
        risk: "standard",
        description: r#"Move or rename an event this caller created itself. Anything else is `update_event`, which asks a person first"#,
        params: &[
            ("id", "string", true, r#"The id the store gave the event"#),
            ("title", "string", false, r#"A new title"#),
            ("date", "string", false, r#"Move it to this day, YYYY-MM-DD"#),
            ("time", "string", false, r#"Move it to this time, HH:MM"#),
            ("duration_min", "integer", false, r#"How long it runs, in minutes; unchanged when not given"#),
            ("notes", "string", false, r#""#),
            ("reminder_minutes", "integer", false, r#"How many minutes before it starts to announce it; unchanged when not given"#),
        ],
    },
    Golden {
        name: "set_view",
        risk: "standard",
        description: r#"Switch between the month, week and day views"#,
        params: &[
            ("view", "string", true, r#"month | week | day"#),
        ],
    },
];


/// The keys of `describe`'s state in every view, and the extra ones the week and the day add.
const KEYS: &[&str] = &[
    "month",
    "year",
    "month_number",
    "selected_day",
    "today",
    "view",
    "week_starts_on",
    "events_on_selected_day",
    "naming",
    "days_with_events",
    "events_this_month",
    "notice",
    "you",
];
const WEEK_KEYS: &[&str] = &["week_start", "week_end", "week"];
const DAY_KEYS: &[&str] = &["day_shown", "events_on_day_grid"];

#[test]
fn the_eleven_actions_are_published_as_they_were() {
    let mut table = Intents::<Calendar>::new();
    Calendar::intents(&mut table);
    let specs = table.specs();
    let names: Vec<&str> = specs.iter().map(|a| a.name.as_str()).collect();
    let before: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    assert_eq!(names, before, "the same actions, in the same order");
    for (spec, golden) in specs.iter().zip(BEFORE) {
        let name = golden.name;
        assert_eq!(spec.description, golden.description, "{name}: description");
        assert_eq!(spec.permission, golden.risk, "{name}: grade");
        assert!(!spec.deferred, "{name}: answered on return, as before");
        assert!(!spec.stateless && spec.expected_seconds.is_none() && spec.explainer.is_none(), "{name}");
        let params: Vec<(&str, &str, bool, &str)> = spec
            .params
            .iter()
            .map(|p| (p.name.as_str(), p.kind, p.required, p.description.as_str()))
            .collect();
        assert_eq!(params, golden.params, "{name}: parameters");
    }
}

/// What a caller sees in `describe`: the eleven, and the framework's `launch`, which every app on
/// the framework publishes. (`present`, the framework's other one, is added by the runner when it
/// serves the surface.)
#[test]
fn describe_lists_the_eleven_and_the_frameworks_launch() {
    let f = open(a_week());
    let actions = f.describe()["actions"].as_array().cloned().unwrap_or_default();
    let mut names: Vec<String> = actions.iter().filter_map(|a| a["name"].as_str().map(String::from)).collect();
    let mut expected: Vec<String> = BEFORE.iter().map(|g| g.name.to_string()).collect();
    expected.push("launch".to_string());
    names.sort();
    expected.sort();
    assert_eq!(names, expected);
    for golden in BEFORE {
        let action = actions.iter().find(|a| a["name"] == json!(golden.name)).expect("published");
        assert_eq!(action["permission"], json!(golden.risk), "{}", golden.name);
        assert_eq!(action["settles"], json!("on return"), "{}", golden.name);
    }
}

/// docs/app-control.md's grade table says what the app publishes, row for row.
#[test]
fn the_grade_table_in_the_guide_is_still_true() {
    let doc = include_str!("../../../../docs/app-control.md");
    let mut listed = Vec::new();
    // The grade table's rows; the service-methods table further down names `calendar.*` methods.
    for row in doc.lines().filter(|l| l.starts_with("| calendar | `") && !l.contains("`calendar.")) {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let grade = cells[3];
        for name in cells[2].split(',').map(|n| n.trim().trim_matches('`')) {
            let golden = BEFORE.iter().find(|g| g.name == name).unwrap_or_else(|| panic!("{name} is not an action"));
            assert_eq!(golden.risk, grade, "{name}");
            listed.push(name);
        }
    }
    let mut all: Vec<&str> = BEFORE.iter().map(|g| g.name).collect();
    // `show_date` was never in the guide's table; every other action is, with its grade.
    all.retain(|n| *n != "show_date");
    all.sort();
    listed.sort();
    assert_eq!(listed, all);
}

fn keys(state: &Value) -> Vec<String> {
    let mut keys: Vec<String> = state.as_object().expect("an object").keys().cloned().collect();
    keys.sort();
    keys
}

fn expected(extra: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = KEYS.iter().chain(extra).map(|k| k.to_string()).collect();
    keys.sort();
    keys
}

#[test]
fn the_month_is_described_as_it_was() {
    let f = open(a_week());
    let d = f.describe();
    assert_eq!(d["summary"], json!("Calendar — September 2026, 2 things on day 23"));
    let s = &d["state"];
    assert_eq!(keys(s), expected(&[]));
    assert_eq!(
        *s,
        json!({
            "month": "September 2026",
            "year": 2026,
            "month_number": 9,
            "selected_day": 23,
            "today": "2026-09-23 Wednesday",
            "view": "month",
            "week_starts_on": "sunday",
            "events_on_selected_day": [
                { "id": "ev-standup", "title": "Standup", "date": "2026-09-23", "time": "09:30 – 09:45",
                  "all_day": false, "may_delete_unasked": false },
                { "id": "ev-review", "title": "Design review", "date": "2026-09-23", "time": "14:00 – 15:30",
                  "all_day": false, "may_delete_unasked": false },
            ],
            "naming": {
                "ev-standup": "Standup, Wed 23 Sep 09:30",
                "ev-review": "Design review, Wed 23 Sep 14:00",
                "ev-dentist": "Dentist, Fri 25 Sep 13:00",
            },
            "days_with_events": [{ "day": 23, "events": 2 }, { "day": 25, "events": 1 }],
            "events_this_month": 3,
            "notice": "",
            "you": "could not be identified",
        })
    );
}

#[test]
fn the_week_is_described_as_it_was() {
    let f = open(a_week());
    f.act("set_view", json!({ "view": "week" })).unwrap();
    let d = f.describe();
    assert_eq!(d["summary"], json!("Calendar — week of 2026-09-20 to 2026-09-26, 3 events on the grid"));
    let s = &d["state"];
    assert_eq!(keys(s), expected(WEEK_KEYS));
    assert_eq!(s["view"], json!("week"));
    assert_eq!(s["week_start"], json!("2026-09-20"));
    assert_eq!(s["week_end"], json!("2026-09-26"));
    let week = s["week"].as_array().unwrap();
    let days: Vec<&str> = week.iter().map(|d| d["day"].as_str().unwrap()).collect();
    assert_eq!(days, ["Sun 20", "Mon 21", "Tue 22", "Wed 23", "Thu 24", "Fri 25", "Sat 26"]);
    assert_eq!(
        week[3]["events"],
        json!([
            { "id": "ev-standup", "title": "Standup", "at": "09:30", "minutes": 15, "may_delete_unasked": false },
            { "id": "ev-review", "title": "Design review", "at": "14:00", "minutes": 90, "may_delete_unasked": false },
        ])
    );
    assert_eq!(week[5]["events"][0]["at"], json!("13:00"));
    assert_eq!(week[0]["events"], json!([]));
}

#[test]
fn the_day_is_described_as_it_was() {
    let f = open(a_week());
    f.act("set_view", json!({ "view": "day" })).unwrap();
    let d = f.describe();
    assert_eq!(d["summary"], json!("Calendar — Wednesday, 23 September 2026, 2 events on the grid"));
    let s = &d["state"];
    assert_eq!(keys(s), expected(DAY_KEYS));
    assert_eq!(s["day_shown"], json!("Wednesday, 23 September 2026"));
    let grid: Vec<&str> =
        s["events_on_day_grid"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert_eq!(grid, ["ev-standup", "ev-review"]);
}

/// The summary in each month case: nothing on the day, one thing, several.
#[test]
fn the_month_summary_says_how_many_things_are_on_the_day() {
    let f = open(a_week());
    f.act("select_day", json!({ "day": 24 })).unwrap();
    assert_eq!(f.summary(), "Calendar — September 2026, nothing on day 24");
    f.act("select_day", json!({ "day": 25 })).unwrap();
    assert_eq!(f.summary(), "Calendar — September 2026, one thing on day 25: Dentist");
    f.act("select_day", json!({ "day": 23 })).unwrap();
    assert_eq!(f.summary(), "Calendar — September 2026, 2 things on day 23");
}
