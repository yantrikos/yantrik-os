//! The tests of the notification store: persistence, the eviction order, redaction for a caller
//! that is not the desktop. Separate from store.rs, which is over the file-size limit, so that file
//! does not grow.

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn temp_store() -> (Store, PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "yantrik-notifications-test-{}-{}.json",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    (Store::open(path.clone()), path)
}

fn req(app: &str, title: &str) -> AddRequest {
    AddRequest {
        app: app.into(),
        title: title.into(),
        ..Default::default()
    }
}

#[test]
fn a_picture_is_kept_only_as_a_short_absolute_path() {
    let (store, path) = temp_store();
    let with = |image: &str| store.add(AddRequest { image: Some(image.into()), ..req("Yantrik", "Screenshot saved") }).image;
    assert_eq!(with("/home/p/Pictures/Screenshots/a.png").as_deref(), Some("/home/p/Pictures/Screenshots/a.png"));
    assert_eq!(with("relative.png"), None);
    assert_eq!(with(&format!("/{}", "a".repeat(600))), None);
    let _ = std::fs::remove_file(path);
}

#[test]
fn every_field_is_bounded_on_the_way_in() {
    let (store, path) = temp_store();
    let stored = store.add(AddRequest {
        app: "a".repeat(500),
        title: "t".repeat(5_000),
        body: "b".repeat(50_000),
        actions: (0..20)
            .map(|i| NotificationAction::new(format!("{}{}", "i".repeat(500), i), "l".repeat(500)))
            .collect(),
        ..Default::default()
    });
    assert_eq!(stored.app.chars().count(), MAX_APP);
    assert_eq!(stored.title.chars().count(), MAX_TITLE);
    assert_eq!(stored.body.chars().count(), MAX_BODY);
    assert_eq!(stored.actions.len(), MAX_ACTIONS);
    assert_eq!(stored.actions[0].id.chars().count(), MAX_ACTION_ID);
    assert_eq!(stored.actions[0].label.chars().count(), MAX_ACTION_LABEL);
    let _ = std::fs::remove_file(path);
}

