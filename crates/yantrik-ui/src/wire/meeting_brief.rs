//! Meeting briefs on the desktop: the card the OS prepares ten minutes before a meeting, its
//! toast, its buttons and what `describe shell` says about it.
//!
//! The companion's worker makes the briefs on its 60 second think tick, off the UI thread, and
//! hands them to [`publish`]; this file draws what was published. Nothing here gathers anything.
//! Every Open goes through `for_you::carry_out`, the opener the notice inbox already has, as the
//! person: the shell opens the item when a pointer presses the row's button, and a mind is never
//! asked (design/calendar-heads-up-2026-10-16.md, design/initiative-inbox-2026-10-16.md).
//!
//! The one interrupt is not drawn here: [`super::meeting_brief_notice`] tells the notifications
//! service a brief is ready, and the service grants the toast like any calendar notice.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Mutex;

use slint::{ComponentHandle, ModelRc, VecModel};
use yantrik_companion::meeting_brief::{Card, Open, Outcome, Outcomes, Press, Row};
use yantrik_ipc_contracts::notifications::notice::{ActionKind, Initiative, NoticeAction, MOMENT_MEETING_BRIEF};

use crate::app_context::AppContext;
use crate::notice_inbox::{self, Opening};
use crate::{App, BriefLineData, MeetingBriefData, MeetingBriefState};

/// How many cards are drawn at once. The rest wait their turn.
const SHOWN: usize = 2;

/// What the worker last published.
#[derive(Default)]
struct Published {
    cards: Vec<Card>,
    outcomes: Outcomes,
}

static PUBLISHED: Mutex<Published> = Mutex::new(Published { cards: Vec::new(), outcomes: Outcomes { opened: 0, dismissed: 0, ignored: 0 } });

/// Cards the person dismissed here, kept until the worker's answers stop holding them. The worker
/// records a dismissal in its own time, behind whatever it is doing, so an answer made before it
/// did would draw the card again.
static DISMISSED: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    /// What the stack last drew, so an unchanged second does not rebuild it.
    static DRAWN: RefCell<Vec<CardView>> = const { RefCell::new(Vec::new()) };
    /// A line under a card: what a button that could not open something says.
    static NOTES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Where an opened card's press is recorded. Set by [`wire`], read where the opener answers.
    static PRESSES: RefCell<Option<std::sync::Arc<crate::bridge::CompanionBridge>>> = const { RefCell::new(None) };
}

fn published() -> std::sync::MutexGuard<'static, Published> {
    PUBLISHED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether the person has meeting briefs on. Read by the worker before it gathers anything.
/// When the store has not said (just started, or its service is down) they are off: nothing is
/// read until the person's choice is known.
pub fn enabled() -> bool {
    enabled_by(crate::notice_desk::initiative_known().as_ref())
}

/// [`enabled`] for the settings as the store last gave them.
fn enabled_by(settings: Option<&Initiative>) -> bool {
    settings.is_some_and(|s| s.moment_on(MOMENT_MEETING_BRIEF))
}

/// The worker's answer for this tick: every live brief, and the ones just made. A brief just made
/// is announced to the notifications service once, unless the person already dismissed it.
pub fn publish(outcome: Outcome) {
    let mut dismissed = DISMISSED.lock().unwrap_or_else(|e| e.into_inner());
    // An answer that no longer holds a dismissed card is the worker having caught up.
    dismissed.retain(|id| outcome.cards.iter().any(|c| &c.id == id));
    for id in &outcome.fresh {
        if let Some(card) = outcome.cards.iter().find(|c| &c.id == id && !dismissed.contains(&c.id)) {
            super::meeting_brief_notice::announce(card);
        }
    }
    let mut held = published();
    held.cards = outcome.cards.into_iter().filter(|c| !dismissed.contains(&c.id)).collect();
    held.outcomes = outcome.outcomes;
}

