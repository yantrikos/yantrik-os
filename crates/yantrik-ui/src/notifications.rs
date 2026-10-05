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

/// The one name a notification's sender goes by, on a toast, in the centre's group header and on
/// Today's row: `notification_sender::toast_name`, called and not remade.
///
/// Sign-off, 4 October: Today printed `n.app` as it was stored, the centre headed its groups with
/// the same raw field, and only the toast cleaned it and named the verified program beside it.
/// Now all three call the same function, so a sender is called one thing everywhere and a name
/// is "Yantrik" alone only when `notification_sender` says the desktop itself sent it.
pub fn sender_name(n: &Notification) -> String {
    crate::notification_sender::toast_name(n)
}

/// What the centre groups a notification under: its sender's name, so a group's header never
/// names anyone its cards were not sent by. Also what "Clear all from this group" matches.
pub fn group_of(n: &Notification) -> String {
    sender_name(n).to_lowercase()
}

/// Convert one notification to the Slint row.
pub fn to_slint_data(n: &Notification) -> crate::NotificationData {
    // The desktop's agent notices say what happened and keep the prompt in the body; anything
    // else goes through the one title rule (notification_groups.rs, notification_title.rs).
    let (title, body) = crate::notification_groups::display_copy(n);
    let name = sender_name(n);
    crate::NotificationData {
        id: n.id.clone().into(),
        app_name: name.clone().into(),
        summary: title.into(),
        body: body.into(),
        urgency: urgency_int(n.urgency),
        time_ago: crate::bridge::format_time_ago(seconds_since(&n.created_at)).into(),
        is_read: n.read,
        sender_line: sender_line(n).into(),
        sender_short: crate::notification_sender::sender_summary(n).into(),
        is_group_header: false,
        group_name: name.into(),
        group_icon: first_letter(&n.app),
        group_count: 0,
        group_unread: 0,
        is_turn_group: false,
        group_key: slint::SharedString::default(),
        expanded: false,
        in_group: false,
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

/// The one row a run of a mind's turn notices folds into: "Yantrik Mind finished 12 turns ·
/// last 2m ago", pressed to show the cards inside it. Read when every card in it is.
fn turn_group_row(run: &crate::notification_groups::Run, open: bool) -> crate::NotificationData {
    let newest = run.notes.first().map(|n| seconds_since(&n.created_at)).unwrap_or(0.0);
    let ago = crate::bridge::format_time_ago(newest);
    let first = run.notes.first();
    crate::NotificationData {
        id: slint::SharedString::default(),
        app_name: first.map(|n| sender_name(n)).unwrap_or_default().into(),
        summary: run.label(&ago).into(),
        body: slint::SharedString::default(),
        urgency: 1,
        time_ago: ago.into(),
        is_read: run.unread() == 0,
        sender_line: slint::SharedString::default(),
        // Every card in a run passed the same test (the desktop itself), so the newest one's
        // line speaks for the run.
        sender_short: first.map(|n| crate::notification_sender::sender_summary(n)).unwrap_or_default().into(),
        is_group_header: false,
        group_name: slint::SharedString::default(),
        group_icon: slint::SharedString::default(),
        group_count: run.notes.len() as i32,
        group_unread: run.unread() as i32,
        is_turn_group: true,
        group_key: run.key().into(),
        expanded: open,
        in_group: false,
        actions: slint::ModelRc::default(),
        source: first.map(|n| n.source.as_str()).unwrap_or_default().into(),
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

/// The centre's list: grouped by sender, newest group first, newest within a group first, with a
/// synthetic header row before each group. Inside a group, a run of one mind's turn notices folds
/// into one row (`notification_groups::fold`), and its cards follow only when the person has
/// opened it.
///
/// Groups used to be ordered alphabetically, so a notification that arrived a second ago sat
/// under "Zoom" at the bottom of the screen if that was where its app's name fell. They are in
/// the order the senders last said something now, which is the order a person is looking for.
pub fn centre_rows(mirror: &NotificationMirror) -> Vec<crate::NotificationData> {
    let showing = mirror.showing();
    let mut order: Vec<String> = Vec::new();
    for n in &showing {
        let key = group_of(n);
        if !order.contains(&key) {
            order.push(key);
        }
    }

    let mut items: Vec<crate::NotificationData> = Vec::new();
    for key in &order {
        let group: Vec<&Notification> = showing.iter().copied().filter(|n| group_of(n) == *key).collect();
        let Some(first) = group.first() else { continue };
        let name = sender_name(first);
        items.push(crate::NotificationData {
            id: slint::SharedString::default(),
            app_name: name.clone().into(),
            // The words on the header: the mind's name for a group of nothing but its turns
            // (`notification_groups::heading`). `group_name` stays the sender's, because it is
            // what "Clear all from this group" matches against `group_of`.
            summary: crate::notification_groups::heading(&name, &group).into(),
            body: slint::SharedString::default(),
            urgency: 0,
            time_ago: slint::SharedString::default(),
            is_read: true,
            sender_line: slint::SharedString::default(),
            sender_short: slint::SharedString::default(),
            is_group_header: true,
            group_name: name.into(),
            group_icon: first_letter(&first.app),
            // How many there are, and — apart, so a total is never read as unread — how many
            // of them are.
            group_count: group.len() as i32,
            group_unread: group.iter().filter(|n| !n.read).count() as i32,
            is_turn_group: false,
            group_key: slint::SharedString::default(),
            expanded: false,
            in_group: false,
            actions: slint::ModelRc::default(),
            source: first.source.as_str().into(),
        });
        for entry in crate::notification_groups::fold(&group) {
            match entry {
                crate::notification_groups::Entry::One(n) => items.push(to_slint_data(n)),
                crate::notification_groups::Entry::Turns(run) => {
                    let open = run.expanded();
                    items.push(turn_group_row(&run, open));
                    if open {
                        items.extend(run.notes.iter().map(|n| crate::NotificationData {
                            in_group: true,
                            ..to_slint_data(n)
                        }));
                    }
                }
            }
        }
    }
    items
}

/// Today's list: the newest few, flat, with their buttons. The same rows the centre draws for
/// the same notifications, from the same `to_slint_data`, so title, body and sender name agree.
pub fn today_rows(mirror: &NotificationMirror) -> Vec<crate::NotificationData> {
    let showing = mirror.showing();
    showing.iter().take(TODAY_SHOWN).map(|n| to_slint_data(n)).collect()
}

/// Put the centre's list, Today's list and the badge on screen.
pub fn sync_to_ui(mirror: &NotificationMirror, ui_weak: &slint::Weak<crate::App>) {
    use slint::ComponentHandle;
    let Some(ui) = ui_weak.upgrade() else { return };
    ui.global::<crate::TodayState>()
        .set_notifications(slint::ModelRc::new(slint::VecModel::from(today_rows(mirror))));
    ui.set_notification_unread_count(mirror.unread_count() as i32);
    ui.set_notification_service_up(mirror.service_up());
    ui.set_notification_service_notice(mirror.notice().unwrap_or_default().into());
    ui.set_notification_list(slint::ModelRc::new(slint::VecModel::from(centre_rows(mirror))));
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

    fn sent_by(desktop: bool, claimed: Option<&str>, verified: &str, pid: i32, exe: &str) -> Option<Sender> {
        Some(Sender { claimed: claimed.map(Into::into), verified: verified.into(), pid, exe: exe.into(), desktop })
    }

    /// One of each sender the sign-off saw, and the ways a program has tried to pass for the
    /// desktop. Newest last, as the store keeps them.
    fn senders() -> Vec<Notification> {
        let shell = "/opt/yantrik/bin/yantrik-ui";
        let mut out = Vec::new();
        let mut push = |id: &str, app: &str, sender: Option<Sender>, source: Source| {
            let mut n = note(id, app, &format!("2026-10-04T09:{:02}:00Z", out.len()));
            n.sender = sender;
            n.source = source;
            out.push(n);
        };
        push("1", "Yantrik", sent_by(true, Some("Yantrik"), "yantrik-ui (pid 4)", 4, shell), Source::Yantrik);
        push("2", "Yantrik Companion", sent_by(true, Some("Yantrik Companion"), "yantrik-ui (pid 4)", 4, shell), Source::Yantrik);
        // Notification 134: a mind that said "Yantrik", filed before the service refused it.
        let hermes = "python -m hermes_cli.main gateway run (pid 689)";
        push("3", "Yantrik", sent_by(false, Some("Yantrik"), hermes, 689, "/venv/bin/python"), Source::Yantrik);
        // A binary called Yantrik that gave no name, filed under its own.
        push("4", "Yantrik", sent_by(false, None, "Yantrik (pid 77)", 77, "/tmp/Yantrik"), Source::Yantrik);
        // A bridge the shell started, and a caller nothing could be established about.
        push("5", "Yantrik", sent_by(false, Some("Yantrik"), "yantrik-ui (pid 9)", 9, shell), Source::Yantrik);
        push("6", "Yantrik", sent_by(false, Some("Yantrik"), "could not be identified", 0, ""), Source::Yantrik);
        // An old record of ours with no sender, and a name over D-Bus.
        push("7", "Yantrik", None, Source::Yantrik);
        push("8", "Yantrik", None, Source::Freedesktop);
        out
    }

    fn mirror_of(items: Vec<Notification>) -> NotificationMirror {
        let mut mirror = NotificationMirror::new();
        mirror.items = items;
        mirror
    }

    /// The group header a card in the centre's list sits under.
    fn header_over(rows: &[crate::NotificationData], id: &str) -> String {
        let at = rows.iter().position(|r| r.id == id).expect("the card is in the centre");
        rows[..at].iter().rev().find(|r| r.is_group_header).expect("under a header").group_name.to_string()
    }

    #[test]
    fn today_and_the_centre_call_a_sender_the_same_thing() {
        let notes = senders();
        let mirror = mirror_of(notes.clone());
        let centre = centre_rows(&mirror);
        // Today shows five; look at all of them by showing them five at a time.
        for chunk in notes.chunks(TODAY_SHOWN) {
            let today = today_rows(&mirror_of(chunk.to_vec()));
            for n in chunk {
                let row = today.iter().find(|r| r.id == n.id.as_str()).expect("on Today");
                let card = centre.iter().find(|r| r.id == n.id.as_str()).expect("in the centre");
                assert_eq!(row.app_name.as_str(), sender_name(n), "Today's name is the toast's, for {}", n.id);
                assert_eq!(card.app_name, row.app_name, "the centre's card agrees, for {}", n.id);
                assert_eq!(header_over(&centre, &n.id), row.app_name.as_str(), "and its group header, for {}", n.id);
                assert_eq!((card.summary.as_str(), card.body.as_str()), (row.summary.as_str(), row.body.as_str()));
            }
        }
        // The desktop's two names are its own, as it filed them.
        assert_eq!(sender_name(&notes[0]), "Yantrik");
        assert_eq!(sender_name(&notes[1]), "Yantrik Companion");
    }

    #[test]
    fn a_sender_that_is_not_the_desktop_is_never_shown_as_yantrik() {
        let notes = senders();
        let mirror = mirror_of(notes.clone());
        let centre = centre_rows(&mirror);
        for n in &notes[2..] {
            let name = sender_name(n);
            assert!(!name.trim().eq_ignore_ascii_case("Yantrik"), "{} is called {name:?}", n.id);
            assert!(!header_over(&centre, &n.id).trim().eq_ignore_ascii_case("Yantrik"), "{} sits under the desktop's header", n.id);
        }
        // Nor does one share the desktop's group: the "Yantrik" header holds the desktop's alone.
        let desktop_group = centre.iter().find(|r| r.is_group_header && r.group_name.as_str() == "Yantrik").unwrap();
        assert_eq!(desktop_group.group_count, 1, "only notification 1 is the desktop's \"Yantrik\"");
        assert_eq!(desktop_group.summary.as_str(), "Yantrik", "and is headed with its name");
    }

    /// The desktop's group of a mind's turns is headed with the mind's name, the way its fold line
    /// names it; it is still keyed and cleared as the desktop's.
    #[test]
    fn a_group_of_a_minds_turns_is_headed_with_the_minds_name() {
        let shell = "/opt/yantrik/bin/yantrik-ui";
        let turns: Vec<Notification> = (1..=3)
            .map(|i| {
                let mut n = note(&i.to_string(), "Yantrik", &format!("2026-10-04T09:0{i}:00Z"));
                n.title = "Yantrik Mind replied".into();
                n.sender = sent_by(true, Some("Yantrik"), "yantrik-ui (pid 4)", 4, shell);
                n.actions = vec![yantrik_ipc_contracts::notifications::NotificationAction::new("show_agent", "Open")];
                n
            })
            .collect();
        let centre = centre_rows(&mirror_of(turns.clone()));
        let header = centre.iter().find(|r| r.is_group_header).unwrap();
        assert_eq!(header.summary.as_str(), "Yantrik Mind");
        assert_eq!(header.group_name.as_str(), "Yantrik", "Clear all still matches the desktop's group");
        let fold = centre.iter().find(|r| r.is_turn_group).unwrap();
        assert!(fold.summary.starts_with("Yantrik Mind finished 3 turns"), "{}", fold.summary);
    }

    #[test]
    fn a_card_cut_in_two_is_titled_by_its_first_sentence_in_both_places() {
        // The sign-off's card, as the old poster filed it.
        let mut n = note("9", "Yantrik Companion", "2026-10-04T08:00:00Z");
        n.title = "Okay, this one's actually good: your calendar shows WebGL debugging at 9:30 and\u{2026}".into();
        n.body = "a 'browser/arcade session' at 10:00.".into();
        let mirror = mirror_of(vec![n]);
        let today = &today_rows(&mirror)[0];
        let card = centre_rows(&mirror).into_iter().find(|r| r.id == "9").unwrap();
        assert!(today.summary.chars().count() <= crate::notification_title::TITLE_CHARS, "{}", today.summary);
        assert!(today.body.starts_with("Okay, this one's actually good"), "{}", today.body);
        assert!(today.body.ends_with("'browser/arcade session' at 10:00."), "{}", today.body);
        assert_eq!((card.summary, card.body), (today.summary.clone(), today.body.clone()));
    }
}
