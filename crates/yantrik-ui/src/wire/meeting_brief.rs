//! Meeting briefs on the desktop: the card the OS prepares ten minutes before a meeting, its
//! toast, its buttons and what `describe shell` says about it.
//!
//! The companion's worker makes the briefs on its 60 second think tick, off the UI thread, and
//! hands them to [`publish`]; this file draws what was published. Nothing here gathers anything.
//! Every Open goes through `for_you::carry_out`, the opener the notice inbox already has, as the
//! person: the shell opens the item when a pointer presses the row's button, and a mind is never
//! asked (design/calendar-heads-up-2026-10-16.md, design/initiative-inbox-2026-10-16.md).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_companion::meeting_brief::{Card, Open, Outcome, Row};
use yantrik_ipc_contracts::notifications::notice::{ActionKind, NoticeAction};

use super::notifications::SILENT_SCREENS;
use crate::app_context::AppContext;
use crate::notice_inbox::{self, Opening, Screen};
use crate::{App, BriefLineData, MeetingBriefData, MeetingBriefState, ToastData};

/// How many cards are drawn at once. The rest wait their turn.
const SHOWN: usize = 2;

/// What the worker last published, and the briefs it has not yet announced.
#[derive(Default)]
struct Published {
    cards: Vec<Card>,
    /// Ids made by the last ticks that no toast has been raised for yet.
    announce: Vec<String>,
}

static PUBLISHED: Mutex<Published> = Mutex::new(Published { cards: Vec::new(), announce: Vec::new() });

thread_local! {
    /// What the stack last drew, so an unchanged second does not rebuild it.
    static DRAWN: RefCell<Vec<CardView>> = const { RefCell::new(Vec::new()) };
    /// A line under a card: what a button that could not open something says.
    static NOTES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Briefs toasted by this shell.
    static TOASTED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn published() -> std::sync::MutexGuard<'static, Published> {
    PUBLISHED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the person has meeting briefs on. Read by the worker before it gathers anything.
pub fn enabled() -> bool {
    !crate::notice_desk::initiative().meeting_briefs_off
}

/// The worker's answer for this tick: every live brief, and the ones just made.
pub fn publish(outcome: Outcome) {
    let mut held = published();
    for id in outcome.fresh {
        if !held.announce.contains(&id) {
            held.announce.push(id);
        }
    }
    held.cards = outcome.cards;
}

/// Take a card off the screen at once; the worker records the dismissal.
fn forget(id: &str) {
    let mut held = published();
    held.cards.retain(|c| c.id != id);
    held.announce.retain(|a| a != id);
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The cards to show at `now`: none when the switch is off, and none once the meeting has ended.
fn live(cards: &[Card], now: i64, enabled: bool) -> Vec<Card> {
    if !enabled {
        return Vec::new();
    }
    cards.iter().filter(|c| c.brief.end > now).cloned().collect()
}

/// "in 9 min", "in 1 min", "now" once it has started.
pub fn in_label(start: i64, now: i64) -> String {
    let secs = start - now;
    if secs <= 0 {
        return "now".to_string();
    }
    let minutes = (secs + 59) / 60;
    if minutes >= 60 {
        return format!("in {} h {} min", minutes / 60, minutes % 60);
    }
    format!("in {minutes} min")
}

/// The rows of a brief that have a button, in the order the buttons number them.
fn rows(card: &Card) -> Vec<&Row> {
    let b = &card.brief;
    b.emails.iter().chain(&b.notes).chain(&b.promises).chain(&b.files).collect()
}

/// One line of a card: its section heading (on the first line of a section only), its words, and
/// the number its Open button hands back, or -1 for a line with nothing to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineView {
    pub heading: String,
    pub text: String,
    pub row: i32,
}

/// One card as the stack draws it, as plain data so an unchanged second is seen to be unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardView {
    pub id: String,
    pub title: String,
    pub when: String,
    pub place: String,
    pub people: String,
    pub lines: Vec<LineView>,
    pub note: String,
}

