//! The shell's view of the one notification store.
//!
//! ## What this used to be
//!
//! A second store. `NotificationStore` kept its own `Vec` in the shell process and wrote it to
//! `~/.yantrik/notifications.json` — a file only the shell could write and only the shell could
//! read. It was fed by the shell's own D-Bus daemon (which raced mako for the bus name) and by
//! `push_toast`, and it was invisible to the notifications service, to every app, and to a mind.
//! Meanwhile the service had a third store, in memory, that nothing ever wrote to.
//!
//! ## What it is now
//!
//! A mirror, not a store. The notifications service owns the file; this holds what the last poll
//! of `notifications.since(revision)` said, so the notification centre and the unread badge can
//! be drawn without a socket call per frame. Nothing here is authoritative: dismissing goes to
//! the service and comes back on the next poll.
//!
//! It also knows whether the service answered, because an empty notification centre and a dead
//! service look identical on screen and mean opposite things.

use std::cell::RefCell;
use std::rc::Rc;

use yantrik_ipc_contracts::notifications::{Notification, Since, Urgency};

/// How many notifications the shell keeps in memory. The service keeps 500; this is the same
/// bound so the notification centre can show everything the store holds without the shell
/// growing without limit if the service's cap is ever raised.
const MAX_MIRRORED: usize = 500;

/// What the last poll said, plus whether there was a last poll.
pub struct NotificationMirror {
    /// Oldest first, as the store hands them over.
    items: Vec<Notification>,
    /// The store revision this mirror is caught up to.
    revision: u64,
    /// `None` when the service answered. Otherwise why it did not, in its own words.
    notice: Option<String>,
    /// Whether a poll has ever succeeded. The first one must not raise 400 toasts for
    /// everything that happened while the machine was off.
    primed: bool,
}

/// Shared handle, kept on the UI thread.
pub type SharedStore = Rc<RefCell<NotificationMirror>>;

/// What one poll means for the toasts: which to raise, and which to take down.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Applied {
    /// New, or replaced by their sender. Each deserves a toast.
    pub fresh: Vec<Notification>,
    /// Dismissed since the last poll, by whatever path — the toast's own ×, the centre's
    /// "Clear all", `yos act notifications dismiss_all`, a button pressed, a sender closing its
    /// own. Whatever toast is up for one of these comes down with it.
    pub gone: Vec<String>,
}

