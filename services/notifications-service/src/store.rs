//! The notification store: one file, bounded, and the same one after a restart.
//!
//! ## What was here before
//!
//! `Mutex<Vec<Notification>>`, created empty in `main`. Every notification on this machine died
//! with the process — and since the shell autostarts this service, that meant every reboot.
//! Nothing in the tree called `notifications.add`, so in practice the vector was always empty and
//! nobody noticed.
//!
//! ## What a store has to do that a vector does not
//!
//! **Survive.** `~/.local/share/yantrik/notifications.json`, written to a temp file and renamed
//! over the real one, so a process killed mid-write leaves the previous file intact rather than
//! half a JSON document that the next start cannot parse.
//!
//! **Be bounded.** Anything on this machine can post here, including programs we did not write.
//! Every string is clamped on the way in (see the contract's `MAX_*`), the newest
//! [`MAX_STORED`](yantrik_ipc_contracts::notifications::MAX_STORED) are kept, and a dismissed
//! notification is pruned a week after it was made.
//!
//! **Say what changed.** The shell draws toasts by polling about once a second. Sending it the
//! whole list every second would be the same list every second; `since(revision)` answers with
//! only what moved. Dismissals and reads are changes too — a client told only about additions
//! would keep a toast up for something the person closed in the notification centre.
//!
//! **Not know about Do Not Disturb.** DND decides whether a toast pops, which is a question
//! about the screen, and the screen is the shell's. Everything is stored and counted either way;
//! there is a test that says so, because "under DND, drop it" is the obvious wrong turn and it
//! loses messages permanently.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use yantrik_ipc_contracts::notifications::*;

/// The file as it is written. A struct rather than a bare array because the revision and the id
/// counter have to come back too: without them a restart would hand out id "1" again for a
/// notification the shell already has, and `since` would go backwards.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OnDisk {
    revision: u64,
    next_id: u64,
    notifications: Vec<Notification>,
}

impl Default for OnDisk {
    fn default() -> Self {
        Self {
            revision: 0,
            next_id: 1,
            notifications: Vec::new(),
        }
    }
}

/// The live store. Oldest first in `notifications`, which is the order it is written in.
pub struct Store {
    path: PathBuf,
    state: Mutex<OnDisk>,
    /// Why the file could not be read at start, if it could not. Reported in `describe` rather
    /// than only logged: a store that silently began empty is exactly the failure this service
    /// was rewritten to stop hiding.
    load_notice: Option<String>,
}

/// Where the store lives. `$XDG_DATA_HOME`, then `~/.local/share`, then a temp directory so the
/// service still runs on a machine with no HOME rather than refusing to start.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("yantrik").join("notifications.json")
}

impl Store {
    /// Open the store at `path`, reading back whatever is there.
    pub fn open(path: PathBuf) -> Self {
        let (state, load_notice) = match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<OnDisk>(&text) {
                Ok(mut on_disk) => {
                    // A file written by an older build, or edited by hand, could disagree with
                    // itself. The counters must be at least as large as what is in the list or
                    // the next id collides with an existing one.
                    let highest = on_disk
                        .notifications
                        .iter()
                        .filter_map(|n| n.id.parse::<u64>().ok())
                        .max()
                        .unwrap_or(0);
                    on_disk.next_id = on_disk.next_id.max(highest + 1);
                    let newest_rev = on_disk.notifications.iter().map(|n| n.revision).max();
                    on_disk.revision = on_disk.revision.max(newest_rev.unwrap_or(0));
                    (on_disk, None)
                }
                Err(e) => (
                    OnDisk::default(),
                    Some(format!(
                        "{} could not be parsed ({e}); the store started empty and will \
                         overwrite that file on the next notification",
                        path.display()
                    )),
                ),
            },
            // Not an error: the first start on a machine has no file.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (OnDisk::default(), None),
            Err(e) => (
                OnDisk::default(),
                Some(format!("{} could not be read: {e}", path.display())),
            ),
        };

