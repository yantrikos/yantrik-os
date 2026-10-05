//! What the notification centre says about a mind's turns: the card's words, and the group a run
//! of them folds into.
//!
//! VM 520 sign-off, 4 October: the centre was a wall of "Yantrik Mind finished: “yes”", one card
//! per turn, each titled with the prompt the person typed. A prompt is not what was done, and
//! twelve cards in a row about the same mind finishing is one thing to know, not twelve. So the
//! desktop's turn notices are titled by what the shell observed, the prompt rides in the body as
//! "You asked: …", and a run of them from one mind folds into "Yantrik Mind finished 12 turns".
//!
//! Display only. Who sent a notification is decided by the notifications service and judged by
//! `notification_sender` (#114, #614); this reads that judgement and never makes one. A notice is
//! treated as the desktop's own only when `toast_program` says the sender is the desktop itself —
//! the same test that lets a toast drop its program name — so a program that titles its
//! notifications "Yantrik Mind finished" is drawn as itself, ungrouped, with its own words.

use std::cell::RefCell;
use std::collections::HashSet;

use yantrik_ipc_contracts::notifications::Notification;

/// The action a turn notice opens its agent with (`wire::agents::Notice`).
const OPEN_AGENT: &str = "show_agent";

/// How a run of turn notices must be to fold: two is already a repeat.
const FOLD_AT: usize = 2;

/// How much of the person's prompt a notice repeats in its body.
pub const PROMPT_CHARS: usize = 60;

thread_local! {
    /// The groups the person opened, by key. On the UI thread, where the centre is drawn; it
    /// outlives the list, which is rebuilt on every poll.
    static EXPANDED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Open or close one group.
pub fn toggle(key: &str) {
    EXPANDED.with(|open| {
        let mut open = open.borrow_mut();
        if !open.remove(key) {
            open.insert(key.to_string());
        }
    });
}

fn is_expanded(key: &str) -> bool {
    EXPANDED.with(|open| open.borrow().contains(key))
}

// ── The words ───────────────────────────────────────────────────────────────────────────────

/// A mind's name as the desktop repeats it under its own: one plain line, no control, bidi or
/// zero-width characters, cut like a claim (`notification_sender::plain`, CLAIM_CHARS). A mind
/// names itself, so its name is a claim the desktop must not let break a line, reorder the rest or
/// run on (security review of #648, M1). Applied to every title and heading that carries one.
pub fn mind_name(raw: &str) -> String {
    let seen: String = raw.chars().filter(|c| !crate::approvals::is_format_char(*c)).collect();
    crate::notification_sender::plain(&seen, crate::notification_sender::CLAIM_CHARS)
}

/// A turn notice's title, from what the shell saw: whether it finished, and how many calls it
/// made. Never the prompt — that is the person's words, and goes in the body, labelled.
pub fn turn_title(mind: &str, ok: bool, calls: usize) -> String {
    let mind = mind_name(mind);
    let made = match calls {
        0 => String::new(),
        1 => " \u{b7} made 1 call".to_string(),
        n => format!(" \u{b7} made {n} calls"),
    };
    match (ok, calls) {
        (true, 0) => format!("{mind} replied"),
        (true, _) => format!("{mind} finished{made}"),
        (false, _) => format!("{mind} could not finish{made}"),
    }
}

/// "You asked: “…”." — the prompt, labelled as the person's, for the start of a body.
pub fn you_asked(prompt: &str) -> String {
    let flat = crate::notification_sender::one_line(prompt);
    let flat = flat.trim();
    let shown = if flat.chars().count() <= PROMPT_CHARS {
        flat.to_string()
    } else {
        format!("{}\u{2026}", flat.chars().take(PROMPT_CHARS - 1).collect::<String>())
    };
    format!("You asked: \u{201c}{shown}\u{201d}.")
}

/// What a card shows as its title and body.
///
/// Notices stored before this change were titled `Yantrik Mind finished: “yes”`. For the
/// desktop's own agent notices, a quoted tail on the title is the prompt: it moves to the body as
/// "You asked: …" and the title keeps what happened. Everything else goes through the one title
/// rule (`notification_title::shown`): a short title is left as sent with its body, and a long
/// one, or none, becomes the first sentence over the whole text. The centre and Today both draw
/// what this returns, through `notifications::to_slint_data`.
pub fn display_copy(n: &Notification) -> (String, String) {
    if from_the_desktop_about_an_agent(n) {
        if let Some((head, quoted)) = n.title.split_once(": \u{201c}") {
            let prompt = quoted.strip_suffix('\u{201d}').unwrap_or(quoted);
            let body = if n.body.is_empty() {
                you_asked(prompt)
            } else {
                format!("{} {}", you_asked(prompt), n.body)
            };
            return (head.to_string(), body);
        }
    }
    crate::notification_title::shown(&n.title, &n.body)
}

// ── Recognising a turn notice ───────────────────────────────────────────────────────────────

/// One of the desktop's notices about one of its agents. The sender judgement is
/// `notification_sender`'s, read and not remade.
fn from_the_desktop_about_an_agent(n: &Notification) -> bool {
    n.sender.is_some()
        && crate::notification_sender::toast_program(n).is_none()
        && n.actions.iter().any(|a| a.id == OPEN_AGENT)
}

/// A turn that ended: which mind, and whether it finished.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnded {
    pub mind: String,
    pub ok: bool,
}

