//! Pending approval requests, and the grants only a person can create.
//!
//! # Why this exists
//!
//! Someone typed "Delete the dentist appointment from my calendar." The mind found the event,
//! called `os_act calendar delete_event`, and the MCP bridge refused it: `delete_event` is
//! graded `sensitive` and the bridge's own ceiling is `standard`. The mind then told the person
//! to "approve it right here in the chat panel (a /approve prompt should appear), or raise
//! YOS_MCP_MAX_PERMISSION in the settings." No such prompt existed. Somebody asked for an
//! ordinary thing and was told to set an environment variable.
//!
//! The missing piece was never a policy. It was a way to *ask*. This module is that: a request
//! a caller can raise, a card the shell puts in front of the person, and a grant that exists
//! only because a person pressed a button.
//!
//! # The invariant: a mind must not be able to approve itself
//!
//! [`grant`] and [`deny`] are `pub(crate)` and have exactly one caller: the Slint callback in
//! `control_approvals.rs` that a person's click reaches. There is no `app.act` action on the
//! shell that grants or denies, and `published_actions_cannot_grant` in `control_approvals.rs`
//! reads the source of every `control*.rs` file and fails if one appears. Everything a caller on
//! the socket *can* do — raise a request, poll it, burn a grant — is `safe`, because none of it
//! decides anything. The decision is a click.
//!
//! That is the whole security argument, and it rests on the surface being small enough to read.
//! Do not add a way to grant from code. If some future automation needs standing permission,
//! that is `tool_permission` in the machine's settings — the owner's standing policy, set at the
//! keyboard — not a grant minted here.
//!
//! # What a grant is bound to
//!
//! One grant, one action, one exact set of arguments, once. [`Store::consume`] compares the
//! canonical JSON of the arguments handed to it against the canonical JSON of the arguments the
//! card showed the person. Key order does not matter (nobody reads JSON key order, and the
//! transport does not preserve it); any value change does, because that is what the person
//! looked at. A second consume of the same grant authorises nothing.
//!
//! # What it deliberately is not
//!
//! Not persistent: the store is memory, and a shell restart drops every request and grant. That
//! is the correct failure — a grant that survives the thing that was asking is a grant nobody
//! remembers giving. Not an "always allow": there is no way to record a standing yes here. Not
//! run-bound (see `design/next-focus-2026-09.md` §3 and its stop rule) — a grant is bound to the
//! arguments, not to a run, because runs do not exist yet.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a request waits for an answer before it stops being one.
///
/// Two minutes is roughly how long a person takes to notice a card, read six lines of arguments
/// and decide. Longer and a forgotten card is still live when they have moved on; shorter and an
/// honest "hold on" loses.
pub const REQUEST_TTL: Duration = Duration::from_secs(120);

/// How long a grant survives being given, if nobody burns it.
///
/// Short on purpose. The grant exists to carry one decision across one round trip; anything
/// slower than that is a different situation and deserves to be asked about again.
pub const GRANT_TTL: Duration = Duration::from_secs(60);

/// How many requests may be waiting at once.
///
/// This is the approval-fatigue bound, not a resource bound. A stack of cards is a stack nobody
/// reads, and a caller that can make ten of them can make the eleventh — the dangerous one —
/// look like more of the same. Three fits on screen and stays legible.
pub const MAX_PENDING: usize = 3;

/// How long a denial silences the identical request.
///
/// Without this, "no" costs the person one click and the caller one retry, which is a losing
/// trade for the person. Re-asking the same thing inside this window is refused with a message
/// that says so; anything else — different arguments, a different action, or the same one two
/// minutes later — is allowed through, because a person changing their mind is normal.
pub const DENIAL_QUIET: Duration = Duration::from_secs(120);

/// How many decided requests are kept for the transcript record.
///
/// Four, not eight. These render as one line each inside the Lens's conversation panel, which
/// has to hold a transcript as well; eight of them pushed the conversation out of its own panel.
const RECORD_TAIL: usize = 4;

// ── Making a card a fixed, readable size ────────────────────────────
//
// The card's height has to be predictable, for two reasons that turned out to be the same bug.
// It is drawn over whatever screen is up, so a card that grows without limit runs off the top or
// the bottom and takes its buttons with it — and the first time this ran on a real machine the
// header was clipped off the top of the screen entirely, so the person could see the arguments
// and the buttons but not who was asking or what the action was.
//
// The fix is on both sides. The markup no longer asks a Rectangle with only conditional children
// how tall it would like to be (it answered zero), and the text that reaches it is bounded here,
// so every element on the card is a known number of lines. An argument is exactly one line,
// because it is rendered as its own `Text` with `wrap: no-wrap`; a value longer than this is cut
// with its true length named, so nothing is hidden in silence.
//
// The one exception is the purpose. It is the app's own sentence about what the person is being
// asked to allow, and cutting it to a fixed number of lines turned out to hide the clause that
// mattered (see `PURPOSE_CHARS`). So it reaches the card whole, and the card bounds its HEIGHT
// instead — the block wraps, and past a fixed height it scrolls — which keeps the buttons on
// screen without deciding for the person which part of the sentence they may read.

/// How much of one argument value the card shows before cutting it.
const ARG_VALUE_CHARS: usize = 60;

/// How many arguments the card lists before summarising the rest.
///
/// Eight covers every action on this desktop (the widest is `add_event` at six). A ninth would
/// be summarised rather than dropped, and the whole set is in `describe shell` regardless — and,
/// more to the point, in the grant, which is bound to all of them whatever the card had room for.
const ARG_ROWS: usize = 8;

/// How much of the "what the ids name" line the card shows before it stops.
///
/// It is drawn as one elided row beside the arguments, so its height is already fixed; this is
/// the bound against the absurd — a name built from arguments an app made up by the kilometre —
/// and it sits well above any event title a naming line on this machine carries. The card
/// leads the person to the name, not past it: what the grant binds to is the arguments box, in
/// full and bounded there.
const TARGET_CHARS: usize = 160;

/// How much of the action's own description the card shows before it stops.
///
/// This was 240, on the belief that every published purpose on this machine is one sentence.
/// It is not: Studio's `set_backend` publishes 585 characters, and the card cut them at
/// "…naming a hosted service means the sentences typed into this app will leav… (585 characters
/// in full)" — mid-word, with no way to read on, and at exactly the clause that said why the
/// grade is what it is. Read that far, the sentence is a flat statement that the prompt leaves
/// the machine; read whole, it is a condition ("naming a hosted service means…") that the
/// arguments underneath either meet or do not. The person was shown the same fragment for
/// `kind: fake`, the direction that stops anything leaving.
///
/// So the purpose is no longer cut to fit the card. The card wraps it and, past a height, scrolls
/// it (see `ApprovalCard` in intent_lens.slint), so the buttons stay on screen however long the
/// app's sentence is. This bound is only against the absurd — a description the size of a
/// document — and it is enforced at a word boundary, because a cut that lands mid-word is what
/// the person hit. Well above the longest description any app on this desktop publishes.
const PURPOSE_CHARS: usize = 2000;

/// Cut to a length without splitting a character, and say that it was cut.
///
/// The same shape as `control::clip`, kept here rather than shared because that one is about
/// what travels over a socket and this one is about what fits on a line a person reads.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}… ({} characters in full)", text.chars().count())
}

/// [`clip`], for prose: the cut lands on the last space before the bound, never inside a word.
///
/// An argument value is cut wherever the bound falls, because a path or an id has no words to
/// respect and the person can see the shape of it from the head. A sentence is different: cut
/// mid-word it reads as a different sentence, which is the defect [`PURPOSE_CHARS`] describes.
/// The bound stays a bound — a run of text with no space in its first `max` characters is cut
/// where [`clip`] would cut it — and the marker still names the true length.
fn clip_at_word(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    let head = match head.trim_end().rfind(char::is_whitespace) {
        Some(at) if at > 0 => head[..at].trim_end().to_string(),
        _ => head,
    };
    format!("{head}… ({} characters in full)", text.chars().count())
}

/// How much of a first sentence the card shows as its one-line summary.
///
/// A first sentence is short by definition; this is only the case of a description with no
/// sentence end in it at all, where [`summary_of`] falls back to the whole text and the word
/// bound has to do the cutting. Two hundred characters is three lines on the card at most.
const SUMMARY_CHARS: usize = 200;

