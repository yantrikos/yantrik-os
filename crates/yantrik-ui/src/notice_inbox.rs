//! The person's inbox of mind notices, as the shell draws it: the Lens's "For you" section, the
//! toast rule for a granted interrupt, the one button's allowlist, and what `describe shell` says
//! (design/initiative-inbox-2026-10-16.md).
//!
//! Pure: everything here reads the mirror's records and a few facts about the screen, so every
//! rule is tested without a window. `wire::for_you` puts it on screen and talks to the service.
//!
//! The inbox is a filtered view of the one store, not a store of its own: a notification with a
//! `notice` record, not dismissed, not of a family the person switched off, not past its
//! `useful_until`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use yantrik_ipc_contracts::notifications::notice::{self, ActionKind, Attention, NoticeAction};
use yantrik_ipc_contracts::notifications::Notification;

use crate::wire::meeting_brief::BriefMessage;

/// How many cards the For you section draws. The rest are a line saying how many more, and all of
/// them are in the notification centre.
pub const SHOWN: usize = 3;

fn parsed(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t).ok().map(|t| t.with_timezone(&Utc))
}

/// Whether `n` is a live card in the inbox at `now`.
pub fn live(n: &Notification, now: DateTime<Utc>) -> bool {
    let Some(card) = &n.notice else { return false };
    !n.dismissed && !card.hidden() && !card.useful_until.as_deref().and_then(parsed).is_some_and(|t| t <= now)
}

/// Whether the OS granted it an interrupt: at once, or held by quiet hours until they end.
pub fn granted_interrupt(n: &Notification) -> bool {
    n.notice.as_ref().is_some_and(|c| c.granted == Attention::Interrupt || c.held_until.is_some())
}

/// The meeting a desktop calendar card is about, as `<event>:<stamp>`, from a notice id
/// `cal-{early,reminder,brief}:<event>:<stamp>`. The three are cards for one meeting, and the shell
/// toasts one of them: whichever it shows first (see `wire::for_you`). A Mind's notice, whatever
/// id it gives itself, and an id of any other shape have none.
pub fn meeting_key(n: &Notification) -> Option<String> {
    let card = n.notice.as_ref().filter(|c| c.harness.id == notice::OS_HARNESS)?;
    let rest = ["cal-early:", "cal-reminder:", "cal-brief:"].iter().find_map(|p| card.notice_id.strip_prefix(p))?;
    let (event, stamp) = rest.split_once(':')?;
    let stamp_ok = stamp.len() == 13 && stamp.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'T' } else { b.is_ascii_digit() });
    let event_ok = !event.is_empty() && event.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (event_ok && stamp_ok).then(|| rest.to_string())
}

/// Whether `n` is a calendar reminder whose meeting had not started when it was made and has now.
///
/// Such a reminder stays useful in the inbox until the meeting ends (its `useful_until`), but a
/// toast saying "starts in 10 minutes" after the start is wrong, so it is never toasted. This is
/// the one place every release path (unlock, Do not disturb, focus, quiet hours) asks, so the
/// rule holds for all of them. A reminder made at or after the start (a "when it starts" one)
/// is for the start itself and is not touched. The id's time is the calendar's wall clock, so
/// `tz` is the zone the calendar service runs in: the desktop's own.
fn meeting_started_since_made<Tz: chrono::TimeZone>(n: &Notification, now: DateTime<Utc>, tz: &Tz) -> bool {
    let Some(card) = n.notice.as_ref().filter(|c| c.harness.id == notice::OS_HARNESS) else { return false };
    let Some(rest) = card.notice_id.strip_prefix("cal-reminder:") else { return false };
    let Some((_, stamp)) = rest.rsplit_once(':') else { return false };
    let Ok(wall) = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M") else { return false };
    // A repeated wall time is read at its earlier instant, as the calendar's reminders place it. A
    // skipped one (spring forward) has none here, though the reminders move it an hour later: it
    // gives `false`, the safe side, so the ordinary rule applies.
    let Some(start) = tz.from_local_datetime(&wall).earliest().map(|t| t.with_timezone(&Utc)) else { return false };
    let made = parsed(&n.created_at);
    now >= start && made.is_some_and(|made| made < start)
}

