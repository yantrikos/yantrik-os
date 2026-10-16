//! Mind notices in the one store (design/initiative-inbox-2026-10-16.md).
//!
//! A notice is a notification with a `notice` record beside it, posted by the harness host on a
//! harness's behalf and honoured only from the desktop (see `main.rs`). This file is the store's
//! half: dedupe and supersede keyed by the harness the host established, the per-harness rate cap,
//! the grant decision against the day's budget (`grant`), the person's feedback and each harness's
//! durable cursor over it, and the settings the decision reads.
//!
//! Every answer that says `stored: true` is given after the file is on the disk; a write that fails
//! takes the card back out and answers "try again".

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use yantrik_ipc_contracts::notifications::notice::{
    self, answer, ActionKind, Feedback, FeedbackKind, FamilySetting, HarnessBinding, Initiative, Notice,
    NoticeHarness, NoticePost,
};
use yantrik_ipc_contracts::notifications::{Notification, Sender, Source, Urgency};

use crate::grant;
use crate::store::{prune, snapshot, OnDisk, Store};

/// Interrupts granted on one local day.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// `YYYY-MM-DD` in the machine's local time.
    pub day: String,
    pub used: u32,
}

/// Feedback waiting for the harness that sent the notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub harness: String,
    pub item: Feedback,
}

/// How long a harness over its 200 undismissed is told to wait: the person has to dismiss some.
const UNDISMISSED_RETRY_SECS: u64 = 600;

/// The harness id this OS ships a mind under, and the account that mind runs as (#411). Nothing
/// else may use that id, bound or not, so it cannot be taken before the Mind first posts.
pub const MIND_ID: &str = "mind";
pub const MIND_ACCOUNT: &str = "yantrik-mind";

/// The uid of `account`, from `/etc/passwd`. `None` on a machine without it.
pub fn account_uid(account: &str) -> Option<u32> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == account).then_some(())?;
        fields.nth(1)?.parse().ok()
    })
}

/// Bind every harness id a store written before bindings existed holds notices under, to the
/// account and program of its newest one. Run when the store is opened, so the upgrade has no
/// window in which an id with queued feedback is unbound (the security delta review on #831).
pub(crate) fn bind_existing(state: &mut OnDisk) {
    let bound: std::collections::BTreeSet<String> = state.bindings.keys().cloned().collect();
    // Oldest first, so the newest record under an id is the one that stays.
    for n in &state.notifications {
        let Some(harness) = n.notice.as_ref().map(|x| &x.harness) else { continue };
        if bound.contains(&harness.id) || (harness.uid.is_none() && harness.program.is_empty()) {
            continue;
        }
        state.bindings.insert(harness.id.clone(), HarnessBinding { uid: harness.uid, program: harness.program.clone() });
    }
}

/// Whether `id` may be used by `uid` running `program`, as [`HarnessBinding`] says. Binds an
/// unbound id when `bind` is set and there is something to bind it to. An id reserved for an
/// account answers to that account only, whatever is bound.
fn check_binding(
    state: &mut OnDisk,
    reserved: &std::collections::BTreeMap<String, u32>,
    id: &str,
    uid: Option<u32>,
    program: &str,
    bind: bool,
) -> Result<(), String> {
    if let Some(&account) = reserved.get(id) {
        if uid != Some(account) {
            return Err(format!("the harness id `{id}` is reserved for the account this OS runs it as; attach under an id of your own"));
        }
        if state.bindings.get(id).is_none_or(|b| b.uid != Some(account)) {
            state.bindings.insert(id.to_string(), HarnessBinding { uid: Some(account), program: String::new() });
        }
        return Ok(());
    }
    match state.bindings.get(id) {
        Some(bound) if bound.admits(uid, program) => Ok(()),
        Some(_) => Err(format!(
            "the harness id `{id}` belongs to another program or account on this machine; attach under an id of your own"
        )),
        None => {
            if bind && (uid.is_some() || !program.is_empty()) {
                state.bindings.insert(id.to_string(), HarnessBinding { uid, program: program.to_string() });
            }
            Ok(())
        }
    }
}

/// A rate cap over `cards`: `per_hour` stored in the last hour, and `undismissed` still showing.
/// `None` when under both, else the answer to give.
fn over_cap(cards: &[&Notification], now: DateTime<Utc>, per_hour: usize, undismissed: usize, who: &str) -> Option<serde_json::Value> {
    let hour_ago = now - chrono::Duration::hours(1);
    let in_hour: Vec<DateTime<Utc>> = cards.iter().filter_map(|n| parsed(&n.created_at)).filter(|t| *t > hour_ago).collect();
    if in_hour.len() >= per_hour {
        let oldest = in_hour.iter().min().copied().unwrap_or(now);
        let wait = (oldest + chrono::Duration::hours(1) - now).num_seconds().max(1) as u64;
        return Some(answer::retry(wait, &format!("{who} sent {per_hour} notices in the last hour")));
    }
    if cards.iter().filter(|n| !n.dismissed).count() >= undismissed {
        return Some(answer::retry(UNDISMISSED_RETRY_SECS, &format!("{who} {undismissed} notices the person has not dismissed")));
    }
    None
}

/// Whether a card was granted an interrupt, now or when quiet hours end, for an obligation.
fn interrupting_obligation(n: &Notification, hour_ago: DateTime<Utc>) -> bool {
    n.notice.as_ref().is_some_and(|x| {
        x.family.is_obligation() && (x.granted == notice::Attention::Interrupt || x.held_until.is_some())
    }) && parsed(&n.created_at).is_some_and(|t| t > hour_ago)
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parsed(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t).ok().map(|t| t.with_timezone(&Utc))
}