/// Take a card off the screen at once, and keep it off until the worker has recorded the dismissal.
fn forget(id: &str) {
    let mut dismissed = DISMISSED.lock().unwrap_or_else(|e| e.into_inner());
    if !dismissed.iter().any(|d| d == id) {
        dismissed.push(id.to_string());
    }
    published().cards.retain(|c| c.id != id);
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
    /// What its Open button opens, for the button's fixed label.
    pub kind: String,
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

/// What a button opens, in a word, for the button's fixed label.
fn kind_of(open: &Open) -> &'static str {
    match open {
        Open::Message { .. } => "email",
        Open::Note { .. } => "note",
        Open::File { .. } => "file",
        Open::Event { .. } => "event",
    }
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
                // What the button opens, whatever section the row is in: a promise opens the
                // email or the event it was found in.
                kind: r.open.as_ref().map(kind_of).unwrap_or_default().to_string(),
            });
            at += 1;
        }
    };
    section("LAST EMAILS WITH THEM", &b.emails);
    section("YOUR NOTES", &b.notes);
    section("PROMISES", &b.promises);
    section("FILES", &b.files);
    for (i, line) in b.description.iter().enumerate() {
        lines.push(LineView { heading: if i == 0 { "ABOUT".to_string() } else { String::new() }, text: line.clone(), row: -1, kind: String::new() });
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
        .map(|l| BriefLineData { heading: l.heading.as_str().into(), text: l.text.as_str().into(), row: l.row, kind: l.kind.as_str().into() })
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
        Open::Message { account, folder, id } => Ok(Opening::Message { account: account.clone(), folder: folder.clone(), id: id.clone() }),
        Open::Note { id } => notice_inbox::plan(&action(ActionKind::OpenNote, id), home),
        Open::File { path } => notice_inbox::plan(&action(ActionKind::OpenFile, path), home),
        Open::Event { id } => notice_inbox::plan(&action(ActionKind::OpenEvent, id), home),
    }
}

