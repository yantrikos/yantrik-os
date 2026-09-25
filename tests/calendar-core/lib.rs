//! The calendar's storage rules, tested without a socket or a desktop.
//!
//! The bugs these cover were all invisible from either side on its own: the app asked for
//! `start`/`end` where the service required `start_date`/`end_date`, and sent `event_id` where
//! it read `id`, so listing and deleting failed on every machine while both files looked right.
//! The payload tests below are the ones that would have caught it — they send what the app
//! sends and read it the way the service reads it.

#[path = "../../services/calendar-service/src/store.rs"]
pub mod store;

/// The week and day views' arithmetic, from the app side of the same wire.
///
/// It is here rather than beside the app because it is the half of the calendar that has no
/// Slint in it: given the events, the selected date and the view, it says which column an event
/// is in, how tall it is drawn and what the seven column headers read. Week and Day were an
/// empty drawing until today -- five `in` properties nothing anywhere ever set -- so every case
/// below is a case that had never been exercised at all.
#[path = "../../apps/calendar/src/views.rs"]
pub mod views;

/// The own-creation rule of #201: who made an event, and what that lets the maker delete
/// without a person being asked. Also from the app side, and also pure — it is a comparison
/// of two strings the machine established, with no socket, no `/proc` and no desktop in it,
/// so it is tested here rather than against a live shell.
#[path = "../../apps/calendar/src/ownership.rs"]
pub mod ownership;

#[cfg(test)]
mod tests {
    use super::ownership::{agent_identity, may_delete_unasked};
    use super::store::EventStore;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yantrik_ipc_contracts::calendar::{
        Attendee, AttendeeStatus, CreateEventParams, DeleteEventParams, EventsParams,
        UpdateEventParams, UpsertRemoteEventParams,
    };