/// Record one response to the notice at `index`: on its own record, and in the queue its
/// harness reads. Nothing for a notification that is not a notice. The caller persists.
pub(crate) fn push_feedback(state: &mut OnDisk, index: usize, kind: FeedbackKind, now: DateTime<Utc>) {
    let seq = state.next_feedback_seq;
    let Some(record) = state.notifications.get_mut(index).and_then(|n| n.notice.as_mut()) else {
        return;
    };
    let item = Feedback {
        seq,
        notice_id: record.notice_id.clone(),
        kind,
        seen_at: record.shown_at.clone(),
        opened_at: record.opened_at.clone(),
        at: rfc3339(now),
    };
    record.feedback.push(item.clone());
    if record.feedback.len() > notice::MAX_FEEDBACK_PER_NOTICE {
        record.feedback.remove(0);
    }
    let harness = record.harness.id.clone();
    state.next_feedback_seq += 1;
    state.feedback_queue.push(Queued { harness, item });
    if state.feedback_queue.len() > notice::MAX_FEEDBACK_QUEUED {
        let excess = state.feedback_queue.len() - notice::MAX_FEEDBACK_QUEUED;
        state.feedback_queue.drain(0..excess);
    }
}

/// The name a notice is filed under: what the harness called itself, unless that borrows the
/// desktop's own name, which is never granted to a mind (#114); then its id.
fn filed_as(harness: &NoticeHarness) -> String {
    let name = harness.name.trim();
    if name.is_empty() || crate::names::borrows_the_desktops_name(name) {
        harness.id.clone()
    } else {
        name.to_string()
    }
}

impl Store {
    /// Store a notice the desktop posted on a harness's behalf, and answer as the wire says.
    ///
    /// `home` is the person's home, which an `open_file` button must resolve under.
    pub fn add_notice(&self, post: NoticePost, home: &std::path::Path) -> serde_json::Value {
        let now = Utc::now();
        let harness = post.harness;
        if harness.id.trim().is_empty() {
            return answer::refused("a notice must name the harness that sent it");
        }
        let mut checked = match notice::check(&post.notice) {
            Ok(checked) => checked,
            Err(why) => return answer::refused(&why),
        };
        if let Some(action) = &checked.action {
            if action.kind == ActionKind::OpenFile && notice::file_under(&action.reference, home).is_none() {
                checked.action = None;
                checked.clamped.push("action".into());
            }
        }

        let mut state = self.lock();
        // Before anything is said about what this id already holds.
        let reserved = self.reserved();
        if let Err(why) = check_binding(&mut state, &reserved, &harness.id, harness.uid, &harness.program, true) {
            tracing::warn!(harness = %harness.id, uid = ?harness.uid, program = %harness.program, "a notice under a harness id bound to another program");
            return answer::refused(&why);
        }
        let mine = |n: &Notification, id: &str| n.notice.as_ref().is_some_and(|x| x.harness.id == harness.id && x.notice_id == id);

        // The same id from the same harness is the same card: answered as stored, unchanged.
        if !post.os_id && state.notifications.iter().any(|n| mine(n, &checked.notice_id)) {
            return answer::stored(&checked.notice_id, true, &[]);
        }

        // The rate caps: per harness id, per account, and for every harness together. An id is
        // the harness's own word, so the per-id cap alone would give a process that attaches as
        // `x1` … `xN` N allowances.
        let notices: Vec<&Notification> = state.notifications.iter().filter(|n| n.notice.is_some()).collect();
        let theirs: Vec<&Notification> =
            notices.iter().copied().filter(|n| n.notice.as_ref().is_some_and(|x| x.harness.id == harness.id)).collect();
        let account: Vec<&Notification> =
            notices.iter().copied().filter(|n| n.notice.as_ref().is_some_and(|x| x.harness.uid == harness.uid)).collect();
        let capped = over_cap(&theirs, now, notice::PER_HOUR, notice::MAX_UNDISMISSED, "this harness has")
            .or_else(|| over_cap(&account, now, notice::PER_HOUR_PER_UID, notice::MAX_UNDISMISSED_PER_UID, "the harnesses of this account have"))
            .or_else(|| over_cap(&notices, now, notice::PER_HOUR_ALL, notice::MAX_UNDISMISSED_ALL, "every harness together has"));
        if let Some(answer) = capped {
            return answer;
        }
        let hour_ago = now - chrono::Duration::hours(1);
        let obligations_here = theirs.iter().filter(|n| interrupting_obligation(n, hour_ago)).count();
        let obligations_all = notices.iter().filter(|n| interrupting_obligation(n, hour_ago)).count();

        state.revision += 1;
        let revision = state.revision;
        if post.os_id {
            checked.notice_id = format!("os-{revision}");
        }

        // Supersede only this harness's own card. One that names any other — another harness's,
        // or one this store does not hold — changes nothing and is named, the same either way, so
        // the answer says nothing about what other minds have sent.
        let mut superseded = None;
        if let Some(target) = &checked.supersedes {
            match state.notifications.iter().position(|n| mine(n, target) && !n.dismissed) {
                Some(index) => {
                    let old = &mut state.notifications[index];
                    old.dismissed = true;
                    old.read = true;
                    old.revision = revision;
                    superseded = Some(old.id.clone());
                }
                None => checked.clamped.push("supersedes".into()),
            }
        }

        let local: DateTime<Local> = now.with_timezone(&Local);
        let today = local.format("%Y-%m-%d").to_string();
        if state.budget.day != today {
            state.budget = Budget { day: today, used: 0 };
        }
        let mut decision = grant::decide(&checked.family, &checked.attention, &state.initiative, local, state.budget.used);
        // An obligation skips the day's budget because the person asked for it; but which family a
        // card is in is the mind's own claim. Past an hour's allowance, it waits in the inbox.
        let would_interrupt = decision.granted == notice::Attention::Interrupt || decision.held_until.is_some();
        if checked.family.is_obligation()
            && would_interrupt
            && (obligations_here >= notice::OBLIGATION_INTERRUPTS_PER_HOUR || obligations_all >= notice::OBLIGATION_INTERRUPTS_PER_HOUR_ALL)
        {
            decision = grant::Decision { granted: notice::Attention::Inbox, reason: notice::GrantReason::Rate, held_until: None, spends: false };
        }
        if decision.spends {
            state.budget.used += 1;
        }

        let id = state.next_id;
        state.next_id += 1;
        let app = filed_as(&harness);
        let record = Notification {
            id: id.to_string(),
            app: app.clone(),
            title: checked.title,
            body: checked.body,
            urgency: Urgency::Normal,
            created_at: rfc3339(now),
            read: false,
            dismissed: false,
            // The one button is the notice's own, executed by the shell from an allowlist; the
            // notification's own buttons forward to an app's control surface, which a mind's
            // card never may.
            actions: Vec::new(),
            source: Source::Yantrik,
            replaces_id: None,
            // The mind is speaking, not the desktop that posted it: the record says who the host
            // established, and is never marked the desktop.
            sender: Some(Sender {
                claimed: Some(if harness.name.trim().is_empty() { harness.id.clone() } else { harness.name.clone() }),
                verified: harness.verified.clone(),
                pid: 0,
                exe: String::new(),
                desktop: false,
            }),
            notice: Some(Notice {
                notice_id: checked.notice_id.clone(),
                family: checked.family,
                why: checked.why,
                evidence: checked.evidence,
                attention: checked.attention,
                useful_until: checked.useful_until,
                supersedes: checked.supersedes,
                action: checked.action,
                harness,
                granted: decision.granted,
                grant_reason: decision.reason,
                held_until: decision.held_until.map(|t| rfc3339(t.with_timezone(&Utc))),
                shown_as: None,
                shown_at: None,
                opened_at: None,
                clamped: checked.clamped.clone(),
                feedback: Vec::new(),
            }),
            revision,
        };
        state.notifications.push(record);
        prune(&mut state, &self.archive_path());
        let snapshot = snapshot(&mut state);
        drop(state);

        if let Err(e) = self.persist(&snapshot) {
            // Not on disk, so not stored: take it back out, give back what it spent, and let the
            // mind try again. The card it superseded stays superseded in memory only until the
            // next successful write, which is the same card the mind will send again.
            let mut state = self.lock();
            state.notifications.retain(|n| n.id != id.to_string());
            if decision.spends {
                state.budget.used = state.budget.used.saturating_sub(1);
            }
            if let Some(old) = superseded {
                for n in state.notifications.iter_mut().filter(|n| n.id == old) {
                    n.dismissed = false;
                }
            }
            return answer::retry(notice::RETRY_UNAVAILABLE_SECS, &format!("the store could not be written: {e}"));
        }
        tracing::info!(id, notice = %checked.notice_id, app = %app, "notice stored");
        let mut reply = answer::stored(&checked.notice_id, false, &checked.clamped);
        reply["id"] = id.to_string().into();
        reply
    }