/// The live cards, in the order the inbox draws them: unread granted interrupts first, then
/// newest first. `items` is the mirror's, oldest first.
pub fn cards<'a>(items: impl Iterator<Item = &'a Notification>, now: DateTime<Utc>) -> Vec<&'a Notification> {
    let mut live: Vec<&Notification> = items.filter(|n| live(n, now)).collect();
    // Newest first is the store's order reversed; a stable sort then lifts the unread interrupts.
    live.reverse();
    live.sort_by_key(|n| !(granted_interrupt(n) && !n.read));
    live
}

/// What `describe shell` reports, so an agent test can read what the person sees: how many cards,
/// how many unread, and the newest five ids and titles.
pub fn describe<'a>(items: impl Iterator<Item = &'a Notification>, now: DateTime<Utc>, mind: bool) -> serde_json::Value {
    let all: Vec<&Notification> = items.collect();
    let live = cards(all.iter().copied(), now);
    let mut newest: Vec<&Notification> = all.iter().copied().filter(|n| self::live(n, now)).collect();
    newest.reverse();
    serde_json::json!({
        "count": live.len(),
        "unread": live.iter().filter(|n| !n.read).count(),
        // A title can name a file (`Download complete: tokens.json`): a mind gets the notification
        // judge's answer, as `notifications.latest` does (#1247, gate 12 audit).
        "newest": newest.iter().take(5).map(|n| {
            let (title, hidden) = crate::control_window_titles::notice_title(&n.title, mind);
            let mut row = serde_json::json!({ "id": n.id, "notice_id": n.notice.as_ref().map(|c| c.notice_id.clone()), "title": title });
            if hidden {
                row["title_hidden"] = serde_json::json!(true);
            }
            row
        }).collect::<Vec<_>>(),
    })
}

/// What the screen is doing, for the toast rule.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Screen {
    /// Lock, login, boot or onboarding: nothing pops over those.
    pub silent: bool,
    pub do_not_disturb: bool,
    pub focus: bool,
    /// Something is fullscreen. This desktop has no signal for it yet; it is here so the rule is
    /// the spec's and wiring one in is one line.
    pub fullscreen: bool,
    /// A call is active. As `fullscreen`.
    pub call: bool,
}

impl Screen {
    fn holds(&self) -> bool {
        self.silent || self.do_not_disturb || self.focus || self.fullscreen || self.call
    }
}

/// What to do about a card's toast, now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toast {
    /// Show it.
    Now,
    /// Not yet: the screen does not allow it, or quiet hours hold it. Asked again later.
    Hold,
    /// Never: it was not granted, has been seen, or is no longer useful.
    Never,
}

/// Whether a card should be toasted now.
///
/// Only a granted interrupt is ever toasted, and only once: a card already shown (as a toast or in
/// the inbox) is not toasted afterwards. One the screen does not allow is held; when the screen
/// allows it again it is toasted if it is still useful and unseen, and past `useful_until` it
/// stays in the inbox. A calendar reminder whose meeting has started is never toasted (see
/// [`meeting_started_since_made`]).
pub fn toast(n: &Notification, screen: Screen, now: DateTime<Utc>) -> Toast {
    let Some(card) = &n.notice else { return Toast::Never };
    if !granted_interrupt(n) || card.shown_as.is_some() || n.read || !live(n, now) {
        return Toast::Never;
    }
    if meeting_started_since_made(n, now, &chrono::Local) {
        return Toast::Never;
    }
    if card.held_until.as_deref().and_then(parsed).is_some_and(|t| t > now) {
        return Toast::Hold;
    }
    if screen.holds() {
        return Toast::Hold;
    }
    Toast::Now
}

/// What a card's button opens, once its `ref` passed its check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    /// A regular file under the person's home, resolved through symlinks.
    File(PathBuf),
    /// A calendar event id, shown in Calendar once Calendar knows it.
    Event(String),
    /// A Notes id.
    Note(String),
    /// A message in Email, exactly. No notice button names one (the allowlist is files, events,
    /// notes and links); the meeting brief's rows do, and open it through the same opener. Only
    /// the brief can make a [`BriefMessage`], so this variant is the brief's by construction.
    Message(BriefMessage),
    /// An http or https URL, and the host the button shows.
    Url { url: String, host: String },
}

