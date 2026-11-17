//! The reminder tests that run the tick against a real store: what lands as the desktop's own
//! notice, the budget and quiet hours, restarts, and withdrawal. Separate from the pure-function
//! tests in the parent only to keep reminders.rs from growing; the helpers are the parent's.

fn calendar() -> (Store, PathBuf, PathBuf) {
    let dir = temp_dir();
    let cal = dir.join("calendar");
    std::fs::create_dir_all(&cal).unwrap();
    (Store::open(dir.join("notifications.json")), dir, cal)
}

#[test]
fn both_announcements_land_as_the_desktops_own_notices_and_the_reminder_replaces_the_early_card() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    write(&cal, &event("standup", "Standup", &at(now, 60), DEFAULT_REMINDER_MINUTES));
    let mut sent = Sent::load(&cal);

    tick(&store, &cal, &mut sent, now);
    let shown = cards(&store);
    assert_eq!(shown.len(), 1, "an early card for the dentist and none for the standup");
    let (n, early) = &shown[0];
    assert_eq!(early.notice_id, "cal-early:dentist:20990318T1000");
    assert_eq!(early.harness.id, notice::OS_HARNESS);
    assert_eq!(n.app, "Calendar");
    let sender = n.sender.as_ref().unwrap();
    assert!(sender.desktop && sender.pid as u32 == std::process::id(), "{sender:?}");
    assert_eq!((early.granted.clone(), early.grant_reason.clone()), (Attention::Interrupt, GrantReason::Granted));
    assert_eq!(store.since(0).changed.len(), 1);

    // Fifty minutes on: both own reminders. The dentist's takes the early card's place, and
    // since that card already interrupted, it goes to the inbox: one toast per event.
    let later = now + ChronoDuration::minutes(50);
    let (early_card, _) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
    store.mark_shown(&early_card.id, Some("toast"), false).unwrap();
    tick(&store, &cal, &mut sent, later);
    let mut shown: Vec<String> = cards(&store).into_iter().map(|(_, x)| x.notice_id).collect();
    shown.sort();
    assert_eq!(shown, ["cal-reminder:dentist:20990318T1000", "cal-reminder:standup:20990318T1000"], "one card per event");
    let (_, own) = card(&store, "cal-reminder:dentist:20990318T1000").unwrap();
    assert_eq!(own.family, Family::Reminder);
    assert_eq!((own.granted, own.grant_reason), (Attention::Inbox, GrantReason::AskedInbox), "the early card was the toast");
    let (_, standup) = card(&store, "cal-reminder:standup:20990318T1000").unwrap();
    assert_eq!(standup.grant_reason, GrantReason::Granted, "an event with no early card: the reminder is the toast");
    let (gone, _) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
    assert!(gone.dismissed);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn minds_flooding_reminders_and_spending_the_budget_first_never_stop_the_persons_reminder_interrupting() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    // Two minds on two accounts, each within every cap of its own, fill the minds' shared
    // hourly limit for reminder interrupts and spend the day's budget.
    let send = |from: &str, uid: u32, id: &str, family: &str| {
        let post = notice::NoticePost {
            notice: serde_json::json!({ "notice_id": id, "family": family, "title": "Stretch", "why": "You asked.", "attention": "interrupt" }),
            harness: notice::NoticeHarness { id: from.into(), uid: Some(uid), name: from.into(), ..Default::default() },
            os_id: false,
        };
        store.add_notice(post, &dir)
    };
    for (from, uid) in [("m0", 1000), ("m1", 1001)] {
        for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
            assert_eq!(send(from, uid, &format!("r{i}"), "reminder")["stored"], true);
        }
    }
    send("m0", 1000, "d0", "discovery");
    send("m1", 1001, "d1", "discovery");
    let theirs = send("m1", 1001, "r-late", "reminder");
    let late = store.get(theirs["id"].as_str().unwrap()).unwrap().notice.unwrap();
    assert_eq!(late.grant_reason, GrantReason::Rate, "the flood is real: a mind's next reminder waits");

    // Then the person's own event comes due.
    write(&cal, &event("dentist", "Dentist", &at(now, 10), DEFAULT_REMINDER_MINUTES));
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    let (_, own) = card(&store, "cal-reminder:dentist:20990318T0910").unwrap();
    assert_eq!((own.granted, own.grant_reason, own.held_until), (Attention::Interrupt, GrantReason::Granted, None));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_dismissed_early_card_is_left_alone() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    let (early, _) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
    store.record_feedback(&early.id, notice::FeedbackKind::NotNow).unwrap();
    let before = card(&store, "cal-early:dentist:20990318T1000").unwrap();
    assert!(before.0.dismissed);
    // Edited after it was dismissed: not brought back, and its words are not touched.
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Oak St"));
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(1));
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
    let after = card(&store, "cal-early:dentist:20990318T1000").unwrap();
    assert_eq!(after.0.revision, before.0.revision, "not touched by the supersede or the edit");
    assert_eq!(after.0.title, "Dentist at 10:00, Elm St");
    assert_eq!(after.1.feedback, before.1.feedback);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_own_reminder_never_spends_the_budget_and_the_third_early_card_waits_in_the_inbox() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    for (id, title) in [("a", "Dentist"), ("b", "Bank"), ("c", "Barber")] {
        write(&cal, &placed(id, title, &at(now, 60), "High St"));
    }
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    let mut reasons: Vec<GrantReason> = cards(&store).into_iter().map(|(_, x)| x.grant_reason).collect();
    reasons.sort_by_key(|r| r.as_str().to_string());
    assert_eq!(reasons, [GrantReason::BudgetSpent, GrantReason::Granted, GrantReason::Granted], "two a day; the third waits");
    // Their own reminders never spend the budget: the two whose early card was shown as a toast
    // go to the inbox (that card was the toast), and the one whose early card waited interrupts,
    // budget spent or not.
    // The shell showed the two granted early cards as toasts; the third was never shown.
    for (n, x) in cards(&store).into_iter().filter(|(_, x)| x.grant_reason == GrantReason::Granted) {
        assert!(x.notice_id.starts_with("cal-early:"));
        store.mark_shown(&n.id, Some("toast"), false).unwrap();
    }
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
    let mut own: Vec<GrantReason> = cards(&store).into_iter().filter(|(_, x)| x.family == Family::Reminder).map(|(_, x)| x.grant_reason).collect();
    own.sort_by_key(|r| r.as_str().to_string());
    assert_eq!(own, [GrantReason::AskedInbox, GrantReason::AskedInbox, GrantReason::Granted]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_persons_calendar_setting_and_quiet_initiative_reach_the_early_card_but_not_the_reminder() {
    for (setting, quiet, expect) in [
        (FamilySetting::Off, false, GrantReason::FamilyOff),
        (FamilySetting::InboxOnly, false, GrantReason::FamilyInboxOnly),
        (FamilySetting::On, true, GrantReason::QuietInitiative),
    ] {
        let (store, dir, cal) = calendar();
        store
            .set_initiative(|s| {
                s.quiet = quiet;
                s.set(&Family::Calendar, setting)
            })
            .unwrap();
        let now = nine_am();
        write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
        let mut sent = Sent::load(&cal);
        tick(&store, &cal, &mut sent, now);
        let (_, early) = card(&store, "cal-early:dentist:20990318T1000").unwrap();
        assert_eq!(early.grant_reason, expect);
        assert_eq!(early.hidden(), setting == FamilySetting::Off, "Off is stored hidden");
        tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
        let (_, own) = card(&store, "cal-reminder:dentist:20990318T1000").unwrap();
        assert_eq!((own.granted, own.grant_reason), (Attention::Interrupt, GrantReason::Granted), "{setting:?} quiet={quiet}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn quiet_hours_hold_both() {
    let (store, dir, cal) = calendar();
    // The grant reads the tick's own clock: quiet from an hour before it to two after.
    let hour = chrono::Timelike::hour(&nine_am().with_timezone(&Local)) as u8;
    let quiet = QuietHours { start: (hour + 23) % 24, end: (hour + 2) % 24 };
    store.set_initiative(|s| { s.quiet_hours = Some(quiet); Ok(()) }).unwrap();
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(50));
    for id in ["cal-early:dentist:20990318T1000", "cal-reminder:dentist:20990318T1000"] {
        let (_, x) = card(&store, id).unwrap();
        assert_eq!((x.granted, x.grant_reason), (Attention::Inbox, GrantReason::QuietHours), "{id}");
        assert!(x.held_until.is_some(), "{id}: toasted when they end, if still useful");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_restart_inside_the_window_announces_nothing_again() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    tick(&store, &cal, &mut Sent::load(&cal), now);
    let revision = store.revision();

    // From `Sent`: a new process reads the record and does not ask the store at all.
    tick(&store, &cal, &mut Sent::load(&cal), now + ChronoDuration::minutes(1));
    assert_eq!(store.revision(), revision);

    // From the duplicate id: the record is gone (pruned, unreadable), and the store answers
    // what it already holds as a duplicate.
    std::fs::remove_file(cal.join(".reminded.json")).unwrap();
    let mut forgot = Sent::load(&cal);
    tick(&store, &cal, &mut forgot, now + ChronoDuration::minutes(2));
    assert_eq!(cards(&store).len(), 1);
    assert_eq!(store.revision(), revision, "nothing changed in the store");
    assert!(forgot.has(Kind::Early, "dentist", &parse_stamp(&at(now, 60)).unwrap()), "and the record is made again");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_store_that_cannot_keep_it_is_asked_again_next_tick() {
    let dir = temp_dir();
    let cal = dir.join("calendar");
    std::fs::create_dir_all(&cal).unwrap();
    // A sidecar that is a directory cannot be read, so the store refuses every write.
    std::fs::create_dir_all(crate::store::sidecar_path(&dir.join("notifications.json"))).unwrap();
    let store = Store::open(dir.join("notifications.json"));
    assert!(store.blocked().is_some());
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    assert!(!sent.has(Kind::Early, "dentist", &parse_stamp(&at(now, 60)).unwrap()), "nothing remembered before success");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_card_of_a_deleted_or_moved_event_is_taken_back_but_never_on_an_incomplete_reading() {
    let (store, dir, cal) = calendar();
    let now = nine_am();
    write(&cal, &placed("dentist", "Dentist", &at(now, 60), "Elm St"));
    write(&cal, &placed("bank", "Bank", &at(now, 60), "High St"));
    let mut sent = Sent::load(&cal);
    tick(&store, &cal, &mut sent, now);
    assert_eq!(store.showing().len(), 2);

    // A file that will not parse makes the reading incomplete: nothing is taken back.
    std::fs::write(cal.join("bank.json"), "{half an ev").unwrap();
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(1));
    assert_eq!(store.showing().len(), 2, "an event that could not be read has not gone");

    // Deleted: its card is taken back, without feedback (the person did nothing).
    std::fs::remove_file(cal.join("bank.json")).unwrap();
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(2));
    let (bank, x) = card(&store, "cal-early:bank:20990318T1000").unwrap();
    assert!(bank.dismissed && x.feedback.is_empty());

    // Moved an hour later: the old card goes, and the new start is announced when its time
    // comes.
    write(&cal, &placed("dentist", "Dentist", &at(now, 120), "Elm St"));
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(3));
    assert!(store.showing().is_empty());
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(60));
    assert_eq!(store.showing().iter().filter_map(|n| n.notice.as_ref().map(|x| x.notice_id.clone())).collect::<Vec<_>>(), ["cal-early:dentist:20990318T1100"]);

    // Edited in place (same start, new title and place): the same card, with the new words.
    let (before, _) = card(&store, "cal-early:dentist:20990318T1100").unwrap();
    write(&cal, &placed("dentist", "Dentist (Dr Rao)", &at(now, 120), "Oak St"));
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(61));
    assert_eq!(store.showing().len(), 1);
    let (after, x) = card(&store, "cal-early:dentist:20990318T1100").unwrap();
    assert_eq!((after.id.as_str(), after.title.as_str()), (before.id.as_str(), "Dentist (Dr Rao) at 11:00, Oak St"));
    assert_eq!((x.grant_reason, x.feedback.len()), (before.notice.unwrap().grant_reason, 0), "not granted or counted again");
    // Unchanged on the next tick: nothing written.
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(62));
    assert_eq!(card(&store, "cal-early:dentist:20990318T1100").unwrap().0.revision, after.revision);
    // The place taken off: the early card, which says it has one, is taken back.
    write(&cal, &event("dentist", "Dentist (Dr Rao)", &at(now, 120), DEFAULT_REMINDER_MINUTES));
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(63));
    assert!(store.showing().is_empty());
    write(&cal, &placed("dentist", "Dentist (Dr Rao)", &at(now, 120), "Oak St"));

    // A mind's card is never the timer's to take back.
    let mut theirs = serde_json::json!({ "notice_id": "cal-early:x", "family": "calendar", "title": "t", "why": "w" });
    theirs["attention"] = "inbox".into();
    let post = notice::NoticePost {
        notice: theirs,
        harness: notice::NoticeHarness { id: "pi".into(), uid: Some(1000), ..Default::default() },
        os_id: false,
    };
    assert_eq!(store.add_notice(post, &dir)["stored"], true);
    tick(&store, &cal, &mut sent, now + ChronoDuration::minutes(64));
    assert!(store.showing().iter().any(|n| n.notice.as_ref().is_some_and(|x| x.harness.id == "pi")));
    let _ = std::fs::remove_dir_all(dir);
}