    /// The person pressed a feedback button on a card. Answers the feedback as recorded.
    pub fn record_feedback(&self, id: &str, kind: FeedbackKind) -> Result<Feedback, String> {
        if !FeedbackKind::BUTTONS.contains(&kind) {
            return Err(format!("`{}` is not a feedback button", kind.as_str()));
        }
        let now = Utc::now();
        let mut state = self.lock();
        let Some(index) = state.notifications.iter().position(|n| n.id == id && n.notice.is_some()) else {
            return Err(format!("no notice with id `{id}`"));
        };
        let family = state.notifications[index].notice.as_ref().map(|x| x.family.clone()).expect("a notice");
        // The two that are settings: the OS applies them itself, whether or not the mind does.
        match kind {
            FeedbackKind::NeverFamily => {
                let setting = if family.is_obligation() { FamilySetting::InboxOnly } else { FamilySetting::Off };
                let _ = state.initiative.set(&family, setting);
            }
            FeedbackKind::InboxOnly => {
                let _ = state.initiative.set(&family, FamilySetting::InboxOnly);
            }
            _ => {}
        }
        state.revision += 1;
        let revision = state.revision;
        let n = &mut state.notifications[index];
        n.read = true;
        if kind.takes_the_card_away() {
            n.dismissed = true;
        }
        n.revision = revision;
        push_feedback(&mut state, index, kind, now);
        let recorded = state.notifications[index].notice.as_ref().and_then(|x| x.feedback.last().cloned()).expect("just pushed");
        let snapshot = snapshot(&mut state);
        drop(state);
        self.persist(&snapshot)?;
        Ok(recorded)
    }

    /// The shell showed a card (`toast` or `inbox`), or opened its button. The first showing is
    /// the one recorded: a card seen in the inbox is not toasted afterwards.
    pub fn mark_shown(&self, id: &str, shown_as: Option<&str>, opened: bool) -> Result<(), String> {
        let now = rfc3339(Utc::now());
        let mut state = self.lock();
        let revision = state.revision + 1;
        let Some(n) = state.notifications.iter_mut().find(|n| n.id == id) else {
            return Err(format!("no notice with id `{id}`"));
        };
        let Some(record) = n.notice.as_mut() else {
            return Err(format!("notification `{id}` is not a notice"));
        };
        let mut changed = false;
        if let Some(how) = shown_as.filter(|h| matches!(*h, "toast" | "inbox")) {
            if record.shown_as.is_none() {
                record.shown_as = Some(how.to_string());
                record.shown_at = Some(now.clone());
                changed = true;
            }
        }
        if opened && record.opened_at.is_none() {
            record.opened_at = Some(now);
            changed = true;
        }
        if !changed {
            return Ok(());
        }
        n.revision = revision;
        state.revision = revision;
        let snapshot = snapshot(&mut state);
        drop(state);
        self.persist(&snapshot)
    }