impl Default for NotificationMirror {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationMirror {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            revision: 0,
            // Not "down" and not "up": nothing has been asked yet, and claiming either before
            // the first poll would put a wrong sentence on the notification centre for a second.
            notice: None,
            primed: false,
        }
    }

    /// The revision to ask for next.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Fold in what the store said changed, and answer with what that means for the toasts.
    ///
    /// A notification earns a toast when it is new, or when its `created_at` moved — which is
    /// what a sender replacing an earlier notification does ("downloading…" becoming
    /// "finished"). Marking one read or dismissing it also changes it, and must not re-raise it.
    ///
    /// A notification that comes back dismissed is named in [`Applied::gone`], so that its toast
    /// leaves the screen with it. This used to answer only with what to raise, and the toasts
    /// were taken down only by the shell's own buttons — so `yos act notifications dismiss_all`
    /// emptied the notification centre to "All caught up" while two critical toasts, which never
    /// expire on their own, sat on the screen for notifications that no longer existed, and
    /// nothing a mind could call would press their ×. The store already says what went away
    /// (`Since::changed` includes dismissals for exactly this reason); the shell was not
    /// listening.
    pub fn apply(&mut self, since: Since) -> Applied {
        let first_poll = !self.primed;
        self.primed = true;
        self.notice = None;

        let mut applied = Applied::default();
        for incoming in since.changed {
            if incoming.dismissed {
                applied.gone.push(incoming.id.clone());
            }
            match self.items.iter().position(|e| e.id == incoming.id) {
                Some(index) => {
                    let replaced = self.items[index].created_at != incoming.created_at;
                    if replaced && !incoming.dismissed {
                        applied.fresh.push(incoming.clone());
                    }
                    self.items[index] = incoming;
                }
                None => {
                    if !first_poll && !incoming.dismissed {
                        applied.fresh.push(incoming.clone());
                    }
                    self.items.push(incoming);
                }
            }
        }
        self.revision = since.revision;

        if self.items.len() > MAX_MIRRORED {
            let excess = self.items.len() - MAX_MIRRORED;
            self.items.drain(0..excess);
        }
        applied
    }

    /// The service could not be reached. Said once per outage by the caller, held here so the
    /// notification centre can print it instead of an empty list.
    pub fn unreachable(&mut self, why: String) {
        self.notice = Some(why);
    }

    pub fn service_up(&self) -> bool {
        self.notice.is_none()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Unread and not dismissed — the badge.
    pub fn unread_count(&self) -> usize {
        self.items
            .iter()
            .filter(|n| !n.read && !n.dismissed)
            .count()
    }

    /// Everything still showing, newest first.
    pub fn showing(&self) -> Vec<&Notification> {
        let mut out: Vec<&Notification> = self.items.iter().filter(|n| !n.dismissed).collect();
        out.reverse();
        out
    }

    /// One notification by id, for a click that has to know who sent it.
    pub fn get(&self, id: &str) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    /// Mark one read here and now, so the badge moves on the click rather than on the next
    /// poll. The service is told separately and its answer overwrites this.
    pub fn mark_read_locally(&mut self, id: &str) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.read = true;
        }
    }

    /// Dismiss one here and now, for the same reason.
    pub fn dismiss_locally(&mut self, id: &str) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.dismissed = true;
            n.read = true;
        }
    }

    /// The three most recent, for `describe shell`.
    pub fn latest_for_describe(&self, limit: usize) -> Vec<serde_json::Value> {
        self.showing()
            .into_iter()
            .take(limit)
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "app": n.app,
                    // Who this machine says sent it, beside `app`, which is who they said.
                    "sender": n.sender,
                    "title": n.title,
                    "urgency": n.urgency.as_str(),
                    "read": n.read,
                    "at": n.created_at,
                })
            })
            .collect()
    }
}

/// Seconds since an RFC 3339 timestamp, for "4 minutes ago".
///
/// A timestamp that will not parse reads as "just now" rather than as a wild number: the store
/// writes these itself, so an unparseable one means a hand-edited file, and a row that says
/// "in 54 years" is worse than one that says nothing useful.
pub fn seconds_since(created_at: &str) -> f64 {
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(then) => (chrono::Utc::now().timestamp() - then.timestamp()).max(0) as f64,
        Err(_) => 0.0,
    }
}

/// The urgency as the 0/1/2 the Slint components have always drawn.
pub fn urgency_int(urgency: Urgency) -> i32 {
    urgency.hint_byte() as i32
}

/// The full line about who sent a notification — the approval card's two facts, in the card's
/// words, on one line. The card leads with `notification_sender::sender_summary` and shows this
/// one behind its Details control (VM 520 sweep, 4 October: a pid on every card is a
/// debugger's line, not a person's).
///
/// Notification 134 on 22 September read `Yantrik` and said something false; the mind that
/// sent it was in `ps` the whole time, and the row had no way to say so. The row's name is
/// what the caller said (or, when it said nothing, the program's own name); this line is what
/// the kernel-stamped pid on the socket resolved to, and it repeats the claim only when there
/// was one — so a reader can see a claim and a fact, and whether they agree.
///
/// Empty for a notification with no sender record: one from before this existed, or one from
/// the freedesktop door, which has not asked the bus who was behind it. The row shows nothing
/// rather than a line that would have to guess.
pub fn sender_line(n: &Notification) -> String {
    let Some(sender) = &n.sender else {
        return String::new();
    };
    let claim = match &sender.claimed {
        // One line whatever the caller sent: the card draws this behind Details with word wrap,
        // and a newline in the claim would otherwise draw a line of the caller's choosing.
        Some(name) => format!(
            "\u{201c}{}\u{201d} says the caller \u{b7} ",
            crate::notification_sender::one_line(name)
        ),
        None => String::new(),
    };
    // The command line in it is the caller's argv. The service cleans it now, but a record
    // stored before that was not (third-pass security review of #614).
    let verified = crate::notification_sender::one_line(&sender.verified);
    if sender.pid == 0 {
        // The card's words for the same situation; "verified: could not be identified" would
        // read as if something had been verified.
        return format!("{claim}{verified} by this machine");
    }
    format!("{claim}verified by this machine: {verified}")
}