#[test]
fn what_was_stored_is_still_there_after_a_restart() {
    let (store, path) = temp_store();
    let first = store.add(req("Downloads", "debian.iso finished"));
    drop(store);

    let reopened = Store::open(path.clone());
    let list = reopened.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, first.id);
    assert_eq!(list[0].title, "debian.iso finished");
    assert_eq!(reopened.unread(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn revision_and_ids_only_go_up_across_a_restart() {
    // The shell polls `since(revision)`. If a restart reset the counter, the shell's stored
    // revision would be ahead of the store's and it would never be told about anything
    // again — a notification centre that silently stops updating.
    let (store, path) = temp_store();
    store.add(req("A", "one"));
    store.add(req("A", "two"));
    let rev_before = store.revision();
    drop(store);

    let reopened = Store::open(path.clone());
    assert_eq!(reopened.revision(), rev_before);
    let third = reopened.add(req("A", "three"));
    assert!(reopened.revision() > rev_before);
    assert_eq!(third.id, "3");
    let _ = std::fs::remove_file(path);
}

#[test]
fn since_reports_what_changed_including_what_went_away() {
    let (store, path) = temp_store();
    let a = store.add(req("A", "one"));
    let mark = store.revision();
    let b = store.add(req("B", "two"));

    let changed = store.since(mark);
    assert_eq!(changed.changed.len(), 1);
    assert_eq!(changed.changed[0].id, b.id);

    // Dismissing is a change. A client told only about additions would keep drawing a toast
    // the person closed from the notification centre.
    let mark = store.revision();
    assert!(store.dismiss(&a.id));
    let changed = store.since(mark);
    assert_eq!(changed.changed.len(), 1);
    assert_eq!(changed.changed[0].id, a.id);
    assert!(changed.changed[0].dismissed);

    // And a client that is fully caught up is told nothing at all.
    assert!(store.since(store.revision()).changed.is_empty());
    let _ = std::fs::remove_file(path);
}

#[test]
fn replaces_id_updates_in_place_and_keeps_the_id() {
    let (store, path) = temp_store();
    let first = store.add(req("Downloads", "debian.iso — 40%"));
    let second = store.add(AddRequest {
        app: "Downloads".into(),
        title: "debian.iso — finished".into(),
        replaces_id: Some(first.id.clone()),
        ..Default::default()
    });
    assert_eq!(second.id, first.id);
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].title, "debian.iso — finished");
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_unknown_replaces_id_becomes_a_new_notification() {
    // Senders hold ids across this service's restarts. Dropping the message would lose it to
    // protect a field.
    let (store, path) = temp_store();
    let stored = store.add(AddRequest {
        app: "Chromium".into(),
        title: "Download complete".into(),
        replaces_id: Some("9999".into()),
        ..Default::default()
    });
    assert_eq!(stored.id, "1");
    assert_eq!(store.list().len(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn dismissed_notifications_are_pruned_after_a_week() {
    let (store, path) = temp_store();
    let old = store.add(req("A", "last month"));
    store.dismiss(&old.id);
    // Backdate it by hand: the store has no clock injection and a test that sleeps for a
    // week is not a test.
    {
        let mut state = store.lock();
        state.notifications[0].created_at = (Utc::now()
            - chrono::Duration::seconds(DISMISSED_TTL_SECS + 60))
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    }
    store.add(req("A", "today"));
    let (_showing, held) = store.held();
    assert_eq!(held, 1, "the month-old dismissed one should be gone");
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_store_is_bounded_at_max_stored() {
    let (store, path) = temp_store();
    for i in 0..(MAX_STORED + 25) {
        store.add(req("Flood", &format!("{i}")));
    }
    let (_showing, held) = store.held();
    assert_eq!(held, MAX_STORED);
    // The oldest went, not the newest.
    assert_eq!(store.list()[0].title, format!("{}", MAX_STORED + 24));
    let _ = std::fs::remove_file(path);
}

#[test]
fn an_action_must_exist_before_it_can_be_invoked() {
    let (store, path) = temp_store();
    let n = store.add(AddRequest {
        app: "Downloads".into(),
        title: "done".into(),
        actions: vec![NotificationAction::new("open_folder", "Open folder")],
        ..Default::default()
    });
    assert!(store.invoke(&n.id, "delete_everything").is_err());
    let invoked = store.invoke(&n.id, "open_folder").expect("the action exists");
    assert!(invoked.dismissed, "pressing a button finishes with the message");
    assert!(store.invoke("nope", "open_folder").is_err());
    let _ = std::fs::remove_file(path);
}

#[test]
fn the_store_has_no_opinion_about_do_not_disturb() {
    // DND decides whether a toast pops, which is a question about the screen. If the store
    // dropped notifications under DND, turning it on would silently destroy messages and the
    // notification centre would be empty for the hours somebody was concentrating.
    let (store, path) = temp_store();
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/store.rs"))
        .expect("the store can read its own source");
    // Only the code above the test module: this test necessarily names the thing it forbids,
    // and a check that fails on its own assertion message proves nothing.
    let code = source
        .split(concat!("#[cfg(", "test)]"))
        .next()
        .unwrap_or_default();
    let mentions: Vec<&str> = code
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .filter(|l| l.contains("do_not_disturb") || l.contains("dnd"))
        .collect();
    assert!(
        mentions.is_empty(),
        "the store must not branch on Do Not Disturb: {mentions:?}"
    );
    store.add(AddRequest {
        app: "A".into(),
        title: "quiet hours".into(),
        urgency: Urgency::Low,
        ..Default::default()
    });
    assert_eq!(store.unread(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn who_sent_it_is_stored_beside_what_they_said_and_survives_a_restart() {
    // Notification 134 on 22 September had `app: "Yantrik"` and nothing else about its
    // sender; the mind that posted it was in `ps` the whole time. Both facts are kept now,
    // and a replacement carries its own sender rather than inheriting the first one's.
    let (store, path) = temp_store();
    let hermes = Sender {
        claimed: Some("Yantrik".into()),
        verified: "python -m hermes_cli.main gateway run (pid 689)".into(),
        pid: 689,
        exe: "/home/yantrik/.hermes/hermes-agent/venv/bin/python".into(),
        desktop: false,
    };
    let first = store.add_from(req("hermes_cli.main", "Studio finished"), Some(hermes.clone()));
    assert_eq!(first.sender.as_ref(), Some(&hermes));

    let shell = Sender {
        claimed: Some("Yantrik".into()),
        verified: "yantrik-ui config.yaml (pid 7456)".into(),
        pid: 7456,
        exe: "/opt/yantrik/bin/yantrik-ui".into(),
        desktop: true,
    };
    let replaced = store.add_from(
        AddRequest { replaces_id: Some(first.id.clone()), ..req("Yantrik", "Update available") },
        Some(shell.clone()),
    );
    assert_eq!(replaced.id, first.id);
    assert_eq!(replaced.sender.as_ref(), Some(&shell));
    drop(store);

    let reopened = Store::open(path.clone());
    assert_eq!(reopened.list()[0].sender.as_ref(), Some(&shell));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_store_written_before_senders_existed_still_loads_whole() {
    // The file on every machine that took the update. One unknown-field error here would
    // start the store empty and report the person's whole history as unparseable.
    let (_store, path) = temp_store();
    std::fs::write(
        &path,
        r#"{"revision":237,"next_id":135,"notifications":[{"id":"134","app":"Yantrik","title":"Studio finished","body":"","urgency":"normal","created_at":"2026-09-23T00:43:53Z","read":false,"dismissed":false,"actions":[],"source":"yantrik","revision":237}]}"#,
    )
    .unwrap();
    let store = Store::open(path.clone());
    assert_eq!(store.load_notice(), None);
    let list = store.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].sender, None, "nothing is invented about an old record");
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_file_that_cannot_be_parsed_is_reported_not_hidden() {
    let (_store, path) = temp_store();
    std::fs::write(&path, "{ this is not json").unwrap();
    let store = Store::open(path.clone());
    assert!(store.list().is_empty());
    assert!(
        store.load_notice().is_some_and(|n| n.contains("could not be parsed")),
        "a store that began empty by accident has to say so"
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn mark_read_clears_the_badge_without_removing_anything() {
    let (store, path) = temp_store();
    store.add(req("A", "one"));
    let b = store.add(req("B", "two"));
    assert_eq!(store.unread(), 2);
    assert_eq!(store.mark_read(Some(&b.id)), 1);
    assert_eq!(store.unread(), 1);
    assert_eq!(store.mark_read(None), 1);
    assert_eq!(store.unread(), 0);
    assert_eq!(store.list().len(), 2, "read is not dismissed");
    // Nothing left to change, so nothing is written and the revision holds still.
    let rev = store.revision();
    assert_eq!(store.mark_read(None), 0);
    assert_eq!(store.revision(), rev);
    let _ = std::fs::remove_file(path);
}