    /// The feedback waiting for `harness`, after taking `ack` as read. Feedback is delivered at
    /// least once: until a later poll acknowledges it, it comes back, across restarts.
    ///
    /// Only to the account and program the id is bound to ([`Binding`]): anything else is refused,
    /// and nothing it sends is taken as an acknowledgement.
    pub fn feedback_for(&self, harness: &str, uid: Option<u32>, program: &str, ack: Option<u64>) -> Result<Vec<Feedback>, String> {
        let reserved = self.reserved();
        let mut state = self.lock();
        check_binding(&mut state, &reserved, harness, uid, program, false)?;
        let cursor = state.feedback_cursor.get(harness).copied().unwrap_or(0);
        if let Some(ack) = ack.filter(|a| *a > cursor) {
            state.feedback_cursor.insert(harness.to_string(), ack);
            state.feedback_queue.retain(|q| q.harness != harness || q.item.seq > ack);
            let snapshot = snapshot(&mut state);
            let pending = Self::pending(&state, harness);
            drop(state);
            if let Err(e) = self.persist(&snapshot) {
                // The cursor is not on disk, so what was acknowledged may come again after a
                // restart: at least once, never lost.
                tracing::warn!(harness, error = %e, "the feedback cursor could not be written");
            }
            return Ok(pending);
        }
        Ok(Self::pending(&state, harness))
    }

    fn pending(state: &OnDisk, harness: &str) -> Vec<Feedback> {
        let cursor = state.feedback_cursor.get(harness).copied().unwrap_or(0);
        state
            .feedback_queue
            .iter()
            .filter(|q| q.harness == harness && q.item.seq > cursor)
            .map(|q| q.item.clone())
            .collect()
    }