/// The card for `card` at `now`: sections with nothing in them are left out.
pub fn card_view(card: &Card, now: i64, note: &str) -> CardView {
    let b = &card.brief;
    let mut lines: Vec<LineView> = Vec::new();
    let mut at = 0i32;
    let mut section = |heading: &str, rows: &[Row]| {
        for (i, r) in rows.iter().enumerate() {
            lines.push(LineView {
                heading: if i == 0 { heading.to_string() } else { String::new() },
                text: r.text.clone(),
                row: if r.open.is_some() { at } else { -1 },
            });
            at += 1;
        }
    };
    section("LAST EMAILS WITH THEM", &b.emails);
    section("YOUR NOTES", &b.notes);
    section("PROMISES", &b.promises);
    section("FILES", &b.files);
    for (i, line) in b.description.iter().enumerate() {
        lines.push(LineView { heading: if i == 0 { "ABOUT".to_string() } else { String::new() }, text: line.clone(), row: -1 });
    }
    CardView {
        id: card.id.clone(),
        title: b.title.clone(),
        when: format!("{}, {}", b.when, in_label(b.start, now)),
        place: b.place.clone().unwrap_or_default(),
        people: if b.people.is_empty() { String::new() } else { format!("With {}", b.people.join(", ")) },
        lines,
        note: note.to_string(),
    }
}

fn slint_card(v: &CardView) -> MeetingBriefData {
    let lines: Vec<BriefLineData> = v
        .lines
        .iter()
        .map(|l| BriefLineData { heading: l.heading.as_str().into(), text: l.text.as_str().into(), row: l.row })
        .collect();
    MeetingBriefData {
        id: v.id.as_str().into(),
        title: v.title.as_str().into(),
        when: v.when.as_str().into(),
        place: v.place.as_str().into(),
        people: v.people.as_str().into(),
        lines: ModelRc::new(VecModel::from(lines)),
        note: v.note.as_str().into(),
    }
}

/// What a row's button opens, once the notice inbox's own check has passed it.
fn opening(open: &Open, home: &std::path::Path) -> Result<Opening, String> {
    let action = |kind: ActionKind, reference: &str| NoticeAction { label: "Open".into(), kind, reference: reference.into() };
    match open {
        Open::Message { subject } => Ok(Opening::Message(subject.clone())),
        Open::Note { title } => notice_inbox::plan(&action(ActionKind::OpenNote, title), home),
        Open::File { path } => notice_inbox::plan(&action(ActionKind::OpenFile, path), home),
        Open::Event { id } => notice_inbox::plan(&action(ActionKind::OpenEvent, id), home),
    }
}

/// What `describe shell` says: whether the switch is on, and per card its title, its start and
/// how many lines each section has. Never a subject, a note title, a file name or a promise: a
/// mind can read describe, and what a brief holds is the person's.
pub fn describe_value(cards: &[Card], enabled: bool, now: i64) -> serde_json::Value {
    let shown = live(cards, now, enabled);
    serde_json::json!({
        "enabled": enabled,
        "cards": shown.iter().map(|c| {
            let n = c.brief.counts();
            serde_json::json!({
                "title": c.brief.title,
                "start": chrono::DateTime::from_timestamp(c.brief.start, 0).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%dT%H:%M:%S").to_string()),
                "counts": { "emails": n.emails, "notes": n.notes, "promises": n.promises, "files": n.files, "description": n.description },
            })
        }).collect::<Vec<_>>(),
    })
}

pub fn describe() -> serde_json::Value {
    describe_value(&published().cards, enabled(), now_unix())
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let state = ui.global::<MeetingBriefState>();

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    state.on_dismiss(move |id| {
        forget(id.as_str());
        // The table is the companion's: the worker records it, and it holds across a restart.
        bridge.dismiss_meeting_brief(id.to_string());
        if let Some(ui) = weak.upgrade() {
            tick(&ui);
        }
    });

    let weak = ui.as_weak();
    state.on_open_row(move |id, row| {
        let Some(ui) = weak.upgrade() else { return };
        open_row(&ui, id.as_str(), row);
    });

    let weak = ui.as_weak();
    state.on_open_event(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        open_event(&ui, id.as_str());
    });
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default()
}

fn card_by_id(id: &str) -> Option<Card> {
    published().cards.iter().find(|c| c.id == id).cloned()
}

/// A line under the card a button was pressed on. `key` is the card's id, then `#` and the row.
fn say(ui: &App, key: &str, text: &str) {
    let card = key.split('#').next().unwrap_or(key).to_string();
    NOTES.with(|n| n.borrow_mut().insert(card, text.to_string()));
    DRAWN.with(|d| d.borrow_mut().clear());
    tick(ui);
}

