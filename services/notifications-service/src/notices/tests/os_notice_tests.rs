//! The tests of the desktop's own notices (the calendar's heads-ups and reminders, filed through
//! add_os_notice): who may post under its id, its feedback, the rate caps it is outside of, and its
//! withdrawal. They are separate from the rest of the store's tests only to keep notices.rs from
//! growing; they share that module's helpers through the parent.

use super::*;

// ── The OS's own notices (design/calendar-heads-up-2026-10-16.md) ──────────────────────

fn os_card(id: &str, family: &str) -> serde_json::Value {
    json!({ "notice_id": id, "family": family, "title": "Dentist in 10 minutes", "why": "Calendar events remind you 10 minutes before.", "attention": "interrupt" })
}

#[test]
fn nothing_on_the_socket_can_post_or_poll_as_the_desktop() {
    let (store, dir) = temp_store();
    let own = yantrik_ipc_transport::owner::own_euid();
    // Whatever account and program the host names, the desktop's own id is not a harness's.
    for (uid, program) in [(own, ""), (own, "yantrik-ui"), (1000, "python3"), (999, "yantrik-mind")] {
        let answer = store.add_notice(post_as(notice::OS_HARNESS, uid, program, os_card("cal-reminder:e1:20990318T1000", "reminder")), &home());
        assert_eq!(answer["stored"], false, "{answer}");
        assert!(answer["refused"].as_str().unwrap().contains("its own notices"), "{answer}");
        assert!(store.feedback_for(notice::OS_HARNESS, Some(uid), program, None).is_err());
    }
    assert!(store.list().is_empty());
    assert!(!store.bindings().get(notice::OS_HARNESS).is_some_and(|b| b.uid != Some(own)), "never bound to anyone else");
    // A squatter that posts first under it does not take it from the OS.
    assert_eq!(store.add_os_notice("Calendar", os_card("cal-reminder:e1:20990318T1000", "reminder"))["stored"], true);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_desktops_feedback_is_recorded_on_the_card_and_never_queued() {
    let (store, dir) = temp_store();
    let id = store.add_os_notice("Calendar", os_card("cal-early:e1:20990318T1000", "calendar"))["id"].as_str().unwrap().to_string();
    let queued = store.lock().feedback_queue.len();
    let recorded = store.record_feedback(&id, FeedbackKind::NeverFamily).unwrap();
    assert_eq!(store.lock().feedback_queue.len(), queued, "nothing polls as the desktop");
    let card = store.get(&id).unwrap();
    assert_eq!(card.notice.unwrap().feedback, vec![recorded]);
    // "Never this kind" on an early card switches Calendar off, as on any family.
    assert_eq!(store.initiative().setting(&Family::Calendar), FamilySetting::Off);
    // And on a reminder card it can only make reminders inbox-only.
    let id = store.add_os_notice("Calendar", os_card("cal-reminder:e1:20990318T1000", "reminder"))["id"].as_str().unwrap().to_string();
    store.record_feedback(&id, FeedbackKind::NeverFamily).unwrap();
    assert_eq!(store.initiative().setting(&Family::Reminder), FamilySetting::InboxOnly);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_desktop_is_outside_the_minds_rate_caps_both_ways_but_inside_the_obligation_limit() {
    let (store, dir) = temp_store();
    // Minds at every cap: 60 in the hour across three harnesses of three accounts.
    for i in 0..notice::PER_HOUR_ALL {
        let from = format!("m{}", i % 3);
        let answer = store.add_notice(post_as(&from, 1000 + (i % 3) as u32, "", notice(&format!("n{i}"), "machine", "inbox")), &home());
        assert_eq!(answer["stored"], true, "{i}: {answer}");
    }
    assert!(store.add_notice(post_as("m3", 1003, "", notice("over", "machine", "inbox")), &home())["retry_after_secs"].is_number(), "every harness together is at its cap");
    // The calendar is not held back by the flood…
    for i in 0..(notice::PER_HOUR + 5) {
        let answer = store.add_os_notice("Calendar", os_card(&format!("cal-reminder:e{i}:20990318T1000"), "reminder"));
        assert_eq!(answer["stored"], true, "{i}: {answer}");
    }
    // …and once the minds' cards are gone, a busy calendar does not crowd them out.
    store.dismiss_all_by(true);
    {
        let mut state = store.lock();
        let old = rfc3339(Utc::now() - chrono::Duration::hours(2));
        for n in state.notifications.iter_mut().filter(|n| !is_os(n)) {
            n.created_at = old.clone();
        }
    }
    for i in 0..(notice::PER_HOUR_ALL + 10) {
        store.add_os_notice("Calendar", os_card(&format!("cal-early:e{i}:20990318T1000"), "calendar"));
    }
    assert_eq!(store.add_notice(post_as("m1", 1001, "", notice("after", "machine", "inbox")), &home())["stored"], true);

    // Minds' reminders, up to the every-harness obligation limit, do not hold the desktop's
    // reminder back (the review of #861)…
    let (store3, dir3) = temp_store();
    for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR_ALL {
        let from = format!("m{}", i % 2);
        let answer = store3.add_notice(post_as(&from, 1000 + (i % 2) as u32, "", notice(&format!("r{i}"), "reminder", "interrupt")), &home());
        assert_eq!(answer["stored"], true, "{answer}");
    }
    let id = store3.add_os_notice("Calendar", os_card("cal-reminder:dentist:20990318T1000", "reminder"))["id"].as_str().unwrap().to_string();
    assert_eq!(store3.get(&id).unwrap().notice.unwrap().grant_reason, GrantReason::Granted, "a mind's flood must not silence the person's own reminder");
    // …while a mind's next one still meets the shared limit, which counts the desktop's too.
    let more = store3.add_notice(post_as("m2", 1002, "", notice("r-more", "reminder", "interrupt")), &home())["id"].as_str().unwrap().to_string();
    assert_eq!(store3.get(&more).unwrap().notice.unwrap().grant_reason, GrantReason::Rate);
    let _ = std::fs::remove_dir_all(dir3);

    // The obligation limit still holds the desktop's reminders: past it, they wait in the inbox.
    let (store, dir2) = temp_store();
    let mut reasons = Vec::new();
    for i in 0..(notice::OBLIGATION_INTERRUPTS_PER_HOUR + 1) {
        let id = store.add_os_notice("Calendar", os_card(&format!("cal-reminder:r{i}:20990318T1000"), "reminder"))["id"].as_str().unwrap().to_string();
        reasons.push(store.get(&id).unwrap().notice.unwrap().grant_reason);
    }
    assert_eq!(reasons.last(), Some(&GrantReason::Rate), "{reasons:?}");
    assert!(reasons[..notice::OBLIGATION_INTERRUPTS_PER_HOUR].iter().all(|r| *r == GrantReason::Granted), "{reasons:?}");
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn a_flood_of_the_minds_reminders_and_a_spent_budget_never_hold_back_the_persons_own_reminder() {
    let (store, dir) = temp_store();
    // Two minds on two accounts, each inside every cap of its own: four reminder-family
    // interrupts each fill the minds' shared hourly limit, and two discovery interrupts spend
    // the day's budget.
    for (from, uid) in [("m0", 1000), ("m1", 1001)] {
        for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
            let answer = store.add_notice(post_as(from, uid, "", notice(&format!("r{i}"), "reminder", "interrupt")), &home());
            assert_eq!(answer["stored"], true, "{from} {i}: {answer}");
        }
    }
    for i in 0..2 {
        store.add_notice(post_as("m0", 1000, "", notice(&format!("d{i}"), "discovery", "interrupt")), &home());
    }
    let reason = |store: &Store, id: &serde_json::Value| store.get(id.as_str().unwrap()).unwrap().notice.unwrap();
    let next = store.add_notice(post_as("m1", 1001, "", notice("one-more", "reminder", "interrupt")), &home());
    assert_eq!(reason(&store, &next["id"]).grant_reason, GrantReason::Rate, "the minds are at their shared limit");
    assert_eq!(store.lock().budget.used, notice::DAILY_INTERRUPTS, "and the day's budget is spent");

    // The person's own reminder interrupts all the same: not demoted, not held, not refused.
    let answer = store.add_os_notice("Calendar", os_card("cal-reminder:dentist:20990318T1000", "reminder"));
    assert_eq!(answer["stored"], true, "{answer}");
    let card = reason(&store, &answer["id"]);
    assert_eq!((card.granted, card.grant_reason, card.held_until), (Attention::Interrupt, GrantReason::Granted, None));

    // Up to the desktop's own hourly limit, and only its own count holds it there.
    for i in 1..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
        let answer = store.add_os_notice("Calendar", os_card(&format!("cal-reminder:e{i}:20990318T1000"), "reminder"));
        assert_eq!(reason(&store, &answer["id"]).grant_reason, GrantReason::Granted, "{i}");
    }
    let answer = store.add_os_notice("Calendar", os_card("cal-reminder:fifth:20990318T1000", "reminder"));
    assert_eq!(reason(&store, &answer["id"]).grant_reason, GrantReason::Rate, "a calendar of 10:00 meetings still waits past its own four");

    // The desktop's reminders still count toward the minds' shared limit, as the design says:
    // a mind on a fresh account, alone in its own count, waits in the inbox.
    let (store, dir2) = temp_store();
    for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
        store.add_os_notice("Calendar", os_card(&format!("cal-reminder:a{i}:20990318T1000"), "reminder"));
    }
    for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
        store.add_notice(post_as("m0", 1000, "", notice(&format!("r{i}"), "reminder", "interrupt")), &home());
    }
    let late = store.add_notice(post_as("m1", 1001, "", notice("r", "reminder", "interrupt")), &home());
    assert_eq!(reason(&store, &late["id"]).grant_reason, GrantReason::Rate);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

#[test]
fn a_desktop_card_is_filed_as_the_desktop_and_only_its_own_are_withdrawn() {
    let (store, dir) = temp_store();
    let os = store.add_os_notice("Calendar", os_card("cal-early:e1:20990318T1000", "calendar"))["id"].as_str().unwrap().to_string();
    let mind = store.add_notice(post("mind", notice("cal-early:e1:20990318T1000", "calendar", "inbox")), &home())["id"].as_str().unwrap().to_string();
    let card = store.get(&os).unwrap();
    assert_eq!(card.app, "Calendar");
    assert!(card.sender.as_ref().is_some_and(|s| s.desktop && s.pid > 0 && s.claimed.is_none()), "{:?}", card.sender);
    assert!(store.get(&mind).unwrap().sender.as_ref().is_some_and(|s| !s.desktop));
    assert_eq!(store.withdraw_os_notices("cal-", |_| false).unwrap(), 1);
    assert!(store.get(&os).unwrap().dismissed && !store.get(&mind).unwrap().dismissed);
    assert_eq!(store.withdraw_os_notices("cal-", |_| false).unwrap(), 0, "already gone");
    let _ = std::fs::remove_dir_all(dir);
}