/// Whether this is the desktop telling the person one of its minds' turns ended.
pub fn turn_ended(n: &Notification) -> Option<TurnEnded> {
    if !from_the_desktop_about_an_agent(n) {
        return None;
    }
    let title = display_copy(n).0;
    // "could not finish" first: it does not contain " finished", but say the order on purpose.
    for (marker, ok) in [(" could not finish", false), (" finished", true), (" replied", true)] {
        if let Some(at) = title.find(marker) {
            let after = &title[at + marker.len()..];
            if after.is_empty() || after.starts_with(" \u{b7}") {
                return Some(TurnEnded { mind: title[..at].to_string(), ok });
            }
        }
    }
    None
}

// ── A group's heading ───────────────────────────────────────────────────────────────────────

/// What the centre heads a group with: its sender's name, unless every card in it is the
/// desktop's own notice that one named mind's turn ended — `turn_ended`, the test the fold uses,
/// which reads `notification_sender`'s judgement and makes none of its own. Then the group is
/// about that mind, and is headed with its name.
///
/// Sign-off, 5 October: a group of nothing but a mind's turns was headed "Yantrik" while its own
/// fold line said "Yantrik Mind finished 29 turns". The heading only: the group is still filed,
/// keyed and cleared under its sender (`notifications::group_of`), and a group holding anything
/// else — an update notice beside the turns — is headed by the sender, because the heading must
/// name everything under it.
pub fn heading(sender: &str, group: &[&Notification]) -> String {
    let mut minds = group.iter().map(|n| turn_ended(n).map(|t| mind_name(&t.mind)));
    match minds.next() {
        Some(Some(mind)) if !mind.trim().is_empty() && minds.all(|m| m.as_deref() == Some(mind.as_str())) => mind,
        _ => sender.to_string(),
    }
}

// ── Folding a run ───────────────────────────────────────────────────────────────────────────

/// One entry in an app's list: a card, or a run of turn notices folded into one.
#[derive(Debug, PartialEq)]
pub enum Entry<'a> {
    One(&'a Notification),
    Turns(Run<'a>),
}

/// Consecutive turn notices from one mind, newest first.
#[derive(Debug, PartialEq)]
pub struct Run<'a> {
    pub mind: String,
    pub notes: Vec<&'a Notification>,
}

impl Run<'_> {
    /// Stable while the run grows at the top: named by its oldest notice, which stays put.
    pub fn key(&self) -> String {
        format!("turns:{}", self.notes.last().map(|n| n.id.as_str()).unwrap_or_default())
    }

    pub fn expanded(&self) -> bool {
        is_expanded(&self.key())
    }

    /// "Yantrik Mind finished 12 turns · last 2m ago", or with how many did not finish.
    pub fn label(&self, newest_ago: &str) -> String {
        let n = self.notes.len();
        let failed = self.notes.iter().filter(|note| turn_ended(note).is_some_and(|t| !t.ok)).count();
        let what = if failed == 0 {
            format!("{} finished {n} turns", self.mind)
        } else {
            format!("{} ended {n} turns, {failed} without finishing", self.mind)
        };
        format!("{what} \u{b7} last {newest_ago}")
    }

    pub fn unread(&self) -> usize {
        self.notes.iter().filter(|n| !n.read).count()
    }
}