/// Check a card's button against the allowlist, now. Executed by the shell, as the person; no
/// mind takes part. Read again at the press, because a file can become a symlink to anywhere
/// between the card being stored and the button being pressed.
pub fn plan(action: &NoticeAction, home: &Path) -> Result<Opening, String> {
    match &action.kind {
        ActionKind::OpenFile => notice::file_under(&action.reference, home)
            .map(Opening::File)
            .ok_or_else(|| "that file is not a file in your home folder any more".to_string()),
        ActionKind::OpenUrl => notice::url_host(&action.reference)
            .map(|host| Opening::Url { url: action.reference.clone(), host })
            .ok_or_else(|| "that is not a web address".to_string()),
        ActionKind::OpenEvent => Ok(Opening::Event(action.reference.clone())),
        ActionKind::OpenNote => Ok(Opening::Note(action.reference.clone())),
        // Words for the Lens, not something to open: `wire::for_you` puts them in the field.
        ActionKind::AskMind => Err("that button puts words in the ask bar; it opens nothing".to_string()),
        ActionKind::Other(kind) => Err(format!("`{kind}` is not something a card's button can do")),
    }
}

/// The largest file a card's button copies to open. Past it the card says so, and the person
/// opens the file from Files, where they chose it themselves.
pub const MAX_COPY_BYTES: u64 = 32 * 1024 * 1024;