/// The first sentence of a published description, for the card's one line (#218).
///
/// The card the live tour hit carried `run_recipe`'s whole paragraph — protocol detail about
/// `describe shell` → `recipes` → `formations` — where a person needs one line. Published
/// actions have no separate short field for a person (the `summary` in a `View` describes the
/// APP, not an action), so the card takes the description's first sentence, which is the
/// sentence that says what the action does; the rest of the paragraph stays on the card under
/// "show more", and whole in `describe` for minds.
///
/// A sentence ends at a period followed by the end of the text, or by whitespace and a capital
/// — how the next sentence starts. A period a lowercase word continues from is inside the
/// sentence, so the periods of "e.g." do not cut it short, and neither does the one in "0.5".
/// A description with no sentence end in it is its own first sentence.
pub fn first_sentence(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    for (at, c) in chars.iter().enumerate() {
        let ends = *c == '.'
            && chars.get(at + 1).map_or(true, |next| {
                next.is_whitespace() && chars.get(at + 2).map_or(true, |after| after.is_uppercase())
            });
        if ends {
            return chars[..=at].iter().collect();
        }
    }
    text.trim().to_string()
}

/// The one person-facing line the card leads with: the first sentence of the app's own
/// description, bounded at a word so a description with no period in it cannot make the card
/// grow past the block that shows it.
pub fn summary_of(purpose: &str) -> String {
    clip_at_word(&first_sentence(purpose), SUMMARY_CHARS)
}

/// What this machine established about the caller, beside what the caller said about itself.
///
/// Deliberately plain data, with no `/proc` knowledge in it. The walking and the judging live in
/// `caller_identity.rs`; this is the answer, carried alongside the self-declared `requester` and
/// never mixed with it. The card prints them as two separate lines, labelled, because the whole
/// point is that a person can tell a claim from a fact.
///
/// Empty by [`Default`] for callers that have nothing to attach — the same thing a `None` pid
/// would produce, so there is one shape to render rather than two.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Verified {
    /// The one line the card shows. Never empty on a real request: `caller_identity` answers
    /// "could not be identified" rather than leaving a blank where a fact should be.
    pub line: String,
    /// The resolved `/proc/<pid>/exe` of the program the line names, for the log and for
    /// `describe shell`. The card shows the command line instead, which is shorter and says
    /// more; the log keeps the path, which is what a person checks afterwards.
    pub exe: String,
    /// The pid the line names. `0` when nothing was established.
    pub pid: i32,
    /// The attached mind the caller's ancestry belongs to, if one matched.
    pub attached_mind: String,
    /// What does not add up about this request, one bounded sentence each.
    ///
    /// Two can arrive today: the claimed name names an attached mind that the ancestry
    /// contradicts, and the claimed grade is not the one the app publishes. Both are drawn in
    /// the same red as "cannot be undone", because they are the same kind of thing — a reason to
    /// stop and read rather than to click — and both are one line, so the card's height stays
    /// arithmetic however many there turn out to be later.
    pub discrepancies: Vec<String>,
    /// The agent this request is for (`pi:c-7f3a91`), or empty for a caller that runs as no agent.
    ///
    /// Taken from the agent token that rode beside the request's arguments, and believed only when
    /// the kernel's caller descends from the harness the token was issued to — never from anything
    /// the request says. It decides which agent's pane draws the card (design decision 4), and the
    /// card names it. The token itself is never kept here, or anywhere a card or a log can show it.
    pub agent: String,
}

impl Verified {
    /// What `describe shell` publishes beside `requester`. Facts and no prose: a caller reading
    /// this has to be able to compare it with `ps`, not to be reassured by it.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "line": self.line,
            "exe": self.exe,
            "pid": self.pid,
            "attached_mind": self.attached_mind,
            "discrepancies": self.discrepancies,
            "agent": self.agent,
        })
    }
}

/// Where a request is. `Consumed` is reported rather than folded into `Granted` or `Expired`:
/// a caller that polls after burning its grant asked a real question, and "the grant you were
/// given has been used" is the true answer to it. Telling it `granted` invites a replay that
/// would be refused anyway; telling it `expired` is simply false.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending,
    Granted,
    Denied,
    Expired,
    Consumed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Granted => "granted",
            Status::Denied => "denied",
            Status::Expired => "expired",
            Status::Consumed => "consumed",
        }
    }

    /// Whether anything more can happen to a request in this state.
    pub fn is_final(self) -> bool {
        !matches!(self, Status::Pending | Status::Granted)
    }
}

/// What the person was asked, and what they said.
#[derive(Clone, Debug)]
struct Record {
    id: String,
    requester: String,
    /// What the machine itself found out about whoever raised this, captured when the request
    /// arrived rather than when somebody looks at the card. The direct peer is usually `yos`,
    /// which has exited within milliseconds — read it late and there is nothing left to read.
    verified: Verified,
    app: String,
    action: String,
    args: serde_json::Value,
    /// The arguments as the grant is bound to them. Computed once, at request time, so the
    /// bytes the person's card was built from are the bytes a consume is compared against.
    canonical: String,
    grade: String,
    purpose: String,
    /// The thing an id in the arguments names, in the app's own words — or empty. #54: a card
    /// for `calendar.delete_event {"id": "01a0c718-…"}` said only the uuid, and a person cannot
    /// answer "may this be deleted?" to a handle. The app publishes an id→name index on its
    /// `describe`, and the card builder looks the arguments up in it here.
    ///
    /// Display only, and deliberately outside the grant: the binding is the arguments, byte for
    /// byte (see [`Store::consume`]), so an id whose meaning the app later changes loosens
    /// nothing the person allowed.
    target: String,
    created: Instant,
    /// Wall-clock `HH:MM` for the transcript record. `Instant` cannot render as a time of day,
    /// and the record a person reads afterwards is about when, not about how long ago.
    created_at: String,
    decided: Option<Instant>,
    decided_at: String,
    /// The person pressed "Allow for this session" rather than "Allow once". It changes nothing
    /// about THIS grant — still single-use, still bound to these exact arguments — only the line
    /// left in the transcript, so a person scrolling back can see which of the two they chose.
    /// The standing part of that decision lives in `mind_mode`, not here.
    session: bool,
    /// The shell took it back before anybody answered: the agent that asked was stopped, or its
    /// harness went. Reported as `expired` — nobody answered — and never grantable.
    withdrawn: bool,
    /// The stored state. Expiry is not stored: it is a fact about the clock, derived on every
    /// read, so a request cannot be alive merely because nothing looked at it.
    state: Status,
}

impl Record {
    fn status(&self, now: Instant) -> Status {
        match self.state {
            Status::Pending if now.duration_since(self.created) >= REQUEST_TTL => Status::Expired,
            Status::Granted => match self.decided {
                Some(at) if now.duration_since(at) >= GRANT_TTL => Status::Expired,
                _ => Status::Granted,
            },
            other => other,
        }
    }
}

/// One request as the UI and `describe` see it.
#[derive(Clone, Debug)]
pub struct Card {
    pub id: String,
    pub requester: String,
    /// Beside `requester`, never folded into it. See [`Verified`].
    pub verified: Verified,
    pub app: String,
    pub action: String,
    pub grade: String,
    pub purpose: String,
    /// The first sentence of `purpose`, bounded at [`SUMMARY_CHARS`] — the one line a person
    /// reads first, while the paragraph it came from stays on the card under "show more".
    /// See [`summary_of`].
    pub summary: String,
    /// The arguments, one `key: value` entry each, in the order a person reads them (sorted, the
    /// same order the grant is bound in — so what is shown and what is bound cannot drift).
    ///
    /// A list rather than one newline-joined string, because each entry becomes its own
    /// single-line `Text` on the card. That is what makes the card's height a known number of
    /// lines instead of something the layout has to discover by measuring wrapped text.
    pub args: Vec<String>,
    /// What the arguments name, in the app's own words — one line, or empty when the app
    /// publishes no index of its ids. #54: the person who cannot answer "may
    /// `delete_event {"id": "01a0c718-…"}?` run?" from a uuid reads this instead. It says
    /// nothing about what the grant covers — the arguments box above is that, byte for byte —
    /// and a cut one names its true length like every other bounded line on the card.
    pub target: String,
    /// A sentence to put in front of the buttons, or empty. See [`warning_for`].
    pub warning: String,
    /// Whether the card may offer "Allow for this session" as a third choice. See
    /// [`may_offer_session_rule`] — computed from the purpose as the app published it, not from
    /// `purpose` above, so that even a description long enough to be cut at [`PURPOSE_CHARS`]
    /// cannot lose the phrase that says the action is irreversible and end up offering a
    /// standing yes for exactly the action that must not have one.
    pub can_session: bool,
    pub status: Status,
    /// The one-line transcript record, once this has been decided. Empty while pending.
    pub record: String,
    pub age_secs: u64,
}