/// Convert one notification to the Slint row.
pub fn to_slint_data(n: &Notification) -> crate::NotificationData {
    crate::NotificationData {
        id: n.id.clone().into(),
        app_name: n.app.clone().into(),
        summary: n.title.clone().into(),
        body: n.body.clone().into(),
        urgency: urgency_int(n.urgency),
        time_ago: crate::bridge::format_time_ago(seconds_since(&n.created_at)).into(),
        is_read: n.read,
        sender_line: sender_line(n).into(),
        sender_short: crate::notification_sender::sender_summary(n).into(),
        is_group_header: false,
        group_name: n.app.clone().into(),
        group_icon: first_letter(&n.app),
        group_count: 0,
        actions: slint::ModelRc::new(slint::VecModel::from(
            n.actions
                .iter()
                // `default` is the freedesktop action for "the person clicked the notification
                // itself", not a button. It is invoked by tapping the row; drawing it as a
                // button beside the row would offer the same thing twice.
                .filter(|a| a.id != "default")
                .map(|a| crate::NotifActionData {
                    id: a.id.clone().into(),
                    label: a.label.clone().into(),
                })
                .collect::<Vec<_>>(),
        )),
        source: n.source.as_str().into(),
    }
}

fn first_letter(app: &str) -> slint::SharedString {
    app.chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
        .into()
}

/// How many of the newest notifications Today lists (the full history is a click away).
pub const TODAY_SHOWN: usize = 5;

