//! One interrupt per meeting: the brief's notice and the event's own reminder, in both orders,
//! and the grant rules the brief is under (design/calendar-heads-up-2026-10-16.md: one code path,
//! no second grant).

use chrono::{Duration as ChronoDuration, Local, Timelike, Utc};
use yantrik_ipc_contracts::notifications::Notification;
use yantrik_ipc_contracts::notifications::notice::{Attention, Family, FamilySetting, GrantReason, Notice, QuietHours};

use super::*;
use crate::brief_notice::{announce, Ready};

fn dir() -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!("yantrik-brief-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("calendar")).unwrap();
    dir
}

/// 2099, so no card is past its `useful_until` on the clock the store expires by.
fn nine_am() -> DateTime<Utc> {
    Utc.from_utc_datetime(&parse_stamp("2099-03-18T09:00:00").unwrap())
}

fn meeting(start: DateTime<Utc>) -> CalendarEvent {
    let stamp = start.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string();
    CalendarEvent {
        id: "orion".into(),
        title: "Orion launch sync".into(),
        description: String::new(),
        start: stamp.clone(),
        end: stamp,
        is_all_day: false,
        location: None,
        attendees: Vec::new(),
        recurrence: None,
        calendar_id: "default".into(),
        remote_id: None,
        creator: None,
        reminder_minutes: DEFAULT_REMINDER_MINUTES,
    }
}

struct Room {
    store: Store,
    root: PathBuf,
    cal: PathBuf,
    sent: Sent,
    start: DateTime<Utc>,
}

impl Room {
    fn new() -> Room {
        let root = dir();
        let cal = root.join("calendar");
        let start = nine_am() + ChronoDuration::minutes(30);
        std::fs::write(cal.join("orion.json"), serde_json::to_string(&meeting(start)).unwrap()).unwrap();
        Room { store: Store::open(root.join("notifications.json")), sent: Sent::load(&cal), root, cal, start }
    }

    fn ready(&self) -> Ready {
        Ready { event_id: "orion".into(), title: "Orion launch sync".into(), start: self.start.timestamp() }
    }

    /// The brief is announced `minutes` before the start; the answer is the store's.
    fn brief(&self, minutes: i64) -> serde_json::Value {
        announce(&self.store, &self.ready(), self.start - ChronoDuration::minutes(minutes))
    }

    /// The calendar's own tick, `minutes` before the start.
    fn tick(&mut self, minutes: i64) {
        tick(&self.store, &self.cal, &mut self.sent, self.start - ChronoDuration::minutes(minutes));
    }

    fn card(&self, id: &str) -> Option<(Notification, Notice)> {
        self.store.since(0).changed.into_iter().find_map(|n| n.notice.clone().filter(|x| x.notice_id == id).map(|x| (n, x)))
    }

    /// Notices that interrupt, whoever sent them: the toasts this meeting makes.
    fn interrupts(&self) -> Vec<String> {
        self.store
            .since(0)
            .changed
            .into_iter()
            .filter(|n| !n.dismissed)
            .filter_map(|n| n.notice)
            .filter(|x| x.granted == Attention::Interrupt && !x.hidden())
            .map(|x| x.notice_id)
            .collect()
    }
}