    static ID: AtomicUsize = AtomicUsize::new(0);

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "calendar-test-{}-{}",
                std::process::id(),
                ID.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&p);
            Self(p)
        }
        fn store(&self) -> EventStore {
            EventStore::new(self.0.clone())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn create(title: &str, start: &str, end: &str) -> CreateEventParams {
        CreateEventParams {
            title: title.into(),
            start: start.into(),
            end: end.into(),
            description: String::new(),
            location: None,
            color: String::new(),
            is_all_day: false,
            attendees: Vec::new(),
            creator: None,
        }
    }

    /// The same create, with a creator on it — what a surface sends when it verified who is
    /// asking and that caller is allowed to be recorded as the maker of the event.
    fn create_by(creator: &str, title: &str, start: &str, end: &str) -> CreateEventParams {
        CreateEventParams { creator: Some(creator.into()), ..create(title, start, end) }
    }

    fn month(year: i32, m: u32, last: u32) -> EventsParams {
        EventsParams {
            start_date: format!("{year:04}-{m:02}-01T00:00:00"),
            end_date: format!("{year:04}-{m:02}-{last:02}T23:59:59"),
        }
    }

    // ── The wire, as both ends speak it ──────────────────────────────

    #[test]
    fn the_listing_request_the_app_sends_is_the_one_the_service_reads() {
        // What the app puts on the wire, serialized exactly as it sends it.
        let sent = serde_json::to_value(month(2026, 9, 30)).unwrap();
        // What the service does with it. Before the contract types this failed, because the
        // app wrote `start`/`end` and the service required `start_date`/`end_date` — and the
        // failure surfaced as an empty month rather than an error.
        let read: EventsParams = serde_json::from_value(sent.clone()).unwrap();
        assert_eq!(read.start_date, "2026-09-01T00:00:00");
        assert_eq!(read.end_date, "2026-09-30T23:59:59");
        assert!(sent.get("start").is_none(), "the old, unread name is gone");
    }

    #[test]
    fn the_delete_request_names_the_event_the_way_the_service_looks_it_up() {
        let sent = serde_json::to_value(DeleteEventParams { id: "abc".into() }).unwrap();
        let read: DeleteEventParams = serde_json::from_value(sent.clone()).unwrap();
        assert_eq!(read.id, "abc");
        assert!(sent.get("event_id").is_none(), "the old, unread name is gone");
    }

    #[test]
    fn optional_fields_may_be_left_out_of_a_create() {
        let bare = serde_json::json!({
            "title": "Dentist", "start": "2026-09-22T10:00:00", "end": "2026-09-22T11:00:00"
        });
        let read: CreateEventParams = serde_json::from_value(bare).unwrap();
        assert_eq!(read.description, "");
        assert_eq!(read.location, None);
    }

    // ── Storing, finding and removing ────────────────────────────────

    #[test]
    fn an_event_that_was_stored_is_in_the_month_it_falls_in() {
        let f = Fixture::new();
        let store = f.store();
        // The directory does not exist yet: a calendar has to take its first appointment.
        let saved = store
            .create(&create("Launch review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .expect("first event on a fresh machine");
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, saved.id);
        assert_eq!(listed[0].title, "Launch review");
        // And it is not in the month next door.
        assert!(store.list(&month(2026, 10, 31)).unwrap().is_empty());
    }

    #[test]
    fn an_event_on_the_last_day_of_the_month_is_in_that_month() {
        let f = Fixture::new();
        let store = f.store();
        store
            .create(&create("Month end", "2026-09-30T18:00:00", "2026-09-30T19:00:00"))
            .unwrap();
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 1);
    }

    #[test]
    fn an_event_spanning_the_first_of_the_month_is_listed_in_both() {
        let f = Fixture::new();
        let store = f.store();
        store
            .create(&create("Overnight", "2026-08-31T22:00:00", "2026-09-01T06:00:00"))
            .unwrap();
        assert_eq!(store.list(&month(2026, 8, 31)).unwrap().len(), 1, "the month it starts in");
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 1, "and the one it ends in");
    }

    #[test]
    fn a_stored_event_survives_a_new_store_over_the_same_directory() {
        let f = Fixture::new();
        f.store()
            .create(&create("Standup", "2026-09-21T09:00:00", "2026-09-21T09:15:00"))
            .unwrap();
        // A restart of the service, or of the machine.
        let listed = f.store().list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Standup");
    }

    #[test]
    fn listing_an_empty_machine_is_empty_not_an_error() {
        let f = Fixture::new();
        assert!(f.store().list(&month(2026, 9, 30)).unwrap().is_empty());
    }

    #[test]
    fn events_come_back_in_the_order_they_happen() {
        let f = Fixture::new();
        let store = f.store();
        store.create(&create("Second", "2026-09-22T15:00:00", "2026-09-22T16:00:00")).unwrap();
        store.create(&create("First", "2026-09-22T09:00:00", "2026-09-22T10:00:00")).unwrap();
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(
            listed.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            vec!["First", "Second"]
        );
    }

    #[test]
    fn delete_removes_only_the_event_named() {
        let f = Fixture::new();
        let store = f.store();
        let keep = store.create(&create("Keep", "2026-09-22T09:00:00", "2026-09-22T10:00:00")).unwrap();
        let drop_it = store.create(&create("Drop", "2026-09-22T11:00:00", "2026-09-22T12:00:00")).unwrap();
        store.delete(&drop_it.id).unwrap();
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, keep.id);
    }

    #[test]
    fn deleting_something_that_was_never_here_is_an_error() {
        let f = Fixture::new();
        // It reported success before, which told a caller a state change had happened when
        // nothing had.
        assert!(f.store().delete("01a0-not-a-real-event").is_err());
    }

    #[test]
    fn an_update_keeps_the_fields_it_was_not_given() {
        let f = Fixture::new();
        let store = f.store();
        let mut params = create("Review", "2026-09-22T14:00:00", "2026-09-22T15:00:00");
        params.description = "bring the release notes".into();
        let saved = store.create(&params).unwrap();

        let updated = store
            .update(&UpdateEventParams {
                id: saved.id.clone(),
                title: Some("Release review".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(updated.title, "Release review");
        assert_eq!(updated.description, "bring the release notes");
        assert_eq!(updated.start, "2026-09-22T14:00:00");
    }

    #[test]
    fn updating_an_event_that_is_not_here_is_an_error() {
        let f = Fixture::new();
        assert!(f
            .store()
            .update(&UpdateEventParams { id: "nope".into(), ..Default::default() })
            .is_err());
    }

    // ── One calendar, including the events that came from another one ─

    fn upsert(remote_id: &str, title: &str, start: &str, end: &str) -> UpsertRemoteEventParams {
        UpsertRemoteEventParams {
            remote_id: remote_id.into(),
            title: title.into(),
            start: start.into(),
            end: end.into(),
            description: String::new(),
            location: None,
            is_all_day: false,
            attendees: Vec::new(),
        }
    }

    #[test]
    fn syncing_the_same_remote_event_twice_stores_it_once() {
        // The companion's Google sync had no idempotent way into this store, so it kept its own
        // SQLite table instead and the Calendar app never saw a synced event at all. The key is
        // the id the event has at the far end.
        let f = Fixture::new();
        let store = f.store();
        let first = store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        let second = store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();

        assert_eq!(first.id, second.id, "the same remote event keeps the id this store gave it");
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 1, "and there is one of it");
        assert_eq!(second.remote_id.as_deref(), Some("goog-1"));
    }

    #[test]
    fn a_remote_event_that_changed_is_edited_rather_than_duplicated() {
        let f = Fixture::new();
        let store = f.store();
        let first = store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        let moved = store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T16:00:00", "2026-09-22T17:00:00"))
            .unwrap();

        assert_eq!(moved.id, first.id);
        assert_eq!(moved.start, "2026-09-22T16:00:00");
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].start, "2026-09-22T16:00:00");
    }

    #[test]
    fn two_different_remote_events_are_two_events() {
        let f = Fixture::new();
        let store = f.store();
        store.upsert_remote(&upsert("goog-1", "One", "2026-09-22T09:00:00", "2026-09-22T10:00:00")).unwrap();
        store.upsert_remote(&upsert("goog-2", "Two", "2026-09-22T11:00:00", "2026-09-22T12:00:00")).unwrap();
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 2);
    }

    #[test]
    fn a_synced_event_is_listed_like_any_other_so_the_app_can_show_it() {
        let f = Fixture::new();
        let store = f.store();
        store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Design review");
    }

    #[test]
    fn editing_a_remote_event_does_not_orphan_it_from_the_calendar_it_came_from() {
        // If an edit dropped the remote id, the next sync would see an event it had never been
        // told about and store a second copy beside this one.
        let f = Fixture::new();
        let store = f.store();
        let synced = store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();

        let edited = store
            .update(&UpdateEventParams {
                id: synced.id.clone(),
                title: Some("Design review (moved)".into()),
                start: Some("2026-09-22T16:00:00".into()),
                end: Some("2026-09-22T17:00:00".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(edited.remote_id.as_deref(), Some("goog-1"));

        // And a re-sync still finds it rather than storing a second one.
        store
            .upsert_remote(&upsert("goog-1", "Design review", "2026-09-22T16:00:00", "2026-09-22T17:00:00"))
            .unwrap();
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 1);
    }

    #[test]
    fn an_event_made_here_can_be_told_which_remote_event_it_became() {
        // The other direction: the mind creates an event, pushes it to Google, and records the id
        // it got there, so the next pull recognises it.
        let f = Fixture::new();
        let store = f.store();
        let made = store.create(&create("Retro", "2026-09-22T14:00:00", "2026-09-22T15:00:00")).unwrap();
        assert_eq!(made.remote_id, None);

        store
            .update(&UpdateEventParams {
                id: made.id.clone(),
                remote_id: Some("goog-9".into()),
                ..Default::default()
            })
            .unwrap();

        let resynced = store
            .upsert_remote(&upsert("goog-9", "Retro", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        assert_eq!(resynced.id, made.id);
        assert_eq!(store.list(&month(2026, 9, 30)).unwrap().len(), 1);
    }

    #[test]
    fn an_upsert_without_a_remote_id_is_refused_rather_than_stored_unkeyed() {
        let f = Fixture::new();
        assert!(f
            .store()
            .upsert_remote(&upsert("  ", "Nameless", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .is_err());
    }

    // ── What a create can carry ──────────────────────────────────────

    #[test]
    fn an_all_day_event_is_stored_as_one() {
        // The mind's calendar tool accepted `all_day` long before this store had anywhere to put
        // it, so an all-day event asked for by the mind was kept as a timed one.
        let f = Fixture::new();
        let store = f.store();
        let mut params = create("Company holiday", "2026-09-23T00:00:00", "2026-09-23T23:59:59");
        params.is_all_day = true;
        let saved = store.create(&params).unwrap();
        assert!(saved.is_all_day);
        assert!(store.list(&month(2026, 9, 30)).unwrap()[0].is_all_day);
    }

    #[test]
    fn attendees_and_a_location_survive_the_round_trip() {
        let f = Fixture::new();
        let store = f.store();
        let mut params = create("Launch review", "2026-09-22T14:00:00", "2026-09-22T15:00:00");
        params.location = Some("Studio".into());
        params.attendees = vec![
            Attendee { name: "Ana".into(), email: "ana@example.com".into(), status: AttendeeStatus::Pending },
            Attendee { name: String::new(), email: "bo@example.com".into(), status: AttendeeStatus::Accepted },
        ];
        store.create(&params).unwrap();

        // Read back through a fresh store, which is the service restarting.
        let listed = f.store().list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].location.as_deref(), Some("Studio"));
        assert_eq!(listed[0].attendees.len(), 2);
        assert_eq!(listed[0].attendees[0].name, "Ana");
        assert_eq!(listed[0].attendees[1].email, "bo@example.com");
        assert_eq!(listed[0].attendees[1].status, AttendeeStatus::Accepted);
    }

    #[test]
    fn a_create_that_says_nothing_about_them_stores_neither() {
        let f = Fixture::new();
        let saved = f
            .store()
            .create(&create("Plain", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        assert!(!saved.is_all_day);
        assert!(saved.attendees.is_empty());
    }

    #[test]
    fn an_update_can_change_the_new_fields_and_leaves_them_alone_when_it_does_not() {
        let f = Fixture::new();
        let store = f.store();
        let mut params = create("Review", "2026-09-22T14:00:00", "2026-09-22T15:00:00");
        params.attendees = vec![Attendee {
            name: "Ana".into(),
            email: "ana@example.com".into(),
            status: AttendeeStatus::Pending,
        }];
        let saved = store.create(&params).unwrap();

        let all_day = store
            .update(&UpdateEventParams {
                id: saved.id.clone(),
                is_all_day: Some(true),
                ..Default::default()
            })
            .unwrap();
        assert!(all_day.is_all_day);
        assert_eq!(all_day.attendees.len(), 1, "an update that said nothing about them kept them");

        let renamed = store
            .update(&UpdateEventParams {
                id: saved.id.clone(),
                title: Some("Release review".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(renamed.is_all_day, "and kept the flag the previous update set");
    }

    #[test]
    fn the_create_request_the_app_sends_is_still_read_by_a_service_that_gained_fields() {
        // The new fields default, so the Calendar app's request — which carries neither — is
        // parsed exactly as it was before them.
        let bare = serde_json::json!({
            "title": "Dentist", "start": "2026-09-22T10:00:00", "end": "2026-09-22T11:00:00"
        });
        let read: CreateEventParams = serde_json::from_value(bare).unwrap();
        assert!(!read.is_all_day);
        assert!(read.attendees.is_empty());
        assert!(read.creator.is_none(), "a request that names no maker records none");
    }

    // ── What the store refuses ───────────────────────────────────────

    #[test]
    fn an_event_needs_a_title() {
        let f = Fixture::new();
        assert!(f.store().create(&create("   ", "2026-09-22T14:00:00", "2026-09-22T15:00:00")).is_err());
    }

    #[test]
    fn a_time_that_is_not_a_time_is_refused_rather_than_stored() {
        let f = Fixture::new();
        let store = f.store();
        // The app's own default used to build this for anything after 23:00 — hour + 1 of 23.
        assert!(store.create(&create("Late", "2026-09-22T23:30:00", "2026-09-22T24:30:00")).is_err());
        assert!(store.list(&month(2026, 9, 30)).unwrap().is_empty(), "nothing half-written");
    }

    #[test]
    fn an_event_cannot_end_before_it_starts() {
        let f = Fixture::new();
        assert!(f
            .store()
            .create(&create("Backwards", "2026-09-22T15:00:00", "2026-09-22T14:00:00"))
            .is_err());
    }

    #[test]
    fn a_listing_with_an_unreadable_range_says_so() {
        let f = Fixture::new();
        let bad = EventsParams { start_date: "last tuesday".into(), end_date: "soon".into() };
        assert!(f.store().list(&bad).is_err());
    }

    #[test]
    fn files_that_are_not_events_are_ignored_rather_than_fatal() {
        let f = Fixture::new();
        let store = f.store();
        store.create(&create("Real", "2026-09-22T14:00:00", "2026-09-22T15:00:00")).unwrap();
        std::fs::write(f.0.join("notes.txt"), "not an event").unwrap();
        std::fs::write(f.0.join("broken.json"), "{ not json").unwrap();
        let listed = store.list(&month(2026, 9, 30)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Real");
    }

    // ── The token an open window follows ─────────────────────────────
    //
    // The Calendar app re-read the store only when the date range on screen changed, which was
    // right while it was the only writer and stopped being right the day this machine got one
    // calendar with several. `calendar.revision` is the cheap question it asks instead, and
    // everything below is about the one property the arrangement rests on: it moves when the
    // stored events move, and never otherwise.

    /// Long enough that two writes are not the same modification time.
    ///
    /// Half the token is an mtime, and a filesystem that keeps them to the second cannot tell two
    /// writes inside one tick apart. A calendar is written at human speed and this is not a
    /// property worth designing a counter for — see the type's own doc — but a test that wrote
    /// twice in a microsecond would be measuring the clock's resolution rather than the token.
    fn settle() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    #[test]
    fn the_revision_moves_when_an_event_is_created() {
        let f = Fixture::new();
        let store = f.store();
        let empty = store.revision();
        assert_eq!(empty.events, 0);

        settle();
        store.create(&create("Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")).unwrap();
        let after = store.revision();
        assert_ne!(after, empty, "a created event has to move the token");
        assert_eq!(after.events, 1);
    }

    #[test]
    fn the_revision_moves_when_an_event_is_edited_in_place() {
        // The half the file count cannot see: nothing is added and nothing is removed, and the
        // window would go on showing the old time.
        let f = Fixture::new();
        let store = f.store();
        let event =
            store.create(&create("Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")).unwrap();
        let before = store.revision();

        settle();
        store
            .update(&UpdateEventParams {
                id: event.id.clone(),
                start: Some("2026-09-22T10:00:00".into()),
                end: Some("2026-09-22T10:15:00".into()),
                ..Default::default()
            })
            .unwrap();
        let after = store.revision();
        assert_ne!(after, before, "an edit in place has to move the token");
        assert_eq!(after.events, before.events, "and it is not the count that moved");
    }

    #[test]
    fn the_revision_moves_when_an_event_is_deleted() {
        let f = Fixture::new();
        let store = f.store();
        let event =
            store.create(&create("Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")).unwrap();
        let before = store.revision();

        settle();
        store.delete(&event.id).unwrap();
        let after = store.revision();
        assert_ne!(after, before);
        assert_eq!(after.events, 0);
    }

    #[test]
    fn reading_the_calendar_does_not_move_the_revision() {
        // The property the whole arrangement rests on. A window polling this must never be the
        // reason it changes, or it would re-list the month every time it asked whether it had to.
        let f = Fixture::new();
        let store = f.store();
        let event =
            store.create(&create("Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")).unwrap();
        let before = store.revision();

        settle();
        store.list(&month(2026, 9, 30)).unwrap();
        store.get(&event.id).unwrap();
        store.revision();
        assert_eq!(store.revision(), before, "listing, getting and asking change nothing");
    }

    #[test]
    fn a_refused_write_does_not_move_the_revision() {
        // A calendar that refuses what it cannot keep must also not claim to have changed. An
        // open window re-listing a month because somebody sent an unparseable time would be
        // work for nothing, every time.
        let f = Fixture::new();
        let store = f.store();
        store.create(&create("Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")).unwrap();
        let before = store.revision();

        settle();
        assert!(store.create(&create("", "2026-09-22T11:00:00", "2026-09-22T12:00:00")).is_err());
        assert!(store.create(&create("Nope", "half past two", "2026-09-22T12:00:00")).is_err());
        assert!(store.delete("01a0-not-a-real-event").is_err());
        assert_eq!(store.revision(), before);
    }

    #[test]
    fn a_machine_with_no_calendar_directory_still_answers() {
        // `list` treats a missing directory as an empty calendar rather than an error, and this
        // has to agree with it: the first event stored on a fresh machine must move the token,
        // and it cannot do that if asking before it existed was a failure.
        let f = Fixture::new();
        let store = f.store();
        assert!(!f.0.exists());
        let empty = store.revision();
        assert_eq!(empty.events, 0);

        settle();
        store.create(&create("First", "2026-09-22T09:00:00", "2026-09-22T10:00:00")).unwrap();
        assert_ne!(store.revision(), empty);
    }

    // ── Who made an event, and what that lets the maker delete (#201) ──
    //
    // An unattended harness could put events on the calendar but never take them off again:
    // `delete_event` is `sensitive` and its own description says the event is not recoverable,
    // so every delete raised an approval card nobody was there to answer. The door #201 chose
    // is `delete_own_event` — `standard`, and the handler only lets it through when the event
    // is on record as created by exactly the identity this caller was verified to be. Both
    // halves are tested here: the rule, as a table over strings, and the record, as something
    // the store keeps and hands back untouched — including across a restart, because the arena
    // creates in one run of the calendar and deletes in another.

    #[test]
    fn only_the_creator_may_delete_an_event_without_being_asked() {
        // The reviewer's table, and then some. Neither side of this comparison ever comes
        // from the request: the creator is what the store kept at creation, and the caller is
        // what the machine established just now — a forged claim in the arguments reaches
        // neither string, which is the row about somebody else's event.
        let recorded = Some("forge.py");
        assert!(
            may_delete_unasked(recorded, Some("forge.py")),
            "the caller that created an event may take it off unasked"
        );
        assert!(
            !may_delete_unasked(recorded, Some("hermes_cli.main")),
            "somebody else's event stays with `delete_event`, which asks — whatever the request claims"
        );
        assert!(
            !may_delete_unasked(None, Some("forge.py")),
            "an event older than the record has no creator to match, even against its true maker"
        );
        assert!(!may_delete_unasked(recorded, None), "a caller nothing could identify is nobody");
        assert!(!may_delete_unasked(None, None), "and two absences are not the same somebody");
        assert!(!may_delete_unasked(Some(""), Some("")), "two blanks are not the same somebody either");
        assert!(
            !may_delete_unasked(Some("  "), Some("  ")),
            "nor are two strings with nothing in them"
        );
        assert!(!may_delete_unasked(recorded, Some("Forge.py")), "the comparison is exact");
        assert!(!may_delete_unasked(recorded, Some("forge.py ")), "and not forgiving about edges");
    }

    #[test]
    fn an_agent_is_one_identity_per_conversation_and_the_spelling_is_written_down_once() {
        // The record side and the compare side must spell an agent the same way or the rule
        // would never fire for agents at all — which is why there is one function that says
        // `agent <mind>:<conversation>` and both sides call it.
        let recorded = agent_identity("mind:42");
        assert_eq!(recorded, "agent mind:42");
        assert!(may_delete_unasked(Some(&recorded), Some(&agent_identity("mind:42"))));
        assert!(
            !may_delete_unasked(Some(&recorded), Some(&agent_identity("mind:43"))),
            "another conversation is another somebody"
        );
        assert!(
            !may_delete_unasked(Some(&recorded), Some("mind:42")),
            "and a program is never an agent: the prefix is what keeps the kinds apart"
        );
    }

    #[test]
    fn the_store_keeps_the_creator_it_was_handed() {
        let f = Fixture::new();
        let store = f.store();
        let saved = store
            .create(&create_by("forge.py", "Harness run", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        assert_eq!(saved.creator.as_deref(), Some("forge.py"));
        let read = store.get(&saved.id).unwrap();
        assert_eq!(read.creator.as_deref(), Some("forge.py"));
        assert!(may_delete_unasked(read.creator.as_deref(), Some("forge.py")));
    }

    #[test]
    fn a_create_that_names_no_maker_stores_none() {
        // The window's own form, a service's reminder, a machine where the caller could not
        // be identified: the event exists, but it belongs to nobody, and nobody's is everybody's
        // — which means `delete_event`, and a person asked.
        let f = Fixture::new();
        let store = f.store();
        let saved =
            store.create(&create("Dentist", "2026-09-22T10:00:00", "2026-09-22T11:00:00")).unwrap();
        assert!(saved.creator.is_none());
        assert!(store.get(&saved.id).unwrap().creator.is_none());
        assert!(!may_delete_unasked(None, Some("forge.py")));
    }

    #[test]
    fn the_record_survives_the_calendar_app_restarting() {
        // The arena requirement: create and delete may happen with the app, and the service,
        // restarted in between. The creator lives in the event's own file, so a fresh store
        // over the same directory — which is what a restart is, from here — reads it back.
        let f = Fixture::new();
        let id = f
            .store()
            .create(&create_by("forge.py", "Harness run", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap()
            .id;

        let reopened = f.store();
        let read = reopened.get(&id).unwrap();
        assert_eq!(read.creator.as_deref(), Some("forge.py"), "the file kept it");
        assert!(may_delete_unasked(read.creator.as_deref(), Some("forge.py")));
    }

    #[test]
    fn an_event_file_from_before_the_record_existed_reads_as_having_no_creator() {
        // Every event already on disk was stored by a service that had no `creator` field. The
        // key is `#[serde(default)]`, so those files still parse — as None, which the rule
        // refuses. Nobody's existing calendar becomes deletable by whoever asks.
        let f = Fixture::new();
        let store = f.store();
        let saved = store
            .create(&create_by("forge.py", "Old file", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        let path = f.0.join(format!("{}.json", saved.id));
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("creator");
        std::fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let read = store.get(&saved.id).unwrap();
        assert!(read.creator.is_none(), "the file without the key parses, with nothing in it");
        assert!(
            !may_delete_unasked(read.creator.as_deref(), Some("forge.py")),
            "and even the caller that made it must ask, because nothing proves that any more"
        );
    }

    #[test]
    fn an_update_keeps_the_creator() {
        // Editing an event is not adopting it. The update path reads the stored event and
        // changes only the fields it was given, so the record rides along untouched.
        let f = Fixture::new();
        let store = f.store();
        let saved = store
            .create(&create_by("forge.py", "Harness run", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        let moved = store
            .update(&UpdateEventParams {
                id: saved.id.clone(),
                start: Some("2026-09-22T15:00:00".into()),
                end: Some("2026-09-22T16:00:00".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(moved.creator.as_deref(), Some("forge.py"));
        assert_eq!(store.get(&saved.id).unwrap().creator.as_deref(), Some("forge.py"));
    }

    #[test]
    fn a_re_sync_keeps_the_creator_the_event_was_stored_with() {
        // `upsert_remote` rebuilds the whole event from the remote's copy of it, and a remote
        // calendar has never heard of this field. Without the explicit carry-over, the first
        // sync after a surface create would wipe the record — and with it the maker's right to
        // delete unasked. The flow is the real one: a caller makes an event, pushes it out,
        // the push notes the remote id, and the next sync finds the event by that id.
        let f = Fixture::new();
        let store = f.store();
        let saved = store
            .create(&create_by("forge.py", "Harness run", "2026-09-22T14:00:00", "2026-09-22T15:00:00"))
            .unwrap();
        store
            .update(&UpdateEventParams {
                id: saved.id.clone(),
                remote_id: Some("remote-1".into()),
                ..Default::default()
            })
            .unwrap();

        let synced = store
            .upsert_remote(&UpsertRemoteEventParams {
                remote_id: "remote-1".into(),
                title: "Harness run (synced)".into(),
                start: "2026-09-22T14:00:00".into(),
                end: "2026-09-22T15:00:00".into(),
                description: String::new(),
                location: None,
                is_all_day: false,
                attendees: Vec::new(),
            })
            .unwrap();
        assert_eq!(synced.id, saved.id, "the sync edited the event it already had");
        assert_eq!(synced.title, "Harness run (synced)", "and the remote's copy of the fields won");
        assert_eq!(
            synced.creator.as_deref(),
            Some("forge.py"),
            "but a sync edits an event, it does not adopt it: the record stays"
        );

        // A remote nobody here ever made, arriving for the first time, has no verified maker.
        let fresh = store
            .upsert_remote(&UpsertRemoteEventParams {
                remote_id: "remote-2".into(),
                title: "Imported".into(),
                start: "2026-09-23T14:00:00".into(),
                end: "2026-09-23T15:00:00".into(),
                description: String::new(),
                location: None,
                is_all_day: false,
                attendees: Vec::new(),
            })
            .unwrap();
        assert!(fresh.creator.is_none(), "a sync stores what a sync knows: nobody made this here");
    }
}

#[cfg(test)]
mod view_tests {
    use super::views::{
        added_clock, all_day_bounds, day_view, last_day_of_month, name_event, named_on,
        naming_index, rescheduled, selected_date, start_and_end, timezone_label, today_line,
        visible_range, week_bounds, week_view, EventRef, NAMING_CAP, Named, SourceEvent, ViewMode,
    };
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("a real date")
    }

    // The id is the title here. The store's is a uuid7 and nothing in these cases turns on its
    // shape, while a readable one makes a failed assertion say which event it was about.
    fn event(title: &str, start: &str, end: &str) -> SourceEvent {
        SourceEvent {
            id: title.into(),
            title: title.into(),
            start: start.into(),
            end: end.into(),
            is_all_day: false,
            color_index: 0,
        }
    }

    fn all_day(title: &str, day: &str) -> SourceEvent {
        SourceEvent {
            id: title.into(),
            title: title.into(),
            start: format!("{day}T00:00:00"),
            end: format!("{day}T23:59:59"),
            is_all_day: true,
            color_index: 0,
        }
    }

    // -- Where an event lands ----------------------------------------

    #[test]
    fn an_event_is_in_the_column_and_at_the_hour_it_was_stored_at() {
        // 2026-09-22 is a Tuesday, so the third column of a week that starts on Sunday.
        let week = week_view(
            &[event("Standup", "2026-09-22T09:30:00", "2026-09-22T09:45:00")],
            date(2026, 9, 22),
        );
        assert_eq!(week.events.len(), 1);
        let block = &week.events[0];
        assert_eq!(block.day_index, 2);
        assert_eq!(block.start_hour, 9);
        assert_eq!(block.start_min, 30);
    }

    #[test]
    fn a_blocks_height_is_the_minutes_between_its_start_and_its_end() {
        let week = week_view(
            &[event("Review", "2026-09-22T14:00:00", "2026-09-22T15:30:00")],
            date(2026, 9, 22),
        );
        assert_eq!(week.events[0].duration_min, 90);
    }

    #[test]
    fn the_day_view_puts_everything_in_the_one_column_in_order() {
        let day = day_view(
            &[
                event("Later", "2026-09-22T16:00:00", "2026-09-22T17:00:00"),
                event("Earlier", "2026-09-22T09:00:00", "2026-09-22T10:00:00"),
            ],
            date(2026, 9, 22),
        );
        let titles: Vec<&str> = day.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Earlier", "Later"]);
        assert!(day.events.iter().all(|e| e.day_index == 0));
        assert_eq!(day.title, "Tuesday, 22 September 2026");
    }

    // -- Midnight ----------------------------------------------------

    #[test]
    fn an_event_running_past_midnight_is_clipped_at_the_day_and_continued_on_the_next() {
        let week = week_view(
            &[event("Deploy window", "2026-09-22T22:00:00", "2026-09-23T01:00:00")],
            date(2026, 9, 22),
        );
        assert_eq!(week.events.len(), 2, "one block per day it covers");

        let tuesday = &week.events[0];
        assert_eq!(tuesday.day_index, 2);
        assert_eq!((tuesday.start_hour, tuesday.start_min), (22, 0));
        assert_eq!(tuesday.duration_min, 120, "clipped at the end of Tuesday");

        let wednesday = &week.events[1];
        assert_eq!(wednesday.day_index, 3);
        assert_eq!((wednesday.start_hour, wednesday.start_min), (0, 0));
        assert_eq!(wednesday.duration_min, 60);
    }

    #[test]
    fn the_day_view_shows_only_the_part_of_a_spanning_event_that_is_on_that_day() {
        let day = day_view(
            &[event("Deploy window", "2026-09-22T22:00:00", "2026-09-23T01:00:00")],
            date(2026, 9, 23),
        );
        assert_eq!(day.events.len(), 1);
        assert_eq!((day.events[0].start_hour, day.events[0].start_min), (0, 0));
        assert_eq!(day.events[0].duration_min, 60);
    }

    #[test]
    fn an_event_ending_exactly_at_midnight_does_not_leave_an_empty_block_on_the_next_day() {
        let week = week_view(
            &[event("Evening", "2026-09-22T22:00:00", "2026-09-23T00:00:00")],
            date(2026, 9, 22),
        );
        assert_eq!(week.events.len(), 1);
        assert_eq!(week.events[0].duration_min, 120);
    }

    // -- What is not on the grid -------------------------------------

    #[test]
    fn an_event_outside_the_week_shown_is_not_in_it() {
        let events = [
            event("This week", "2026-09-22T09:00:00", "2026-09-22T10:00:00"),
            event("Next week", "2026-09-29T09:00:00", "2026-09-29T10:00:00"),
            event("Last week", "2026-09-15T09:00:00", "2026-09-15T10:00:00"),
        ];
        let week = week_view(&events, date(2026, 9, 22));
        let titles: Vec<&str> = week.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["This week"]);
    }

    #[test]
    fn an_all_day_event_is_not_given_an_hour_it_never_had() {
        let week = week_view(&[all_day("Company holiday", "2026-09-23")], date(2026, 9, 22));
        assert!(week.events.is_empty(), "nothing on the hour grid");
        assert_eq!(week.all_day[3], ["Company holiday"]);
        // The column header is the only place on a grid of hours that can say so.
        assert_eq!(week.labels[3], "Wed 23 \u{b7} 1 all day");

        let day = day_view(&[all_day("Company holiday", "2026-09-23")], date(2026, 9, 23));
        assert!(day.events.is_empty());
        assert_eq!(day.all_day, ["Company holiday"]);
        assert_eq!(day.title, "Wednesday, 23 September 2026 \u{b7} 1 all day");
    }

    #[test]
    fn an_event_whose_time_will_not_parse_is_skipped_rather_than_drawn_or_fatal() {
        let events = [
            event("Nonsense start", "next tuesday", "2026-09-22T10:00:00"),
            event("Nonsense end", "2026-09-22T09:00:00", "soon"),
            event("Backwards", "2026-09-22T15:00:00", "2026-09-22T14:00:00"),
            event("Empty", "", ""),
            event("Real", "2026-09-22T11:00:00", "2026-09-22T12:00:00"),
        ];
        let week = week_view(&events, date(2026, 9, 22));
        let titles: Vec<&str> = week.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Real"]);

        let day = day_view(&events, date(2026, 9, 22));
        assert_eq!(day.events.len(), 1);
    }

    // -- The week, and where it starts -------------------------------

    #[test]
    fn the_week_starts_on_sunday_because_the_month_grid_does() {
        // calendar.slint draws Sun..Sat over the month, so a date's column has to be the same
        // number in both views or the same day is in two places.
        let (start, end) = week_bounds(date(2026, 9, 22));
        assert_eq!(start, date(2026, 9, 20));
        assert_eq!(end, date(2026, 9, 26));

        // A Sunday is the start of its own week, not the end of the one before.
        assert_eq!(week_bounds(date(2026, 9, 20)).0, date(2026, 9, 20));
    }

    #[test]
    fn the_column_headers_read_as_the_dates_of_the_week() {
        let week = week_view(&[], date(2026, 9, 22));
        assert_eq!(
            week.labels,
            ["Sun 20", "Mon 21", "Tue 22", "Wed 23", "Thu 24", "Fri 25", "Sat 26"]
        );
    }

    // -- A week that is in two months --------------------------------

    #[test]
    fn a_week_straddling_a_month_boundary_holds_events_from_both_months() {
        // 2026-10-01 is a Thursday, so its week runs Sun 27 September to Sat 3 October.
        let events = [
            event("September", "2026-09-29T09:00:00", "2026-09-29T10:00:00"),
            event("October", "2026-10-01T09:00:00", "2026-10-01T10:00:00"),
        ];
        let week = week_view(&events, date(2026, 10, 1));
        let placed: Vec<(&str, i32)> =
            week.events.iter().map(|e| (e.title.as_str(), e.day_index)).collect();
        assert_eq!(placed, [("September", 2), ("October", 4)]);
    }

    #[test]
    fn the_week_view_asks_the_store_for_the_days_the_week_needs_not_just_the_month() {
        // The app fetched exactly the month on screen, so the September half of this week was
        // never read and the week drew four empty columns without saying why.
        let (from, to) = visible_range(2026, 10, ViewMode::Week, 1);
        assert_eq!(from, date(2026, 9, 27), "back to the Sunday the week starts on");
        assert_eq!(to, date(2026, 10, 31), "and still the whole month for the month grid");

        // The last week of a month reaches the other way.
        let (from, to) = visible_range(2026, 9, ViewMode::Week, 30);
        assert_eq!(from, date(2026, 9, 1));
        assert_eq!(to, date(2026, 10, 3));

        // The month view asks for the month and no more.
        assert_eq!(
            visible_range(2026, 9, ViewMode::Month, 22),
            (date(2026, 9, 1), date(2026, 9, 30))
        );
    }

    // -- Which day the week and day views are about ------------------

    #[test]
    fn a_month_arrived_at_with_nothing_picked_opens_on_its_first_day() {
        assert_eq!(selected_date(2026, 9, 0), date(2026, 9, 1));
        assert_eq!(selected_date(2026, 9, -1), date(2026, 9, 1));
    }

    #[test]
    fn a_day_that_is_past_the_end_of_a_shorter_month_is_clamped_rather_than_lost() {
        assert_eq!(selected_date(2026, 9, 31), date(2026, 9, 30));
        assert_eq!(selected_date(2026, 2, 31), date(2026, 2, 28));
        assert_eq!(last_day_of_month(2024, 2), 29);
        assert_eq!(last_day_of_month(2026, 12), 31);
    }

    // -- Saving ------------------------------------------------------

    #[test]
    fn an_events_end_is_its_start_plus_how_long_it_runs() {
        let (start, end) = start_and_end("2026-09-22", "14:00", 90).unwrap();
        assert_eq!(start, "2026-09-22T14:00:00");
        assert_eq!(end, "2026-09-22T15:30:00");
    }

    #[test]
    fn a_late_event_ends_at_the_end_of_its_day_rather_than_at_a_time_that_does_not_exist() {
        // "23:30 plus an hour" was written as hour + 1 and produced T24:30:00, which the store
        // refuses outright -- so the evening appointment was simply not kept.
        let (_, end) = start_and_end("2026-09-22", "23:30", 60).unwrap();
        assert_eq!(end, "2026-09-22T23:59:00");
        let (_, end) = start_and_end("2026-09-22", "22:30", 120).unwrap();
        assert_eq!(end, "2026-09-22T23:59:00");
    }

    #[test]
    fn a_date_or_a_time_that_will_not_parse_is_refused_rather_than_sent_to_the_store() {
        assert!(start_and_end("next friday", "14:00", 60).is_none());
        assert!(start_and_end("2026-09-22", "half past two", 60).is_none());
    }

    #[test]
    fn the_timezone_strip_says_the_offset_it_can_actually_know() {
        assert_eq!(timezone_label(5 * 3600 + 1800), "Times are local, UTC+05:30");
        assert_eq!(timezone_label(0), "Times are local, UTC+00:00");
        assert_eq!(timezone_label(-(7 * 3600 + 1800)), "Times are local, UTC-07:30");
    }

    /// "What is on my calendar today" is the question a calendar exists to answer, and the
    /// mind that asked it used to run `date` through the shell to learn which day today even
    /// is — sensitive, so knowing the day raised an approval card (#207). The describe now
    /// says it: the date and its weekday, in the shape the issue asked for.
    #[test]
    fn describe_says_which_day_today_is() {
        assert_eq!(today_line(date(2026, 9, 23)), "2026-09-23 Wednesday");
        assert_eq!(today_line(date(2026, 1, 4)), "2026-01-04 Sunday");

        // And the app really publishes it — the line above would keep passing if the field
        // were dropped from the describe. Pinned against the source, as `control.rs` in the
        // shell pins its own describe wiring, because the closure needs a live window.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/calendar/src/main.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        assert!(
            src.contains(".with(\"today\", views::today_line("),
            "the calendar's describe must carry `today`"
        );
    }

    // ── Naming an event a caller did not store ───────────────────────
    //
    // `delete_event` and `update_event` take the store's id, which a mind has after `add_event`
    // or after reading `describe`. A person's instruction does not carry one — "drop the dentist
    // on Thursday" — so a title and a date are the other way in, and the whole risk of that way
    // is picking the wrong one silently.

    fn stored(id: &str, title: &str, start: &str, end: &str) -> EventRef {
        EventRef {
            id: id.into(),
            title: title.into(),
            start: start.into(),
            end: end.into(),
            is_all_day: false,
        }
    }

    fn a_day() -> Vec<EventRef> {
        vec![
            stored("a", "Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00"),
            stored("b", "Dentist", "2026-09-22T11:00:00", "2026-09-22T12:00:00"),
            stored("c", "Dentist", "2026-09-23T11:00:00", "2026-09-23T12:00:00"),
        ]
    }

    #[test]
    fn a_title_and_a_date_that_name_one_event_name_that_one() {
        match named_on(&a_day(), "Dentist", "2026-09-22") {
            Named::One(event) => assert_eq!(event.id, "b", "not the one on the next day"),
            other => panic!("expected exactly one, got {other:?}"),
        }
    }

    #[test]
    fn the_title_is_matched_without_case_or_surrounding_space() {
        // What a person types, and what a model passes on from what a person typed.
        match named_on(&a_day(), "  dentist ", "2026-09-22") {
            Named::One(event) => assert_eq!(event.id, "b"),
            other => panic!("expected exactly one, got {other:?}"),
        }
    }

    #[test]
    fn a_title_that_is_only_part_of_one_names_nothing() {
        // Deliberately not a substring match. "Stand" removing "Standup" is the beginning of
        // "standup" removing "standup with the platform team" on a day that has both.
        assert_eq!(named_on(&a_day(), "Stand", "2026-09-22"), Named::None);
        assert_eq!(named_on(&a_day(), "Standup", "2026-09-24"), Named::None);
    }

    #[test]
    fn two_events_of_one_name_on_one_day_are_ambiguous_and_both_are_handed_back() {
        // The refusal that matters. Picking either would remove the wrong appointment half the
        // time and report success — which is the trash-icon bug 617dac9 fixed, rebuilt on the
        // control surface. The candidates come back so the caller can say which by id.
        let mut day = a_day();
        day.push(stored("d", "Dentist", "2026-09-22T16:00:00", "2026-09-22T17:00:00"));
        match named_on(&day, "Dentist", "2026-09-22") {
            Named::Ambiguous(candidates) => {
                let ids: Vec<&str> = candidates.iter().map(|e| e.id.as_str()).collect();
                assert_eq!(ids, vec!["b", "d"]);
                // And the times, because two ids alone do not tell a caller which is which.
                assert_eq!(candidates[0].start, "2026-09-22T11:00:00");
                assert_eq!(candidates[1].start, "2026-09-22T16:00:00");
            }
            other => panic!("expected an ambiguity, got {other:?}"),
        }
    }

    // ── Moving an appointment ────────────────────────────────────────

    #[test]
    fn moving_an_event_to_another_time_keeps_how_long_it_runs() {
        // "Move the standup to 10:00" says nothing about length. An update that reset it to the
        // default hour would be changing something nobody asked about.
        let moved = rescheduled(
            "2026-09-22T09:00:00",
            "2026-09-22T09:15:00",
            false,
            None,
            Some("10:00"),
            None,
        )
        .unwrap()
        .expect("a time was given, so it moves");
        assert_eq!(moved, ("2026-09-22T10:00:00".into(), "2026-09-22T10:15:00".into()));
    }

    #[test]
    fn moving_an_event_to_another_day_keeps_its_time_and_its_length() {
        let moved =
            rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, Some("2026-10-01"), None, None)
                .unwrap()
                .expect("a date was given, so it moves");
        assert_eq!(moved, ("2026-10-01T09:00:00".into(), "2026-10-01T09:15:00".into()));
    }

    #[test]
    fn a_length_given_is_the_length_it_gets_and_the_start_stays() {
        let moved =
            rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, None, None, Some(45))
                .unwrap()
                .expect("a duration was given, so it changes");
        assert_eq!(moved, ("2026-09-22T09:00:00".into(), "2026-09-22T09:45:00".into()));
    }

    #[test]
    fn an_update_that_says_nothing_about_when_leaves_the_times_alone() {
        // A rename is an update too, and rebuilding the timestamps for one is how a rename comes
        // to move a meeting.
        assert_eq!(
            rescheduled("2026-09-22T09:00:00", "2026-09-22T09:15:00", false, None, None, None)
                .unwrap(),
            None
        );
    }

    #[test]
    fn an_event_moved_late_in_the_day_ends_at_the_end_of_it_rather_than_at_a_time_that_is_not_one() {
        // The same clamp `start_and_end` applies on create: 23:30 plus an hour used to be
        // "24:30", which the store refuses outright.
        let moved = rescheduled(
            "2026-09-22T09:00:00",
            "2026-09-22T10:00:00",
            false,
            None,
            Some("23:30"),
            None,
        )
        .unwrap()
        .expect("a time was given");
        assert_eq!(moved.1, "2026-09-22T23:59:00");
    }

    #[test]
    fn an_all_day_event_moved_to_another_day_is_still_all_day() {
        let moved =
            rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, Some("2026-09-25"), None, None)
                .unwrap()
                .expect("a date was given");
        assert_eq!(moved, ("2026-09-25T00:00:00".into(), "2026-09-25T23:59:00".into()));
        assert_eq!(all_day_bounds("2026-09-25").unwrap(), moved);
    }

    #[test]
    fn a_time_on_an_all_day_event_is_refused_rather_than_quietly_making_it_a_timed_one() {
        // Applying it would turn a day-long event into a one-hour appointment at a time nobody
        // chose, and the caller would be told it had been moved.
        let refused =
            rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, None, Some("09:00"), None)
                .unwrap_err();
        assert!(refused.contains("all day"), "{refused}");
        let refused =
            rescheduled("2026-09-22T00:00:00", "2026-09-22T23:59:00", true, None, None, Some(30))
                .unwrap_err();
        assert!(refused.contains("all day"), "{refused}");
    }

    #[test]
    fn an_update_that_cannot_be_worked_out_says_which_part_it_could_not_read() {
        // A file edited by hand can hold anything. The caller gets the reason rather than a
        // guessed time or a panic in a dispatch.
        let refused =
            rescheduled("2026-09-22T09:00:00", "whenever", false, None, Some("10:00"), None)
                .unwrap_err();
        assert!(refused.contains("duration_min"), "it has to name the way out: {refused}");

        let refused =
            rescheduled("whenever", "2026-09-22T10:00:00", false, None, Some("10:00"), None)
                .unwrap_err();
        assert!(refused.contains("starts at"), "{refused}");

        let refused =
            rescheduled("2026-09-22T09:00:00", "2026-09-22T10:00:00", false, Some("next friday"), None, None)
                .unwrap_err();
        assert!(refused.contains("next friday"), "{refused}");

        let refused =
            rescheduled("2026-09-22T09:00:00", "2026-09-22T10:00:00", false, None, None, Some(-5))
                .unwrap_err();
        assert!(refused.contains("negative"), "{refused}");
    }

    #[test]
    fn a_whole_day_runs_from_midnight_to_the_last_minute_of_it() {
        assert_eq!(
            all_day_bounds("2026-09-22").unwrap(),
            ("2026-09-22T00:00:00".to_string(), "2026-09-22T23:59:00".to_string())
        );
        assert!(all_day_bounds("the 22nd").is_none());
    }

    #[test]
    fn an_all_day_add_needs_no_time() {
        // The describe says `time` is not used with `all_day`, and the call sent that way used
        // to be refused as "needs argument `time`" before it reached anything (#297). A
        // whole-day event has no clock: the answer is the empty one, and a `time` that arrived
        // anyway is not consulted, exactly as `all_day` promises.
        assert_eq!(added_clock(None, true).unwrap(), "");
        assert_eq!(added_clock(Some("14:30"), true).unwrap(), "");
    }

    #[test]
    fn a_timed_add_without_a_time_is_refused_in_a_sentence_that_names_time() {
        // The optionality stops where the whole day stops: an event at a particular hour is
        // nothing without it. The refusal is the handler's own sentence, not the format
        // complaint about an empty string, and it points at the way out.
        let refused = added_clock(None, false).unwrap_err();
        assert!(refused.contains("`time`"), "{refused}");
        assert!(refused.contains("all_day"), "{refused}");
        let refused = added_clock(Some("   "), false).unwrap_err();
        assert!(refused.contains("`time`"), "{refused}");
        // A `time` that is there but cannot hold a clock keeps its format refusal.
        let refused = added_clock(Some("1430"), false).unwrap_err();
        assert!(refused.contains("14:30") && refused.contains("1430"), "{refused}");
        assert_eq!(added_clock(Some(" 14:30 "), false).unwrap(), "14:30");
    }

    #[test]
    fn an_all_day_event_the_app_stores_is_one_the_grid_leaves_alone() {
        // The two halves agreeing: what `all_day_bounds` writes is what `all_day_columns` reads,
        // so an all-day event added through the surface is on its day's header and not given an
        // hour it never had.
        let (start, end) = all_day_bounds("2026-09-22").unwrap();
        let event = SourceEvent {
            id: "a".into(),
            title: "Conference".into(),
            start,
            end,
            is_all_day: true,
            color_index: 0,
        };
        let week = week_view(&[event], date(2026, 9, 22));
        assert!(week.events.is_empty(), "an all-day event is not on the hour grid");
        assert_eq!(week.all_day[2], vec!["Conference".to_string()], "Tuesday's column");
    }

    // ── The name an approval card asks permission with (#54) ─────────
    //
    // `delete_event` recommends the store's id and the grant is bound to it byte for byte —
    // the reliable way in, and the one a person cannot read. `named_on` goes title and date
    // to id; these go id back, which is what the describe's naming index is built from.

    #[test]
    fn a_timed_event_is_named_by_title_day_and_start() {
        // The event from the issue: a Friday afternoon appointment, asked away by its uuid.
        assert_eq!(
            name_event(&stored(
                "01a0c718-3931-7342-b9c7-8de36140ddb0",
                "Dentist",
                "2026-09-25T13:00:00",
                "2026-09-25T13:45:00"
            )),
            "Dentist, Fri 25 Sep 13:00"
        );
        assert_eq!(
            name_event(&stored("b", "Standup", "2026-09-22T09:00:00", "2026-09-22T09:15:00")),
            "Standup, Tue 22 Sep 09:00"
        );
    }

    #[test]
    fn an_all_day_event_says_it_is_all_day() {
        // The grids leave these to the day header; reading out the midnight an all-day row is
        // stored at would be inventing a time nobody gave.
        let mut holiday = stored("d", "Company holiday", "2026-09-23T00:00:00", "2026-09-23T23:59:00");
        holiday.is_all_day = true;
        assert_eq!(name_event(&holiday), "Company holiday, Wed 23 Sep, all day");
    }

    #[test]
    fn a_start_that_will_not_parse_adds_no_time() {
        // A file edited by hand can hold anything. The title alone is short; a made-up time
        // inside the sentence a person decides on would be worse than that.
        assert_eq!(name_event(&stored("m", "Mystery", "sometime", "whenever")), "Mystery");
    }

    #[test]
    fn the_index_keeps_every_name_until_the_cap_then_drops_the_oldest() {
        let day = a_day();
        let whole = naming_index(&day);
        assert_eq!(whole.len(), day.len(), "an ordinary range keeps every name");
        assert!(
            whole.contains(&(day[0].id.clone(), name_event(&day[0]))),
            "and the values are `name_event`'s, not a second sentence for the same thing"
        );

        // A range far past the cap: one event an hour, oldest first, plus a hand-edited row
        // whose start will not parse — which counts as oldest of all (see `naming_index`).
        let mut events: Vec<EventRef> = (0..NAMING_CAP + 10)
            .map(|i| {
                stored(
                    &format!("e{i}"),
                    &format!("Event {i}"),
                    &format!("2026-01-{:02}T{:02}:00:00", 1 + i / 24, i % 24),
                    &format!("2026-01-{:02}T{:02}:00:00", 1 + i / 24, i % 24),
                )
            })
            .collect();
        events.push(stored("junk", "Hand-edited", "sometime", "whenever"));
        let index = naming_index(&events);
        assert_eq!(index.len(), NAMING_CAP, "the cap holds whatever the range");
        for gone in (0..10).map(|i| format!("e{i}")).chain(["junk".to_string()]) {
            assert!(
                !index.iter().any(|(id, _)| *id == gone),
                "{gone} is at the old end and was dropped"
            );
        }
        let newest = &events[NAMING_CAP + 9];
        assert!(
            index.contains(&(newest.id.clone(), name_event(newest))),
            "what an action taken now was read from keeps its name"
        );
    }

    /// The index is worth only what the app actually publishes. Pinned against the source, as
    /// `describe_says_which_day_today_is` pins the day line, because the closure needs a live
    /// window — and dropping this key is exactly the quiet half of #54: the card keeps working,
    /// it just stops saying what the thing is.
    #[test]
    fn the_describe_publishes_what_its_ids_stand_for() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../apps/calendar/src/main.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        assert!(
            src.contains(".with(\"naming\", serde_json::Value::Object(naming))"),
            "the calendar's describe must carry the id→name index"
        );
        assert!(
            src.contains("views::naming_index(&event_refs(&s.events))"),
            "built by the capped index over the events in hand, not a second formatter in main.rs"
        );
    }
}