fn open_row(ui: &App, id: &str, row: i32) {
    if crate::control::locked_screen(ui.get_current_screen()) {
        return;
    }
    let Some(card) = card_by_id(id) else { return };
    let Some(open) = usize::try_from(row).ok().and_then(|i| rows(&card).get(i).and_then(|r| r.open.clone())) else { return };
    let key = format!("{id}#{row}");
    match opening(&open, &home()) {
        Ok(opening) => super::for_you::carry_out(ui, &key, opening, say),
        Err(why) => say(ui, &key, &format!("Not opened: {why}.")),
    }
}

fn open_event(ui: &App, id: &str) {
    if crate::control::locked_screen(ui.get_current_screen()) {
        return;
    }
    let Some(card) = card_by_id(id) else { return };
    super::for_you::carry_out(ui, &format!("{id}#event"), Opening::Event(card.brief.event_id.clone()), say);
}

/// Once a second, after the notification poll: draw the cards, and toast a new one once. Reads
/// what the worker published; gathers nothing.
pub fn tick(ui: &App) {
    let now = now_unix();
    let on = enabled();
    let (cards, due) = {
        let held = published();
        (live(&held.cards, now, on), held.announce.clone())
    };

    let drawn: Vec<CardView> = cards
        .iter()
        .take(SHOWN)
        .map(|c| {
            let note = NOTES.with(|n| n.borrow().get(&c.id).cloned()).unwrap_or_default();
            card_view(c, now, &note)
        })
        .collect();
    if DRAWN.with(|d| *d.borrow() != drawn) {
        let model: Vec<MeetingBriefData> = drawn.iter().map(slint_card).collect();
        DRAWN.with(|d| *d.borrow_mut() = drawn);
        ui.global::<MeetingBriefState>().set_cards(ModelRc::new(VecModel::from(model)));
    }

    announce(ui, &cards, &due);
}

/// Toast each new brief once. One the screen does not allow (lock, do not disturb, focus) is held
/// and toasted when it does, while its meeting has not ended; the card itself is on the desktop
/// either way.
fn announce(ui: &App, cards: &[Card], due: &[String]) {
    if due.is_empty() {
        return;
    }
    let screen = Screen {
        silent: SILENT_SCREENS.contains(&ui.get_current_screen()),
        do_not_disturb: ui.get_dnd_mode(),
        focus: ui.get_focus_mode(),
        fullscreen: false,
        call: false,
    };
    let mut done: Vec<String> = Vec::new();
    for id in due {
        let live_card = cards.iter().find(|c| &c.id == id);
        match live_card {
            // Gone (ended, dismissed, or switched off): nothing to toast, and nothing to wait for.
            None => done.push(id.clone()),
            Some(_) if screen.holds() => {}
            Some(card) => {
                if TOASTED.with(|t| t.borrow_mut().insert(id.clone())) {
                    toast(ui, card);
                }
                done.push(id.clone());
            }
        }
    }
    if !done.is_empty() {
        published().announce.retain(|a| !done.contains(a));
    }
}