        Self {
            path,
            state: Mutex::new(state),
            load_notice,
        }
    }

    /// Why the store began empty, when it should not have.
    pub fn load_notice(&self) -> Option<&str> {
        self.load_notice.as_deref()
    }

    /// Where the file is, for `describe` — a person diagnosing this wants the path, not a
    /// promise that persistence happens.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Post a notification, or replace one, from a door that established nothing about the
    /// sender. Answers the notification as stored.
    pub fn add(&self, req: AddRequest) -> Notification {
        self.add_from(req, None)
    }

    /// The same, recording who this machine established sent it.
    ///
    /// The sender is a separate argument and not a field of the request on purpose: a request
    /// is what the caller wrote, and this is the one thing about a notification the caller must
    /// not be able to write. The socket handler fills it in from the kernel's peer credentials;
    /// the freedesktop door passes `None`, because it has not asked the bus (#114).
    pub fn add_from(&self, req: AddRequest, sender: Option<Sender>) -> Notification {
        let mut state = self.lock();
        state.revision += 1;
        let revision = state.revision;
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        let app = clamp(req.app.trim(), MAX_APP);
        let app = if app.is_empty() { "unknown".to_string() } else { app };
        let title = clamp(req.title.trim(), MAX_TITLE);
        let body = clamp(&req.body, MAX_BODY);
        let actions: Vec<NotificationAction> = req
            .actions
            .into_iter()
            .filter(|a| !a.id.trim().is_empty())
            .take(MAX_ACTIONS)
            .map(|a| NotificationAction {
                id: clamp(a.id.trim(), MAX_ACTION_ID),
                label: clamp(a.label.trim(), MAX_ACTION_LABEL),
                args: a.args,
            })
            .collect();

        // Replacing reuses the id, because the freedesktop spec promises the sender that the id
        // it passed as `replaces_id` is the id it can go on closing. Our own senders get the
        // same deal, which is what makes "downloading… / finished" one line on screen and not
        // two.
        if let Some(target) = req.replaces_id.as_deref() {
            if let Some(existing) = state.notifications.iter_mut().find(|n| n.id == target) {
                existing.app = app;
                existing.title = title;
                existing.body = body;
                existing.urgency = req.urgency;
                existing.actions = actions;
                existing.source = req.source;
                // Whoever replaced it is who is speaking now. "downloading…" from the download
                // manager and "finished" from something else are not one line about one file.
                existing.sender = sender;
                existing.created_at = now;
                existing.read = false;
                existing.dismissed = false;
                existing.revision = revision;
                let replaced = existing.clone();
                let snapshot = state.clone();
                drop(state);
                self.persist(&snapshot);
                return replaced;
            }
            // An id nobody knows. The spec leaves this to the server, and the useful answer is a
            // new notification: the alternative is dropping a message because the sender was
            // holding a stale id, which is common after this service restarts.
        }

        let id = state.next_id;
        state.next_id += 1;
        let notification = Notification {
            id: id.to_string(),
            app,
            title,
            body,
            urgency: req.urgency,
            created_at: now,
            read: false,
            dismissed: false,
            actions,
            source: req.source,
            replaces_id: req.replaces_id,
            sender,
            revision,
        };
        state.notifications.push(notification.clone());
        prune(&mut state);
        let snapshot = state.clone();
        drop(state);
        self.persist(&snapshot);
        notification
    }

    /// Everything still showing, newest first.
    pub fn list(&self) -> Vec<Notification> {
        let state = self.lock();
        let mut out: Vec<Notification> = state
            .notifications
            .iter()
            .filter(|n| !n.dismissed)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    /// What changed since `revision`, and where the store is now.
    pub fn since(&self, revision: u64) -> Since {
        let state = self.lock();
        Since {
            revision: state.revision,
            changed: state
                .notifications
                .iter()
                .filter(|n| n.revision > revision)
                .cloned()
                .collect(),
        }
    }

    /// The current revision, without copying anything.
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// How many are unread and not dismissed — the badge.
    pub fn unread(&self) -> usize {
        self.lock()
            .notifications
            .iter()
            .filter(|n| !n.read && !n.dismissed)
            .count()
    }

    /// Everything still showing, oldest first, without copying the dismissed ones.
    pub fn showing(&self) -> Vec<Notification> {
        self.lock()
            .notifications
            .iter()
            .filter(|n| !n.dismissed)
            .cloned()
            .collect()
    }

    /// One notification by id, dismissed or not.
    pub fn get(&self, id: &str) -> Option<Notification> {
        self.lock()
            .notifications
            .iter()
            .find(|n| n.id == id)
            .cloned()
    }

    /// Dismiss one. `false` when there was nothing by that id to dismiss — the caller is told,
    /// rather than being answered "done" over a notification that never existed.
    pub fn dismiss(&self, id: &str) -> bool {
        let mut state = self.lock();
        let revision = state.revision + 1;
        let Some(n) = state
            .notifications
            .iter_mut()
            .find(|n| n.id == id && !n.dismissed)
        else {
            return false;
        };
        n.dismissed = true;
        n.read = true;
        n.revision = revision;
        state.revision = revision;
        let snapshot = state.clone();
        drop(state);
        self.persist(&snapshot);
        true
    }

    /// Dismiss everything showing. Answers how many actually changed.
    pub fn dismiss_all(&self) -> usize {
        let mut state = self.lock();
        let revision = state.revision + 1;
        let mut changed = 0;
        for n in state.notifications.iter_mut().filter(|n| !n.dismissed) {
            n.dismissed = true;
            n.read = true;
            n.revision = revision;
            changed += 1;
        }
        if changed == 0 {
            return 0;
        }
        state.revision = revision;
        let snapshot = state.clone();
        drop(state);
        self.persist(&snapshot);
        changed
    }

    /// Mark one read, or with `None` everything showing. Answers how many changed.
    pub fn mark_read(&self, id: Option<&str>) -> usize {
        let mut state = self.lock();
        let revision = state.revision + 1;
        let mut changed = 0;
        for n in state.notifications.iter_mut() {
            let matches = match id {
                Some(want) => n.id == want,
                None => !n.dismissed,
            };
            if matches && !n.read {
                n.read = true;
                n.revision = revision;
                changed += 1;
            }
        }
        if changed == 0 {
            return 0;
        }
        state.revision = revision;
        let snapshot = state.clone();
        drop(state);
        self.persist(&snapshot);
        changed
    }

    /// Record that an action was invoked, and hand back the notification it was on.
    ///
    /// The store does not carry the action out — it does not know how to open a folder — but it
    /// is the only thing that knows whether that action exists, and the caller needs the
    /// notification's `source` to decide whether a freedesktop `ActionInvoked` has to go out.
    ///
    /// Invoking an action dismisses the notification, which is what the spec's clients expect:
    /// a person who pressed the button is done with the message.
    pub fn invoke(&self, id: &str, action_id: &str) -> Result<Notification, String> {
        let mut state = self.lock();
        let revision = state.revision + 1;
        let Some(n) = state.notifications.iter_mut().find(|n| n.id == id) else {
            return Err(format!("no notification with id `{id}`"));
        };
        if !n.actions.iter().any(|a| a.id == action_id) {
            let offered: Vec<&str> = n.actions.iter().map(|a| a.id.as_str()).collect();
            return Err(if offered.is_empty() {
                format!("notification `{id}` has no actions")
            } else {
                format!(
                    "notification `{id}` has no action `{action_id}`; it offers: {}",
                    offered.join(", ")
                )
            });
        }
        n.dismissed = true;
        n.read = true;
        n.revision = revision;
        let invoked = n.clone();
        state.revision = revision;
        let snapshot = state.clone();
        drop(state);
        self.persist(&snapshot);
        Ok(invoked)
    }

    /// How many are held, showing and dismissed-but-kept.
    pub fn held(&self) -> (usize, usize) {
        let state = self.lock();
        let showing = state.notifications.iter().filter(|n| !n.dismissed).count();
        (showing, state.notifications.len())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OnDisk> {
        // A poisoned lock means some other call panicked mid-mutation. The data is still a valid
        // `OnDisk` — every mutation above completes before it can unwind — and losing every
        // notification on the machine because one call panicked is the worse outcome.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Write the file: temp beside it, then rename over.
    ///
    /// Takes a snapshot rather than the guard so the lock is released before the disk write —
    /// a D-Bus `Notify` arriving during a 5 ms write should not have to wait for it.
    fn persist(&self, state: &OnDisk) {
        if let Some(dir) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                tracing::warn!(dir = %dir.display(), error = %e, "cannot create the notification store directory");
                return;
            }
        }
        let json = match serde_json::to_string(state) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "cannot serialise the notification store");
                return;
            }
        };
        // Named after the process, so two services started by mistake cannot rename each other's
        // half-written temp file over the real one.
        let temp = self.path.with_extension(format!("tmp{}", std::process::id()));
        if let Err(e) = std::fs::write(&temp, json) {
            tracing::warn!(path = %temp.display(), error = %e, "cannot write the notification store");
            return;
        }
        if let Err(e) = std::fs::rename(&temp, &self.path) {
            tracing::warn!(path = %self.path.display(), error = %e, "cannot replace the notification store");
            let _ = std::fs::remove_file(&temp);
        }
    }
}

/// Drop what the store is not allowed to keep: dismissed notifications older than a week, then
/// the oldest of whatever is left over the cap.
fn prune(state: &mut OnDisk) {
    let cutoff = Utc::now().timestamp() - DISMISSED_TTL_SECS;
    state.notifications.retain(|n| {
        if !n.dismissed {
            return true;
        }
        match DateTime::parse_from_rfc3339(&n.created_at) {
            Ok(t) => t.timestamp() > cutoff,
            // A timestamp we cannot read is not a reason to delete somebody's history.
            Err(_) => true,
        }
    });
    if state.notifications.len() > MAX_STORED {
        let excess = state.notifications.len() - MAX_STORED;
        state.notifications.drain(0..excess);
    }
}

#[cfg(test)]
mod tests {
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
}