/// Canonical JSON: object keys sorted, everything else as serde renders it.
///
/// `serde_json`'s map is a `BTreeMap` today, so `to_string` already sorts — but that is a
/// feature flag away from being insertion order (`preserve_order`), and a grant that silently
/// stops matching when somebody enables a Cargo feature is the worst kind of bug to find. The
/// sort is written out so the guarantee belongs to this function.
///
/// Arrays keep their order. An array is data, and `["a","b"]` is not `["b","a"]`.
pub fn canonical(value: &serde_json::Value) -> String {
    fn walk(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for key in keys {
                    out.insert(key.clone(), walk(&map[key]));
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(walk).collect())
            }
            other => other.clone(),
        }
    }
    walk(value).to_string()
}

/// The arguments as one readable line each, in the order the grant is bound in.
///
/// Bounded on both axes — [`ARG_VALUE_CHARS`] per line, [`ARG_ROWS`] lines — so the card is a
/// known height. Neither bound hides anything in silence: a cut value names its true length and
/// a cut list names how many are left.
pub fn args_rows(value: &serde_json::Value) -> Vec<String> {
    let Some(map) = value.as_object() else {
        // Not an object. Show it rather than hiding it: a caller that sent something odd should
        // not get a card that looks empty.
        return match value {
            serde_json::Value::Null => vec!["(no arguments)".to_string()],
            other => vec![clip(&other.to_string(), ARG_VALUE_CHARS)],
        };
    };
    if map.is_empty() {
        return vec!["(no arguments)".to_string()];
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();

    let mut rows: Vec<String> = keys
        .iter()
        .take(ARG_ROWS)
        .map(|key| {
            let value = &map[*key];
            // A string argument reads better without its quotes; everything else is shown as
            // JSON, because `true` and `"true"` are different answers to the same question.
            let shown = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{key}: {}", clip(&shown, ARG_VALUE_CHARS))
        })
        .collect();
    if keys.len() > ARG_ROWS {
        rows.push(format!(
            "… and {} more argument(s); all of them are in `describe shell`, and the grant is \
             bound to all of them",
            keys.len() - ARG_ROWS
        ));
    }
    rows
}

/// Does the app's own sentence about this action say it cannot be taken back?
///
/// Pulled out of [`warning_for`] because two features now need the same judgement and they must
/// not be allowed to disagree: the card draws a red line when this is true, and
/// `mind_mode::person_add_rule` refuses to mint a standing yes when it is. A person offered
/// "stop asking me about this" for something the app says is irreversible has been offered the
/// wrong thing.
///
/// And a third now needs it, which is why the phrase list is not here any more: every app's
/// dispatch asks the same question before it runs an action (`gate::decide`), so the reading of
/// the sentence lives in `yantrik_ipc_transport::gate::unrecoverable` and this is that function.
/// One list on the machine; the MCP bridge's copy is checked against it.
pub fn unrecoverable(purpose: &str) -> bool {
    yantrik_app_runtime::control::unrecoverable(purpose)
}

/// May the card offer "Allow for this session" for this action?
///
/// Two exclusions, both of them about what a standing yes would cost if it were wrong. A
/// `dangerous` action is the one the whole card exists for, and an action the app itself says
/// cannot be undone is one where a second, unwatched run is the damage. Everything else — the
/// routine `sensitive` surface a long job trips over forty times — is exactly what the session
/// rule is for.
pub fn may_offer_session_rule(grade: &str, purpose: &str) -> bool {
    grade != "dangerous" && !unrecoverable(purpose)
}

/// The warning line, or empty.
///
/// Two sources, because the two things a person needs warning about are different. The grade is
/// the OS's own judgement about the action; the purpose is the app's own sentence about it, and
/// `delete_event` publishes "It is not recoverable" there. A card that shows the grade but drops
/// that sentence is the exact failure commit d73760d fixed one layer down.
pub fn warning_for(grade: &str, purpose: &str) -> String {
    let unrecoverable = unrecoverable(purpose);

    match (grade == "dangerous", unrecoverable) {
        (true, true) => "This is graded dangerous and the app says it cannot be undone.".into(),
        (true, false) => "This is graded dangerous — it can destroy work or state.".into(),
        (false, true) => "The app says this cannot be undone.".into(),
        (false, false) => String::new(),
    }
}

/// What [`Store::request`] answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requested {
    pub id: String,
    /// Always `pending`. Named rather than assumed, because the caller puts it in a JSON reply
    /// and a literal there would be a second place for the truth to live.
    pub status: Status,
    /// False when this call found an identical question already on screen and handed back its
    /// card rather than making a second one.
    ///
    /// The caller needs to know, because raising a card takes the screen off whatever the person
    /// was using. Doing that once, when the question first appears, is the point; doing it again
    /// every time something repeats the same request is a way to hold somebody's screen hostage
    /// with a call that is graded `safe`.
    pub fresh: bool,
}