impl Drop for Room {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const BRIEF: &str = "cal-brief:orion:20990318T0930";
const REMINDER: &str = "cal-reminder:orion:20990318T0930";
const EARLY: &str = "cal-early:orion:20990318T0930";

/// An event with a place, which the calendar gives an early "leave early" heads-up.
fn with_a_place(room: &Room) {
    let mut event = meeting(room.start);
    event.location = Some("Room 4".into());
    std::fs::write(room.cal.join("orion.json"), serde_json::to_string(&event).unwrap()).unwrap();
}

#[test]
fn the_brief_is_the_calendars_own_notice_and_says_the_time() {
    let room = Room::new();
    assert_eq!(room.brief(10)["stored"], true);
    let (n, brief) = room.card(BRIEF).expect("stored under the deterministic id");
    assert_eq!((brief.family.clone(), brief.harness.id.as_str()), (Family::Calendar, notice::OS_HARNESS));
    assert_eq!(n.title, "Orion launch sync in 10 minutes");
    assert_eq!(n.body, "Your brief is ready.");
    assert_eq!((brief.granted, brief.grant_reason), (Attention::Interrupt, GrantReason::Granted));
    assert_eq!(brief.useful_until.as_deref(), Some("2099-03-18T09:30:00Z"), "a toast held past the start is never shown");
    assert_eq!(room.brief(9)["duplicate"], true, "asking again is answered as a duplicate");
}

#[test]
fn a_brief_for_a_meeting_that_has_started_posts_nothing() {
    let room = Room::new();
    assert_eq!(room.brief(0)["stored"], false);
    assert_eq!(room.brief(-3)["stored"], false);
    assert!(room.card(BRIEF).is_none());
}

#[test]
fn one_toast_per_meeting_when_the_brief_comes_first() {
    let mut room = Room::new();
    room.brief(10);
    room.tick(10);
    assert!(room.card(REMINDER).is_none(), "the brief's notice already holds the interrupt");
    assert_eq!(room.interrupts(), [BRIEF]);
    // And it stays so: the reminder is recorded as told, and is not posted a tick later.
    room.tick(9);
    assert!(room.card(REMINDER).is_none());
}

#[test]
fn one_toast_per_meeting_when_the_reminder_comes_first() {
    let mut room = Room::new();
    room.tick(10);
    assert_eq!(room.interrupts(), [REMINDER]);
    // The brief arrives a minute later. It takes the reminder's place in the inbox and does not
    // interrupt again.
    assert_eq!(room.brief(9)["stored"], true);
    let (_, brief) = room.card(BRIEF).unwrap();
    assert_eq!((brief.granted, brief.grant_reason), (Attention::Inbox, GrantReason::AskedInbox));
    let (old, _) = room.card(REMINDER).unwrap();
    assert!(old.dismissed, "one card per meeting in the inbox");
    assert_eq!(room.interrupts(), Vec::<String>::new(), "no second toast");
}

#[test]
fn a_reminder_set_earlier_has_already_toasted_so_the_brief_stays_in_the_inbox() {
    let mut room = Room::new();
    let mut event = meeting(room.start);
    event.reminder_minutes = 30;
    std::fs::write(room.cal.join("orion.json"), serde_json::to_string(&event).unwrap()).unwrap();
    room.tick(30);
    assert!(room.card(REMINDER).is_some());
    room.brief(10);
    assert_eq!(room.card(BRIEF).unwrap().1.granted, Attention::Inbox);
    assert!(room.card(REMINDER).unwrap().0.dismissed);
}

#[test]
fn the_third_interrupt_of_the_day_goes_to_the_inbox() {
    let room = Room::new();
    for id in ["other-1", "other-2"] {
        let post = notice::NoticePost {
            notice: serde_json::json!({ "notice_id": id, "family": "discovery", "title": "Found", "why": "x", "attention": "interrupt" }),
            harness: notice::NoticeHarness { id: "mind".into(), uid: Some(1000), name: "mind".into(), ..Default::default() },
            os_id: false,
        };
        assert_eq!(room.store.add_notice(post, &room.root)["stored"], true);
    }
    room.brief(10);
    let (_, brief) = room.card(BRIEF).unwrap();
    assert_eq!((brief.granted, brief.grant_reason), (Attention::Inbox, GrantReason::BudgetSpent));
}

#[test]
fn the_persons_calendar_setting_and_quiet_initiative_apply_to_the_brief() {
    for (setting, quiet, expect) in [
        (FamilySetting::Off, false, GrantReason::FamilyOff),
        (FamilySetting::InboxOnly, false, GrantReason::FamilyInboxOnly),
        (FamilySetting::On, true, GrantReason::QuietInitiative),
    ] {
        let mut room = Room::new();
        room.store
            .set_initiative(|s| {
                s.quiet = quiet;
                s.set(&Family::Calendar, setting)
            })
            .unwrap();
        room.brief(10);
        let (_, brief) = room.card(BRIEF).unwrap();
        assert_eq!((brief.granted.clone(), brief.grant_reason.clone()), (Attention::Inbox, expect), "{setting:?} quiet={quiet}");
        assert_eq!(brief.hidden(), setting == FamilySetting::Off);
        // The person's own reminder is not the brief's to swallow: it still arrives.
        room.tick(10);
        assert!(room.card(REMINDER).is_some_and(|(_, r)| r.granted == Attention::Interrupt), "{setting:?} quiet={quiet}");
    }
}

#[test]
fn a_brief_the_person_switched_off_does_not_take_the_reminders_card() {
    let mut room = Room::new();
    room.tick(10);
    room.store.set_initiative(|s| s.set(&Family::Calendar, FamilySetting::Off)).unwrap();
    room.brief(9);
    assert!(!room.card(REMINDER).unwrap().0.dismissed);
}

#[test]
fn quiet_hours_hold_the_brief_and_leave_the_interrupt_to_the_reminder() {
    let room = Room::new();
    // Quiet around the clock the announcement is made on.
    let hour = (room.start - ChronoDuration::minutes(10)).with_timezone(&Local).hour() as u8;
    room.store.set_initiative(|s| { s.quiet_hours = Some(QuietHours { start: (hour + 23) % 24, end: (hour + 1) % 24 }); Ok(()) }).unwrap();
    room.brief(10);
    let (_, brief) = room.card(BRIEF).unwrap();
    assert_eq!(brief.granted, Attention::Inbox);
    assert_eq!(brief.held_until, None, "nothing is held to be toasted when quiet hours end: the reminder is");
    assert!(!crate::brief_notice::covers(&room.store, "orion", &parse_stamp("2099-03-18T09:30:00").unwrap()));
}

#[test]
fn the_brief_of_a_moved_or_deleted_event_is_taken_back_and_a_live_one_is_kept() {
    let mut room = Room::new();
    room.brief(10);
    room.tick(10);
    assert!(!room.card(BRIEF).unwrap().0.dismissed, "the calendar's own sweep keeps a live event's brief");
    std::fs::remove_file(room.cal.join("orion.json")).unwrap();
    room.tick(9);
    assert!(room.card(BRIEF).unwrap().0.dismissed);
}

#[test]
fn the_early_heads_up_is_not_a_second_moment_and_the_brief_takes_its_place() {
    let mut room = Room::new();
    with_a_place(&room);
    room.tick(45);
    assert_eq!(room.interrupts(), [EARLY], "the early card toasts, as it always did");
    room.brief(10);
    let (_, brief) = room.card(BRIEF).unwrap();
    assert_eq!((brief.granted, brief.grant_reason), (Attention::Inbox, GrantReason::AskedInbox));
    assert!(room.card(EARLY).unwrap().0.dismissed, "one card per meeting in the inbox");
    assert_eq!(room.interrupts(), Vec::<String>::new(), "no second toast, and the card that had one is not shown again");
    // The reminder that follows is the same meeting's third moment, and it is covered.
    room.tick(10);
    assert!(room.card(REMINDER).is_none(), "the early card's interrupt is the meeting's one");
}

#[test]
fn a_brief_that_comes_before_the_early_heads_up_holds_the_interrupt() {
    let mut room = Room::new();
    with_a_place(&room);
    room.brief(50);
    room.tick(45);
    assert_eq!(room.interrupts(), [BRIEF]);
    room.tick(10);
    assert!(room.card(REMINDER).is_none());
    assert_eq!(room.interrupts(), [BRIEF]);
}

#[test]
fn an_early_card_that_never_interrupted_leaves_the_reminder_to_the_person() {
    let mut room = Room::new();
    with_a_place(&room);
    room.store.set_initiative(|s| s.set(&Family::Calendar, FamilySetting::InboxOnly)).unwrap();
    room.tick(45);
    assert_eq!(room.interrupts(), Vec::<String>::new());
    room.brief(10);
    assert_eq!(room.card(BRIEF).unwrap().1.granted, Attention::Inbox);
    room.tick(10);
    assert!(room.card(REMINDER).is_some_and(|(_, r)| r.granted == Attention::Interrupt), "the reminder is still the person's own");
}