fn toast(ui: &App, card: &Card) {
    let b = &card.brief;
    let at = b.when.split('\u{2013}').next().unwrap_or("").to_string();
    super::toast::push(
        ui,
        ToastData {
            id: card.id.as_str().into(),
            app_name: "Meeting brief".into(),
            summary: format!("Your brief for {} at {at} is ready.", b.title).into(),
            body: "".into(),
            urgency: 1,
            icon_char: "B".into(),
            actions: ModelRc::new(VecModel::from(Vec::new())),
            thumbnail: Default::default(),
            has_thumbnail: false,
        },
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::meeting_brief::{Brief, Row};

    fn row(text: &str, open: Option<Open>) -> Row {
        Row { text: text.into(), open }
    }

    fn card(start: i64, end: i64) -> Card {
        Card {
            id: format!("meeting:e1:{start}"),
            brief: Brief {
                event_id: "e1".into(),
                title: "Orion launch".into(),
                start,
                end,
                when: "10:00\u{2013}10:30".into(),
                place: Some("Room 4".into()),
                people: vec!["Maya Chen".into(), "Ravi Rao".into()],
                fingerprint: "f".into(),
                emails: vec![row("Maya \u{b7} SECRET-SUBJECT \u{b7} 2 d", Some(Open::Message { subject: "SECRET-SUBJECT".into() }))],
                notes: vec![row("SECRET-NOTE-TITLE", Some(Open::Note { title: "SECRET-NOTE-TITLE".into() }))],
                promises: vec![row("You told Ravi: SECRET-PROMISE", None), row("Maya said: SECRET-PROMISE-2", None)],
                files: vec![],
                description: vec!["SECRET-DESCRIPTION".into()],
            },
        }
    }

    #[test]
    fn the_header_says_when_and_how_soon() {
        assert_eq!(in_label(1_000 + 9 * 60, 1_000), "in 9 min");
        assert_eq!(in_label(1_000 + 1, 1_000), "in 1 min");
        assert_eq!(in_label(1_000, 1_000), "now");
        assert_eq!(in_label(1_000, 2_000), "now");
        let data = card_view(&card(1_000 + 9 * 60, 5_000), 1_000, "");
        assert_eq!(data.when, "10:00\u{2013}10:30, in 9 min");
        assert_eq!(data.people, "With Maya Chen, Ravi Rao");
        assert_eq!(data.place, "Room 4");
    }

    #[test]
    fn a_section_with_nothing_in_it_is_left_out_and_headings_come_once() {
        let data = card_view(&card(1_000, 5_000), 0, "");
        let lines: Vec<(String, i32)> = data.lines.iter().map(|l| (l.heading.clone(), l.row)).collect();
        let headings: Vec<&str> = lines.iter().map(|(h, _)| h.as_str()).filter(|h| !h.is_empty()).collect();
        assert_eq!(headings, ["LAST EMAILS WITH THEM", "YOUR NOTES", "PROMISES", "ABOUT"], "no FILES section: it was empty");
        // Rows are numbered across sections; a promise made by hand and the description have no button.
        let rows_: Vec<i32> = lines.iter().map(|(_, r)| *r).collect();
        assert_eq!(rows_, [0, 1, -1, -1, -1]);
    }

    #[test]
    fn a_rows_number_names_the_row_its_button_opens() {
        let c = card(1_000, 5_000);
        let all = rows(&c);
        assert_eq!(all.len(), 4);
        assert_eq!(all[1].open, Some(Open::Note { title: "SECRET-NOTE-TITLE".into() }));
    }

    #[test]
    fn the_card_goes_when_the_meeting_ends_and_when_the_switch_is_off() {
        let cards = [card(1_000, 2_800)];
        assert_eq!(live(&cards, 2_799, true).len(), 1);
        assert!(live(&cards, 2_800, true).is_empty(), "the meeting ended");
        assert!(live(&cards, 1_000, false).is_empty(), "switched off");
    }

    #[test]
    fn describe_has_counts_and_never_contents() {
        let value = describe_value(&[card(1_000, 5_000)], true, 900);
        let text = value.to_string();
        assert!(!text.contains("SECRET"), "describe leaked a brief's contents: {text}");
        assert_eq!(value["enabled"], true);
        assert_eq!(value["cards"][0]["title"], "Orion launch");
        assert_eq!(value["cards"][0]["counts"], serde_json::json!({ "emails": 1, "notes": 1, "promises": 2, "files": 0, "description": 1 }));
        assert!(value["cards"][0]["start"].as_str().is_some_and(|s| s.len() == 19));
        assert_eq!(describe_value(&[card(1_000, 5_000)], false, 900)["cards"], serde_json::json!([]));
    }

    #[test]
    fn an_opening_goes_through_the_notice_inboxs_own_check() {
        let home = std::env::temp_dir().join(format!("meeting-brief-open-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("orion.md"), "x").unwrap();
        let file = std::fs::canonicalize(home.join("orion.md")).unwrap();
        let home = std::fs::canonicalize(&home).unwrap();
        assert!(matches!(opening(&Open::File { path: file.to_string_lossy().into() }, &home), Ok(Opening::File(_))));
        assert!(opening(&Open::File { path: "/etc/passwd".into() }, &home).is_err(), "not a file in the home folder");
        assert!(matches!(opening(&Open::Note { title: "t".into() }, &home), Ok(Opening::Note(_))));
        assert!(matches!(opening(&Open::Event { id: "e".into() }, &home), Ok(Opening::Event(_))));
        assert!(matches!(opening(&Open::Message { subject: "s".into() }, &home), Ok(Opening::Message(_))));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn publish_announces_a_brief_once_and_a_dismissal_takes_it_away() {
        let c = card(1_000, 5_000);
        publish(Outcome { cards: vec![c.clone()], fresh: vec![c.id.clone()] });
        publish(Outcome { cards: vec![c.clone()], fresh: vec![c.id.clone()] });
        assert_eq!(published().announce.iter().filter(|a| **a == c.id).count(), 1);
        forget(&c.id);
        assert!(published().cards.is_empty() && published().announce.is_empty());
    }
}