/// The requests this shell is holding. See the module doc for what may mutate it.
pub struct Store {
    records: Vec<Record>,
    next: u64,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Store { records: Vec::new(), next: 1 }
    }

    /// Raise a request. The only thing this does is make a card appear.
    ///
    /// `now` and `at` are passed in rather than read here so the tests can move the clock. The
    /// public wrappers below supply the real ones; nothing outside this module can pick a time.
    ///
    /// `target` is the one line saying what an id in the arguments names, worked out from the
    /// target app's own index by whoever raises the card — see [`Card::target`]. It is not part
    /// of the identity of the question either: the same action with the same arguments is the
    /// same question however its ids read, whether or not the app has said why.
    #[allow(clippy::too_many_arguments)]
    pub fn request(
        &mut self,
        requester: &str,
        verified: Verified,
        app: &str,
        action: &str,
        args: serde_json::Value,
        grade: &str,
        purpose: &str,
        target: &str,
        now: Instant,
        at: &str,
    ) -> Result<Requested, String> {
        self.prune(now);

        let canonical = canonical(&args);

        // Asked and still waiting: the same question, so the same card. A caller that retries
        // (a poll that timed out, a model that repeated itself) must not put a second identical
        // card in front of the person — that is how a stack of cards becomes noise.
        //
        // The same question from ANOTHER agent is not the same question: who is asking is half of
        // what the person reads, the card is drawn in the asker's pane, and one agent's Allow must
        // not answer another's request.
        if let Some(existing) = self.records.iter().find(|r| {
            r.status(now) == Status::Pending
                && r.app == app
                && r.action == action
                && r.canonical == canonical
                && r.verified.agent == verified.agent
        }) {
            return Ok(Requested {
                id: existing.id.clone(),
                status: Status::Pending,
                fresh: false,
            });
        }

        // Asked and answered no. Re-asking immediately is how a refusal becomes a war of
        // attrition the person loses by clicking Allow to make it stop.
        if let Some(denied) = self.records.iter().find(|r| {
            r.state == Status::Denied
                && r.app == app
                && r.action == action
                && r.canonical == canonical
                && denied_recently(r, now)
        }) {
            let ago = denied.decided.map(|d| now.duration_since(d).as_secs()).unwrap_or(0);
            return Err(format!(
                "the person denied `{app}.{action}` with these exact arguments {ago}s ago, so \
                 this was not put in front of them again. Do not ask a third time unless they \
                 bring it up themselves."
            ));
        }

        let waiting = self.records.iter().filter(|r| r.status(now) == Status::Pending).count();
        if waiting >= MAX_PENDING {
            return Err(format!(
                "{waiting} approval requests are already waiting for an answer, which is the \
                 most this shell will show at once. A person cannot read a stack of them, and a \
                 stack is how the one that matters gets waved through. Wait for the ones on \
                 screen to be answered or to expire ({}s each), then ask again.",
                REQUEST_TTL.as_secs()
            ));
        }

        let id = format!("appr-{}", self.next);
        self.next += 1;
        self.records.push(Record {
            id: id.clone(),
            requester: requester.trim().to_string(),
            verified,
            app: app.to_string(),
            action: action.to_string(),
            args,
            canonical,
            grade: grade.to_string(),
            purpose: purpose.trim().to_string(),
            target: target.trim().to_string(),
            created: now,
            created_at: at.to_string(),
            decided: None,
            decided_at: String::new(),
            session: false,
            withdrawn: false,
            state: Status::Pending,
        });
        Ok(Requested { id, status: Status::Pending, fresh: true })
    }

    /// Take back every request still waiting for `agent`: it was stopped, or its harness went, and
    /// a card for work that is no longer happening must not become a grant (design, "When a
    /// harness dies"). The ids withdrawn.
    ///
    /// This only ever makes the desktop less permissive — a withdrawn request is refused, never
    /// granted — so it is not a decision a person has to make, and it is not reachable from any
    /// action on the socket: the shell calls it when it stops an agent or sees its harness go.
    pub(crate) fn withdraw_for_agent(&mut self, agent: &str, now: Instant, at: &str) -> Vec<String> {
        if agent.is_empty() {
            return Vec::new();
        }
        let mut withdrawn = Vec::new();
        for record in self.records.iter_mut() {
            if record.verified.agent == agent && record.status(now) == Status::Pending {
                record.state = Status::Expired;
                record.withdrawn = true;
                record.decided = Some(now);
                record.decided_at = at.to_string();
                withdrawn.push(record.id.clone());
            }
        }
        withdrawn
    }

    /// Where a request stands. `None` means no request by that id — which is not the same as
    /// expired, and a caller that cannot tell those apart will retry forever on a typo.
    pub fn status(&self, id: &str, now: Instant) -> Option<Status> {
        self.records.iter().find(|r| r.id == id).map(|r| r.status(now))
    }

    /// A person pressed Allow. **UI only** — see the module doc.
    pub(crate) fn grant(&mut self, id: &str, now: Instant, at: &str) -> Result<(), String> {
        self.decide(id, Status::Granted, now, at)
    }

    /// A person pressed "Allow for this session". **UI only** — see the module doc.
    ///
    /// The grant itself is identical to [`Store::grant`]: one action, these arguments, once.
    /// What differs is the record line, because "did I say yes to this once or for the rest of
    /// the day?" is a question people ask afterwards and the transcript is where they look.
    /// The standing part is a rule in `mind_mode`, which the caller adds beside this.
    pub(crate) fn grant_for_session(
        &mut self,
        id: &str,
        now: Instant,
        at: &str,
    ) -> Result<(), String> {
        self.decide(id, Status::Granted, now, at)?;
        if let Some(record) = self.records.iter_mut().find(|r| r.id == id) {
            record.session = true;
        }
        Ok(())
    }

    /// A person pressed Deny. **UI only** — see the module doc.
    pub(crate) fn deny(&mut self, id: &str, now: Instant, at: &str) -> Result<(), String> {
        self.decide(id, Status::Denied, now, at)
    }

    fn decide(
        &mut self,
        id: &str,
        decision: Status,
        now: Instant,
        at: &str,
    ) -> Result<(), String> {
        let Some(record) = self.records.iter_mut().find(|r| r.id == id) else {
            return Err(format!("no approval request `{id}`"));
        };
        // Deciding twice is a double click, not a second decision. And a card that has already
        // expired must not become a grant: the request it stood for is gone, and the person
        // clicking now is answering a question nobody is asking any more.
        match record.status(now) {
            Status::Pending => {
                record.state = decision;
                record.decided = Some(now);
                record.decided_at = at.to_string();
                Ok(())
            }
            other => Err(format!(
                "`{id}` is {} and cannot be decided now",
                other.as_str()
            )),
        }
    }

    /// Burn a grant, if the triple matches exactly. Succeeds at most once per grant.
    ///
    /// The refusal says which part differed, because a caller that is told only "no" will
    /// retry the same thing. Telling it the arguments changed is what makes it stop and look.
    pub fn consume(
        &mut self,
        id: &str,
        app: &str,
        action: &str,
        args: &serde_json::Value,
        now: Instant,
    ) -> Result<(), String> {
        let Some(record) = self.records.iter_mut().find(|r| r.id == id) else {
            return Err(format!(
                "no approval request `{id}` — it may have been dropped when the shell restarted. \
                 Ask again."
            ));
        };

        match record.status(now) {
            Status::Granted => {}
            Status::Pending => {
                return Err(format!(
                    "`{id}` has not been answered yet; nothing was authorised. Keep polling \
                     approval_status, or let it expire."
                ))
            }
            Status::Denied => {
                return Err(format!(
                    "the person denied `{id}`; nothing was authorised and nothing was run."
                ))
            }
            Status::Consumed => {
                return Err(format!(
                    "`{id}` was already used. A grant authorises one action once; this second \
                     use authorises nothing. Ask again if the action still needs doing."
                ))
            }
            Status::Expired => {
                return Err(format!(
                    "`{id}` has expired — a grant lasts {}s and a request {}s. Nothing was \
                     authorised. Ask again.",
                    GRANT_TTL.as_secs(),
                    REQUEST_TTL.as_secs()
                ))
            }
        }

        // The triple, one part at a time, so the refusal names the part that moved.
        if record.app != app {
            return Err(format!(
                "`{id}` was approved for app `{}`, not `{app}`. Nothing was authorised.",
                record.app
            ));
        }
        if record.action != action {
            return Err(format!(
                "`{id}` was approved for action `{}.{}`, not `{}.{action}`. Nothing was \
                 authorised.",
                record.app, record.action, record.app
            ));
        }
        let given = canonical(args);
        if record.canonical != given {
            return Err(format!(
                "`{id}` was approved for `{}.{}` with arguments {}, and this call carries {}. \
                 The person approved what they were shown; a different argument is a different \
                 action. Nothing was authorised.",
                record.app, record.action, record.canonical, given
            ));
        }

        record.state = Status::Consumed;
        Ok(())
    }

    /// Everything the UI and `describe` show: what is waiting, then what was recently decided.
    pub fn cards(&self, now: Instant) -> Vec<Card> {
        let mut decided: Vec<Card> = Vec::new();
        let mut pending: Vec<Card> = Vec::new();
        for record in &self.records {
            let status = record.status(now);
            let card = Card {
                id: record.id.clone(),
                requester: record.requester.clone(),
                verified: record.verified.clone(),
                app: record.app.clone(),
                action: record.action.clone(),
                grade: record.grade.clone(),
                purpose: clip_at_word(&record.purpose, PURPOSE_CHARS),
                summary: summary_of(&record.purpose),
                args: args_rows(&record.args),
                // At a word, not at the bound: a cut in the middle of the name is the
                // `PURPOSE_CHARS` mistake rebuilt — "13:0" and "13:00… " are not the same
                // sentence about when the appointment is.
                target: clip_at_word(&record.target, TARGET_CHARS),
                warning: warning_for(&record.grade, &record.purpose),
                can_session: may_offer_session_rule(&record.grade, &record.purpose),
                status,
                record: record_line(record, status),
                age_secs: now.duration_since(record.created).as_secs(),
            };
            if status == Status::Pending {
                pending.push(card);
            } else {
                decided.push(card);
            }
        }
        // Newest last in the record strip (a transcript reads downwards); the pending cards go
        // underneath them, because the thing waiting on you belongs closest to the buttons.
        let skip = decided.len().saturating_sub(RECORD_TAIL);
        let mut out: Vec<Card> = decided.into_iter().skip(skip).collect();
        out.extend(pending);
        out
    }

    /// Only what is waiting for an answer. What `describe shell` publishes.
    pub fn pending(&self, now: Instant) -> Vec<Card> {
        self.cards(now).into_iter().filter(|c| c.status == Status::Pending).collect()
    }

    /// Drop what nobody will look at again, so a long session does not grow a list forever.
    ///
    /// Decided records are kept well past their decision: they are the transcript, and the
    /// denial window reads them. Ten minutes covers both and is far shorter than a session.
    fn prune(&mut self, now: Instant) {
        const KEEP: Duration = Duration::from_secs(600);
        self.records.retain(|r| {
            let status = r.status(now);
            if !status.is_final() {
                return true;
            }
            let since = r.decided.unwrap_or(r.created);
            now.duration_since(since) < KEEP
        });
    }
}

fn denied_recently(record: &Record, now: Instant) -> bool {
    record
        .decided
        .map(|at| now.duration_since(at) < DENIAL_QUIET)
        .unwrap_or(false)
}

/// The line that stays in the conversation after the card is gone.
fn record_line(record: &Record, status: Status) -> String {
    let what = format!("{}.{}", record.app, record.action);
    let allowed = if record.session { "Allowed for this session" } else { "Allowed once" };
    match status {
        Status::Pending => String::new(),
        Status::Granted => format!("{allowed}: {what} — {}", record.decided_at),
        Status::Consumed => format!("{allowed}: {what} — {}", record.decided_at),
        Status::Denied => format!("Denied: {what} — {}", record.decided_at),
        Status::Expired if record.state == Status::Granted => {
            format!("{allowed}: {what} — {} (grant expired unused)", record.decided_at)
        }
        Status::Expired if record.withdrawn => {
            format!("Withdrawn: {what} — {} (the agent that asked was stopped or is gone)", record.decided_at)
        }
        Status::Expired => format!("Not answered: {what} — asked {}", record.created_at),
    }
}