/// What `describe shell` says: whether the switch is on, per card its title, its start and how
/// many lines each section has, and how the briefs of the last 30 days ended, as three counts.
/// Never a subject, a note title, a file name or a promise: a mind can read describe, and what a
/// brief holds is the person's.
pub fn describe_value(cards: &[Card], enabled: bool, now: i64, outcomes: Outcomes) -> serde_json::Value {
    let shown = live(cards, now, enabled);
    serde_json::json!({
        "enabled": enabled,
        "outcomes": { "opened": outcomes.opened, "dismissed": outcomes.dismissed, "ignored": outcomes.ignored },
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
    let held = published();
    describe_value(&held.cards, enabled(), now_unix(), held.outcomes)
}

pub fn wire(ui: &App, ctx: &AppContext) {
    let state = ui.global::<MeetingBriefState>();

    let bridge = ctx.bridge.clone();
    let weak = ui.as_weak();
    state.on_dismiss(move |id| {
        forget(id.as_str());
        // The table is the companion's: it records the press, and it holds across a restart.
        bridge.meeting_brief_press(id.to_string(), Press::Dismissed);
        if let Some(ui) = weak.upgrade() {
            tick(&ui);
        }
    });

    // The card is "opened" when something opened, which only the opener knows: `say` is told.
    PRESSES.with(|p| *p.borrow_mut() = Some(ctx.bridge.clone()));
    let weak = ui.as_weak();
    state.on_open_row(move |id, row| {
        if let Some(ui) = weak.upgrade() {
            open_row(&ui, id.as_str(), row);
        }
    });

    let weak = ui.as_weak();
    state.on_open_event(move |id| {
        if let Some(ui) = weak.upgrade() {
            open_event(&ui, id.as_str());
        }
    });
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default()
}

fn card_by_id(id: &str) -> Option<Card> {
    published().cards.iter().find(|c| c.id == id).cloned()
}

/// What an opener's answer on `key` (the card's id, then `#` and the row) means for the card:
/// empty words are something having opened, which is the card's "opened" outcome, recorded
/// through `record`. A row that ends "Not opened: ..." is not.
fn answered(key: &str, text: &str, record: impl FnOnce(String, Press)) {
    if text.is_empty() {
        record(key.split('#').next().unwrap_or(key).to_string(), Press::Opened);
    }
}

/// A line under the card a button was pressed on, and the card's outcome if it opened.
fn say(ui: &App, key: &str, text: &str) {
    answered(key, text, |id, press| {
        PRESSES.with(|p| {
            if let Some(bridge) = p.borrow().as_ref() {
                bridge.meeting_brief_press(id, press);
            }
        })
    });
    let card = key.split('#').next().unwrap_or(key).to_string();
    NOTES.with(|n| n.borrow_mut().insert(card, text.to_string()));
    DRAWN.with(|d| d.borrow_mut().clear());
    tick(ui);
}

/// Press a row's Open. Nothing happens on a locked screen, for a card that has gone, or for a row
/// with nothing to open. The answer comes to [`say`].
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

/// Once a second, after the notification poll: draw the cards. Reads what the worker published;
/// gathers nothing, and raises nothing: the toast is the notifications service's to grant.
pub fn tick(ui: &App) {
    let now = now_unix();
    let cards = live(&published().cards, now, enabled());

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_companion::meeting_brief::{Brief, Row};

    fn row(text: &str, open: Option<Open>) -> Row {
        Row { text: text.into(), open }
    }

    fn message(id: &str) -> Open {
        Open::Message { account: "me@home.test".into(), folder: "Sent".into(), id: id.into() }
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
                complete: true,
                missing_since: None,
                emails: vec![row("Maya \u{b7} SECRET-SUBJECT \u{b7} 2 d", Some(message("m-1")))],
                notes: vec![row("SECRET-NOTE-TITLE", Some(Open::Note { id: "note-1.md".into() }))],
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
    fn a_buttons_label_names_what_it_opens_not_the_section_it_is_in() {
        let mut c = card(1_000, 5_000);
        c.brief.promises = vec![
            row("You told Ravi: send the numbers", Some(message("m-2"))),
            row("Maya said: share the deck", Some(Open::Event { id: "e9".into() })),
            row("by hand", None),
        ];
        let kinds: Vec<String> = card_view(&c, 0, "").lines.into_iter().map(|l| l.kind).collect();
        assert_eq!(&kinds[..4], ["email", "note", "email", "event"], "the first section's rows and the promises' own");
        assert_eq!(kinds[4], "", "a row with nothing to open has no label");
    }

    #[test]
    fn a_rows_number_names_the_row_its_button_opens() {
        let c = card(1_000, 5_000);
        let all = rows(&c);
        assert_eq!(all.len(), 4);
        assert_eq!(all[1].open, Some(Open::Note { id: "note-1.md".into() }));
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
        let value = describe_value(&[card(1_000, 5_000)], true, 900, Outcomes { opened: 2, dismissed: 1, ignored: 4 });
        let text = value.to_string();
        assert!(!text.contains("SECRET"), "describe leaked a brief's contents: {text}");
        assert_eq!(value["enabled"], true);
        assert_eq!(value["cards"][0]["title"], "Orion launch");
        assert_eq!(value["cards"][0]["counts"], serde_json::json!({ "emails": 1, "notes": 1, "promises": 2, "files": 0, "description": 1 }));
        assert!(value["cards"][0]["start"].as_str().is_some_and(|s| s.len() == 19));
        assert_eq!(value["outcomes"], serde_json::json!({ "opened": 2, "dismissed": 1, "ignored": 4 }), "three counts and nothing else");
        assert_eq!(describe_value(&[card(1_000, 5_000)], false, 900, Outcomes::default())["cards"], serde_json::json!([]));
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
        assert!(matches!(opening(&Open::Note { id: "note-1.md".into() }, &home), Ok(Opening::Note(_))));
        assert!(matches!(opening(&Open::Event { id: "e".into() }, &home), Ok(Opening::Event(_))));
        // The exact message, from the folder it is in: a subject names nothing here.
        assert_eq!(
            opening(&message("m-1"), &home),
            Ok(Opening::Message { account: "me@home.test".into(), folder: "Sent".into(), id: "m-1".into() })
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_dismissed_card_is_not_drawn_again_by_an_answer_made_before_the_worker_recorded_it() {
        let c = card(2_000, 6_000);
        let answer = |cards: Vec<Card>| Outcome { cards, fresh: Vec::new(), outcomes: Outcomes::default() };
        publish(answer(vec![c.clone()]));
        assert_eq!(published().cards.len(), 1);
        forget(&c.id);
        assert!(published().cards.is_empty());
        // The worker was busy with a turn and answers from before the dismissal: still not drawn.
        publish(answer(vec![c.clone()]));
        assert!(published().cards.is_empty(), "the card came back");
        // Once it has caught up and its answer no longer holds the card, the note is dropped.
        publish(answer(vec![]));
        assert!(DISMISSED.lock().unwrap().iter().all(|d| *d != c.id));
    }

    #[test]
    fn a_card_is_opened_when_something_opened_and_not_when_its_row_says_it_was_not() {
        let key = "meeting:e1:2000#1";
        let mut recorded = Vec::new();
        answered(key, "Not opened: no message with that id.", |id, press| recorded.push((id, press)));
        answered(key, "Not opened: the app did not accept it.", |id, press| recorded.push((id, press)));
        assert!(recorded.is_empty(), "a press that opened nothing is not an outcome");
        answered(key, "", |id, press| recorded.push((id, press)));
        answered("meeting:e1:2000#event", "", |id, press| recorded.push((id, press)));
        assert_eq!(recorded, [("meeting:e1:2000".to_string(), Press::Opened), ("meeting:e1:2000".to_string(), Press::Opened)]);
    }

    #[test]
    fn the_switch_is_off_until_the_store_has_said_it_is_on() {
        assert!(!enabled_by(None), "unknown is off: nothing is read until the person's choice is known");
        assert!(enabled_by(Some(&Initiative::default())));
        let mut off = Initiative::default();
        off.set_moment(MOMENT_MEETING_BRIEF, false);
        assert!(!enabled_by(Some(&off)));
    }
}