    /// Take every card past its `useful_until` out of the inbox, telling its sender it expired.
    /// The record stays (dismissed) and is archived with the rest when it ages out.
    pub fn expire_due(&self, now: DateTime<Utc>) {
        let mut state = self.lock();
        let due: Vec<usize> = state
            .notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| !n.dismissed)
            .filter(|(_, n)| {
                n.notice.as_ref().and_then(|x| x.useful_until.as_deref()).and_then(parsed).is_some_and(|t| t <= now)
            })
            .map(|(i, _)| i)
            .collect();
        if due.is_empty() {
            return;
        }
        state.revision += 1;
        let revision = state.revision;
        for &index in &due {
            let n = &mut state.notifications[index];
            n.dismissed = true;
            n.revision = revision;
            push_feedback(&mut state, index, FeedbackKind::Expired, now);
        }
        let snapshot = snapshot(&mut state);
        drop(state);
        let _ = self.persist(&snapshot);
    }

    /// Forget what `harness` is bound to, so the next notice under it binds it afresh. The
    /// person's way out when something took an id first. Answers whether there was a binding.
    pub fn unbind(&self, harness: &str) -> Result<bool, String> {
        let mut state = self.lock();
        if state.bindings.remove(harness).is_none() {
            return Ok(false);
        }
        state.revision += 1;
        let snapshot = snapshot(&mut state);
        drop(state);
        self.persist(&snapshot)?;
        Ok(true)
    }

    /// Every harness id and what it is bound to.
    pub fn bindings(&self) -> std::collections::BTreeMap<String, HarnessBinding> {
        self.lock().bindings.clone()
    }

    /// The person's initiative settings.
    pub fn initiative(&self) -> Initiative {
        self.lock().initiative.clone()
    }

    /// Change them. The store holds them, so the decision and the screen read the same settings.
    pub fn set_initiative(&self, change: impl FnOnce(&mut Initiative) -> Result<(), String>) -> Result<Initiative, String> {
        let mut state = self.lock();
        let mut next = state.initiative.clone();
        change(&mut next)?;
        if next == state.initiative {
            return Ok(next);
        }
        state.initiative = next.clone();
        // A revision, so the shell's poll (which carries the settings) is not the only one that
        // hears: `since` answers with them every time.
        state.revision += 1;
        let snapshot = snapshot(&mut state);
        drop(state);
        self.persist(&snapshot)?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU32, Ordering};
    use yantrik_ipc_contracts::notifications::notice::{Attention, Family, GrantReason};
    use yantrik_ipc_contracts::notifications::{AddRequest, MAX_STORED};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_store() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "yantrik-notices-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::open(dir.join("notifications.json")), dir)
    }

    fn harness(id: &str) -> NoticeHarness {
        NoticeHarness { id: id.into(), uid: Some(1000), verified: format!("{id} (pid 7)"), name: id.into(), program: String::new() }
    }

    fn post(from: &str, notice: serde_json::Value) -> NoticePost {
        NoticePost { notice, harness: harness(from), os_id: false }
    }

    fn notice(id: &str, family: &str, attention: &str) -> serde_json::Value {
        json!({ "notice_id": id, "family": family, "title": "Stretch", "why": "You asked.", "attention": attention })
    }

    fn home() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn stored_means_on_disk_and_survives_a_restart() {
        let (store, dir) = temp_store();
        let answer = store.add_notice(post("mind", notice("rem-1", "reminder", "interrupt")), &home());
        assert_eq!(answer["stored"], true, "{answer}");
        assert_eq!(answer["notice_id"], "rem-1");
        drop(store);
        let reopened = Store::open(dir.join("notifications.json"));
        let n = reopened.list().into_iter().find(|n| n.notice.is_some()).expect("the card is still there");
        assert_eq!(n.notice.unwrap().notice_id, "rem-1");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dedupe_is_by_harness_and_notice_id() {
        let (store, dir) = temp_store();
        store.add_notice(post("mind", notice("n-1", "machine", "inbox")), &home());
        let again = store.add_notice(post("mind", notice("n-1", "discovery", "interrupt")), &home());
        assert_eq!(again, json!({ "stored": true, "notice_id": "n-1", "duplicate": true }));
        assert_eq!(store.list().len(), 1, "the stored card is unchanged");
        let other = store.add_notice(post("hermes", notice("n-1", "machine", "inbox")), &home());
        assert!(other.get("duplicate").is_none(), "the same id from another harness is a separate card");
        assert_eq!(store.list().len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn supersedes_replaces_only_this_harnesss_own_card() {
        let (store, dir) = temp_store();
        store.add_notice(post("hermes", notice("theirs", "machine", "inbox")), &home());
        store.add_notice(post("mind", notice("mine", "machine", "inbox")), &home());
        let mut n = notice("mine-2", "machine", "inbox");
        n["supersedes"] = "mine".into();
        let ok = store.add_notice(post("mind", n), &home());
        assert!(ok.get("clamped").is_none(), "{ok}");
        let showing: Vec<String> = store.list().iter().filter_map(|n| n.notice.as_ref().map(|x| x.notice_id.clone())).collect();
        assert!(!showing.contains(&"mine".into()) && showing.contains(&"mine-2".into()), "{showing:?}");

        let mut n = notice("mine-3", "machine", "inbox");
        n["supersedes"] = "theirs".into();
        let answer = store.add_notice(post("mind", n), &home());
        assert_eq!(answer["clamped"], json!(["supersedes"]), "{answer}");
        assert!(store.list().iter().any(|n| n.notice.as_ref().is_some_and(|x| x.notice_id == "theirs")), "another harness's card stays");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_budget_is_shared_by_every_harness_and_kept_in_the_store() {
        let (store, dir) = temp_store();
        let reason = |store: &Store, id: &str| {
            store.list().into_iter().find_map(|n| n.notice.filter(|x| x.notice_id == id)).map(|x| (x.granted, x.grant_reason)).unwrap()
        };
        store.add_notice(post("mind", notice("a", "discovery", "interrupt")), &home());
        store.add_notice(post("hermes", notice("b", "calendar", "interrupt")), &home());
        drop(store);
        // Across a restart, too.
        let store = Store::open(dir.join("notifications.json"));
        store.add_notice(post("mind", notice("c", "machine", "interrupt")), &home());
        assert_eq!(reason(&store, "a"), (Attention::Interrupt, GrantReason::Granted));
        assert_eq!(reason(&store, "b"), (Attention::Interrupt, GrantReason::Granted));
        assert_eq!(reason(&store, "c"), (Attention::Inbox, GrantReason::BudgetSpent));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_harness_past_its_hourly_cap_is_told_when_to_retry() {
        let (store, dir) = temp_store();
        for i in 0..notice::PER_HOUR {
            assert_eq!(store.add_notice(post("mind", notice(&format!("n{i}"), "machine", "inbox")), &home())["stored"], true);
        }
        let over = store.add_notice(post("mind", notice("one-more", "machine", "inbox")), &home());
        assert_eq!(over["stored"], false);
        assert!(over["retry_after_secs"].as_u64().is_some_and(|s| s > 0 && s <= 3600), "{over}");
        // Another harness has its own cap.
        assert_eq!(store.add_notice(post("hermes", notice("h1", "machine", "inbox")), &home())["stored"], true);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_46_word_body_is_stored_cut_with_clamped_recorded() {
        let (store, dir) = temp_store();
        let mut n = notice("long", "delegated", "inbox");
        n["body"] = (1..=46).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ").into();
        n["action"] = json!({ "kind": "format_disk", "ref": "/dev/sda", "label": "Do it" });
        let answer = store.add_notice(post("mind", n), &home());
        assert_eq!(answer["clamped"], json!(["body", "action"]), "{answer}");
        let stored = store.list().into_iter().find(|n| n.notice.is_some()).unwrap();
        assert_eq!(stored.body.split_whitespace().count(), 45);
        assert_eq!(stored.notice.as_ref().unwrap().clamped, vec!["body", "action"]);
        assert!(stored.notice.unwrap().action.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn feedback_goes_to_the_sender_only_and_comes_back_until_acked_across_a_restart() {
        let (store, dir) = temp_store();
        let answer = store.add_notice(post("mind", notice("rem-1", "reminder", "interrupt")), &home());
        let id = answer["id"].as_str().unwrap().to_string();
        store.mark_shown(&id, Some("toast"), false).unwrap();
        let recorded = store.record_feedback(&id, FeedbackKind::NotNow).unwrap();
        assert!(recorded.seen_at.is_some(), "seen_at is when it was shown");
        assert!(store.feedback_for("hermes", Some(1000), "", None).unwrap().is_empty(), "nobody else hears it");
        assert_eq!(store.feedback_for("mind", Some(1000), "", None).unwrap().len(), 1);
        drop(store);
        let store = Store::open(dir.join("notifications.json"));
        let again = store.feedback_for("mind", Some(1000), "", None).unwrap();
        assert_eq!(again.len(), 1, "still there after a restart");
        assert_eq!(again[0].kind, FeedbackKind::NotNow);
        assert!(store.feedback_for("mind", Some(1000), "", Some(again[0].seq)).unwrap().is_empty(), "acknowledged");
        drop(store);
        let store = Store::open(dir.join("notifications.json"));
        assert!(store.feedback_for("mind", Some(1000), "", None).unwrap().is_empty(), "and the cursor survived the restart");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn never_this_kind_and_inbox_only_change_the_settings() {
        let (store, dir) = temp_store();
        let a = store.add_notice(post("mind", notice("d1", "discovery", "interrupt")), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&a, FeedbackKind::NeverFamily).unwrap();
        assert_eq!(store.initiative().setting(&Family::Discovery), FamilySetting::Off);
        let r = store.add_notice(post("mind", notice("r1", "reminder", "interrupt")), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&r, FeedbackKind::NeverFamily).unwrap();
        assert_eq!(store.initiative().setting(&Family::Reminder), FamilySetting::InboxOnly, "a reminder is never Off");
        // A family now off is stored hidden and still answered as stored.
        let hidden = store.add_notice(post("mind", notice("d2", "discovery", "interrupt")), &home());
        assert_eq!(hidden["stored"], true);
        let record = store.list().into_iter().find_map(|n| n.notice.filter(|x| x.notice_id == "d2")).unwrap();
        assert!(record.hidden());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dismissed_or_expired_card_tells_its_sender() {
        let (store, dir) = temp_store();
        let id = store.add_notice(post("mind", notice("x", "machine", "inbox")), &home())["id"].as_str().unwrap().to_string();
        store.dismiss(&id);
        let mut n = notice("y", "calendar", "inbox");
        n["useful_until"] = "2020-01-01T00:00:00Z".into();
        store.add_notice(post("mind", n), &home());
        store.since(0);
        let kinds: Vec<FeedbackKind> = store.feedback_for("mind", Some(1000), "", None).unwrap().into_iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![FeedbackKind::Dismissed, FeedbackKind::Expired]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn six_hundred_notify_sends_do_not_evict_an_acknowledged_notice() {
        let (store, dir) = temp_store();
        store.add_notice(post("mind", notice("rem-keep", "reminder", "interrupt")), &home());
        for i in 0..600 {
            store.add(AddRequest { app: "notify-send".into(), title: format!("spam {i}"), ..Default::default() });
        }
        assert!(
            store.list().iter().any(|n| n.notice.as_ref().is_some_and(|x| x.notice_id == "rem-keep")),
            "the notice was pushed out by plain notifications"
        );
        assert_eq!(store.held().1, MAX_STORED + 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dismissed_notice_past_the_ttl_is_in_the_archive_not_gone() {
        let (store, dir) = temp_store();
        let id = store.add_notice(post("mind", notice("old", "delegated", "inbox")), &home())["id"].as_str().unwrap().to_string();
        store.dismiss(&id);
        {
            let mut state = store.lock();
            state.notifications[0].created_at = rfc3339(Utc::now() - chrono::Duration::days(8));
        }
        store.add(AddRequest { app: "a".into(), title: "today".into(), ..Default::default() });
        assert!(store.get(&id).is_none(), "it left the live store");
        let archive = std::fs::read_to_string(dir.join("notices-archive.jsonl")).expect("an archive");
        assert!(archive.contains("\"notice_id\":\"old\""), "{archive}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_plain_text_notice_gets_an_os_made_id() {
        let (store, dir) = temp_store();
        let answer = store.add_notice(
            NoticePost { notice: notice("os", "delegated", "inbox"), harness: harness("hermes"), os_id: true },
            &home(),
        );
        assert!(answer["notice_id"].as_str().unwrap().starts_with("os-"), "{answer}");
        // Two of them are two cards, not a duplicate.
        let second = store.add_notice(
            NoticePost { notice: notice("os", "delegated", "inbox"), harness: harness("hermes"), os_id: true },
            &home(),
        );
        assert_ne!(second["notice_id"], answer["notice_id"]);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn post_as(from: &str, uid: u32, program: &str, notice: serde_json::Value) -> NoticePost {
        let mut harness = harness(from);
        harness.uid = Some(uid);
        harness.program = program.into();
        NoticePost { notice, harness, os_id: false }
    }

    #[test]
    fn a_non_desktop_caller_cannot_dismiss_or_mark_read_a_notice() {
        let (store, dir) = temp_store();
        let id = store.add_notice(post("mind", notice("rem-1", "reminder", "interrupt")), &home())["id"].as_str().unwrap().to_string();
        let plain = store.add(AddRequest { app: "a".into(), title: "plain".into(), ..Default::default() });
        assert!(store.dismiss_by(&id, false).is_err());
        assert!(store.mark_read_by(Some(&id), false).is_err());
        // Everything at once leaves the notice where it was, and takes the plain one.
        assert_eq!(store.mark_read_by(None, false), Ok(1));
        assert_eq!(store.dismiss_all_by(false), 1);
        let card = store.get(&id).unwrap();
        assert!(!card.dismissed && !card.read, "the card is still live and unread");
        assert!(store.get(&plain.id).unwrap().dismissed);
        assert!(store.feedback_for("mind", Some(1000), "", None).unwrap().is_empty(), "no feedback was forged");
        // The person, on the desktop, still can.
        assert_eq!(store.dismiss_by(&id, true), Ok(true));
        assert_eq!(store.feedback_for("mind", Some(1000), "", None).unwrap()[0].kind, FeedbackKind::Dismissed);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_list_for_anyone_but_the_desktop_has_no_feedback_evidence_or_button() {
        let (store, dir) = temp_store();
        let mut n = notice("d1", "discovery", "inbox");
        n["evidence"] = json!(["seen on the page"]);
        n["action"] = json!({ "kind": "open_url", "ref": "https://example.com/" });
        let id = store.add_notice(post("mind", n), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&id, FeedbackKind::Useful).unwrap();
        let theirs = store.list_for(false).into_iter().find_map(|n| n.notice).unwrap();
        assert!(theirs.feedback.is_empty() && theirs.evidence.is_empty() && theirs.action.is_none(), "{theirs:?}");
        let desktop = store.list_for(true).into_iter().find_map(|n| n.notice).unwrap();
        assert!(!desktop.feedback.is_empty() && !desktop.evidence.is_empty() && desktop.action.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_harness_id_belongs_to_the_account_and_program_that_first_used_it() {
        let (store, dir) = temp_store();
        assert_eq!(store.add_notice(post_as("mind", 999, "yantrik-mind", notice("rem-1", "reminder", "inbox")), &home())["stored"], true);
        // Another account attaching as `mind` while the Mind is away: refused before it hears
        // anything about what `mind` holds, and it cannot read or acknowledge the Mind's feedback.
        let id = store.list().into_iter().find(|n| n.notice.is_some()).unwrap().id;
        store.record_feedback(&id, FeedbackKind::NotNow).unwrap();
        let impostor = store.add_notice(post_as("mind", 1000, "python3", notice("rem-1", "reminder", "inbox")), &home());
        assert!(impostor["refused"].is_string() && impostor.get("duplicate").is_none(), "{impostor}");
        let next = store.add_notice(post_as("mind", 1000, "python3", notice("rem-2", "reminder", "inbox")), &home());
        assert!(next["refused"].is_string(), "{next}");
        assert!(store.feedback_for("mind", Some(1000), "python3", Some(u64::MAX)).is_err());
        // The same account under another program is refused too.
        assert!(store.add_notice(post_as("mind", 999, "bash", notice("rem-3", "reminder", "inbox")), &home())["refused"].is_string());
        // The Mind itself still reads its feedback, unacknowledged.
        assert_eq!(store.feedback_for("mind", Some(999), "yantrik-mind", None).unwrap().len(), 1);
        drop(store);
        let store = Store::open(dir.join("notifications.json"));
        assert!(store.add_notice(post_as("mind", 1000, "python3", notice("rem-4", "reminder", "inbox")), &home())["refused"].is_string(), "the binding survives a restart");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_rollback_to_a_build_from_before_notices_loses_nothing() {
        // The driven test of #831 on VM 520: an older service rewrites notifications.json from its
        // own struct, dropping every field it does not know.
        let (store, dir) = temp_store();
        let id = store.add_notice(post_as("mind", 999, "", notice("rem-1", "reminder", "interrupt")), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&id, FeedbackKind::NotNow).unwrap();
        store.set_initiative(|s| { s.quiet = true; Ok(()) }).unwrap();
        drop(store);
        let main = dir.join("notifications.json");
        let written = std::fs::read_to_string(&main).unwrap();
        assert!(!written.contains("\"notice\"") && !written.contains("initiative"), "the main file holds only what an old build knows: {written}");

        // The old build, as it would: its own three fields, a notification of its own, written back.
        let mut old: serde_json::Value = serde_json::from_str(&written).unwrap();
        old.as_object_mut().unwrap().retain(|k, _| ["revision", "next_id", "notifications"].contains(&k.as_str()));
        let next = old["next_id"].as_u64().unwrap();
        old["notifications"].as_array_mut().unwrap().push(json!({
            "id": next.to_string(), "app": "a", "title": "while rolled back", "body": "", "urgency": "normal",
            "created_at": "2026-10-16T12:00:00Z", "read": false, "dismissed": false, "actions": [], "source": "yantrik", "revision": 1
        }));
        old["next_id"] = (next + 1).into();
        std::fs::write(&main, old.to_string()).unwrap();

        // Forward again.
        let store = Store::open(main.clone());
        let card = store.get(&id).unwrap();
        let record = card.notice.expect("the notice record is joined back");
        assert_eq!(record.feedback.len(), 1);
        assert_eq!(record.harness.uid, Some(999));
        assert!(store.initiative().quiet, "the person's settings survived");
        assert_eq!(store.feedback_for("mind", Some(999), "", None).unwrap().len(), 1, "and the feedback queue");
        assert!(store.get(&next.to_string()).is_some_and(|n| n.notice.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_notice_record_whose_notification_is_gone_goes_to_the_archive() {
        let (store, dir) = temp_store();
        store.add_notice(post("mind", notice("rem-1", "reminder", "inbox")), &home());
        drop(store);
        // An old build that started empty, say because it could not read the file.
        std::fs::write(dir.join("notifications.json"), r#"{"revision":1,"next_id":1,"notifications":[]}"#).unwrap();
        let store = Store::open(dir.join("notifications.json"));
        assert!(store.list().is_empty());
        let archive = std::fs::read_to_string(dir.join("notices-archive.jsonl")).expect("an archive");
        assert!(archive.contains("\"orphan_of\"") && archive.contains("rem-1"), "{archive}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_accounts_uid_is_read_from_the_password_file() {
        assert_eq!(account_uid("root"), Some(0));
        assert_eq!(account_uid("no-such-account-here"), None);
    }

    #[test]
    fn since_for_anyone_but_the_desktop_is_redacted_as_list_is() {
        let (store, dir) = temp_store();
        let mut n = notice("d1", "discovery", "inbox");
        n["evidence"] = json!(["SECRET-EVIDENCE"]);
        n["action"] = json!({ "kind": "open_url", "ref": "https://example.com/" });
        let id = store.add_notice(post("mind", n), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&id, FeedbackKind::Useful).unwrap();
        let theirs = store.since_for(0, false);
        let card = theirs.changed.iter().find_map(|n| n.notice.clone()).unwrap();
        assert!(card.feedback.is_empty() && card.evidence.is_empty() && card.action.is_none(), "{card:?}");
        assert!(theirs.harness_bindings.is_none(), "the bindings are the desktop's");
        let desktop = store.since_for(0, true);
        assert!(desktop.changed.iter().find_map(|n| n.notice.clone()).is_some_and(|c| !c.evidence.is_empty() && !c.feedback.is_empty()));
        assert!(desktop.harness_bindings.unwrap().contains_key("mind"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_store_from_before_bindings_binds_its_ids_when_opened() {
        let (store, dir) = temp_store();
        let id = store.add_notice(post_as("mind", 999, "", notice("rem-1", "reminder", "inbox")), &home())["id"].as_str().unwrap().to_string();
        store.record_feedback(&id, FeedbackKind::NotNow).unwrap();
        // As a store written before this change: the records, the queued feedback, no bindings.
        {
            let mut state = store.lock();
            state.bindings.clear();
            let snapshot = snapshot(&mut state);
            drop(state);
            store.persist(&snapshot).unwrap();
        }
        drop(store);
        let store = Store::open(dir.join("notifications.json"));
        assert!(store.feedback_for("mind", Some(1000), "python3", Some(u64::MAX)).is_err(), "no upgrade window");
        assert_eq!(store.feedback_for("mind", Some(999), "", None).unwrap().len(), 1, "unacknowledged");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_minds_id_cannot_be_squatted_and_any_other_can_be_forgotten() {
        let (store, dir) = temp_store();
        store.reserve("mind", 999);
        // Something else gets there first: refused, and the Mind is not locked out.
        assert!(store.add_notice(post_as("mind", 1000, "python3", notice("r1", "reminder", "inbox")), &home())["refused"].is_string());
        assert_eq!(store.add_notice(post_as("mind", 999, "", notice("r1", "reminder", "inbox")), &home())["stored"], true);
        // Another id taken first by the wrong program: the person forgets the binding, and the
        // next notice under it binds it afresh.
        store.add_notice(post_as("hermes", 1000, "python3", notice("h1", "machine", "inbox")), &home());
        assert!(store.add_notice(post_as("hermes", 1000, "hermes", notice("h2", "machine", "inbox")), &home())["refused"].is_string());
        assert_eq!(store.unbind("hermes"), Ok(true));
        assert_eq!(store.add_notice(post_as("hermes", 1000, "hermes", notice("h2", "machine", "inbox")), &home())["stored"], true);
        assert_eq!(store.bindings()["hermes"].program, "hermes");
        assert_eq!(store.unbind("nobody"), Ok(false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn new_harness_ids_do_not_buy_one_account_more_notices() {
        let (store, dir) = temp_store();
        let mut stored = 0;
        for i in 0..notice::PER_HOUR_PER_UID + 10 {
            let answer = store.add_notice(post(&format!("x{i}"), notice("n", "machine", "inbox")), &home());
            if answer["stored"] == true {
                stored += 1;
            } else {
                assert!(answer["retry_after_secs"].as_u64().is_some(), "{answer}");
            }
        }
        assert_eq!(stored, notice::PER_HOUR_PER_UID);
        // Another account has its own allowance, up to what every harness together may send.
        assert_eq!(store.add_notice(post_as("other", 2000, "", notice("n", "machine", "inbox")), &home())["stored"], true);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn obligations_past_their_hourly_interrupts_wait_in_the_inbox() {
        let (store, dir) = temp_store();
        for i in 0..notice::OBLIGATION_INTERRUPTS_PER_HOUR {
            store.add_notice(post("mind", notice(&format!("r{i}"), "reminder", "interrupt")), &home());
        }
        store.add_notice(post("mind", notice("r-over", "reminder", "interrupt")), &home());
        let record = store.list().into_iter().find_map(|n| n.notice.filter(|x| x.notice_id == "r-over")).unwrap();
        assert_eq!((record.granted, record.grant_reason), (Attention::Inbox, GrantReason::Rate));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eviction_takes_from_the_harness_holding_the_most() {
        let (store, dir) = temp_store();
        store.add_notice(post_as("mind", 999, "", notice("rem-keep", "reminder", "inbox")), &home());
        {
            // A flood already stored, older than the hour so no rate cap stands in the way.
            let mut state = store.lock();
            let template = state.notifications[0].clone();
            let old = rfc3339(Utc::now() - chrono::Duration::hours(2));
            for i in 0..notice::MAX_NOTICES_STORED {
                let mut n = template.clone();
                n.id = format!("f{i}");
                n.created_at = old.clone();
                let record = n.notice.as_mut().unwrap();
                record.notice_id = format!("f{i}");
                record.harness = harness(&format!("x{}", i % 3));
                state.notifications.push(n);
            }
            // The Mind's card is the oldest of all.
            state.notifications[0].created_at = rfc3339(Utc::now() - chrono::Duration::hours(3));
        }
        store.add(AddRequest { app: "a".into(), title: "prune now".into(), ..Default::default() });
        assert!(
            store.list().iter().any(|n| n.notice.as_ref().is_some_and(|x| x.notice_id == "rem-keep")),
            "the one card of the harness holding the fewest was pushed out"
        );
        assert_eq!(store.list().iter().filter(|n| n.notice.is_some()).count(), notice::MAX_NOTICES_STORED);
        let _ = std::fs::remove_dir_all(dir);
    }
}