// ── The one store this shell has ────────────────────────────────────

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::new()))
}

/// A poisoned lock means a previous holder panicked mid-update. The records are plain data with
/// no invariant that a panic could have half-broken, so the contents are still readable and
/// refusing every approval forever is the worse outcome.
fn locked() -> std::sync::MutexGuard<'static, Store> {
    store().lock().unwrap_or_else(|e| e.into_inner())
}

fn hhmm() -> String {
    crate::app_context::current_time_hhmm()
}

pub fn request(
    requester: &str,
    verified: Verified,
    app: &str,
    action: &str,
    args: serde_json::Value,
    grade: &str,
    purpose: &str,
    target: &str,
) -> Result<Requested, String> {
    locked().request(
        requester,
        verified,
        app,
        action,
        args,
        grade,
        purpose,
        target,
        Instant::now(),
        &hhmm(),
    )
}

pub fn status(id: &str) -> Option<Status> {
    locked().status(id, Instant::now())
}

/// **UI only.** See the module doc: the single caller is the Allow button's callback.
pub(crate) fn grant(id: &str) -> Result<(), String> {
    locked().grant(id, Instant::now(), &hhmm())
}

/// **UI only.** See the module doc: the single caller is the "Allow for this session" callback.
pub(crate) fn grant_for_session(id: &str) -> Result<(), String> {
    locked().grant_for_session(id, Instant::now(), &hhmm())
}

/// **UI only.** See the module doc: the single caller is the Deny button's callback.
pub(crate) fn deny(id: &str) -> Result<(), String> {
    locked().deny(id, Instant::now(), &hhmm())
}

/// One card by id, as the UI sees it. Used by the session-rule callback, which needs the
/// action's grade and purpose to decide whether a rule may exist for it at all.
pub fn card(id: &str) -> Option<Card> {
    cards().into_iter().find(|c| c.id == id)
}

/// Which agent a request was asked for — empty for none — while the store still holds it.
pub fn agent_of(id: &str) -> Option<String> {
    locked().records.iter().find(|r| r.id == id).map(|r| r.verified.agent.clone())
}

/// How a request came out, as the pane of the agent that asked says it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Allowed,
    Denied,
    /// Nobody answered in time — or the store no longer holds it, which is the same thing to
    /// whoever is still waiting.
    Unanswered,
    Withdrawn,
}

impl Store {
    /// `None` while the request is still waiting; else how it came out and the line it leaves. An
    /// allowed request stays allowed after its grant is spent or runs out: that is what the person
    /// said.
    pub fn outcome(&self, id: &str, now: Instant) -> Option<(Outcome, String)> {
        let Some(record) = self.records.iter().find(|r| r.id == id) else {
            return Some((Outcome::Unanswered, String::new()));
        };
        let status = record.status(now);
        let outcome = match (status, record.state) {
            (Status::Pending, _) => return None,
            (_, Status::Granted | Status::Consumed) => Outcome::Allowed,
            (_, Status::Denied) => Outcome::Denied,
            _ if record.withdrawn => Outcome::Withdrawn,
            _ => Outcome::Unanswered,
        };
        Some((outcome, record_line(record, status)))
    }
}

/// See [`Store::outcome`].
pub fn outcome(id: &str) -> Option<(Outcome, String)> {
    locked().outcome(id, Instant::now())
}

/// Take back what `agent` is still waiting on. See [`Store::withdraw_for_agent`]: the shell's own
/// call when it stops an agent or sees its harness go, never an action on the socket.
pub(crate) fn withdraw_for_agent(agent: &str) -> Vec<String> {
    locked().withdraw_for_agent(agent, Instant::now(), &hhmm())
}

pub fn consume(
    id: &str,
    app: &str,
    action: &str,
    args: &serde_json::Value,
) -> Result<(), String> {
    locked().consume(id, app, action, args, Instant::now())
}

pub fn cards() -> Vec<Card> {
    locked().cards(Instant::now())
}

pub fn pending() -> Vec<Card> {
    locked().pending(Instant::now())
}

#[cfg(test)]
mod approvals_tests {
    use super::*;

    fn args(json: serde_json::Value) -> serde_json::Value {
        json
    }

    /// A fresh store per test. Nothing here touches the process-wide one, so the tests do not
    /// have to run in any order and cannot interfere with each other.
    /// What the machine works out for itself about a Hermes request, as `caller_identity`
    /// would hand it over. A fixture here rather than a real walk: this module's job is to
    /// carry it beside the claim without mixing them, and that is what these tests check.
    fn verified() -> Verified {
        Verified {
            line: "python -m hermes_cli.main gateway (pid 696) \u{b7} the attached mind".into(),
            exe: "/home/pranab/hermes-agent/venv/bin/python".into(),
            pid: 696,
            attached_mind: "Hermes Agent".into(),
            discrepancies: Vec::new(),
            agent: String::new(),
        }
    }