/// Fold an app's notifications (newest first) into entries: each run of [`FOLD_AT`] or more
/// consecutive turn notices from one mind becomes one entry; everything else stays a card.
pub fn fold<'a>(notes: &[&'a Notification]) -> Vec<Entry<'a>> {
    let mut out: Vec<Entry<'a>> = Vec::new();
    let mut run: Option<Run<'a>> = None;
    let flush = |run: Option<Run<'a>>, out: &mut Vec<Entry<'a>>| {
        let Some(run) = run else { return };
        if run.notes.len() >= FOLD_AT {
            out.push(Entry::Turns(run));
        } else {
            out.extend(run.notes.into_iter().map(Entry::One));
        }
    };
    for &n in notes {
        match (turn_ended(n), run.as_mut()) {
            (Some(t), Some(r)) if r.mind == t.mind => r.notes.push(n),
            (Some(t), _) => {
                flush(run.take(), &mut out);
                run = Some(Run { mind: t.mind, notes: vec![n] });
            }
            (None, _) => {
                flush(run.take(), &mut out);
                out.push(Entry::One(n));
            }
        }
    }
    flush(run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_ipc_contracts::notifications::{NotificationAction, Sender, Source, Urgency};

    fn desktop() -> Sender {
        Sender {
            claimed: Some("Yantrik".into()),
            verified: "yantrik-ui (pid 4)".into(),
            pid: 4,
            exe: "/opt/yantrik/bin/yantrik-ui".into(),
            desktop: true,
        }
    }

    fn note(id: &str, title: &str, sender: Option<Sender>) -> Notification {
        Notification {
            id: id.into(),
            app: "Yantrik".into(),
            title: title.into(),
            body: "Its turn is done.".into(),
            urgency: Urgency::Normal,
            created_at: "2026-10-04T09:00:00Z".into(),
            read: false,
            dismissed: false,
            actions: vec![NotificationAction::new(OPEN_AGENT, "Open")],
            source: Source::Yantrik,
            replaces_id: None,
            sender,
            revision: 1,
        }
    }

    #[test]
    fn a_title_says_what_happened_and_never_quotes_the_prompt() {
        assert_eq!(turn_title("Yantrik Mind", true, 0), "Yantrik Mind replied");
        assert_eq!(turn_title("Yantrik Mind", true, 3), "Yantrik Mind finished \u{b7} made 3 calls");
        assert_eq!(turn_title("pi", false, 1), "pi could not finish \u{b7} made 1 call");
        assert_eq!(you_asked("yes"), "You asked: \u{201c}yes\u{201d}.");
    }

    #[test]
    fn an_old_notice_titled_with_the_prompt_shows_the_prompt_in_its_body() {
        let n = note("1", "Yantrik Mind finished: \u{201c}yes\u{201d}", Some(desktop()));
        let (title, body) = display_copy(&n);
        assert_eq!(title, "Yantrik Mind finished");
        assert_eq!(body, "You asked: \u{201c}yes\u{201d}. Its turn is done.");
    }

    #[test]
    fn a_program_posting_the_desktops_words_is_shown_as_sent_and_never_grouped() {
        let mut impostor = desktop();
        impostor.desktop = false;
        impostor.verified = "python3 evil.py (pid 66)".into();
        let n = note("1", "Yantrik Mind finished: \u{201c}yes\u{201d}", Some(impostor));
        assert_eq!(display_copy(&n).0, n.title);
        assert_eq!(turn_ended(&n), None);
        // And one with no sender record at all.
        let n = note("2", "Yantrik Mind finished", None);
        assert_eq!(turn_ended(&n), None);
    }

    #[test]
    fn a_run_of_one_minds_turns_folds_and_anything_between_breaks_it() {
        let notes = [
            note("5", "Yantrik Mind finished \u{b7} made 2 calls", Some(desktop())),
            note("4", "Yantrik Mind replied", Some(desktop())),
            note("3", "Yantrik Mind could not finish: \u{201c}deploy\u{201d}", Some(desktop())),
            note("2", "Update available \u{2014} 0.9", Some(desktop())),
            note("1", "Yantrik Mind finished: \u{201c}yes\u{201d}", Some(desktop())),
        ];
        let refs: Vec<&Notification> = notes.iter().collect();
        let entries = fold(&refs);
        assert_eq!(entries.len(), 3, "{entries:?}");
        let Entry::Turns(run) = &entries[0] else { panic!("the first three fold") };
        assert_eq!(run.notes.len(), 3);
        assert_eq!(run.key(), "turns:3", "named by its oldest notice");
        assert_eq!(run.label("2m ago"), "Yantrik Mind ended 3 turns, 1 without finishing \u{b7} last 2m ago");
        assert!(matches!(entries[1], Entry::One(n) if n.id == "2"));
        assert!(matches!(entries[2], Entry::One(n) if n.id == "1"), "a run of one stays a card");
    }

    #[test]
    fn two_minds_make_two_runs() {
        let notes = [
            note("4", "pi finished", Some(desktop())),
            note("3", "pi replied", Some(desktop())),
            note("2", "Yantrik Mind replied", Some(desktop())),
            note("1", "Yantrik Mind replied", Some(desktop())),
        ];
        let refs: Vec<&Notification> = notes.iter().collect();
        let entries = fold(&refs);
        assert_eq!(entries.len(), 2);
        let Entry::Turns(second) = &entries[1] else { panic!("second run") };
        assert_eq!(second.label("just now"), "Yantrik Mind finished 2 turns \u{b7} last just now");
    }

    /// A group of one mind's turns is headed with the mind's name; a group with anything else in
    /// it, or two minds' turns, or a program posting the desktop's words, keeps its sender's.
    #[test]
    fn a_group_of_one_minds_turns_is_headed_with_its_name() {
        let turns = [
            note("3", "Yantrik Mind finished \u{b7} made 2 calls", Some(desktop())),
            note("2", "Yantrik Mind replied", Some(desktop())),
            note("1", "Yantrik Mind could not finish", Some(desktop())),
        ];
        let refs: Vec<&Notification> = turns.iter().collect();
        assert_eq!(heading("Yantrik", &refs), "Yantrik Mind");

        let update = note("4", "Update available \u{2014} 0.9", Some(desktop()));
        let mixed: Vec<&Notification> = std::iter::once(&update).chain(turns.iter()).collect();
        assert_eq!(heading("Yantrik", &mixed), "Yantrik", "an update notice is not the mind's");

        let pi = note("5", "pi replied", Some(desktop()));
        let two: Vec<&Notification> = std::iter::once(&pi).chain(turns.iter()).collect();
        assert_eq!(heading("Yantrik", &two), "Yantrik", "two minds' turns are the desktop's to head");

        let mut impostor = desktop();
        impostor.desktop = false;
        let fake = note("6", "Yantrik Mind finished", Some(impostor));
        assert_eq!(heading("python3", &[&fake]), "python3", "the words alone make nothing a turn notice");
        assert_eq!(heading("Yantrik", &[]), "Yantrik");
    }

    /// A mind's name is its own claim: in a title and in a heading it is one plain line, with no
    /// newline to start a line of the desktop's own, no bidi or zero-width character to reorder or
    /// hide anything, and cut short. A mind may call itself "System Update"; it is still headed as
    /// a mind's turns only when the desktop's own check says so, and is shown as the plain name.
    #[test]
    fn a_minds_name_is_one_plain_line_wherever_the_desktop_repeats_it() {
        assert_eq!(mind_name("pi\nYantrik: update installed"), "pi Yantrik: update insta\u{2026}");
        assert_eq!(mind_name("evil\u{202E}gnp\u{2066}"), "evilgnp");
        assert_eq!(mind_name("Sys\u{200B}tem\u{2060} Up\u{200D}date"), "System Update");
        assert_eq!(turn_title("pi\n\u{202E}x", true, 0), "pi x replied");
        assert_eq!(turn_title(&"m".repeat(60), true, 0), format!("{}\u{2026} replied", "m".repeat(24)));

        let named = |mind: &str| note("1", &turn_title(mind, true, 0), Some(desktop()));
        for (raw, shown) in [
            ("Yantrik\nMind", "Yantrik Mind"),
            ("Ya\u{200B}ntrik\u{202E} Mind", "Yantrik Mind"),
            ("System Update", "System Update"),
        ] {
            let notes = [named(raw), named(raw)];
            let refs: Vec<&Notification> = notes.iter().collect();
            let h = heading("Yantrik", &refs);
            assert_eq!(h, shown, "{raw:?}");
            assert!(!h.chars().any(|c| c.is_control() || crate::approvals::is_format_char(c)), "{h:?}");
        }
        // Words alone make nothing a mind's: "System Update" from a program is headed by its sender.
        let mut impostor = desktop();
        impostor.desktop = false;
        let fake = note("9", "System Update replied", Some(impostor));
        assert_eq!(heading("python3", &[&fake]), "python3");
    }

    #[test]
    fn a_group_opens_and_closes() {
        assert!(!is_expanded("turns:x"));
        toggle("turns:x");
        assert!(is_expanded("turns:x"));
        toggle("turns:x");
        assert!(!is_expanded("turns:x"));
    }
}