/// Put the whole list on screen: grouped by app, newest group first, newest within a group
/// first, with a synthetic header row before each group.
///
/// Groups used to be ordered alphabetically, so a notification that arrived a second ago sat
/// under "Zoom" at the bottom of the screen if that was where its app's name fell. They are in
/// the order the apps last said something now, which is the order a person is looking for.
pub fn sync_to_ui(mirror: &NotificationMirror, ui_weak: &slint::Weak<crate::App>) {
    use slint::ComponentHandle;
    let Some(ui) = ui_weak.upgrade() else { return };

    let showing = mirror.showing();
    let mut order: Vec<String> = Vec::new();
    for n in &showing {
        let key = n.app.to_lowercase();
        if !order.contains(&key) {
            order.push(key);
        }
    }

    let mut items: Vec<crate::NotificationData> = Vec::new();
    for key in &order {
        let group: Vec<&&Notification> = showing
            .iter()
            .filter(|n| n.app.to_lowercase() == *key)
            .collect();
        let Some(first) = group.first() else { continue };
        items.push(crate::NotificationData {
            id: slint::SharedString::default(),
            app_name: first.app.clone().into(),
            summary: first.app.clone().into(),
            body: slint::SharedString::default(),
            urgency: 0,
            time_ago: slint::SharedString::default(),
            is_read: true,
            sender_line: slint::SharedString::default(),
            sender_short: slint::SharedString::default(),
            is_group_header: true,
            group_name: first.app.clone().into(),
            group_icon: first_letter(&first.app),
            group_count: group.len() as i32,
            actions: slint::ModelRc::default(),
            source: first.source.as_str().into(),
        });
        for n in group {
            items.push(to_slint_data(n));
        }
    }

    // Today's list: the newest few, flat, with their buttons.
    ui.global::<crate::TodayState>().set_notifications(slint::ModelRc::new(slint::VecModel::from(
        showing.iter().take(TODAY_SHOWN).map(|n| to_slint_data(n)).collect::<Vec<_>>(),
    )));
    ui.set_notification_unread_count(mirror.unread_count() as i32);
    ui.set_notification_service_up(mirror.service_up());
    ui.set_notification_service_notice(mirror.notice().unwrap_or_default().into());
    ui.set_notification_list(slint::ModelRc::new(slint::VecModel::from(items)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::{Sender, Source};

    fn note(id: &str, app: &str, created_at: &str) -> Notification {
        Notification {
            id: id.into(),
            app: app.into(),
            title: format!("from {app}"),
            body: String::new(),
            urgency: Urgency::Normal,
            created_at: created_at.into(),
            read: false,
            dismissed: false,
            actions: Vec::new(),
            source: Source::Yantrik,
            replaces_id: None,
            sender: None,
            revision: 1,
        }
    }

    #[test]
    fn the_row_says_who_sent_it_in_the_cards_words() {
        // Notification 134, as the service records it now: filed under the program, the claim
        // beside it, and the verified line the approval card would have shown for the same pid.
        let mut n = note("134", "hermes_cli.main", "2026-09-23T00:43:53Z");
        n.sender = Some(Sender {
            claimed: Some("Yantrik".into()),
            verified: "python -m hermes_cli.main gateway run --replace (pid 689)".into(),
            pid: 689,
            exe: "/home/yantrik/.hermes/hermes-agent/venv/bin/python".into(),
            desktop: false,
        });
        let line = sender_line(&n);
        assert!(line.starts_with("\u{201c}Yantrik\u{201d} says the caller"), "{line}");
        assert!(line.contains("verified by this machine: python -m hermes_cli.main"), "{line}");
        assert!(line.ends_with("(pid 689)"), "{line}");

        // No claim, no claim on the line: the name on the row is the machine's, and the line
        // says only what was verified.
        n.sender = Some(Sender {
            claimed: None,
            verified: "a program started from a terminal: yantrik-terminal (pid 812)".into(),
            pid: 812,
            exe: "/opt/yantrik/bin/yantrik-terminal".into(),
            desktop: false,
        });
        assert_eq!(
            sender_line(&n),
            "verified by this machine: a program started from a terminal: yantrik-terminal (pid 812)"
        );

        // Nothing established is said in the card's words, not as a verification of nothing.
        n.sender = Some(Sender {
            claimed: Some("Yantrik".into()),
            verified: "could not be identified".into(),
            pid: 0,
            exe: String::new(),
            desktop: false,
        });
        assert_eq!(
            sender_line(&n),
            "\u{201c}Yantrik\u{201d} says the caller \u{b7} could not be identified by this machine"
        );

        // An old record, and a freedesktop one: no line, rather than a guess.
        n.sender = None;
        assert_eq!(sender_line(&n), "");
    }

    #[test]
    fn the_first_poll_raises_no_toasts() {
        // Otherwise every boot opens with a wall of toasts for everything that happened while
        // the machine was off — which is how a notification system gets turned off.
        let mut mirror = NotificationMirror::new();
        let applied = mirror.apply(Since {
            revision: 3,
            changed: vec![
                note("1", "Downloads", "2026-09-21T09:00:00Z"),
                note("2", "Calendar", "2026-09-21T09:01:00Z"),
            ],
        });
        assert!(applied.fresh.is_empty());
        assert_eq!(mirror.unread_count(), 2);
        assert_eq!(mirror.revision(), 3);
    }

    #[test]
    fn a_new_notification_after_that_is_a_toast() {
        let mut mirror = NotificationMirror::new();
        mirror.apply(Since { revision: 1, changed: vec![note("1", "A", "2026-09-21T09:00:00Z")] });
        let applied = mirror.apply(Since {
            revision: 2,
            changed: vec![note("2", "B", "2026-09-21T09:05:00Z")],
        });
        assert_eq!(applied.fresh.len(), 1);
        assert_eq!(applied.fresh[0].id, "2");
    }

    #[test]
    fn being_read_or_dismissed_does_not_re_raise_a_toast() {
        let mut mirror = NotificationMirror::new();
        mirror.apply(Since { revision: 1, changed: vec![note("1", "A", "2026-09-21T09:00:00Z")] });
        let mut read = note("1", "A", "2026-09-21T09:00:00Z");
        read.read = true;
        let applied = mirror.apply(Since { revision: 2, changed: vec![read] });
        assert!(applied.fresh.is_empty());
        assert!(applied.gone.is_empty(), "reading a notification does not take its toast down");
        let mut gone = note("1", "A", "2026-09-21T09:00:00Z");
        gone.dismissed = true;
        let applied = mirror.apply(Since { revision: 3, changed: vec![gone] });
        assert!(applied.fresh.is_empty());
        assert_eq!(applied.gone, vec!["1".to_string()], "`dismiss(id)` takes the toast with it");
        assert_eq!(mirror.unread_count(), 0);
        assert!(mirror.showing().is_empty());
    }

    #[test]
    fn dismiss_all_from_outside_the_shell_takes_every_toast_down() {
        // 22 September, from the desk of the VM: `yos act notifications dismiss_all` left the
        // notification centre saying "All caught up" with two critical toasts still on the
        // screen. The poll that brought the dismissals only ever said what to raise, and a
        // critical toast never expires on its own, so the only way down was its own × — which
        // nothing published could press.
        let mut mirror = NotificationMirror::new();
        mirror.apply(Since { revision: 1, changed: vec![] });
        let mut first = note("1", "Yantrik", "2026-09-22T18:00:00Z");
        first.urgency = Urgency::Critical;
        let mut second = note("2", "Yantrik", "2026-09-22T18:01:00Z");
        second.urgency = Urgency::Critical;
        let applied = mirror.apply(Since {
            revision: 2,
            changed: vec![first.clone(), second.clone()],
        });
        assert_eq!(applied.fresh.len(), 2, "both are raised");
        assert!(applied.gone.is_empty());

        // `dismiss_all` marks both in one revision, and the next poll carries both back.
        for n in [&mut first, &mut second] {
            n.dismissed = true;
            n.read = true;
            n.revision = 3;
        }
        let applied = mirror.apply(Since { revision: 3, changed: vec![first, second] });
        assert!(applied.fresh.is_empty(), "a dismissal is not news");
        assert_eq!(applied.gone, vec!["1".to_string(), "2".to_string()]);
        assert!(mirror.showing().is_empty(), "the centre and the toasts agree");
    }

    #[test]
    fn a_replaced_notification_is_raised_again() {
        // "debian.iso — 40%" becoming "debian.iso — finished" is news, and it keeps the same id.
        let mut mirror = NotificationMirror::new();
        mirror.apply(Since {
            revision: 1,
            changed: vec![note("1", "Downloads", "2026-09-21T09:00:00Z")],
        });
        let applied = mirror.apply(Since {
            revision: 2,
            changed: vec![note("1", "Downloads", "2026-09-21T09:07:00Z")],
        });
        assert_eq!(applied.fresh.len(), 1);
        assert_eq!(mirror.showing().len(), 1, "it replaced, it did not add");
    }

    #[test]
    fn a_service_that_did_not_answer_is_not_an_empty_list() {
        let mut mirror = NotificationMirror::new();
        assert!(mirror.service_up(), "nothing has been asked yet");
        mirror.unreachable("the notifications service is unreachable".into());
        assert!(!mirror.service_up());
        assert!(mirror.notice().is_some());
        // And a successful poll clears it without anyone having to remember to.
        mirror.apply(Since { revision: 1, changed: vec![] });
        assert!(mirror.service_up());
    }

    #[test]
    fn the_mirror_is_bounded() {
        let mut mirror = NotificationMirror::new();
        mirror.apply(Since { revision: 1, changed: vec![] });
        for i in 0..(MAX_MIRRORED + 20) {
            mirror.apply(Since {
                revision: i as u64 + 2,
                changed: vec![note(&i.to_string(), "Flood", "2026-09-21T09:00:00Z")],
            });
        }
        assert_eq!(mirror.showing().len(), MAX_MIRRORED);
    }

    #[test]
    fn an_unparseable_timestamp_reads_as_just_now_not_as_a_wild_number() {
        assert_eq!(seconds_since("not a timestamp"), 0.0);
        assert!(seconds_since("2020-01-01T00:00:00Z") > 0.0);
    }

    /// Today lists the five newest, in the store's own newest-first order, so a sixth arriving
    /// pushes the oldest off Today (it stays in the notification centre).
    #[test]
    fn today_lists_the_five_newest_in_the_stores_order() {
        assert_eq!(TODAY_SHOWN, 5);
        let mut mirror = NotificationMirror::new();
        mirror.items = (1..=7).map(|i| note(&i.to_string(), "Files", &format!("2026-10-02T09:0{i}:00Z"))).collect();
        let ids: Vec<&str> = mirror.showing().into_iter().take(TODAY_SHOWN).map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["7", "6", "5", "4", "3"]);
        let source = include_str!("notifications.rs");
        assert!(source.contains("showing.iter().take(TODAY_SHOWN)"), "sync_to_ui hands Today exactly that slice");
    }
}