    fn ask(store: &mut Store, now: Instant) -> String {
        store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                args(serde_json::json!({"id": "evt-3", "confirm": true})),
                "sensitive",
                "Delete an event from the calendar. It is not recoverable.",
                "",
                now,
                "12:03",
            )
            .expect("a first request is accepted")
            .id
    }

    #[test]
    fn approvals_a_grant_authorises_once_and_only_once() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        let call = serde_json::json!({"id": "evt-3", "confirm": true});

        assert_eq!(store.status(&id, now), Some(Status::Pending));
        store.grant(&id, now, "12:03").expect("a person pressed Allow");
        assert_eq!(store.status(&id, now), Some(Status::Granted));

        store
            .consume(&id, "calendar", "delete_event", &call, now)
            .expect("the grant covers exactly this call");

        assert_eq!(store.status(&id, now), Some(Status::Consumed));
        let again = store
            .consume(&id, "calendar", "delete_event", &call, now)
            .expect_err("a grant is single use");
        assert!(again.contains("already used"), "{again}");
    }

    #[test]
    fn approvals_key_order_does_not_matter_but_any_value_does() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.grant(&id, now, "12:03").unwrap();

        // The same arguments, written the other way round. JSON objects are unordered and the
        // transport does not promise an order, so a reshuffle must not invalidate a grant.
        let reordered = serde_json::json!({"confirm": true, "id": "evt-3"});
        store
            .consume(&id, "calendar", "delete_event", &reordered, now)
            .expect("key order is not part of what the person approved");
    }

    #[test]
    fn approvals_a_changed_argument_invalidates_the_grant() {
        for changed in [
            serde_json::json!({"id": "evt-4", "confirm": true}),
            serde_json::json!({"id": "evt-3", "confirm": false}),
            serde_json::json!({"id": "evt-3"}),
            serde_json::json!({"id": "evt-3", "confirm": true, "force": true}),
            // `"true"` is not `true`. A string that looks like a boolean is a different value
            // and the person approved the one they were shown.
            serde_json::json!({"id": "evt-3", "confirm": "true"}),
        ] {
            let mut store = Store::new();
            let now = Instant::now();
            let id = ask(&mut store, now);
            store.grant(&id, now, "12:03").unwrap();
            let err = store
                .consume(&id, "calendar", "delete_event", &changed, now)
                .expect_err("a different argument is a different action");
            assert!(err.contains("Nothing was authorised"), "{changed}: {err}");
            assert!(err.contains("arguments"), "the refusal names the part that moved: {err}");
            assert_eq!(
                store.status(&id, now),
                Some(Status::Granted),
                "a refused consume must not burn the grant"
            );
        }
    }

    #[test]
    fn approvals_a_changed_app_or_action_invalidates_the_grant() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.grant(&id, now, "12:03").unwrap();
        let call = serde_json::json!({"id": "evt-3", "confirm": true});

        let wrong_app = store
            .consume(&id, "notes", "delete_event", &call, now)
            .expect_err("a grant is bound to one app");
        assert!(wrong_app.contains("app `calendar`"), "{wrong_app}");

        let wrong_action = store
            .consume(&id, "calendar", "delete_all_events", &call, now)
            .expect_err("a grant is bound to one action");
        assert!(wrong_action.contains("calendar.delete_event"), "{wrong_action}");
    }

    #[test]
    fn approvals_denial_prevents_consumption() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.deny(&id, now, "12:03").expect("a person pressed Deny");
        assert_eq!(store.status(&id, now), Some(Status::Denied));

        let err = store
            .consume(
                &id,
                "calendar",
                "delete_event",
                &serde_json::json!({"id": "evt-3", "confirm": true}),
                now,
            )
            .expect_err("a denial authorises nothing");
        assert!(err.contains("denied"), "{err}");

        // And granting afterwards is not a second chance at the same card.
        let flip = store.grant(&id, now, "12:04").expect_err("a decided card is decided");
        assert!(flip.contains("denied"), "{flip}");
    }

    #[test]
    fn approvals_an_unanswered_request_expires() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        let later = now + REQUEST_TTL + Duration::from_secs(1);

        assert_eq!(store.status(&id, later), Some(Status::Expired));
        let err = store.grant(&id, later, "12:06").expect_err("an expired card cannot be granted");
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn approvals_a_grant_expires_unused() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.grant(&id, now, "12:03").unwrap();

        let later = now + GRANT_TTL + Duration::from_secs(1);
        assert_eq!(store.status(&id, later), Some(Status::Expired));
        let err = store
            .consume(
                &id,
                "calendar",
                "delete_event",
                &serde_json::json!({"id": "evt-3", "confirm": true}),
                later,
            )
            .expect_err("an expired grant authorises nothing");
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn approvals_flooding_is_refused() {
        let mut store = Store::new();
        let now = Instant::now();
        for n in 0..MAX_PENDING {
            store
                .request(
                    "hermes",
                    verified(),
                    "calendar",
                    "delete_event",
                    serde_json::json!({"id": format!("evt-{n}")}),
                    "sensitive",
                    "Delete an event.",
                    "",
                    now,
                    "12:03",
                )
                .unwrap_or_else(|e| panic!("request {n} should be accepted: {e}"));
        }
        let err = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-99"}),
                "sensitive",
                "Delete an event.",
                "",
                now,
                "12:03",
            )
            .expect_err("a flood is refused");
        assert!(err.contains("already waiting"), "{err}");
        assert_eq!(store.pending(now).len(), MAX_PENDING);
    }

    #[test]
    fn approvals_the_same_question_twice_is_one_card() {
        let mut store = Store::new();
        let now = Instant::now();
        let first = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-3"}),
                "sensitive",
                "Delete an event.",
                "",
                now,
                "12:03",
            )
            .unwrap();
        let second = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-3"}),
                "sensitive",
                "Delete an event.",
                "",
                now,
                "12:03",
            )
            .unwrap();
        assert_eq!(first.id, second.id, "a retry must not stack a second identical card");
        assert_eq!(store.pending(now).len(), 1);
        assert!(first.fresh, "the first ask is what puts the card on screen");
        assert!(
            !second.fresh,
            "a repeat must not read as new, or it would raise the shell over the person's work \
             again — which is a way to hold their screen with a `safe` call"
        );
    }

    #[test]
    fn approvals_a_denial_silences_the_identical_request() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.deny(&id, now, "12:03").unwrap();

        let err = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-3", "confirm": true}),
                "sensitive",
                "Delete an event.",
                "",
                now + Duration::from_secs(5),
                "12:03",
            )
            .expect_err("asking again straight after a no is refused");
        assert!(err.contains("denied"), "{err}");

        // Different arguments are a different question, and always allowed through.
        store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-9"}),
                "sensitive",
                "Delete an event.",
                "",
                now + Duration::from_secs(5),
                "12:03",
            )
            .expect("a different question is not the denied one");

        // And once the quiet window has passed, the person may be asked again.
        store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "evt-3", "confirm": true}),
                "sensitive",
                "Delete an event.",
                "",
                now + DENIAL_QUIET + Duration::from_secs(1),
                "12:03",
            )
            .expect("a denial is not permanent");
    }

    #[test]
    fn approvals_an_unknown_id_is_not_an_expiry() {
        let store = Store::new();
        assert_eq!(store.status("appr-404", Instant::now()), None);
    }

    #[test]
    fn approvals_canonical_json_sorts_keys_and_keeps_array_order() {
        let a = serde_json::json!({"b": 1, "a": {"d": 4, "c": 3}});
        let b = serde_json::json!({"a": {"c": 3, "d": 4}, "b": 1});
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(canonical(&a), r#"{"a":{"c":3,"d":4},"b":1}"#);

        let one = serde_json::json!({"to": ["ann", "bob"]});
        let other = serde_json::json!({"to": ["bob", "ann"]});
        assert_ne!(
            canonical(&one),
            canonical(&other),
            "array order is data — two recipients in the other order is a different send"
        );
    }

    #[test]
    fn approvals_what_the_caller_says_and_what_the_machine_knows_stay_apart() {
        // The whole of issue #43 in one assertion: the card carries two answers to "who is
        // asking" and neither is allowed to become the other. Folding them — showing only the
        // verified line, or letting the claim overwrite it — would put the machine's authority
        // behind a string the caller chose, which is what the card used to do.
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        let card = store.cards(now).into_iter().find(|c| c.id == id).expect("the card");

        assert_eq!(card.requester, "hermes", "the claim is untouched");
        assert_eq!(card.verified.pid, 696);
        assert_eq!(card.verified.attached_mind, "Hermes Agent");
        assert!(card.verified.line.contains("pid 696"), "{}", card.verified.line);
        assert_ne!(card.requester, card.verified.line);

        // And `describe shell` publishes them under separate keys for the same reason.
        let json = card.verified.to_json();
        assert_eq!(json["pid"], 696);
        assert_eq!(json["exe"], "/home/pranab/hermes-agent/venv/bin/python");
        assert_eq!(json["attached_mind"], "Hermes Agent");
        assert!(json.get("requester").is_none(), "the claim does not live in here");
    }

    #[test]
    fn approvals_an_unidentifiable_caller_carries_an_empty_fact_not_a_flattering_one() {
        // A caller the machine could not place must not inherit the last one's identity, and
        // must not silently fall back to believing its own name. `Default` is the whole of it;
        // the card turns the empty line into "could not be identified" (see `row_for`).
        let mut store = Store::new();
        let now = Instant::now();
        let id = store
            .request(
                "Your bank",
                Verified::default(),
                "files",
                "delete",
                serde_json::json!({"name": "taxes.pdf"}),
                "dangerous",
                "Delete a file. It is not recoverable.",
                "",
                now,
                "12:03",
            )
            .unwrap()
            .id;

        let card = store.cards(now).into_iter().find(|c| c.id == id).expect("the card");
        assert_eq!(card.requester, "Your bank", "what it said is still shown, verbatim");
        assert_eq!(card.verified.pid, 0);
        assert_eq!(card.verified.exe, "");
        assert_eq!(card.verified.attached_mind, "");
        assert!(card.verified.discrepancies.is_empty());
    }

    #[test]
    fn approvals_the_card_shows_what_the_grant_is_bound_to() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        let cards = store.pending(now);
        assert_eq!(cards.len(), 1);
        let card = &cards[0];
        assert_eq!(card.id, id);
        assert_eq!(card.requester, "hermes");
        assert_eq!(card.grade, "sensitive");
        // Every argument the grant is bound to is on the card, one line each, in the same order.
        assert_eq!(card.args, vec!["confirm: true".to_string(), "id: evt-3".to_string()]);
        assert_eq!(
            card.warning, "The app says this cannot be undone.",
            "the app's own sentence about recoverability has to reach the person"
        );
        // Nothing in the store's own `ask` line resolved to a name, and that empty row stays
        // empty rather than showing a placeholder.
        assert_eq!(card.target, "");
    }

    /// #54: a person asked "may this be deleted?" about `{"id": "01a0c718-…"}` could not answer
    /// from a handle. The app's naming index turns the id back into the appointment on a line
    /// beside the arguments — and stays out of what the grant binds to.
    #[test]
    fn approvals_the_card_names_the_thing_not_the_handle() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "01a0c718-3931-7342-b9c7-8de36140ddb0"}),
                "sensitive",
                "Take an event off the calendar. It is not recoverable.",
                "id 01a0c718… is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}",
                now,
                "12:03",
            )
            .unwrap()
            .id;
        let card = store.pending(now).into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(card.target, "id 01a0c718… is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}");
        // One row, or the card's height stops being arithmetic.
        assert!(!card.target.contains('\n'));

        // The same question from the same agent, while it is still waiting, is one card however
        // its ids read — the naming line cannot smuggle a second card onto the screen beside the
        // first, because it is not part of what makes a question the question it is.
        let again = store
            .request(
                "hermes",
                verified(),
                "calendar",
                "delete_event",
                serde_json::json!({"id": "01a0c718-3931-7342-b9c7-8de36140ddb0"}),
                "sensitive",
                "Take an event off the calendar. It is not recoverable.",
                "a different sentence about the same event",
                now,
                "12:04",
            )
            .unwrap();
        assert!(!again.fresh, "the repeat found the card already up, not a second one");
        assert_eq!(again.id, id);

        // A grant is bound to the arguments the person was shown, byte for byte — the name is a
        // thing said about them, not another thing approved beside them. So consume reads the
        // uuid alone, and the uuid alone is what decides it.
        store.grant(&id, now, "12:05").unwrap();
        store
            .consume(
                &id,
                "calendar",
                "delete_event",
                &serde_json::json!({"id": "01a0c718-3931-7342-b9c7-8de36140ddb0"}),
                now,
            )
            .expect("the name changes nothing the grant is bound to");
    }

    /// The card is a known number of lines, and nothing is hidden in silence.
    ///
    /// This is the other half of the clipped-header bug. The markup was asking a Rectangle with
    /// only conditional children how tall it wanted to be and getting zero — but even with that
    /// fixed, a card whose text can grow without limit runs off the screen and takes its buttons
    /// with it. So every element is bounded here: one line per argument, a fixed number of
    /// arguments. The purpose is the exception, and has its own test below.
    #[test]
    fn approvals_the_card_is_a_bounded_number_of_lines() {
        let long = "x".repeat(400);
        let mut args = serde_json::Map::new();
        for n in 0..12 {
            args.insert(format!("arg{n:02}"), serde_json::json!(long));
        }
        let rows = args_rows(&serde_json::Value::Object(args));

        assert_eq!(rows.len(), ARG_ROWS + 1, "eight arguments, then one line about the rest");
        for row in &rows[..ARG_ROWS] {
            assert!(
                !row.contains('\n'),
                "each argument is its own single-line Text on the card: {row}"
            );
            assert!(
                row.chars().count() < ARG_VALUE_CHARS + 40,
                "a long value is cut so the row stays one line: {} chars",
                row.chars().count()
            );
            assert!(row.contains("characters in full"), "and says it was cut: {row}");
        }
        assert!(rows[ARG_ROWS].contains("4 more argument"), "{}", rows[ARG_ROWS]);
        assert!(
            rows[ARG_ROWS].contains("bound to all of them"),
            "the person must not think the grant only covers what fitted: {}",
            rows[ARG_ROWS]
        );

    }

    /// What Studio publishes for `set_backend`, verbatim, as `yos describe studio` prints it.
    /// 585 characters: the longest description on this desktop, and the one the card cut.
    const SET_BACKEND_PURPOSE: &str = "Choose where pictures are made from now on, and write that \
        choice down in the configuration file. Graded `sensitive` because it decides where every \
        later prompt goes: naming a hosted service means the sentences typed into this app will \
        leave this machine and may cost money. `generate` and `variations` are regraded the moment \
        this lands, so a caller cannot point Studio at a service and generate in the same breath \
        under the old, local grade. No key is taken here — only the NAME of an environment \
        variable that holds one, which is read at call time and never stored, logged or shown.";

    fn ask_studio(store: &mut Store, now: Instant, args: serde_json::Value) -> Card {
        let id = store
            .request("hermes", verified(), "studio", "set_backend", args, "sensitive",
                SET_BACKEND_PURPOSE, "", now, "19:32")
            .unwrap()
            .id;
        store.pending(now).into_iter().find(|c| c.id == id).expect("the card")
    }

    /// The app's sentence reaches the card whole. Cutting it is what hid the clause that mattered.
    ///
    /// On 22 September the card for `studio.set_backend` read "…naming a hosted service means the
    /// sentences typed into this app will leav… (585 characters in full)": cut mid-word at 240,
    /// with the condition turned into a flat statement and no way to read the rest. The same
    /// fragment was shown for `kind: fake`, the direction that stops anything leaving.
    #[test]
    fn approvals_the_purpose_reaches_the_card_whole() {
        assert_eq!(SET_BACKEND_PURPOSE.chars().count(), 585, "the fixture is the real sentence");

        let mut store = Store::new();
        let now = Instant::now();
        let back = ask_studio(&mut store, now, serde_json::json!({"kind": "fake"}));
        assert_eq!(back.purpose, SET_BACKEND_PURPOSE, "every character, none of them replaced");
        assert!(!back.purpose.contains("characters in full"));
        assert_eq!(
            back.summary,
            "Choose where pictures are made from now on, and write that choice down in the \
             configuration file.",
            "and its first sentence is the card's one line"
        );
        assert!(
            back.purpose.contains("will leave this machine and may cost money."),
            "the clause the cut fell on is the one that says why to care"
        );

        // The card owes the person the app's words and the arguments; it does not add a
        // hosted-service warning of its own, in either direction. The shell has no idea what
        // `kind: fake` means to Studio — that `fake` stays on this machine is Studio's knowledge,
        // and it is in the sentence above, read whole. What differs between the two cards is the
        // argument box, and a red line the shell could not stand behind would read as the OS
        // vouching for a danger it has not established.
        assert_eq!(back.warning, "", "nothing the shell can vouch for, so nothing in red");
        let away = ask_studio(
            &mut store,
            now,
            serde_json::json!({"kind": "openai-images", "model": "gpt-image-1"}),
        );
        assert_eq!(away.warning, "");
        assert_eq!(away.purpose, back.purpose, "the same sentence, because it is the app's");
        assert_ne!(away.args, back.args, "and the arguments are what tell the two apart");
        assert_eq!(back.args, vec!["kind: fake".to_string()]);

        // The purpose is still bounded — against a description the size of a document, not
        // against a long sentence — and the bound respects words. 1000 characters, the size of
        // the longest published sentence with room to spare, arrives whole.
        let long = ask_studio(&mut store, now, serde_json::json!({"kind": "comfyui"}));
        assert_eq!(long.purpose.chars().count(), 585);
        let thousand = "word ".repeat(200);
        assert_eq!(clip_at_word(&thousand, PURPOSE_CHARS), thousand);
        assert!(1000 < PURPOSE_CHARS, "the bound is well above any sentence an app publishes");

        // Past the bound the cut lands between words and names the true length; the marker is
        // the same one an argument value carries, so a person learns one convention.
        let document = "sentence ".repeat(400);
        let cut = clip_at_word(&document, PURPOSE_CHARS);
        assert!(cut.ends_with("… (3600 characters in full)"), "{cut}");
        let head = cut.split('…').next().unwrap();
        assert!(head.ends_with("sentence"), "cut at a word boundary, not inside one: {head:?}");
        assert!(!head.ends_with(' '), "and without a trailing space before the marker");
        assert!(head.chars().count() <= PURPOSE_CHARS);
        assert!(head.chars().count() > PURPOSE_CHARS - 20, "close to the bound, not far short of it");

        // A run with no space in it — nothing to respect — is cut where `clip` would cut it,
        // so the bound is a bound and not a wish.
        let unbroken = "x".repeat(PURPOSE_CHARS + 5);
        assert_eq!(clip_at_word(&unbroken, PURPOSE_CHARS), clip(&unbroken, PURPOSE_CHARS));
    }

    /// What `shell.run_recipe` publishes, verbatim — the description on the card the live tour
    /// could not answer (#218): a paragraph of protocol detail where a person needs one line.
    const RUN_RECIPE_PURPOSE: &str = "Start a recipe with its inputs: a built-in one by its name \
        or id, or one a mind made. A formation — Council, Red team, Build, Writers' room; \
        `describe shell` → `recipes` → `formations` lists each with its `inputs` — hands work to \
        roles from the agent catalog: each works in its own pane on the Agents screen, its row \
        saying which recipe it works for, and the recipe's stages light on the Recipes screen as \
        they answer; its result comes as the recipe's completion. Answers with the run's id; \
        `describe shell` → `recipes` shows how it goes, and `cancel_recipe` stops it and lets its \
        agents go. An agent another agent or a recipe started cannot start a formation.";

    /// The card leads with one person-facing line: the app's own first sentence. The paragraph
    /// still reaches the card whole — this adds the line, it does not cut what was there.
    #[test]
    fn approvals_the_card_leads_with_one_person_facing_line() {
        assert_eq!(
            first_sentence(RUN_RECIPE_PURPOSE),
            "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made."
        );
        // A period inside a sentence does not end it: the cut is at a period the text goes on
        // from, which is how "e.g." and "0.5" survive.
        assert_eq!(
            first_sentence("Runs e.g. the Council, then reports."),
            "Runs e.g. the Council, then reports."
        );
        assert_eq!(first_sentence("Waits 0.5 s, then retries."), "Waits 0.5 s, then retries.");
        // A description with no sentence end in it is its own first sentence; empty stays empty.
        assert_eq!(first_sentence("Delete an event"), "Delete an event");
        assert_eq!(first_sentence(""), "");

        // A runaway first sentence is bounded at a word, and says it was cut.
        let runaway = summary_of(&"word ".repeat(100));
        assert!(runaway.contains("characters in full"), "{runaway}");
        assert!(runaway.chars().count() < SUMMARY_CHARS + 40, "{} chars", runaway.chars().count());

        let mut store = Store::new();
        let now = Instant::now();
        let id = store
            .request(
                "pi 0.87",
                for_agent("pi:c-7f3a91"),
                "shell",
                "run_recipe",
                args(serde_json::json!({"recipe": "writers-room"})),
                "sensitive",
                RUN_RECIPE_PURPOSE,
                "",
                now,
                "12:03",
            )
            .unwrap()
            .id;
        let card = store.pending(now).into_iter().find(|c| c.id == id).expect("the card");
        assert_eq!(
            card.summary,
            "Start a recipe with its inputs: a built-in one by its name or id, or one a mind made."
        );
        assert_eq!(card.purpose, RUN_RECIPE_PURPOSE, "the paragraph still reaches the card whole");
        assert!(card.purpose.starts_with(card.summary.as_str()));

        // An action with nothing published gets an empty line, not a made-up one; the card hides
        // the row, and its purpose block already says "(the app publishes no description…)".
        let none = store
            .request(
                "pi 0.87",
                for_agent("pi:c-7f3a91"),
                "shell",
                "agent_run",
                args(serde_json::json!({"command": "true"})),
                "sensitive",
                "",
                "",
                now,
                "12:04",
            )
            .unwrap()
            .id;
        let card = store.pending(now).into_iter().find(|c| c.id == none).expect("the card");
        assert_eq!(card.summary, "");
    }

    #[test]
    fn approvals_a_dangerous_grade_always_warns() {
        assert!(warning_for("dangerous", "Kill a process.").contains("dangerous"));
        assert!(warning_for("dangerous", "Erase the disk. It is not recoverable.")
            .contains("cannot be undone"));
        assert!(warning_for("standard", "Open a note.").is_empty());
    }

    /// The predicate the card draws its warning from is the same one that decides whether a
    /// standing yes may exist. Two features, one judgement — if they ever disagree, a person is
    /// offered "stop asking me about this" for an action the card has just called irreversible.
    #[test]
    fn approvals_a_session_rule_is_never_offered_for_what_the_card_warns_about() {
        assert!(may_offer_session_rule("sensitive", "Move a file to another folder."));
        assert!(may_offer_session_rule("standard", "Open a note."));

        for (grade, purpose) in [
            ("dangerous", "End a process."),
            ("sensitive", "Delete an event from the calendar. It is not recoverable."),
            ("sensitive", "Erase the device. This cannot be undone."),
            ("sensitive", "Remove the container permanently."),
        ] {
            assert!(
                !may_offer_session_rule(grade, purpose),
                "a rule must not be offered for `{purpose}`"
            );
            assert!(
                !warning_for(grade, purpose).is_empty(),
                "and the card must already be warning about it: `{purpose}`"
            );
        }
    }

    /// The card says which of the two the person chose, because they will ask afterwards.
    #[test]
    fn approvals_a_session_grant_says_so_in_the_transcript() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.grant_for_session(&id, now, "12:03").expect("a person pressed the third button");

        let card = store.cards(now).into_iter().find(|c| c.id == id).unwrap();
        assert_eq!(card.record, "Allowed for this session: calendar.delete_event — 12:03");

        // And it is still one grant, for these arguments, once — the standing part lives in
        // `mind_mode`, not in the store.
        let call = serde_json::json!({"id": "evt-3", "confirm": true});
        store.consume(&id, "calendar", "delete_event", &call, now).unwrap();
        let again = store
            .consume(&id, "calendar", "delete_event", &call, now)
            .expect_err("a session rule does not make the grant reusable");
        assert!(again.contains("already used"), "{again}");
    }

    #[test]
    fn approvals_a_decision_leaves_a_line_in_the_transcript() {
        let mut store = Store::new();
        let now = Instant::now();
        let id = ask(&mut store, now);
        store.grant(&id, now, "12:03").unwrap();
        let record = store.cards(now).into_iter().find(|c| c.id == id).unwrap().record;
        assert_eq!(record, "Allowed once: calendar.delete_event — 12:03");

        let mut store = Store::new();
        let id = ask(&mut store, now);
        store.deny(&id, now, "12:05").unwrap();
        let record = store.cards(now).into_iter().find(|c| c.id == id).unwrap().record;
        assert_eq!(record, "Denied: calendar.delete_event — 12:05");
    }

    fn for_agent(agent: &str) -> Verified {
        Verified { agent: agent.into(), ..verified() }
    }

    fn ask_as(store: &mut Store, agent: &str, now: Instant) -> Requested {
        store
            .request(
                "pi 0.87",
                for_agent(agent),
                "shell",
                "agent_run",
                args(serde_json::json!({"command": "rm -rf build"})),
                "sensitive",
                "",
                "",
                now,
                "12:03",
            )
            .unwrap()
    }

    /// Design decision 4: an approval carries its agent, and the same question from another agent
    /// is another question — its own card, in its own pane, answered on its own.
    #[test]
    fn approvals_the_same_question_from_two_agents_is_two_cards() {
        let mut store = Store::new();
        let now = Instant::now();
        let pi = ask_as(&mut store, "pi:c-7f3a91", now);
        let again = ask_as(&mut store, "pi:c-7f3a91", now);
        assert_eq!((again.id.as_str(), again.fresh), (pi.id.as_str(), false), "the same agent asking twice is one card");
        let ds = ask_as(&mut store, "deepseek:c-02be44", now);
        assert!(ds.fresh && ds.id != pi.id, "another agent asking the same thing is a card of its own");
        store.grant(&pi.id, now, "12:04").unwrap();
        assert_eq!(store.status(&ds.id, now), Some(Status::Pending), "one agent's Allow answers only its own request");
        // What each agent's pane is told: allowed, with the line the Lens shows; the other still waiting.
        assert_eq!(store.outcome(&pi.id, now), Some((Outcome::Allowed, "Allowed once: shell.agent_run — 12:04".into())));
        assert_eq!(store.outcome(&ds.id, now), None);
        store.consume(&pi.id, "shell", "agent_run", &serde_json::json!({"command": "rm -rf build"}), now).unwrap();
        assert_eq!(store.outcome(&pi.id, now).map(|o| o.0), Some(Outcome::Allowed), "spent, it is still what the person said");
        assert_eq!(store.outcome("appr-999", now).map(|o| o.0), Some(Outcome::Unanswered), "a request the store no longer holds");
        let card = store.cards(now).into_iter().find(|c| c.id == ds.id).unwrap();
        assert_eq!(card.verified.agent, "deepseek:c-02be44");
        assert_eq!(card.verified.to_json()["agent"], "deepseek:c-02be44", "describe says which agent asked");
    }

    /// "When a harness dies": a card for work that is no longer happening is refused, never
    /// granted — and only that agent's cards are taken back.
    #[test]
    fn approvals_a_withdrawn_request_cannot_be_granted_and_says_why() {
        let mut store = Store::new();
        let now = Instant::now();
        let pi = ask_as(&mut store, "pi:c-7f3a91", now).id;
        let ds = ask_as(&mut store, "deepseek:c-02be44", now).id;
        assert_eq!(store.withdraw_for_agent("", now, "12:04"), Vec::<String>::new(), "no agent, nothing taken back");
        assert_eq!(store.withdraw_for_agent("pi:c-7f3a91", now, "12:04"), vec![pi.clone()]);
        assert_eq!(store.status(&pi, now), Some(Status::Expired), "a poller hears that nobody answered");
        assert!(store.grant(&pi, now, "12:05").is_err(), "and a late click grants nothing");
        let err = store
            .consume(&pi, "shell", "agent_run", &serde_json::json!({"command": "rm -rf build"}), now)
            .unwrap_err();
        assert!(err.contains("expired"), "{err}");
        let record = store.cards(now).into_iter().find(|c| c.id == pi).unwrap().record;
        assert!(record.starts_with("Withdrawn: shell.agent_run — 12:04"), "{record}");
        assert_eq!(store.outcome(&pi, now), Some((Outcome::Withdrawn, record)));
        assert_eq!(store.status(&ds, now), Some(Status::Pending), "another agent's card stays up");
    }
}