/// A read-only copy of `path`, in `into`, for an app to open: the file a card's button names is
/// opened from here, never in place.
///
/// Two things this settles (the security review on #831, findings 3 and 5). The spec says a card
/// opens a file read-only, and no app here has a read-only mode: the copy is mode 0400, which the
/// Editor answers with "This file is read-only. Use Save As." And the path was judged a moment
/// ago by [`plan`], while the app would resolve it again later: the copy is read from a handle
/// opened now, and refused if that handle is not the file that was judged — a component swapped
/// for a symlink in between is caught here, and nothing after this reads the original.
pub fn read_only_copy(path: &Path, into: &Path, name: &str) -> Result<PathBuf, String> {
    use std::io::Read;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::File::open(path).map_err(|e| format!("cannot read it: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("cannot read it: {e}"))?;
    if !meta.is_file() {
        return Err("it is not a file".into());
    }
    // Judged on the handle, as `home_paths::stat` judges a path: a second name is the same bytes
    // as the other one, which may be a key, and no link says where.
    if std::os::unix::fs::MetadataExt::nlink(&meta) > 1 {
        return Err("it has another name elsewhere, which could be anything".into());
    }
    let opened = std::fs::read_link(format!("/proc/self/fd/{}", std::os::fd::AsRawFd::as_raw_fd(&file)))
        .map_err(|e| format!("cannot tell what was opened: {e}"))?;
    if opened != path {
        return Err("it changed while it was being opened".into());
    }
    if meta.len() > MAX_COPY_BYTES {
        return Err("it is too large to open from a card; open it from Files".into());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(into).map_err(|e| format!("cannot copy it: {e}"))?;
    std::fs::set_permissions(into, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("cannot copy it: {e}"))?;
    let copy = into.join(name);
    let _ = std::fs::remove_file(&copy);
    let mut out = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&copy)
        .map_err(|e| format!("cannot copy it: {e}"))?;
    let mut bytes = Vec::new();
    (&mut file).take(MAX_COPY_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("cannot read it: {e}"))?;
    if bytes.len() as u64 > MAX_COPY_BYTES {
        let _ = std::fs::remove_file(&copy);
        return Err("it is too large to open from a card; open it from Files".into());
    }
    std::io::Write::write_all(&mut out, &bytes).map_err(|e| format!("cannot copy it: {e}"))?;
    Ok(copy)
}

/// The label the button shows. A URL's button shows its host whatever the mind called it, so a
/// label cannot dress one site up as another.
pub fn button_label(action: &NoticeAction) -> String {
    match (&action.kind, notice::url_host(&action.reference)) {
        (ActionKind::OpenUrl, Some(host)) => format!("Open {host}"),
        _ => crate::notification_sender::one_line(&action.label),
    }
}

/// "Why you got this", with its evidence, as one plain block.
pub fn why_line(n: &Notification) -> String {
    let Some(card) = &n.notice else { return String::new() };
    let why = crate::notification_sender::one_line(&card.why);
    if card.evidence.is_empty() {
        return why;
    }
    let evidence: Vec<String> = card.evidence.iter().map(|e| crate::notification_sender::one_line(e)).collect();
    format!("{why} · {}", evidence.join(", "))
}

/// The ids of the buttons a mind's notice carries on its row in Notifications. They begin with
/// `notice:` so the row's press goes to the card's own handlers (`wire::for_you`), not to the
/// notifications service's sender-action path, which only freedesktop and app notices use.
pub const ROW_OPEN: &str = "notice:open";
pub const ROW_ASK: &str = "notice:ask";
pub const ROW_NOT_NOW: &str = "notice:not_now";
pub const ROW_WHY: &str = "notice:why";

/// The buttons a mind's notice shows wherever it is listed: its action, its ask (words for the
/// Lens), "Not now" and "Why you got this", as `(id, label)`. A notice with no card has none.
pub fn row_actions(n: &Notification) -> Vec<(&'static str, String)> {
    let Some(card) = &n.notice else { return Vec::new() };
    let mut out = Vec::new();
    if let Some(a) = &card.action {
        out.push((ROW_OPEN, button_label(a)));
    }
    if let Some(a) = &card.ask {
        out.push((ROW_ASK, button_label(a)));
    }
    out.push((ROW_NOT_NOW, "Not now".to_string()));
    out.push((ROW_WHY, "Why you got this".to_string()));
    out
}

/// The "prepare" moment waiting for the person, for the line under the desktop's greeting: the
/// newest live, unread `prep:` card. It is gone once the card is opened, dismissed (both mark it
/// read or dismissed) or the event starts (`useful_until`, which `live` checks).
pub fn waiting_moment<'a>(items: impl Iterator<Item = &'a Notification>, now: DateTime<Utc>) -> Option<&'a Notification> {
    let mut found: Vec<&Notification> = items
        .filter(|n| !n.read && live(n, now))
        .filter(|n| n.notice.as_ref().is_some_and(|c| c.harness.id == notice::OS_HARNESS && c.notice_id.starts_with("prep:")))
        .collect();
    found.pop()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::notice::{Family, GrantReason, Notice, NoticeHarness};
    use yantrik_ipc_contracts::notifications::{Source, Urgency};

    fn card(id: &str, granted: Attention, read: bool) -> Notification {
        Notification {
            id: id.into(),
            app: "mind".into(),
            title: format!("card {id}"),
            body: String::new(),
            urgency: Urgency::Normal,
            created_at: "2026-10-16T10:00:00Z".into(),
            read,
            dismissed: false,
            actions: vec![],
            source: Source::Yantrik,
            replaces_id: None,
            sender: None,
            image: None,
            notice: Some(Notice {
                notice_id: format!("n-{id}"),
                family: Family::Reminder,
                why: "You asked.".into(),
                evidence: vec![],
                attention: granted.clone(),
                useful_until: None,
                supersedes: None,
                action: None,
                ask: None,
                harness: NoticeHarness { id: "mind".into(), ..Default::default() },
                granted,
                grant_reason: GrantReason::Granted,
                held_until: None,
                shown_as: None,
                shown_at: None,
                opened_at: None,
                clamped: vec![],
                feedback: vec![],
            }),
            revision: 1,
        }
    }

    fn now() -> DateTime<Utc> {
        parsed("2026-10-16T12:00:00Z").unwrap()
    }

    fn prep(id: &str, until: Option<&str>) -> Notification {
        let mut n = card(id, Attention::Interrupt, false);
        let c = n.notice.as_mut().unwrap();
        c.notice_id = format!("prep:{id}");
        c.harness.id = notice::OS_HARNESS.into();
        c.useful_until = until.map(String::from);
        c.action = Some(NoticeAction { kind: ActionKind::OpenNote, reference: "Launch prep".into(), label: "Open Launch prep".into() });
        c.ask = Some(NoticeAction { kind: ActionKind::AskMind, reference: "Make a shot list".into(), label: "Ask Pi for a shot list".into() });
        n
    }

    #[test]
    fn the_desktop_line_shows_while_the_moment_waits_and_goes_on_open_dismiss_or_start() {
        let until = Some("2026-10-16T15:00:00Z");
        let mut n = prep("e1", until);
        assert!(waiting_moment([n.clone()].iter(), now()).is_some(), "waiting");
        assert!(waiting_moment([n.clone()].iter(), parsed("2026-10-16T15:00:00Z").unwrap()).is_none(), "the event started");
        n.read = true;
        assert!(waiting_moment([n.clone()].iter(), now()).is_none(), "opened");
        n.read = false;
        n.dismissed = true;
        assert!(waiting_moment([n].iter(), now()).is_none(), "dismissed");
        let other = card("9", Attention::Interrupt, false);
        assert!(waiting_moment([other].iter(), now()).is_none(), "not a prepare card");
    }

    #[test]
    fn the_notification_row_carries_the_moments_four_actions() {
        let n = prep("e1", None);
        let got: Vec<(&str, String)> = row_actions(&n);
        let ids: Vec<&str> = got.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, [ROW_OPEN, ROW_ASK, ROW_NOT_NOW, ROW_WHY]);
        assert_eq!(got[0].1, "Open Launch prep");
        assert_eq!(got[1].1, "Ask Pi for a shot list");
        assert!(row_actions(&card("1", Attention::Inbox, false)).iter().all(|(i, _)| *i == ROW_NOT_NOW || *i == ROW_WHY));
    }

    #[test]
    fn unread_interrupts_come_first_then_newest_first() {
        let items = [
            card("1", Attention::Interrupt, false),
            card("2", Attention::Inbox, false),
            card("3", Attention::Inbox, false),
            card("4", Attention::Interrupt, true),
        ];
        let order: Vec<&str> = cards(items.iter(), now()).iter().map(|n| n.id.as_str()).collect();
        assert_eq!(order, ["1", "4", "3", "2"]);
    }

    /// Gate 12 audit: `inbox.newest[].title` is the notification judge's too, for a mind only.
    #[test]
    fn the_inbox_title_that_names_a_secret_is_hidden_from_a_mind_only() {
        let mut secret = card("1", Attention::Inbox, false);
        secret.title = "Download complete: tokens.json".into();
        let plain = card("2", Attention::Inbox, false);
        let items = [secret, plain];
        let mind = describe(items.iter(), now(), true);
        assert!(!mind.to_string().contains("tokens"), "{mind}");
        assert!(mind["newest"].as_array().is_some_and(|rows| rows.iter().any(|r| r["title_hidden"] == true)));
        let person = describe(items.iter(), now(), false);
        assert!(person.to_string().contains("tokens.json") && !person.to_string().contains("title_hidden"), "{person}");
    }

    #[test]
    fn hidden_dismissed_and_expired_cards_are_not_in_the_inbox() {
        let mut off = card("1", Attention::Inbox, false);
        off.notice.as_mut().unwrap().grant_reason = GrantReason::FamilyOff;
        let mut gone = card("2", Attention::Inbox, false);
        gone.dismissed = true;
        let mut late = card("3", Attention::Inbox, false);
        late.notice.as_mut().unwrap().useful_until = Some("2026-10-16T11:00:00Z".into());
        let plain = Notification { notice: None, ..card("4", Attention::Inbox, false) };
        let items = [off, gone, late, plain, card("5", Attention::Inbox, false)];
        let d = describe(items.iter(), now(), false);
        assert_eq!(d["count"], 1);
        assert_eq!(d["newest"][0]["notice_id"], "n-5");
    }

    #[test]
    fn a_granted_interrupt_is_toasted_once_and_held_while_the_screen_is_busy() {
        let n = card("1", Attention::Interrupt, false);
        assert_eq!(toast(&n, Screen::default(), now()), Toast::Now);
        for busy in [
            Screen { focus: true, ..Default::default() },
            Screen { fullscreen: true, ..Default::default() },
            Screen { call: true, ..Default::default() },
            Screen { silent: true, ..Default::default() },
        ] {
            assert_eq!(toast(&n, busy, now()), Toast::Hold, "{busy:?}");
        }
        let mut seen = n.clone();
        seen.notice.as_mut().unwrap().shown_as = Some("inbox".into());
        assert_eq!(toast(&seen, Screen::default(), now()), Toast::Never, "seen in the inbox is seen");
        let mut late = n.clone();
        late.notice.as_mut().unwrap().useful_until = Some("2026-10-16T11:59:00Z".into());
        assert_eq!(toast(&late, Screen::default(), now()), Toast::Never, "no longer useful: it stays in the inbox");
        assert_eq!(toast(&card("2", Attention::Inbox, false), Screen::default(), now()), Toast::Never);
    }

    #[test]
    fn quiet_hours_hold_a_card_until_they_end() {
        let mut n = card("1", Attention::Inbox, false);
        n.notice.as_mut().unwrap().held_until = Some("2026-10-16T13:00:00Z".into());
        assert_eq!(toast(&n, Screen::default(), now()), Toast::Hold);
        assert_eq!(toast(&n, Screen::default(), parsed("2026-10-16T13:00:01Z").unwrap()), Toast::Now);
    }

    /// Gate 13, step 4(e): a reminder granted at T-11 under the lock and released at T+1.
    #[test]
    fn a_reminder_held_under_the_lock_is_not_toasted_once_its_meeting_has_started() {
        let mut n = desktop_card("cal-reminder:orion:20261116T1030");
        {
            let c = n.notice.as_mut().unwrap();
            c.useful_until = Some("2026-11-16T11:00:00Z".into());
        }
        n.created_at = "2026-11-16T10:19:00Z".into();
        let at = |t: &str| parsed(t).unwrap();
        // Locked at T-11 (granted, held until the lock ends): nothing is decided yet.
        assert!(meeting_started_since_made(&n, at("2026-11-16T10:31:00Z"), &Utc));
        assert!(!meeting_started_since_made(&n, at("2026-11-16T10:29:59Z"), &Utc));
        // Unlocked at T+1: no toast, and the card is still in the inbox until the meeting ends.
        assert!(live(&n, at("2026-11-16T10:31:00Z")));
        assert_eq!(cards([&n].into_iter(), at("2026-11-16T10:31:00Z")).len(), 1);
        assert!(!meeting_started_since_made(&n, at("2026-11-16T10:20:00Z"), &Utc), "before the start it is toasted as ever");
        // Any other reason to hold is the same: quiet hours that end after the start.
        n.notice.as_mut().unwrap().held_until = Some("2026-11-16T10:30:30Z".into());
        assert!(meeting_started_since_made(&n, at("2026-11-16T10:31:00Z"), &Utc));
    }

    #[test]
    fn a_reminder_made_at_the_start_and_other_cards_are_left_to_the_ordinary_rule() {
        let at = |t: &str| parsed(t).unwrap();
        // "Remind me when it starts": made at the start, so the start is not news.
        let mut now_reminder = desktop_card("cal-reminder:orion:20261116T1030");
        now_reminder.created_at = "2026-11-16T10:30:02Z".into();
        assert!(!meeting_started_since_made(&now_reminder, at("2026-11-16T10:30:05Z"), &Utc));
        // A mind's own card with a calendar-looking id, and the early card, are not reminders of ours.
        let mut mind = desktop_card("cal-reminder:orion:20261116T1030");
        mind.notice.as_mut().unwrap().harness = NoticeHarness { id: "mind".into(), ..Default::default() };
        mind.created_at = "2026-11-16T10:19:00Z".into();
        assert!(!meeting_started_since_made(&mind, at("2026-11-16T10:40:00Z"), &Utc));
        assert_eq!(toast(&mind, Screen::default(), at("2026-11-16T10:40:00Z")), Toast::Now);
        // The zone matters: 10:30 at UTC+2 is 08:30 UTC, so at 08:31 UTC it has started.
        let plus2 = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        let mut reminder = desktop_card("cal-reminder:orion:20261116T1030");
        reminder.created_at = "2026-11-16T08:19:00Z".into();
        assert!(meeting_started_since_made(&reminder, at("2026-11-16T08:31:00Z"), &plus2));
        assert!(!meeting_started_since_made(&reminder, at("2026-11-16T08:31:00Z"), &Utc));
    }

    #[test]
    fn a_button_opens_only_what_its_check_allows() {
        let home = std::env::temp_dir().join(format!("for-you-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("report.pdf"), "x").unwrap();
        let action = |kind: ActionKind, reference: &str| NoticeAction { label: "Open".into(), kind, reference: reference.into() };
        let file = home.join("report.pdf");
        assert!(matches!(plan(&action(ActionKind::OpenFile, file.to_str().unwrap()), &home), Ok(Opening::File(_))));
        assert!(plan(&action(ActionKind::OpenFile, "/etc/passwd"), &home).is_err());
        assert!(plan(&action(ActionKind::OpenUrl, "javascript:alert(1)"), &home).is_err());
        assert!(plan(&action(ActionKind::Other("run".into()), "x"), &home).is_err());
        let url = action(ActionKind::OpenUrl, "https://bank.example/login");
        assert_eq!(button_label(&NoticeAction { label: "Open your bank".into(), ..url }), "Open bank.example");
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_cards_file_opens_as_a_private_read_only_copy_of_what_was_judged() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("for-you-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home")).unwrap();
        let file = root.join("home/plan.md");
        std::fs::write(&file, "the plan").unwrap();
        let file = std::fs::canonicalize(file).unwrap();
        let into = root.join("copies");
        let copy = read_only_copy(&file, &into, "7-plan.md").unwrap();
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "the plan");
        assert_eq!(std::fs::metadata(&copy).unwrap().permissions().mode() & 0o777, 0o400);
        assert_eq!(std::fs::metadata(&into).unwrap().permissions().mode() & 0o777, 0o700);
        // Pressed again: a fresh copy, not an error.
        assert!(read_only_copy(&file, &into, "7-plan.md").is_ok());
        // A path that is now a symlink to somewhere else is not the file that was judged.
        std::fs::write(root.join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(root.join("secret"), root.join("home/swapped.md")).unwrap();
        assert!(read_only_copy(&root.join("home/swapped.md"), &into, "8-swapped.md").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    fn desktop_card(notice_id: &str) -> Notification {
        let mut n = card("1", Attention::Interrupt, false);
        let c = n.notice.as_mut().unwrap();
        c.notice_id = notice_id.into();
        c.harness = NoticeHarness { id: notice::OS_HARNESS.into(), ..Default::default() };
        n
    }

    #[test]
    fn the_three_calendar_cards_of_one_meeting_share_a_key() {
        let key = |id: &str| meeting_key(&desktop_card(id));
        let reminder = key("cal-reminder:orion:20990318T0930");
        assert_eq!(reminder.as_deref(), Some("orion:20990318T0930"));
        assert_eq!(key("cal-brief:orion:20990318T0930"), reminder);
        assert_eq!(key("cal-early:orion:20990318T0930"), reminder);
        assert_ne!(key("cal-reminder:orion:20990318T1000"), reminder, "another start is another meeting");
        assert_ne!(key("cal-reminder:other:20990318T0930"), reminder, "so is another event");
        assert_eq!(key("os-12"), None);
        assert_eq!(key("cal-reminder:orion:garbage"), None);
        assert_eq!(key("cal-reminder:orion"), None);
    }

    #[test]
    fn a_minds_notice_has_no_meeting_even_with_a_calendar_id() {
        let mut n = desktop_card("cal-brief:orion:20990318T0930");
        n.notice.as_mut().unwrap().harness = NoticeHarness { id: "mind".into(), ..Default::default() };
        assert_eq!(meeting_key(&n), None);
    }
}
