//! What the mind may do without being asked — four modes, and what makes each of them safe.
//!
//! # Why this exists
//!
//! `approvals.rs` shipped one fixed policy: anything above the bridge's ceiling puts a card in
//! front of the person and waits. That is right for the action somebody is watching and wrong for
//! both ends of the day. A long trusted job — "tidy these forty files" — becomes forty cards, and
//! a person clicking Allow forty times is a person who will click Allow on the forty-first
//! without reading it, which is the exact failure `approvals.rs` is built to avoid. At the other
//! end, "just look, don't touch" has no expression at all: the only way to stop a mind acting was
//! to lower the machine ceiling and remember to put it back.
//!
//! So the desktop gains what a coding agent's CLI has had for a while: a mode. `plan` reads and
//! does not touch. `ask` is today's behaviour. `auto` stops asking about the routine sensitive
//! things. `bypass` stops asking entirely, for a while, and says so loudly.
//!
//! # The grade is not the only thing that decides
//!
//! [`Modes::decide`] also takes whether the action's own published purpose says it cannot be
//! undone (`approvals::unrecoverable`). `auto` asks about those exactly as it asks about a
//! `dangerous` one, because the mode menu promises "You are still asked about the destructive
//! ones" and `calendar.delete_event` — graded `sensitive`, published as "It is not recoverable"
//! — ran under it with nobody asked. See the 21 September entry in
//! `design/mind-modes-2026-09-21.md`.
//!
//! # The mode is always BELOW the machine ceiling
//!
//! `tool_permission` — the owner's standing policy, enforced inside every app's runtime with a
//! `CEILING:` refusal — is untouched by any of this. A mode decides what happens to an action at
//! or below that wall; nothing here can move the wall. [`Modes::decide`] takes the ceiling and
//! refuses above it before it looks at the mode at all, so even `bypass` cannot reach past it.
//!
//! # Only a person can make this more permissive
//!
//! The same invariant as a grant, enforced the same way. [`Modes::person_set_mode`],
//! [`Modes::person_add_rule`] and [`Modes::person_revoke_rule`] are `pub(crate)`, and their only
//! callers are the Slint callbacks in `control_approvals::wire` that a click arrives on. The
//! socket gets [`lower_from_socket`], which refuses anything that would loosen the mode and says
//! where a person can change it.
//!
//! A mind putting ITSELF into plan mode is useful and harmless — it is the mind saying "check my
//! work before I touch anything" — so lowering is published. Raising is not, and
//! `mind_mode_only_a_person_can_raise_the_mode` in `control_approvals.rs` reads the source of
//! every `control*.rs` to keep it that way.
//!
//! # What is deliberately not here
//!
//! No standing permission that survives a restart: `bypass` is never written to the settings file
//! and a session rule dies with the shell. The standing policy on this machine is
//! `tool_permission`, set at the keyboard, and a second one minted from a card would be a second
//! place for the truth to live. See `design/mind-modes-2026-09-21.md`.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::approvals;

/// The permission ladder, loosest last. The same four words every app publishes its actions with.
pub const GRADES: [&str; 4] = ["safe", "standard", "sensitive", "dangerous"];

/// Where a grade sits on the ladder, or `None` for a word this OS does not define.
///
/// `None` is not "safe". An ungradeable action is refused everywhere it appears — running
/// something whose cost was never read is the failure that matters here.
pub fn grade_rank(grade: &str) -> Option<usize> {
    GRADES.iter().position(|g| *g == grade)
}

const SENSITIVE: usize = 2;
const DANGEROUS: usize = 3;

/// How long the in-memory audit list holds, for the menu and for `describe shell`.
const AUDIT_MEMORY: usize = 50;

/// How many entries `describe shell` publishes. Ten is what fits in a menu and in a glance.
pub const AUDIT_PUBLISHED: usize = 10;

/// How large the audit file may get before the oldest lines are dropped.
///
/// A log that grows forever on a desktop is a log that fills a disk, and the entries that matter
/// are the recent ones — "what has this thing been doing today". 200 KiB is roughly two thousand
/// entries, which is far more than a session produces.
const AUDIT_FILE_MAX: u64 = 200 * 1024;

/// How many lines survive a trim.
const AUDIT_FILE_KEEP: usize = 400;

// ── The four modes ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Read, and do not touch. Every write is refused with an explanation, so the mind can say
    /// what it WOULD do and a person can read the plan before any of it happens.
    Plan,
    /// The behaviour `approvals.rs` shipped: routine actions run, sensitive ones raise a card.
    Ask,
    /// Sensitive actions run without asking; `dangerous` still raises a card — and so does
    /// anything the app's own published purpose says cannot be undone, whatever its grade.
    Auto,
    /// Everything below the machine ceiling runs. Time-boxed, never persisted.
    Bypass,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Plan => "plan",
            Mode::Ask => "ask",
            Mode::Auto => "auto",
            Mode::Bypass => "bypass",
        }
    }

    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim().to_ascii_lowercase().as_str() {
            "plan" => Some(Mode::Plan),
            "ask" => Some(Mode::Ask),
            "auto" => Some(Mode::Auto),
            "bypass" => Some(Mode::Bypass),
            _ => None,
        }
    }

    /// How much the mind may do unasked. Higher is looser; this is the only ordering that
    /// decides whether a change is a raise or a lowering.
    pub fn permissiveness(self) -> u8 {
        match self {
            Mode::Plan => 0,
            Mode::Ask => 1,
            Mode::Auto => 2,
            Mode::Bypass => 3,
        }
    }

    /// The word on the chip in the status bar.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Plan => "Plan",
            Mode::Ask => "Ask",
            Mode::Auto => "Auto",
            Mode::Bypass => "Bypass",
        }
    }

    /// One line of plain words, for the menu and for a refusal. No grades in it: a person
    /// choosing a mode should not have to know the ladder to understand the choice.
    ///
    /// `Auto`'s sentence is unchanged by the 21 September fix and that is the point of the fix.
    /// "You are still asked about the destructive ones" was already the promise; what was wrong
    /// was that the table only kept it for the `dangerous` grade, while `calendar.delete_event`
    /// — `sensitive`, published as "It is not recoverable" — ran with nobody asked. The table
    /// moved to the sentence, not the sentence to the table. Settings shows this string and the
    /// mode menu shows its own copy of it (`components/mind_mode_menu.slint`); both stay true.
    pub fn meaning(self) -> &'static str {
        match self {
            Mode::Plan => "Look, don't touch. It can read anything and change nothing.",
            Mode::Ask => "It asks you before anything that could matter.",
            Mode::Auto => "It gets on with things. You are still asked about the destructive ones.",
            Mode::Bypass => "It does not ask. Everything the machine allows, it does.",
        }
    }
}

/// How long a bypass lasts. Chosen on the confirmation, never assumed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bypass {
    Minutes15,
    Hour,
    /// No deadline — but still not persisted, so it ends at the next restart at the latest.
    UntilRestart,
}

impl Bypass {
    pub fn parse(text: &str) -> Option<Bypass> {
        match text.trim().to_ascii_lowercase().as_str() {
            "15m" | "15min" | "15" => Some(Bypass::Minutes15),
            "1h" | "hour" | "60m" => Some(Bypass::Hour),
            "restart" | "until_restart" | "session" => Some(Bypass::UntilRestart),
            _ => None,
        }
    }

    fn deadline(self, now: Instant) -> Option<Instant> {
        let real = match self {
            Bypass::Minutes15 => 15 * 60,
            Bypass::Hour => 60 * 60,
            // Nothing to shorten. "Until the shell restarts" means what it says, and the test
            // hook below deliberately cannot reach it — a bypass with no clock has no moment of
            // lapse for a notification to be about.
            Bypass::UntilRestart => return None,
        };
        Some(now + Duration::from_secs(shortened(real)))
    }
}

/// The one thing on this machine that changes how long a bypass lasts, and it is a test hook.
///
/// Verifying the lapse notification on a real machine otherwise means sitting in front of it for
/// fifteen minutes, which is how a check stops being run. So the shell reads a duration out of
/// the environment it was STARTED in — once, into a `OnceLock`, so nothing on the socket, no
/// click and no settings file can reach it afterwards:
///
/// ```sh
/// YANTRIK_BYPASS_SECONDS=20 yantrik-ui      # every timed bypass ends after 20 seconds
/// ```
///
/// **It can only ever make a bypass SHORTER.** The value is clamped to the duration the person
/// actually chose ([`shortened_by`]), because a hook that could extend one would be a way to
/// hold a machine in "do not ask me anything" for longer than anybody agreed to — which is the
/// single outcome this whole feature is arranged to prevent. Shortening is a tightening, and
/// tightening is the direction everything here is allowed to move in.
const BYPASS_SECONDS_ENV: &str = "YANTRIK_BYPASS_SECONDS";

fn bypass_seconds_override() -> Option<u64> {
    static OVERRIDE: OnceLock<Option<u64>> = OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let raw = std::env::var(BYPASS_SECONDS_ENV).ok()?;
        let secs: u64 = raw.trim().parse().ok()?;
        if secs == 0 {
            // A bypass that ends the instant it starts is not a shorter bypass, it is a broken
            // control: the confirmation would say "15 minutes" and the chip would never appear.
            return None;
        }
        tracing::warn!(
            secs,
            env = BYPASS_SECONDS_ENV,
            "a test hook is shortening every timed bypass on this shell"
        );
        Some(secs)
    })
}

fn shortened(real: u64) -> u64 {
    shortened_by(bypass_seconds_override(), real)
}

/// Pure, so "the hook can only shorten" is a test rather than a promise in a comment.
fn shortened_by(hook: Option<u64>, real: u64) -> u64 {
    match hook {
        Some(secs) => secs.min(real),
        None => real,
    }
}

/// Seconds since the epoch.
///
/// The audit is written with one of these, so the window a lapse counts over has to be measured
/// with the same clock — an `Instant` cannot be compared with a line in a file somebody reads
/// tomorrow. Both clocks are therefore carried: `Instant` decides when the bypass ends, and this
/// one decides which audit entries were inside it.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// What [`Modes::decide`] answers.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Decision {
    /// Run it.
    ///
    /// `unasked` is true when `ask` mode would have put a card up for this and the current mode
    /// did not — which is exactly the set of actions the audit log exists for. A person who
    /// loosened the mode is owed a record of what that bought.
    Run { unasked: bool },
    /// Put a card in front of the person and wait.
    Ask,
    /// Do not run it and do not ask. `why` is written to be relayed to a person as-is.
    Refuse { why: String },
}

/// A standing yes for one `(app, action)` with any arguments, until the shell restarts.
///
/// Deliberately not bound to arguments the way a grant is. A grant answers "may I delete THIS
/// event"; a rule answers "stop asking me about listing events" — and a rule that had to match
/// arguments would never match twice, which is the same as no rule at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Rule {
    pub app: String,
    pub action: String,
}

/// A bypass that ended because its clock ran out, rather than because somebody ended it.
///
/// The distinction is the whole of why this type exists. A person who presses `Ask` has just
/// watched the chip change and needs telling nothing; a person who set "1 hour" and walked away
/// has no way at all to learn that the machine went back to asking, and a person sitting in
/// front of it discovers it by being surprised when the next card appears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Lapse {
    /// The mode the machine is back in — what the bypass was covering over.
    pub back_to: Mode,
    /// When the bypass started, seconds since the epoch. The audit entries the notification
    /// counts carry the same clock; see [`unix_now`].
    pub started_unix: u64,
}

/// A [`Lapse`] with the one number the person actually wants: what the bypass bought.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BypassEnded {
    pub back_to: Mode,
    /// How many actions ran unasked inside the window that just closed.
    pub unasked: usize,
}

/// The mode this shell is in. See the module doc for what may mutate it.
#[derive(Clone, Debug)]
pub struct Modes {
    mode: Mode,
    /// What a lapsing bypass returns to. Only meaningful while `mode` is `Bypass`, and kept
    /// rather than recomputed so "back to where you were" is a fact rather than a guess.
    previous: Mode,
    /// `None` while in bypass means "until the shell restarts".
    bypass_until: Option<Instant>,
    /// When the running bypass started, on the audit's clock. Only meaningful while `mode` is
    /// `Bypass`; it is what makes "N things ran without asking **while bypass was on**" a
    /// countable claim rather than a guess at the whole log.
    bypass_started_unix: u64,
    /// A lapse that has happened and has not been reported yet. Armed by [`Modes::lapse`] and
    /// taken exactly once by [`Modes::take_lapse`] — a one-second tick must post one
    /// notification, not sixty a minute.
    lapsed: Option<Lapse>,
    rules: Vec<Rule>,
}

impl Default for Modes {
    fn default() -> Self {
        Modes::new(Mode::Ask)
    }
}

impl Modes {
    pub fn new(mode: Mode) -> Self {
        // A machine must not come up in bypass, so nothing can construct one that has.
        let mode = if mode == Mode::Bypass { Mode::Ask } else { mode };
        Modes {
            mode,
            previous: mode,
            bypass_until: None,
            bypass_started_unix: 0,
            // A freshly built `Modes` owes nobody a notification. This is what makes an
            // "until the shell restarts" bypass silent on the next boot: it is never persisted,
            // so the machine comes up having no memory that one was ever running, and there is
            // no lapse to announce because nothing lapsed — the shell simply restarted.
            lapsed: None,
            rules: Vec::new(),
        }
    }

    /// The mode in force right now.
    ///
    /// Derived rather than stored, like an approval's expiry: a bypass cannot still be in force
    /// merely because no timer happened to fire. [`Modes::lapse`] makes the same fact visible to
    /// the screen; this is what every decision reads.
    pub fn mode(&self, now: Instant) -> Mode {
        if self.mode == Mode::Bypass {
            if let Some(until) = self.bypass_until {
                if now >= until {
                    return self.previous;
                }
            }
        }
        self.mode
    }

    /// What a bypass will fall back to. The mode itself when there is no bypass running.
    pub fn previous(&self, now: Instant) -> Mode {
        if self.mode(now) == Mode::Bypass {
            self.previous
        } else {
            self.mode(now)
        }
    }

    /// How much of the bypass is left. `None` when not in bypass, or when it runs until restart.
    pub fn bypass_left(&self, now: Instant) -> Option<Duration> {
        if self.mode(now) != Mode::Bypass {
            return None;
        }
        self.bypass_until.map(|until| until.saturating_duration_since(now))
    }

    /// True while a bypass with no deadline is running.
    pub fn bypass_until_restart(&self, now: Instant) -> bool {
        self.mode(now) == Mode::Bypass && self.bypass_until.is_none()
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Fold an expired bypass back into the stored mode. Returns whether anything changed.
    ///
    /// The decision path does not need this — [`Modes::mode`] already derives it — but the chip
    /// in the status bar is read from stored state, and a countdown that reaches zero and then
    /// keeps saying "Bypass" is a lie about what the machine is doing.
    ///
    /// It also arms the one notice a lapse owes the person, because this is the only place on
    /// the machine that can tell "the clock ran out" from "somebody chose something else". Both
    /// end a bypass; only one of them is news.
    pub fn lapse(&mut self, now: Instant) -> bool {
        let effective = self.mode(now);
        if effective != self.mode {
            self.lapsed = Some(Lapse { back_to: effective, started_unix: self.bypass_started_unix });
            self.mode = effective;
            self.bypass_until = None;
            return true;
        }
        false
    }

    /// The lapse nobody has been told about yet, if there is one. Taken, not read.
    ///
    /// The tick that first crosses the deadline is the one that reports it; every tick after
    /// that finds nothing. Without this the one-second refresh in `control_approvals::wire`
    /// would post the same notification for as long as the shell ran.
    pub fn take_lapse(&mut self) -> Option<Lapse> {
        self.lapsed.take()
    }

    /// A person chose a mode. **UI only** — see the module doc.
    ///
    /// `bypass` is only ever reached through this function, which is only ever reached from a
    /// click on a confirmation that says what it means. There is no other constructor for it.
    ///
    /// `started_unix` is the wall clock, carried beside `now` because the audit is written with
    /// one and a lapse has to count the entries inside its own window. See [`unix_now`].
    pub(crate) fn person_set_mode(
        &mut self,
        mode: Mode,
        bypass: Bypass,
        now: Instant,
        started_unix: u64,
    ) {
        self.lapse(now);
        // Whatever the clock did a moment ago, the person is at the keyboard choosing a mode
        // right now and the menu in front of them says which. Announcing "the mind is back in
        // Ask mode" over the top of somebody who has just pressed Plan would be telling them
        // something that is no longer true.
        self.lapsed = None;
        if mode == Mode::Bypass {
            // Remember where to come back to, and do not let a bypass chosen twice make its own
            // previous mode `bypass` — that would strand the machine there when it lapsed.
            if self.mode != Mode::Bypass {
                self.previous = self.mode;
                // Only the first of two back-to-back bypasses starts the window. A person who
                // extends one is in one bypass as far as they are concerned, and the count they
                // are shown at the end should cover all of it.
                self.bypass_started_unix = started_unix;
            }
            self.mode = Mode::Bypass;
            self.bypass_until = bypass.deadline(now);
            return;
        }
        self.mode = mode;
        self.previous = mode;
        self.bypass_until = None;
    }

    /// Lower the mode from the socket. Raising is refused.
    ///
    /// The refusal says where a person can do it, because a mind told only "no" will either
    /// retry or invent a way — which is the whole lesson of the `/approve` prompt that did not
    /// exist (see `approvals.rs`).
    pub fn lower_to(&mut self, mode: Mode, now: Instant) -> Result<Mode, String> {
        self.lapse(now);
        let current = self.mode(now);
        if mode == current {
            return Ok(current);
        }
        if mode.permissiveness() > current.permissiveness() {
            return Err(format!(
                "the desktop is in `{}` mode and only the person at this machine can loosen that. \
                 Nothing was changed. They do it from the mode chip in the status bar, or in \
                 Settings → AI & Intelligence. You can go the other way from here — \
                 set_mind_mode to `{}` or `plan` — and saying what you would do and waiting is \
                 usually the faster route anyway.",
                current.as_str(),
                if current == Mode::Bypass { "auto" } else { "ask" },
            ));
        }
        // Lowering out of a bypass ends it rather than leaving a deadline that would later
        // "lapse" the machine back into something looser than what was just chosen.
        self.mode = mode;
        self.previous = mode;
        self.bypass_until = None;
        // Same reason as `person_set_mode`: something has just set the mode deliberately, so a
        // notice saying where the clock would have put us is about a machine that no longer
        // exists. A bypass ended from the socket is not a lapse either way.
        self.lapsed = None;
        Ok(mode)
    }

    /// A person pressed "Allow for this session". **UI only** — see the module doc.
    pub(crate) fn person_add_rule(
        &mut self,
        app: &str,
        action: &str,
        grade: &str,
        purpose: &str,
    ) -> Result<(), String> {
        if !approvals::may_offer_session_rule(grade, purpose) {
            // The card does not draw this button for such an action, so reaching here means the
            // published grade or purpose changed between the paint and the press. Refuse rather
            // than mint a standing yes for something the person was never offered one for.
            return Err(format!(
                "`{app}.{action}` cannot be allowed for a whole session: it is either graded \
                 dangerous or the app says it cannot be undone. Allow it once instead."
            ));
        }
        if self.rules.iter().any(|r| r.app == app && r.action == action) {
            return Ok(());
        }
        self.rules.push(Rule { app: app.to_string(), action: action.to_string() });
        Ok(())
    }

    /// A person pressed the ✕ beside a rule. **UI only** — see the module doc.
    pub(crate) fn person_revoke_rule(&mut self, app: &str, action: &str) {
        self.rules.retain(|r| !(r.app == app && r.action == action));
    }

    fn rule_covers(&self, app: &str, action: &str) -> bool {
        self.rules.iter().any(|r| r.app == app && r.action == action)
    }

    /// The whole decision table, in one place.
    ///
    /// Order matters and is the security argument: the machine ceiling is consulted BEFORE the
    /// mode, so no mode — not even bypass — can reach past the owner's standing policy, and
    /// nothing above it is ever put in front of a person either. A card nobody's answer could
    /// satisfy teaches them that the card is noise.
    ///
    /// `unrecoverable` is [`approvals::unrecoverable`] applied to the action's own published
    /// purpose, and it is a parameter rather than something this function looks up because the
    /// shell cannot read another app's surface from inside a lock. It is passed as the fact it
    /// is, not as the sentence it came from, so there is exactly one place on this machine that
    /// turns a published sentence into a decision. See [`decide`], the one caller.
    pub fn decide(
        &self,
        grade: &str,
        app: &str,
        action: &str,
        unrecoverable: bool,
        ceiling: &str,
        now: Instant,
    ) -> Decision {
        let Some(rank) = grade_rank(grade) else {
            return Decision::Refuse {
                why: format!(
                    "`{app}.{action}` is graded `{grade}`, which is not a level this OS defines. \
                     Nothing was run."
                ),
            };
        };

        if let Some(ceiling_rank) = grade_rank(ceiling) {
            if rank > ceiling_rank {
                return Decision::Refuse {
                    why: format!(
                        "`{app}.{action}` is graded {grade}, and this machine does not allow \
                         callers like this past {ceiling} (`tool_permission`, on the AI page in \
                         Settings). The person was NOT asked, because nothing they could answer \
                         would let it run — this limit is the machine's standing policy, and no \
                         mode changes it. Say what you were trying to do; only someone at the \
                         keyboard can change that setting."
                    ),
                };
            }
        }

        // The app's own sentence about the action, and the one input on this table that is not a
        // grade.
        //
        // Found live on 21 September 2026: `calendar.delete_event` is graded `sensitive` and its
        // published purpose says "It is not recoverable". In `auto` it ran with nobody asked —
        // while the mode menu was promising "You are still asked about the destructive ones".
        // The menu was right and the table was wrong: a person reading that sentence expects a
        // card for something the app itself says cannot be taken back, and the grade ladder has
        // no rung for "recoverable" to sit on. So the sentence decides too.
        //
        // `safe` is excluded deliberately. A read cannot destroy anything, so wording that
        // happens to match — a `safe` action describing something else as permanent — must not
        // be able to turn a look into a question. (Nothing published on this OS today is both
        // `safe` and matching; see the design note.)
        let irreversible = unrecoverable && rank > 0;

        // What `ask` mode would have done. This is what the audit log records, and it is also
        // the only place the phrase "unasked" means anything. It carries `irreversible` so that
        // an action `ask` would now have stopped for is still logged when a looser mode runs it.
        let would_ask = rank >= SENSITIVE || irreversible;

        match self.mode(now) {
            Mode::Plan => {
                if rank == 0 {
                    Decision::Run { unasked: false }
                } else {
                    Decision::Refuse {
                        why: format!(
                            "the desktop is in plan mode, so `{app}.{action}` was NOT run and \
                             nothing on this machine was changed. This is a setting, not a \
                             failure, and not something to work around: reading is still open to \
                             you. Say what you WOULD do — the exact actions and arguments — and \
                             let the person decide. They can switch the mode from the chip in the \
                             status bar."
                        ),
                    }
                }
            }
            // Unchanged by the rule above, and the one mode that is: bypass does not ask, by
            // definition and by what its confirmation says. A person who pressed "Stop asking me
            // anything" on a red panel with a countdown has answered this question already, and
            // a card after that would make the confirmation a lie. It is written down instead —
            // `would_ask` is true here, so it lands in the audit.
            Mode::Bypass => Decision::Run { unasked: would_ask },
            Mode::Auto => {
                if rank >= DANGEROUS || irreversible {
                    self.ask_or_rule(app, action, irreversible)
                } else {
                    Decision::Run { unasked: would_ask }
                }
            }
            Mode::Ask => {
                if would_ask {
                    self.ask_or_rule(app, action, irreversible)
                } else {
                    Decision::Run { unasked: false }
                }
            }
        }
    }

    /// A session rule turns an "ask" into a "run" — and only ever that way round. It can never
    /// make something run that the mode would have refused, because refusals are decided above.
    ///
    /// And it never covers an action the app says cannot be undone. The card refuses to OFFER
    /// one (`approvals::may_offer_session_rule`), which used to be the whole of the guarantee —
    /// but that is checked once, at the moment somebody presses the button, and an app that
    /// rewords its own purpose afterwards would leave a live rule standing over an action that
    /// has since become irreversible. Checked here as well, the two cannot disagree, and the
    /// `auto` rule above means something: a card raised because the app says it cannot be undone
    /// must not be answered by a rule the card would never have offered.
    fn ask_or_rule(&self, app: &str, action: &str, irreversible: bool) -> Decision {
        if !irreversible && self.rule_covers(app, action) {
            Decision::Run { unasked: true }
        } else {
            Decision::Ask
        }
    }
}

// ── The one set of modes this shell has ─────────────────────────────

fn modes() -> &'static Mutex<Modes> {
    static MODES: OnceLock<Mutex<Modes>> = OnceLock::new();
    MODES.get_or_init(|| Mutex::new(Modes::new(stored_mode())))
}

/// A poisoned lock means a previous holder panicked mid-update. The state is four plain fields
/// with no invariant a panic could have half-broken, and failing every decision closed forever
/// is the worse outcome — so the contents are read through. Same reasoning as `approvals::locked`.
fn locked() -> std::sync::MutexGuard<'static, Modes> {
    modes().lock().unwrap_or_else(|e| e.into_inner())
}

/// What the settings file says, clamped to something a machine may boot into.
fn stored_mode() -> Mode {
    let saved = crate::wire::settings::mind_mode();
    match Mode::parse(&saved) {
        // A settings file that says `bypass` was hand-edited or written by a past bug. Booting
        // into it would mean a machine that does not ask, from the first second, with nobody
        // having chosen that in this sitting.
        Some(Mode::Bypass) | None => Mode::Ask,
        Some(mode) => mode,
    }
}

pub fn current() -> Mode {
    locked().mode(Instant::now())
}

/// Fold an expired bypass back. Returns whether the screen has something new to show.
pub fn lapse() -> bool {
    locked().lapse(Instant::now())
}

/// The one notice a lapsing bypass owes the person, or `None`.
///
/// Call it straight after [`lapse`], on the same tick. It answers `Some` exactly once per
/// lapse — the tick that crossed the deadline takes it, and the fifty-nine after it in that
/// minute find nothing.
///
/// Only a lapse. A bypass a person switched off, or one the socket lowered out of, arms nothing:
/// they already know, because they did it. See [`Modes::lapse`].
pub fn take_lapse_notice() -> Option<BypassEnded> {
    let lapse = locked().take_lapse()?;
    // The in-memory list, not the file: the file is bounded at 400 lines and a bypass window is
    // measured in minutes, so anything inside it that the memory has already dropped was one of
    // fifty actions in fifteen minutes and the count is the least of that person's problems.
    let unasked = unasked_during(&recent(AUDIT_MEMORY), lapse.started_unix);
    Some(BypassEnded { back_to: lapse.back_to, unasked })
}

/// How many actions ran unasked inside one bypass window.
///
/// `mode` is what the action actually ran under, as the bridge reported it. An action a session
/// RULE covered is recorded as `rule` and is deliberately NOT counted: it would have run in
/// `ask` mode too, and this sentence is about what the bypass itself bought.
pub fn unasked_during(entries: &[AuditEntry], since_unix: u64) -> usize {
    entries
        .iter()
        .filter(|e| e.mode == Mode::Bypass.as_str() && e.unix >= since_unix)
        .count()
}

/// What the "Bypass ended" notification says.
///
/// Two things, one sentence each: where the machine is now, in its own published words so this
/// and the mode menu can never describe `auto` differently — and what the bypass cost, which is
/// the half nobody can see any other way.
///
/// Here rather than in `wire::notifications` because it is the part worth a test, and a test for
/// a sentence should not need a notification service.
pub fn bypass_ended_body(ended: &BypassEnded) -> String {
    format!(
        "The mind is back in {} mode. {} {}",
        ended.back_to.label(),
        ended.back_to.meaning(),
        unasked_phrase(ended.unasked),
    )
}

/// The count, said the way a person would say it.
///
/// Zero is a different sentence rather than the number nought, and one is the word rather than
/// the digit. "It did 0 things without asking" is the shape of a machine reading a counter out
/// loud, and this notification exists to be read by somebody who has just come back to their
/// desk.
fn unasked_phrase(count: usize) -> String {
    match count {
        0 => "Nothing ran without asking while bypass was on.".to_string(),
        1 => "It did one thing without asking while bypass was on.".to_string(),
        n => format!("It did {n} things without asking while bypass was on."),
    }
}

/// The decision for one action, against this machine's own ceiling.
///
/// `unrecoverable` is [`approvals::unrecoverable`] over the action's published purpose. It is
/// the caller's to work out, because working it out means reading another app's control surface
/// over a socket and this runs inside the mode lock. `control_approvals::request_approval` is
/// the only caller; it reads the purpose the app itself publishes and does not take the
/// requester's word for it.
pub fn decide(grade: &str, app: &str, action: &str, unrecoverable: bool) -> Decision {
    let ceiling = crate::control_approvals::machine_ceiling();
    locked().decide(grade, app, action, unrecoverable, &ceiling, Instant::now())
}

/// Lower the mode from the socket. See [`Modes::lower_to`]; raising is refused.
pub fn lower_from_socket(mode: &str) -> Result<Mode, String> {
    let Some(wanted) = Mode::parse(mode) else {
        return Err(format!(
            "`{mode}` is not a mode. This desktop has four: plan (read only), ask (you are asked \
             about anything that matters), auto (only destructive actions are asked about), \
             bypass (nothing is asked). From here you can only tighten it."
        ));
    };
    if wanted == Mode::Bypass {
        return Err(
            "bypass cannot be entered from here at all — it is the one mode that has to be \
             chosen at the keyboard, on a confirmation that says what it means and for how long. \
             Nothing was changed."
                .to_string(),
        );
    }
    let settled = locked().lower_to(wanted, Instant::now())?;
    persist(settled);
    Ok(settled)
}

/// **UI only.** See the module doc: the single caller is the mode menu's callback.
pub(crate) fn person_set_mode(mode: Mode, bypass: Bypass) {
    locked().person_set_mode(mode, bypass, Instant::now(), unix_now());
    persist(mode);
}

/// **UI only.** See the module doc: the single caller is the card's "Allow for this session".
pub(crate) fn person_add_rule(
    app: &str,
    action: &str,
    grade: &str,
    purpose: &str,
) -> Result<(), String> {
    locked().person_add_rule(app, action, grade, purpose)
}

/// **UI only.** See the module doc: the single caller is the ✕ beside a rule in the menu.
pub(crate) fn person_revoke_rule(app: &str, action: &str) {
    locked().person_revoke_rule(app, action);
}

/// What goes in the settings file for a mode that is live right now.
///
/// A machine that booted into bypass would be a machine nobody had chosen that for in this
/// sitting, which is the one outcome this whole feature must not produce. So a bypass stores the
/// mode it will fall back to, and the file is always something safe to start from. Pure, and
/// separate from the write, because "bypass is never persisted" is a property worth a test and
/// a test should not need a settings file.
fn to_store(mode: Mode, previous: Mode) -> Mode {
    match mode {
        Mode::Bypass => {
            // Belt and braces: a `previous` of bypass would defeat the whole point, and
            // `person_set_mode` already refuses to record one.
            if previous == Mode::Bypass {
                Mode::Ask
            } else {
                previous
            }
        }
        other => other,
    }
}

fn persist(mode: Mode) {
    let previous = locked().previous(Instant::now());
    crate::wire::settings::set_mind_mode(to_store(mode, previous).as_str());
}

// ── What every app reads ────────────────────────────────────────────
//
// The apps enforce the mode now (issue #116): `yantrik_app_runtime::control::Registry::act`
// refuses a call above what the mode allows unless it carries a grant, whichever door it came
// through — the MCP bridge, `yos act`, or a raw client on the socket. An app cannot ask this
// shell over the socket on every call: the shell's own actions cross the same dispatch, and a
// shell asking itself is a call that cannot be answered until it returns. So the mode is
// published the way the ceiling already is — a small file beside `settings.yaml`, rewritten
// whenever the mode or the rules change, read by every dispatch per call.
//
// Bypass IS written here, unlike in `settings.yaml`. This file is a fact about now, not a mode
// to boot into, and the next shell start rewrites it before anything can read a stale one. A
// bypass with a deadline carries it, so a shell that died mid-bypass leaves a file the apps stop
// trusting at the minute the person was promised. A bypass "until restart" carries no minute,
// so the file also names the shell that wrote it — its pid, the start time the kernel gives
// that pid, and the boot the machine was in — and the apps read a file whose shell is not
// running as `ask` (#154). The boot id is the part a reboot cannot leave standing: the file
// survives one on disk, and in theory a new process could come up under the same pid at the
// same start tick, so without it the old shell's name could still match (#333). The rules ride
// under the same name: they were answers to cards a dead shell will never raise again.

/// This process's pid, the start time the kernel gives it, and the boot the machine is in, read
/// once: all three are facts for the lifetime of the shell, and every publish writes the same
/// identity.
fn shell_identity() -> Option<&'static (u32, u64, String)> {
    static IDENTITY: OnceLock<Option<(u32, u64, String)>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let pid = std::process::id();
            let start = yantrik_app_runtime::control::proc_start_ticks(pid)?;
            let boot = yantrik_app_runtime::control::boot_id()?;
            Some((pid, start, boot))
        })
        .as_ref()
}

/// The file's contents for this state.
///
/// Separate from the write for the reason [`to_store`] is, and free of arguments this process
/// cannot supply itself, so the test can drive what is written through the runtime's own reader
/// without a file. The countdown is not in it — the deadline is — so a running bypass rewrites
/// nothing on any tick.
pub fn policy_json(modes: &Modes, now: Instant, now_unix: u64) -> String {
    let rules: Vec<serde_json::Value> = modes
        .rules()
        .iter()
        .map(|r| serde_json::json!({"app": r.app, "action": r.action}))
        .collect();
    let mut doc = serde_json::json!({
        "mode": modes.mode(now).as_str(),
        "previous": modes.previous(now).as_str(),
        "bypass_expires_unix": modes.bypass_left(now).map(|left| now_unix + left.as_secs()),
        "session_rules": rules,
        "note": "Written by the shell whenever the mind mode or a session rule changes; every \
                 app's dispatch reads it before running an action. Editing it changes nothing \
                 the shell shows, and the next change overwrites it.",
    });
    match shell_identity() {
        Some((pid, start, boot)) => {
            doc["shell_pid"] = serde_json::json!(pid);
            doc["shell_start_ticks"] = serde_json::json!(start);
            doc["boot_id"] = serde_json::json!(boot);
        }
        // No /proc to read the identity from, so the file names no shell and the apps read it
        // the way they read one an older shell wrote. Said out loud: it means nothing on this
        // machine bounds a dead shell's mode to its own lifetime.
        None => tracing::warn!(
            "could not read this shell's start time or the boot id from /proc; the mode file \
             will name no shell, and the apps cannot tell a dead shell's mode from a live one"
        ),
    }
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// Write the file, if what it would say has changed.
///
/// Called from `control_approvals::publish_mode`, which every change of mode or rule already
/// passes through — a person's click, `set_mind_mode` from the socket, a lapsing bypass, the
/// shell starting. Atomic through a rename, so an app reading mid-write sees the old file or
/// the new one and never half of either. A failure is logged and the apps fall back to `ask`,
/// which is the strict side of every mistake this could make.
pub fn publish_policy_file() {
    static LAST: Mutex<String> = Mutex::new(String::new());
    let text = policy_json(&locked(), Instant::now(), unix_now());
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if *last == text {
        return;
    }
    let path = yantrik_app_runtime::control::mode_path();
    let written = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text.as_bytes())?;
        std::fs::rename(&temp, &path)
    })();
    match written {
        Ok(()) => *last = text,
        Err(e) => tracing::warn!(
            path = %path.display(),
            error = %e,
            "could not publish the mind mode; apps will treat this desktop as `ask`"
        ),
    }
}

/// Everything the UI and `describe shell` show about the mode.
pub fn snapshot() -> serde_json::Value {
    let now = Instant::now();
    let guard = locked();
    let mode = guard.mode(now);
    let rules: Vec<serde_json::Value> = guard
        .rules()
        .iter()
        .map(|r| serde_json::json!({"app": r.app, "action": r.action}))
        .collect();
    let left = guard.bypass_left(now);
    let until_restart = guard.bypass_until_restart(now);
    let previous = guard.previous(now);
    // Before `machine_ceiling()`, which reads a file. Holding this lock across a file read is
    // the shape of a stall nobody can reproduce.
    drop(guard);

    serde_json::json!({
        "mode": mode.as_str(),
        "means": mode.meaning(),
        "previous": previous.as_str(),
        "ceiling": crate::control_approvals::machine_ceiling(),
        // A number while a bypass is counting down; null when there is nothing counting. A
        // caller that needs to know "bypass, but with no end" reads `bypass_until_restart`.
        "bypass_expires_in_secs": match left {
            Some(d) => serde_json::json!(d.as_secs()),
            None => serde_json::Value::Null,
        },
        "bypass_until_restart": until_restart,
        "session_rules": serde_json::Value::Array(rules),
    })
}

/// The session rules as one short string, for the UI's "has anything changed" check.
///
/// Cheap on purpose: the screen asks this once a second, and [`snapshot`] reads the machine
/// ceiling off disk, which is not a thing to do sixty times a minute for a menu nobody has open.
pub fn rules_summary() -> String {
    locked()
        .rules()
        .iter()
        .map(|r| format!("{}.{}", r.app, r.action))
        .collect::<Vec<_>>()
        .join(",")
}

/// The chip's text: the mode, plus the countdown while one is running.
///
/// The countdown is always visible during a bypass and never rounded up, because a person
/// glancing at the bar is asking "how long am I exposed for" and 59s must not read as "1m".
pub fn chip_label() -> String {
    let now = Instant::now();
    let guard = locked();
    let mode = guard.mode(now);
    if mode != Mode::Bypass {
        return mode.label().to_string();
    }
    match guard.bypass_left(now) {
        Some(left) => {
            let secs = left.as_secs();
            if secs >= 60 {
                format!("Bypass {}m", secs / 60)
            } else {
                format!("Bypass {secs}s")
            }
        }
        None => "Bypass · no end".to_string(),
    }
}

// ── Everything it did without being asked ───────────────────────────
//
// A mode that stops the asking has to replace it with something, or "auto" is just a quieter way
// of not knowing. The card was the record; without it, the record is this. Two copies for two
// different questions: the in-memory list answers "what has it just done" for the menu, and the
// file answers "what did it do while I was at lunch" after a restart has dropped the memory.

/// One action that ran without anybody being asked about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditEntry {
    /// Local `HH:MM`, which is what a person reading a list wants.
    pub at: String,
    /// Seconds since the epoch, for the file — a list of `HH:MM` with no date is unreadable a
    /// day later.
    pub unix: u64,
    pub mode: String,
    /// What the caller said it was. Self-declared; see `verified` beside it, and issue #43.
    pub requester: String,
    /// What the machine established for itself about that caller — the program the kernel says
    /// opened the socket. Written down beside the claim rather than instead of it, because a
    /// log that kept only one of the two would be the same gap in a different file.
    pub verified: crate::approvals::Verified,
    pub app: String,
    pub action: String,
    /// One `key: value` per entry, bounded exactly the way the card bounds them — same function,
    /// so a person reading the log and a person reading a card see the same arguments.
    pub args: Vec<String>,
    pub grade: String,
    /// What happened when it ran: `ok`, `failed`, or whatever the reporter said.
    pub outcome: String,
}

impl AuditEntry {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "at": self.at,
            "unix": self.unix,
            "mode": self.mode,
            "requester": self.requester,
            "verified": self.verified.to_json(),
            "app": self.app,
            "action": self.action,
            "args": self.args,
            "grade": self.grade,
            "outcome": self.outcome,
        })
    }

    /// The one line the menu shows.
    pub fn line(&self) -> String {
        format!("{} · {}.{} — {}", self.at, self.app, self.action, self.outcome)
    }
}

fn audit() -> &'static Mutex<Vec<AuditEntry>> {
    static AUDIT: OnceLock<Mutex<Vec<AuditEntry>>> = OnceLock::new();
    AUDIT.get_or_init(|| Mutex::new(Vec::new()))
}

/// The audit file. `pub(crate)` for the mind panel, which reads it back after a restart has
/// emptied the in-memory list (see `mind_panel::recent_acts`).
pub(crate) fn audit_path() -> String {
    // Under test, a file of the run's own: a test never writes into the person's record.
    #[cfg(test)]
    return std::env::temp_dir()
        .join(format!("yantrik-mind-audit-under-test-{}.jsonl", std::process::id()))
        .to_string_lossy()
        .into_owned();
    #[cfg(not(test))]
    {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{home}/.local/share/yantrik/mind-audit.jsonl")
    }
}

/// Record one unasked action. Nothing here authorises anything; it only writes down what was.
#[allow(clippy::too_many_arguments)]
pub fn record(
    mode: &str,
    requester: &str,
    verified: &approvals::Verified,
    app: &str,
    action: &str,
    args: &serde_json::Value,
    grade: &str,
    outcome: &str,
) -> AuditEntry {
    let entry = AuditEntry {
        at: crate::app_context::current_time_hhmm(),
        unix: unix_now(),
        mode: mode.to_string(),
        requester: requester.trim().to_string(),
        verified: verified.clone(),
        app: app.to_string(),
        action: action.to_string(),
        args: approvals::args_rows(args),
        grade: grade.to_string(),
        outcome: outcome.trim().to_string(),
    };

    if let Ok(mut list) = audit().lock() {
        list.push(entry.clone());
        let over = list.len().saturating_sub(AUDIT_MEMORY);
        if over > 0 {
            list.drain(..over);
        }
    }
    append_to_file(&entry);
    entry
}

/// Append one line, then trim if the file has grown past its bound.
///
/// Append-and-fsync rather than temp+rename for the normal case: a rename per action would
/// rewrite the whole log on every write, and the thing being protected against is losing the
/// record of what a machine did while nobody was watching — a torn last line is survivable, a
/// missing file is not. The trim is the one place that does use temp+rename, because that one
/// genuinely replaces the file.
fn append_to_file(entry: &AuditEntry) {
    use std::io::Write;

    let path = audit_path();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            tracing::warn!(error = %e, "could not make the audit directory");
            return;
        }
    }
    let line = entry.to_json().to_string();
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| {
            writeln!(file, "{line}")?;
            file.sync_all()
        });
    if let Err(e) = written {
        tracing::warn!(error = %e, path = %path, "could not write the mind audit log");
        return;
    }
    trim_file(&path);
}

fn trim_file(path: &str) {
    let too_big = std::fs::metadata(path).map(|m| m.len() > AUDIT_FILE_MAX).unwrap_or(false);
    if !too_big {
        return;
    }
    let Ok(body) = std::fs::read_to_string(path) else { return };
    let lines: Vec<&str> = body.lines().collect();
    let keep = lines.len().saturating_sub(AUDIT_FILE_KEEP);
    let kept = lines[keep..].join("\n");
    let temp = format!("{path}.new");
    if std::fs::write(&temp, format!("{kept}\n")).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

/// The most recent entries, newest last, for the menu and for `describe shell`.
pub fn recent(count: usize) -> Vec<AuditEntry> {
    let Ok(list) = audit().lock() else { return Vec::new() };
    let skip = list.len().saturating_sub(count);
    list[skip..].to_vec()
}

/// What `describe shell` publishes under `mind_audit_recent`.
pub fn recent_for_describe() -> serde_json::Value {
    serde_json::Value::Array(
        recent(AUDIT_PUBLISHED).into_iter().map(|e| e.to_json()).collect(),
    )
}

#[cfg(test)]
mod mind_mode_tests {
    use super::*;

    fn at(mode: Mode) -> Modes {
        Modes::new(mode)
    }

    fn ran(d: &Decision) -> bool {
        matches!(d, Decision::Run { .. })
    }

    /// The whole table, four modes by four grades, with the ceiling out of the way.
    ///
    /// Written as a table rather than as sixteen assertions because the table IS the feature:
    /// somebody changing one cell should have to change one line here and see the other fifteen
    /// stay put.
    ///
    /// Every case here is an action the app says nothing about recoverability for, which is the
    /// ordinary case; the second table, in
    /// `mind_mode_what_cannot_be_undone_is_asked_about_in_auto`, is the same sixteen cells for
    /// an action whose purpose says it cannot be undone.
    #[test]
    fn mind_mode_the_decision_table_is_what_the_doc_says() {
        let now = Instant::now();
        // (mode, grade, expected)
        let expect: &[(Mode, &str, Decision)] = &[
            (Mode::Plan, "safe", Decision::Run { unasked: false }),
            (Mode::Plan, "standard", Decision::Refuse { why: String::new() }),
            (Mode::Plan, "sensitive", Decision::Refuse { why: String::new() }),
            (Mode::Plan, "dangerous", Decision::Refuse { why: String::new() }),
            (Mode::Ask, "safe", Decision::Run { unasked: false }),
            (Mode::Ask, "standard", Decision::Run { unasked: false }),
            (Mode::Ask, "sensitive", Decision::Ask),
            (Mode::Ask, "dangerous", Decision::Ask),
            (Mode::Auto, "safe", Decision::Run { unasked: false }),
            (Mode::Auto, "standard", Decision::Run { unasked: false }),
            (Mode::Auto, "sensitive", Decision::Run { unasked: true }),
            (Mode::Auto, "dangerous", Decision::Ask),
            (Mode::Bypass, "safe", Decision::Run { unasked: false }),
            (Mode::Bypass, "standard", Decision::Run { unasked: false }),
            (Mode::Bypass, "sensitive", Decision::Run { unasked: true }),
            (Mode::Bypass, "dangerous", Decision::Run { unasked: true }),
        ];

        for (mode, grade, want) in expect {
            let mut modes = at(Mode::Ask);
            if *mode == Mode::Bypass {
                modes.person_set_mode(Mode::Bypass, Bypass::Hour, now, 0);
            } else {
                modes.person_set_mode(*mode, Bypass::Hour, now, 0);
            }
            // `files.move`, not `calendar.delete_event`: this table is about the grade alone, and
            // the action it is driven with must be one the app says nothing about undoing.
            let got = modes.decide(grade, "files", "move", false, "dangerous", now);
            match (want, &got) {
                (Decision::Refuse { .. }, Decision::Refuse { why }) => {
                    assert!(!why.is_empty(), "{mode:?}/{grade}: a refusal has to say why");
                }
                (a, b) => assert_eq!(a, b, "{mode:?} with a {grade} action"),
            }
        }
    }

    /// The defect, as a table: what the app says about undoing is the second input.
    ///
    /// Found live on 21 September 2026 — `calendar.delete_event`, graded `sensitive`, published
    /// as "It is not recoverable", ran in `auto` with nobody asked while the mode menu said
    /// "You are still asked about the destructive ones". The four cells that moved are `ask` and
    /// `auto` at `standard` and `sensitive`; everything else is what it was.
    #[test]
    fn mind_mode_what_cannot_be_undone_is_asked_about_in_auto() {
        let now = Instant::now();
        let expect: &[(Mode, &str, Decision)] = &[
            // A read is a read. Wording that matches cannot turn looking into a question.
            (Mode::Plan, "safe", Decision::Run { unasked: false }),
            (Mode::Ask, "safe", Decision::Run { unasked: false }),
            (Mode::Auto, "safe", Decision::Run { unasked: false }),
            (Mode::Bypass, "safe", Decision::Run { unasked: false }),
            // Plan refuses every write already, for its own reasons.
            (Mode::Plan, "standard", Decision::Refuse { why: String::new() }),
            (Mode::Plan, "sensitive", Decision::Refuse { why: String::new() }),
            (Mode::Plan, "dangerous", Decision::Refuse { why: String::new() }),
            // `ask` asks about it whatever its grade, so that `auto` is never stricter than the
            // mode below it — the one thing that would make this table unreadable.
            (Mode::Ask, "standard", Decision::Ask),
            (Mode::Ask, "sensitive", Decision::Ask),
            (Mode::Ask, "dangerous", Decision::Ask),
            // The change. `auto` asks, exactly as it does for `dangerous`.
            (Mode::Auto, "standard", Decision::Ask),
            (Mode::Auto, "sensitive", Decision::Ask),
            (Mode::Auto, "dangerous", Decision::Ask),
            // Bypass does not ask. It says so on a red confirmation with a countdown, and a
            // card after that would make the confirmation a lie — so it is written down
            // instead, which is what `unasked: true` at `standard` means here.
            (Mode::Bypass, "standard", Decision::Run { unasked: true }),
            (Mode::Bypass, "sensitive", Decision::Run { unasked: true }),
            (Mode::Bypass, "dangerous", Decision::Run { unasked: true }),
        ];

        for (mode, grade, want) in expect {
            let mut modes = at(Mode::Ask);
            modes.person_set_mode(*mode, Bypass::Hour, now, 0);
            let got = modes.decide(grade, "calendar", "delete_event", true, "dangerous", now);
            match (want, &got) {
                (Decision::Refuse { .. }, Decision::Refuse { why }) => {
                    assert!(!why.is_empty(), "{mode:?}/{grade}: a refusal has to say why");
                }
                (a, b) => assert_eq!(a, b, "{mode:?} with an unrecoverable {grade} action"),
            }
        }

        // And the cell the defect was actually reported from, said twice so the diff reads: the
        // same action, the same mode, the same grade, and the only difference is the sentence
        // the app publishes about it.
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Auto, Bypass::Hour, now, 0);
        assert_eq!(
            modes.decide("sensitive", "calendar", "delete_event", false, "dangerous", now),
            Decision::Run { unasked: true },
            "a recoverable sensitive action is what `auto` is for"
        );
        assert_eq!(
            modes.decide("sensitive", "calendar", "delete_event", true, "dangerous", now),
            Decision::Ask,
            "and one the app says cannot be undone is what the menu already promised"
        );
    }

    /// Nothing above the machine ceiling is put in front of a person, this rule included.
    ///
    /// The ordering is the security argument and it must not be softened by a second reason to
    /// ask: a card no answer of theirs could satisfy teaches them the card is noise, whether the
    /// card is there for the grade or for the app's sentence.
    #[test]
    fn mind_mode_the_ceiling_still_outranks_what_cannot_be_undone() {
        let now = Instant::now();
        for mode in [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass] {
            let mut modes = at(Mode::Ask);
            modes.person_set_mode(mode, Bypass::Hour, now, 0);
            let got = modes.decide("sensitive", "calendar", "delete_event", true, "standard", now);
            let Decision::Refuse { why } = got else {
                panic!("{mode:?} asked about something above the machine ceiling");
            };
            assert!(why.contains("tool_permission"), "{mode:?}: {why}");
            assert!(why.contains("NOT asked"), "{mode:?}: {why}");
        }
    }

    /// Plan mode says what it is, so the mind can relay it rather than reporting a fault.
    #[test]
    fn mind_mode_plan_refuses_in_words_a_person_can_read() {
        let now = Instant::now();
        let modes = at(Mode::Plan);
        let Decision::Refuse { why } = modes.decide("standard", "notes", "write", false, "dangerous", now)
        else {
            panic!("plan mode must refuse a write");
        };
        assert!(why.contains("plan mode"), "{why}");
        assert!(why.contains("nothing on this machine was changed"), "{why}");
        assert!(why.to_lowercase().contains("would do"), "it has to ask for the plan: {why}");
    }

    /// No mode reaches past the owner's standing policy, and nothing above it is ever asked about.
    #[test]
    fn mind_mode_the_machine_ceiling_is_above_every_mode() {
        let now = Instant::now();
        for mode in [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass] {
            let mut modes = at(Mode::Ask);
            modes.person_set_mode(mode, Bypass::UntilRestart, now, 0);
            let got = modes.decide("dangerous", "system", "kill", false, "standard", now);
            let Decision::Refuse { why } = got else {
                panic!("{mode:?} let a dangerous action past a `standard` machine ceiling");
            };
            assert!(why.contains("tool_permission"), "{mode:?}: {why}");
            assert!(why.contains("NOT asked"), "{mode:?}: {why}");
        }

        // And at the ceiling, the mode decides again as normal.
        let modes = at(Mode::Ask);
        assert_eq!(
            modes.decide("standard", "notes", "write", false, "standard", now),
            Decision::Run { unasked: false }
        );
    }

    #[test]
    fn mind_mode_an_undefined_grade_is_refused_in_every_mode() {
        let now = Instant::now();
        for mode in [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass] {
            let mut modes = at(Mode::Ask);
            modes.person_set_mode(mode, Bypass::Hour, now, 0);
            let got = modes.decide("spicy", "notes", "write", false, "dangerous", now);
            assert!(
                matches!(got, Decision::Refuse { .. }),
                "{mode:?} ran an action whose grade this OS does not define"
            );
        }
    }

    #[test]
    fn mind_mode_bypass_expires_back_to_what_it_was() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Auto, Bypass::Hour, now, 0);
        modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 0);

        assert_eq!(modes.mode(now), Mode::Bypass);
        assert_eq!(modes.bypass_left(now).map(|d| d.as_secs()), Some(15 * 60));
        assert_eq!(modes.previous(now), Mode::Auto, "it has to come back to where it was");

        let later = now + Duration::from_secs(15 * 60 + 1);
        assert_eq!(modes.mode(later), Mode::Auto, "a lapsed bypass is not still in force");
        assert_eq!(
            modes.decide("dangerous", "system", "kill", false, "dangerous", later),
            Decision::Ask,
            "and the mode it came back to is the one deciding"
        );
        assert!(modes.lapse(later), "the screen has something new to show");
        assert!(!modes.lapse(later), "and only once");
        assert_eq!(modes.mode(later), Mode::Auto);
    }

    /// What this shell publishes is what every app enforces (issue #116). The runtime reads
    /// the mode from the file this writes, so the two sides are driven through each other: a
    /// `Modes` in each state, the text this would write, and the runtime's own reader of it.
    /// A reader and a writer tested apart could each pass while the apps enforced nothing.
    #[test]
    fn mind_mode_what_the_shell_publishes_is_what_the_apps_enforce() {
        use yantrik_app_runtime::control::mode_from;
        let now = Instant::now();
        let unix = 1_800_000_000;
        let mut modes = at(Mode::Ask);
        assert_eq!(mode_from(&policy_json(&modes, now, unix), unix).name, "ask");

        modes.person_set_mode(Mode::Auto, Bypass::Hour, now, 0);
        modes.person_add_rule("calendar", "move_event", "sensitive", "Move an event").unwrap();
        let read = mode_from(&policy_json(&modes, now, unix), unix);
        assert_eq!(read.name, "auto");
        assert_eq!(read.session_rules, vec![("calendar".to_string(), "move_event".to_string())]);

        // A bypass carries its deadline: an app trusts it until then and not a second longer,
        // even if this shell is no longer there to fold it back.
        modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, unix);
        let text = policy_json(&modes, now, unix);
        assert!(text.contains(&format!("\"bypass_expires_unix\": {}", unix + 15 * 60)), "{text}");
        assert_eq!(mode_from(&text, unix).name, "bypass");
        assert_eq!(mode_from(&text, unix + 15 * 60).name, "auto", "back to what it was");
        assert_eq!(mode_from(&text, unix).session_rules.len(), 1, "the rule rides along");

        // Until restart carries no deadline, and the runtime trusts it until the next write.
        modes.person_set_mode(Mode::Bypass, Bypass::UntilRestart, now, unix);
        let text = policy_json(&modes, now, unix);
        assert!(text.contains("\"bypass_expires_unix\": null"), "{text}");
        assert_eq!(mode_from(&text, unix + 86_400).name, "bypass");

        // It is trusted because the file names the shell that wrote it and that shell — this
        // test's own process — is running. The same file left behind by a shell that DIED
        // reads as `ask`, which is the whole of #154's first item: a pid under a start time
        // the kernel does not give it names nothing alive. And the name carries the boot it
        // was written in (#333), which the file cannot keep across a reboot: the same pid and
        // start time under a boot that has ended also reads as `ask`.
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["shell_pid"].as_u64(), Some(std::process::id() as u64));
        let start = doc["shell_start_ticks"].as_u64().expect("the file carries a start time");
        assert_eq!(
            doc["boot_id"].as_str().map(str::to_string),
            yantrik_app_runtime::control::boot_id(),
            "the file carries this boot's id"
        );
        let mut dead = doc.clone();
        dead["shell_start_ticks"] = serde_json::json!(start + 1);
        assert_eq!(mode_from(&dead.to_string(), unix).name, "ask");
        let mut rebooted = doc.clone();
        rebooted["boot_id"] = serde_json::json!("00000000-0000-0000-0000-000000000000");
        assert_eq!(mode_from(&rebooted.to_string(), unix).name, "ask");
    }

    /// Choosing bypass twice must not strand the machine there.
    #[test]
    fn mind_mode_bypass_twice_still_comes_back_to_the_real_mode() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 0);
        modes.person_set_mode(Mode::Bypass, Bypass::Hour, now + Duration::from_secs(60), 0);
        assert_eq!(modes.previous(now), Mode::Ask, "not `bypass`");
        let later = now + Duration::from_secs(60 * 60 + 61);
        assert_eq!(modes.mode(later), Mode::Ask);
    }

    #[test]
    fn mind_mode_bypass_until_restart_never_lapses_on_its_own() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::UntilRestart, now, 0);
        let much_later = now + Duration::from_secs(48 * 60 * 60);
        assert_eq!(modes.mode(much_later), Mode::Bypass);
        assert!(modes.bypass_left(much_later).is_none());
        assert!(modes.bypass_until_restart(much_later));
    }

    /// A machine must not boot into bypass, whatever the settings file says.
    ///
    /// Two halves: nothing writes it, and nothing reads it even if something did. The second
    /// half is what makes a hand-edited `settings.yaml` harmless.
    #[test]
    fn mind_mode_bypass_is_never_persisted_and_never_booted_into() {
        assert_eq!(to_store(Mode::Bypass, Mode::Auto).as_str(), "auto");
        assert_eq!(to_store(Mode::Bypass, Mode::Plan).as_str(), "plan");
        assert_eq!(to_store(Mode::Bypass, Mode::Bypass).as_str(), "ask");
        assert_eq!(to_store(Mode::Auto, Mode::Ask).as_str(), "auto");
        assert_ne!(
            to_store(Mode::Bypass, Mode::Ask),
            Mode::Bypass,
            "there is no path that writes `bypass` to the settings file"
        );

        assert_eq!(Modes::new(Mode::Bypass).mode(Instant::now()), Mode::Ask);
        assert_eq!(Modes::new(Mode::Auto).mode(Instant::now()), Mode::Auto);
        assert_eq!(
            crate::wire::settings::UserSettings::default().mind_mode,
            "ask",
            "a machine that has never been told comes up asking, which is what shipped before"
        );
    }

    /// A rule covers any arguments for its own action, and nothing else at all.
    #[test]
    fn mind_mode_a_session_rule_covers_any_args_but_only_its_own_action() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes
            .person_add_rule("files", "move", "sensitive", "Move a file to another folder.")
            .expect("a recoverable sensitive action may have a rule");

        assert_eq!(
            modes.decide("sensitive", "files", "move", false, "dangerous", now),
            Decision::Run { unasked: true },
            "the rule is what stops the asking, and it is recorded as unasked"
        );
        // Same action, different arguments — a rule is deliberately not argument-bound.
        assert_eq!(
            modes.decide("sensitive", "files", "move", false, "dangerous", now),
            Decision::Run { unasked: true }
        );
        assert_eq!(
            modes.decide("sensitive", "files", "delete", false, "dangerous", now),
            Decision::Ask,
            "a rule for one action is not a rule for its neighbour"
        );
        assert_eq!(
            modes.decide("sensitive", "calendar", "move", false, "dangerous", now),
            Decision::Ask,
            "nor for the same word in another app"
        );

        modes.person_revoke_rule("files", "move");
        assert_eq!(
            modes.decide("sensitive", "files", "move", false, "dangerous", now),
            Decision::Ask
        );
    }

    /// A session rule never covers an action the app says cannot be undone — in any mode.
    ///
    /// Two layers, and this is the second. The card refuses to OFFER one
    /// (`approvals::may_offer_session_rule`), which is checked once, at the press; this is the
    /// table refusing to honour one, which is checked on every call. They exist separately
    /// because an app can reword its own purpose after a rule was made, and because the `auto`
    /// rule above would be worthless otherwise: a card raised because the app says the action
    /// cannot be undone must not be answerable by a rule the card would never have offered.
    #[test]
    fn mind_mode_a_session_rule_never_covers_what_cannot_be_undone() {
        let now = Instant::now();
        for mode in [Mode::Ask, Mode::Auto] {
            let mut modes = at(Mode::Ask);
            // Straight into the field: `person_add_rule` refuses this pair, which is the first
            // layer. What is being tested here is what happens if one exists anyway.
            modes.rules =
                vec![Rule { app: "calendar".into(), action: "delete_event".into() }];
            modes.person_set_mode(mode, Bypass::Hour, now, 0);

            assert_eq!(
                modes.decide("sensitive", "calendar", "delete_event", true, "dangerous", now),
                Decision::Ask,
                "{mode:?}: a rule must not answer a card raised because it cannot be undone"
            );
            // And the rule is a real rule — it is the sentence that disarms it, not the rule
            // being missing.
            assert_eq!(
                modes.decide("sensitive", "calendar", "delete_event", false, "dangerous", now),
                Decision::Run { unasked: true },
                "{mode:?}: the same rule covers the same action when it can be undone"
            );
        }
    }

    /// The two kinds of action a session rule is never offered for.
    #[test]
    fn mind_mode_no_session_rule_for_dangerous_or_unrecoverable() {
        let mut modes = at(Mode::Ask);
        let err = modes
            .person_add_rule("system", "kill", "dangerous", "End a process.")
            .expect_err("dangerous is always a card");
        assert!(err.contains("Allow it once instead"), "{err}");

        let err = modes
            .person_add_rule(
                "calendar",
                "delete_event",
                "sensitive",
                "Delete an event from the calendar. It is not recoverable.",
            )
            .expect_err("the app's own sentence about recoverability decides too");
        assert!(err.contains("cannot be undone"), "{err}");

        assert!(modes.rules().is_empty(), "a refused rule must not be stored anyway");
    }

    /// A rule cannot reach past the ceiling or out of plan mode either.
    #[test]
    fn mind_mode_a_session_rule_is_not_a_way_around_anything() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_add_rule("files", "move", "sensitive", "Move a file.").unwrap();

        let above = modes.decide("sensitive", "files", "move", false, "standard", now);
        assert!(
            matches!(above, Decision::Refuse { .. }),
            "a rule must not carry anything past the machine ceiling"
        );

        modes.person_set_mode(Mode::Plan, Bypass::Hour, now, 0);
        let planned = modes.decide("sensitive", "files", "move", false, "dangerous", now);
        assert!(
            matches!(planned, Decision::Refuse { .. }),
            "and plan mode outranks a rule made before it"
        );
    }

    #[test]
    fn mind_mode_the_socket_can_lower_and_cannot_raise() {
        let now = Instant::now();
        let mut modes = at(Mode::Auto);

        // Down is fine, and is a real change.
        assert_eq!(modes.lower_to(Mode::Plan, now).unwrap(), Mode::Plan);
        assert_eq!(modes.mode(now), Mode::Plan);

        // Up is not, in any of its shapes.
        for wanted in [Mode::Ask, Mode::Auto, Mode::Bypass] {
            let err = modes
                .lower_to(wanted, now)
                .expect_err("only a person raises the mode");
            assert!(err.contains("only the person at this machine"), "{err}");
            assert!(err.contains("status bar"), "the refusal says where: {err}");
            assert_eq!(modes.mode(now), Mode::Plan, "and nothing moved");
        }

        // Asking for the mode it is already in is not a raise.
        assert_eq!(modes.lower_to(Mode::Plan, now).unwrap(), Mode::Plan);
    }

    /// Lowering out of a bypass ends it, rather than leaving a deadline that would later
    /// "lapse" the machine back into something looser than what was just chosen.
    #[test]
    fn mind_mode_lowering_out_of_bypass_does_not_leave_it_armed() {
        let now = Instant::now();
        let mut modes = at(Mode::Auto);
        modes.person_set_mode(Mode::Bypass, Bypass::Hour, now, 0);
        modes.lower_to(Mode::Ask, now).expect("bypass → ask is a lowering");
        let later = now + Duration::from_secs(60 * 60 + 1);
        assert_eq!(modes.mode(later), Mode::Ask, "not back to auto an hour later");
        assert!(modes.bypass_left(later).is_none());
    }

    #[test]
    fn mind_mode_the_ladder_is_the_one_the_os_publishes() {
        assert_eq!(grade_rank("safe"), Some(0));
        assert_eq!(grade_rank("dangerous"), Some(3));
        assert_eq!(grade_rank("Dangerous"), None, "grades arrive lowercase or not at all");
        assert!(Mode::Plan.permissiveness() < Mode::Ask.permissiveness());
        assert!(Mode::Ask.permissiveness() < Mode::Auto.permissiveness());
        assert!(Mode::Auto.permissiveness() < Mode::Bypass.permissiveness());
    }

    #[test]
    fn mind_mode_the_chip_counts_down_and_never_rounds_up() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 0);
        // 59 seconds must not read as "1m": the chip answers "how long am I exposed for".
        let nearly = now + Duration::from_secs(15 * 60 - 59);
        let left = modes.bypass_left(nearly).unwrap().as_secs();
        assert_eq!(left, 59);
        assert_eq!(format!("Bypass {left}s"), "Bypass 59s");
    }

    #[test]
    fn mind_mode_an_audit_entry_reads_as_a_sentence() {
        let entry = AuditEntry {
            at: "12:03".into(),
            unix: 1_790_000_000,
            mode: "auto".into(),
            requester: "Hermes Agent 0.9.2".into(),
            // The claim above and the fact beside it: the log keeps both, or it is the same
            // unverifiable string in a different file (issue #43).
            verified: crate::approvals::Verified {
                line: "python -m hermes_cli.main gateway (pid 696) \u{b7} the attached mind".into(),
                exe: "/home/pranab/hermes-agent/venv/bin/python".into(),
                pid: 696,
                attached_mind: "Hermes Agent".into(),
                discrepancies: Vec::new(),
                agent: String::new(),
            },
            app: "files".into(),
            action: "move".into(),
            args: vec!["from: /a".into(), "to: /b".into()],
            grade: "sensitive".into(),
            outcome: "ok".into(),
        };
        assert_eq!(entry.line(), "12:03 · files.move — ok");
        let json = entry.to_json();
        for key in
            ["at", "unix", "mode", "requester", "verified", "app", "action", "args", "grade", "outcome"]
        {
            assert!(json.get(key).is_some(), "the log is missing `{key}`");
        }
        // Not folded into `requester`. A reader of the log has to be able to tell the name the
        // caller chose from the program the kernel named, and one field cannot carry both.
        assert_eq!(json["requester"], "Hermes Agent 0.9.2");
        assert_eq!(json["verified"]["pid"], 696);
        assert_eq!(json["verified"]["attached_mind"], "Hermes Agent");
    }

    // ── A bypass that ran out on its own ────────────────────────────

    fn audit_entry(mode: &str, unix: u64) -> AuditEntry {
        AuditEntry {
            at: "12:03".into(),
            unix,
            mode: mode.into(),
            requester: "Hermes Agent".into(),
            // Nothing established: the count is about which window an entry fell in and what it
            // ran under, and it must not start depending on who the machine thinks was calling.
            verified: crate::approvals::Verified::default(),
            app: "files".into(),
            action: "move".into(),
            args: vec![],
            grade: "sensitive".into(),
            outcome: "ok".into(),
        }
    }

    /// The lapse is noticed once, by the clock, and by nothing else.
    ///
    /// The one-second tick in `control_approvals::wire` already folded an expired bypass back so
    /// the chip would stop saying "Bypass"; it threw the `bool` away. This is that same tick
    /// learning to say something — and the property that matters is that the second tick after
    /// the deadline says nothing, or a person who walked away comes back to nine hundred
    /// notifications.
    #[test]
    fn mind_mode_a_lapse_is_reported_once_and_only_once() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Auto, Bypass::Hour, now, 0);
        modes.person_set_mode(Mode::Bypass, Bypass::Minutes15, now, 1_790_000_000);

        // Still running: nothing to say.
        let halfway = now + Duration::from_secs(7 * 60);
        assert!(!modes.lapse(halfway));
        assert_eq!(modes.take_lapse(), None, "a live bypass is not news");

        let after = now + Duration::from_secs(15 * 60 + 1);
        assert!(modes.lapse(after), "the clock ran out");
        let lapse = modes.take_lapse().expect("that is the notification");
        assert_eq!(lapse.back_to, Mode::Auto, "it says where the machine actually is now");
        assert_eq!(lapse.started_unix, 1_790_000_000, "and the window the count covers");

        // Two more ticks, a second apart, exactly as the refresh timer produces them.
        let later = after + Duration::from_secs(1);
        assert!(!modes.lapse(later));
        assert_eq!(modes.take_lapse(), None, "one lapse is one notification");
        assert!(!modes.lapse(later + Duration::from_secs(1)));
        assert_eq!(modes.take_lapse(), None);
    }

    /// A bypass a PERSON ended announces nothing. They just did it, on a menu that says so.
    #[test]
    fn mind_mode_a_person_ending_bypass_announces_nothing() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::Hour, now, 0);
        modes.person_set_mode(Mode::Ask, Bypass::Hour, now + Duration::from_secs(60), 0);
        assert_eq!(modes.take_lapse(), None, "they pressed Ask; they know");

        // Nor does the clock later announce a bypass that was already over.
        let much_later = now + Duration::from_secs(60 * 60 + 1);
        assert!(!modes.lapse(much_later));
        assert_eq!(modes.take_lapse(), None);

        // And the socket lowering out of one is not a lapse either.
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::Hour, now, 0);
        modes.lower_to(Mode::Plan, now).expect("bypass → plan is a lowering");
        assert_eq!(modes.take_lapse(), None);
    }

    /// "Until the shell restarts" never lapses, so it never announces — including on the next
    /// boot, where the machine has no memory that one was ever running.
    #[test]
    fn mind_mode_until_restart_announces_nothing_ever() {
        let now = Instant::now();
        let mut modes = at(Mode::Ask);
        modes.person_set_mode(Mode::Bypass, Bypass::UntilRestart, now, 0);
        let days_later = now + Duration::from_secs(48 * 60 * 60);
        assert!(!modes.lapse(days_later), "there was never a deadline to cross");
        assert_eq!(modes.take_lapse(), None);

        // The next boot. `bypass` is never written to the settings file (`to_store`) and is
        // clamped on the way back in anyway, so the shell comes up in `ask` owing nobody an
        // explanation — a notification about a bypass nobody is in would be a lie.
        assert_ne!(to_store(Mode::Bypass, Mode::Ask), Mode::Bypass);
        let mut booted = Modes::new(Mode::Bypass);
        assert_eq!(booted.mode(now), Mode::Ask);
        assert_eq!(booted.take_lapse(), None, "a fresh shell announces nothing");
        assert!(!booted.lapse(now));
    }

    /// What the bypass bought, counted the way the notification counts it.
    #[test]
    fn mind_mode_the_count_is_what_the_bypass_itself_bought() {
        let start = 1_790_000_000;
        let entries = vec![
            // Before the window opened. Somebody else's auto-mode afternoon.
            audit_entry("auto", start - 600),
            audit_entry("bypass", start - 1),
            // Inside it.
            audit_entry("bypass", start),
            audit_entry("bypass", start + 30),
            // Inside it, but covered by a session rule — it would have run in `ask` mode too,
            // so it is not something the bypass bought.
            audit_entry("rule", start + 40),
            // Inside it, and asked about: `auto` never appears while a bypass is in force, but
            // the filter is on what the action RAN under, not on when it happened.
            audit_entry("auto", start + 50),
        ];
        assert_eq!(unasked_during(&entries, start), 2);
        assert_eq!(unasked_during(&entries, start + 31), 0, "a window with nothing in it");
        assert_eq!(unasked_during(&[], start), 0, "and no audit at all");
    }

    /// The sentence a person reads. Zero, one and several are three different sentences.
    #[test]
    fn mind_mode_the_lapse_notice_reads_like_a_person_wrote_it() {
        let none = bypass_ended_body(&BypassEnded { back_to: Mode::Ask, unasked: 0 });
        assert_eq!(
            none,
            "The mind is back in Ask mode. It asks you before anything that could matter. \
             Nothing ran without asking while bypass was on."
        );

        let one = bypass_ended_body(&BypassEnded { back_to: Mode::Ask, unasked: 1 });
        assert!(one.contains("It did one thing without asking"), "{one}");
        assert!(!one.contains(" 1 "), "one is a word here, not a digit: {one}");
        assert!(!one.contains("1 things"), "{one}");

        let many = bypass_ended_body(&BypassEnded { back_to: Mode::Auto, unasked: 7 });
        assert!(many.contains("It did 7 things without asking"), "{many}");
        // The mode's own published sentence, so the notification and the menu can never
        // describe `auto` differently.
        assert!(many.contains(Mode::Auto.meaning()), "{many}");
        assert!(many.starts_with("The mind is back in Auto mode."), "{many}");

        for count in [0usize, 1, 2, 50] {
            let body = bypass_ended_body(&BypassEnded { back_to: Mode::Plan, unasked: count });
            // "It did 0 things" is the shape of a machine reading a counter out loud. (Not
            // `!contains("0 things")`: fifty of them contains it, which is how this assertion
            // was wrong the first time.)
            assert!(!body.contains("did 0 things"), "{body}");
            assert!(body.ends_with("while bypass was on."), "{body}");
        }
    }

    /// The duration hook is a test hook and can only ever tighten.
    #[test]
    fn mind_mode_the_duration_hook_can_only_shorten() {
        assert_eq!(shortened_by(None, 900), 900, "unset changes nothing");
        assert_eq!(shortened_by(Some(20), 900), 20, "it can cut fifteen minutes to twenty seconds");
        assert_eq!(
            shortened_by(Some(9_000), 900),
            900,
            "and it can never extend one: a hook that could hold a machine in bypass for longer \
             than the person agreed to would be the backdoor this whole feature exists to avoid"
        );
        assert_eq!(shortened_by(Some(9_000), 60 * 60), 60 * 60);
        // Nothing reaches "until the shell restarts": it has no deadline to shorten.
        assert_eq!(Bypass::UntilRestart.deadline(Instant::now()), None);
    }

    // ── The table, written out so the bridge's copy can be checked against it ──
    //
    // `Modes::decide` above and `decide` in `deploy/yantrik-os/yos-mcp` are the same table
    // written twice. That is deliberate — the alternative is a second round trip per `os_act`,
    // and the mode already arrives on a read the bridge was making anyway — and the design note
    // lists it as the top open item, because two copies drift silently in the direction nobody
    // tests. So this writes the table out as data and the bridge's selftest reads it back.
    //
    // The core of the file (mode × grade × unrecoverable × ceiling × rule) comes straight out of
    // production `Modes::decide`. Two thin layers are modelled here rather than there, and the
    // file marks which so nobody reads a capped vector as something the shell decided:
    //
    //   * `env_cap` (`YOS_MCP_MAX_PERMISSION`) is a cap a harness puts on ITSELF. The shell has
    //     no business enforcing it and does not, so there is no production Rust to generate it
    //     from — see `capped`.
    //   * the browser tools are not on the shell's surface at all — see `web_outcome`.

    const OUT_RUN: &str = "run";
    const OUT_RUN_LOGGED: &str = "run_logged";
    const OUT_ASK: &str = "ask";
    const OUT_REFUSE_GRADE: &str = "refuse_grade";
    const OUT_REFUSE_CEILING: &str = "refuse_ceiling";
    const OUT_REFUSE_MODE: &str = "refuse_mode";

    /// Which outcome a `Decision` is, as one word both implementations can name.
    ///
    /// The three refusals are not distinguished by the type — a refusal is a sentence for a
    /// person — so this reads the sentence, by the one marker each of them carries. It panics
    /// when a refusal matches none or more than one, so rewording a refusal fails loudly here
    /// instead of quietly relabelling a vector.
    fn outcome_of(decision: &Decision) -> &'static str {
        let why = match decision {
            Decision::Run { unasked: false } => return OUT_RUN,
            Decision::Run { unasked: true } => return OUT_RUN_LOGGED,
            Decision::Ask => return OUT_ASK,
            Decision::Refuse { why } => why,
        };
        let markers = [
            ("not a level this OS defines", OUT_REFUSE_GRADE),
            ("tool_permission", OUT_REFUSE_CEILING),
            ("plan mode", OUT_REFUSE_MODE),
        ];
        let hit: Vec<&'static str> =
            markers.iter().filter(|(m, _)| why.contains(m)).map(|(_, o)| *o).collect();
        assert_eq!(
            hit.len(),
            1,
            "a refusal has to be one of the three this table makes, and this one matched {}: \
             {why}\n\nIf you reworded a refusal, reword the marker here with it — a vector that \
             cannot be classified is worse than no vector.",
            hit.len()
        );
        hit[0]
    }

    /// `YOS_MCP_MAX_PERMISSION`, as the design note defines it.
    ///
    /// A cap a harness puts on ITSELF, and it can only ever be stricter: it bounds what may run
    /// WITHOUT a person being asked, so anything above it that the mode would have run quietly
    /// becomes a question instead. It cannot loosen anything — a desktop in `ask` mode asks
    /// whatever this is set to, and a desktop in `plan` mode has already refused — and a value
    /// that is not on the ladder is not a permission to do anything, so it is treated as unset.
    fn capped(decision: Decision, grade: &str, cap: Option<&str>) -> Decision {
        let (Some(cap_rank), Some(rank)) =
            (cap.and_then(grade_rank), grade_rank(grade))
        else {
            return decision;
        };
        match decision {
            Decision::Run { .. } if rank > cap_rank => Decision::Ask,
            other => other,
        }
    }

    /// The plan row of the doc's table for the browser tools.
    ///
    /// Nothing on the shell's surface decides this — there is no browser there — so it is the
    /// doc's own sentence written once: reading a page is looking, and putting something into
    /// one is not. The machine ceiling and the grade ladder never enter it, because a page
    /// element carries no grade.
    fn web_outcome(tool: &str, mode: Mode) -> &'static str {
        const WRITES: [&str; 3] = ["web_go", "web_click", "web_type"];
        if WRITES.contains(&tool) && mode == Mode::Plan {
            OUT_REFUSE_MODE
        } else {
            OUT_RUN
        }
    }

    fn modes_for(mode: Mode, rules: &[(&str, &str)], now: Instant) -> Modes {
        let mut modes = Modes::new(Mode::Ask);
        modes.person_set_mode(mode, Bypass::Hour, now, 0);
        // Straight into the field rather than through `person_add_rule`, because the generator
        // has to be able to put a rule on an action the card would never offer one for — one
        // graded `dangerous`, or one the app says cannot be undone. Those combinations are
        // unreachable from a click and are generated anyway, because `decide` has to refuse to
        // honour such a rule on its own rather than trusting the card to have never made one.
        modes.rules =
            rules.iter().map(|(a, b)| Rule { app: a.to_string(), action: b.to_string() }).collect();
        modes
    }

    /// The pair of names a vector is generated under, so an id reads.
    ///
    /// The app and action are cosmetic to `decide` — it matches a rule on the pair and never
    /// looks at what they mean — but a vector reading `calendar.delete_event` beside
    /// `"unrecoverable": true` is one a person can check against a real machine, and one
    /// reading `files.move` beside it is not.
    fn subject(unrecoverable: bool) -> (&'static str, &'static str) {
        if unrecoverable {
            ("calendar", "delete_event")
        } else {
            ("files", "move")
        }
    }

    /// Every case both implementations have to agree about.
    fn all_vectors() -> Vec<serde_json::Value> {
        let now = Instant::now();
        let mut out: Vec<serde_json::Value> = Vec::new();

        let modes_all = [Mode::Plan, Mode::Ask, Mode::Auto, Mode::Bypass];
        // The fifth is not a grade. `None` is not `safe`, and an action whose cost was never
        // read is refused in every mode — the case most likely to be got wrong twice.
        let grades = ["safe", "standard", "sensitive", "dangerous", "spicy"];
        // `safe` as a machine ceiling is not a configuration anybody has; the three the AI page
        // offers are these.
        let ceilings = ["standard", "sensitive", "dangerous"];
        // Whether the action's own published purpose says it cannot be undone. A full axis
        // rather than a property of one named case, because it decides the table now (`auto`
        // asks about such an action exactly as it asks about a `dangerous` one) and a dimension
        // that only varies along one other dimension proves nothing about the corners.
        let undoable = [false, true];

        // (name, the rules the shell would be publishing for THIS subject)
        let rule_cases: &[(&str, fn(&str, &str) -> Vec<(String, String)>)] = &[
            ("none", |_, _| vec![]),
            ("same", |app, action| vec![(app.to_string(), action.to_string())]),
            // A rule for the app's own neighbour. It must never cover the action beside it.
            ("other", |app, _| vec![(app.to_string(), "rename".to_string())]),
        ];

        for mode in modes_all {
            for grade in grades {
                for cannot_undo in undoable {
                    let (app, action) = subject(cannot_undo);
                    for ceiling in ceilings {
                        for (rule_name, make_rules) in rule_cases {
                            let rules = make_rules(app, action);
                            let borrowed: Vec<(&str, &str)> =
                                rules.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                            let modes = modes_for(mode, &borrowed, now);
                            let decision =
                                modes.decide(grade, app, action, cannot_undo, ceiling, now);
                            // The id names every input, in the order the file's `_` note
                            // explains them, so a failing vector can be read without looking it
                            // up. `unrecoverable=` spells the field it comes from rather than
                            // an inverted word, because a file carrying both `undoable` and
                            // `unrecoverable` is one somebody reads the wrong way round once.
                            let id = format!(
                                "act/{}/{grade}/unrecoverable={cannot_undo}/ceiling={ceiling}/rule={rule_name}",
                                mode.as_str(),
                            );
                            out.push(serde_json::json!({
                                "id": id,
                                "layer": "shell",
                                "tool": "os_act",
                                "app": app,
                                "action": action,
                                "grade": grade,
                                "mode": mode.as_str(),
                                "ceiling": ceiling,
                                "rules": rules.iter()
                                    .map(|(a, b)| serde_json::json!([a, b]))
                                    .collect::<Vec<_>>(),
                                "env_cap": serde_json::Value::Null,
                                "unrecoverable": cannot_undo,
                                "expect": outcome_of(&decision),
                            }));
                        }
                    }
                }
            }
        }

        // The harness's own cap. Ceiling held at `dangerous` so nothing is refused above it and
        // the cap is the only thing moving. The `unrecoverable` axis is here too, because both
        // it and the cap turn a quiet run into a question and the interesting question is what
        // happens when both do at once.
        for mode in modes_all {
            for grade in ["safe", "standard", "sensitive", "dangerous"] {
                for cannot_undo in undoable {
                    let (app, action) = subject(cannot_undo);
                    for cap in ["standard", "sensitive", "dangerous"] {
                        for (rule_name, make_rules) in &rule_cases[..2] {
                            let rules = make_rules(app, action);
                            let borrowed: Vec<(&str, &str)> =
                                rules.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
                            let modes = modes_for(mode, &borrowed, now);
                            let decision =
                                modes.decide(grade, app, action, cannot_undo, "dangerous", now);
                            let decision = capped(decision, grade, Some(cap));
                            let id = format!(
                                "cap/{}/{grade}/unrecoverable={cannot_undo}/cap={cap}/rule={rule_name}",
                                mode.as_str(),
                            );
                            out.push(serde_json::json!({
                                "id": id,
                                "layer": "harness_cap",
                                "tool": "os_act",
                                "app": app,
                                "action": action,
                                "grade": grade,
                                "mode": mode.as_str(),
                                "ceiling": "dangerous",
                                "rules": rules.iter()
                                    .map(|(a, b)| serde_json::json!([a, b]))
                                    .collect::<Vec<_>>(),
                                "env_cap": cap,
                                "unrecoverable": cannot_undo,
                                "expect": outcome_of(&decision),
                            }));
                        }
                    }
                }
            }
        }

        // A cap that is not on the ladder is not a permission to do anything, so it is treated
        // as unset rather than as whatever an index lookup would have done with it.
        for mode in modes_all {
            for cannot_undo in undoable {
                let (app, action) = subject(cannot_undo);
                let modes = modes_for(mode, &[], now);
                let decision =
                    modes.decide("sensitive", app, action, cannot_undo, "dangerous", now);
                let decision = capped(decision, "sensitive", Some("nonsense"));
                out.push(serde_json::json!({
                    "id": format!(
                        "cap/{}/sensitive/unrecoverable={cannot_undo}/cap=nonsense",
                        mode.as_str(),
                    ),
                    "layer": "harness_cap",
                    "tool": "os_act",
                    "app": app,
                    "action": action,
                    "grade": "sensitive",
                    "mode": mode.as_str(),
                    "ceiling": "dangerous",
                    "rules": Vec::<serde_json::Value>::new(),
                    "env_cap": "nonsense",
                    "unrecoverable": cannot_undo,
                    "expect": outcome_of(&decision),
                }));
            }
        }

        // The browser. No grade, no rule, and no published purpose to read — a page element is
        // not an action an app describes — so `unrecoverable` is false and is not an axis here.
        for mode in modes_all {
            for tool in ["web_read", "web_text", "web_find", "web_go", "web_click", "web_type"] {
                out.push(serde_json::json!({
                    "id": format!("web/{}/{tool}", mode.as_str()),
                    "layer": "browser",
                    "tool": tool,
                    "app": "the browser",
                    "action": tool,
                    "grade": serde_json::Value::Null,
                    "mode": mode.as_str(),
                    "ceiling": "dangerous",
                    "rules": Vec::<serde_json::Value>::new(),
                    "env_cap": serde_json::Value::Null,
                    "unrecoverable": false,
                    "expect": web_outcome(tool, mode),
                }));
            }
        }

        out
    }

    /// The file, exactly as it is checked in: one vector per line, so a diff reads.
    fn vectors_document() -> String {
        let mut text = String::new();
        text.push_str("{\n");
        text.push_str(
            "  \"_\": [\n\
             \x20   \"Generated. Do not hand-edit: YANTRIK_WRITE_VECTORS=1 cargo test --offline\",\n\
             \x20   \"--profile fast -p yantrik-ui --bin yantrik-ui mind_mode_write_vectors\",\n\
             \x20   \"\",\n\
             \x20   \"mind_mode::Modes::decide in crates/yantrik-ui/src/mind_mode.rs and decide()\",\n\
             \x20   \"in deploy/yantrik-os/yos-mcp are the same table written twice, on purpose:\",\n\
             \x20   \"the bridge deciding for itself costs one read of the shell instead of two.\",\n\
             \x20   \"Two copies drift. These vectors are what stops it being silent — the Rust\",\n\
             \x20   \"side writes them and a test asserts the file still matches; the Python side\",\n\
             \x20   \"is driven through every one of them by yos-mcp-selftest.py.\",\n\
             \x20   \"\",\n\
             \x20   \"layer: `shell` came straight out of Modes::decide. `harness_cap` is\",\n\
             \x20   \"YOS_MCP_MAX_PERMISSION, which the shell does not enforce and should not —\",\n\
             \x20   \"it is a cap a harness puts on itself — and `browser` is the plan-mode rule\",\n\
             \x20   \"for tools that are not on the shell's surface at all. Both are modelled in\",\n\
             \x20   \"the generator, which the generator says so about.\",\n\
             \x20   \"\",\n\
             \x20   \"unrecoverable is approvals::unrecoverable over the action's own published\",\n\
             \x20   \"purpose, and it is an INPUT to both tables since 21 September 2026: in auto\",\n\
             \x20   \"an action the app says cannot be undone is asked about exactly as a\",\n\
             \x20   \"dangerous one is, and no session rule covers one in any mode. It was\",\n\
             \x20   \"carried and ignored before that, under the name `recoverable`, which is why\",\n\
             \x20   \"the axis was already here to turn on. safe is excluded: a read destroys\",\n\
             \x20   \"nothing, so matching wording cannot make one into a question.\"\n\
             \x20 ],\n",
        );
        text.push_str(
            "  \"outcomes\": {\n\
             \x20   \"run\": \"ran; nobody was asked and nothing is written down\",\n\
             \x20   \"run_logged\": \"ran unasked; `ask` mode would have raised a card, so it goes in the audit\",\n\
             \x20   \"ask\": \"a card in front of the person, and a wait\",\n\
             \x20   \"refuse_grade\": \"the grade is not one this OS defines; None is not safe\",\n\
             \x20   \"refuse_ceiling\": \"above tool_permission; nobody is asked, in any mode\",\n\
             \x20   \"refuse_mode\": \"plan mode: nothing was changed, say what you would do\"\n\
             \x20 },\n",
        );
        text.push_str("  \"vectors\": [\n");
        let vectors = all_vectors();
        for (index, vector) in vectors.iter().enumerate() {
            text.push_str("    ");
            text.push_str(&serde_json::to_string(vector).expect("a vector is plain json"));
            if index + 1 < vectors.len() {
                text.push(',');
            }
            text.push('\n');
        }
        text.push_str("  ]\n}\n");
        text
    }

    fn vectors_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("deploy")
            .join("yantrik-os")
            .join("mind-mode-vectors.json")
    }

    /// Write the vectors out. Gated, because a test that rewrites its own expectation is not a
    /// test — it is a way for a change to `decide` to pass CI by regenerating what it broke.
    ///
    /// ```sh
    /// YANTRIK_WRITE_VECTORS=1 cargo test --offline --profile fast \
    ///   -p yantrik-ui --bin yantrik-ui mind_mode_write_vectors
    /// ```
    #[test]
    fn mind_mode_write_vectors() {
        if std::env::var("YANTRIK_WRITE_VECTORS").as_deref() != Ok("1") {
            return;
        }
        let path = vectors_path();
        std::fs::write(&path, vectors_document())
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        println!("wrote {} vectors to {}", all_vectors().len(), path.display());
    }

    /// This table and the dispatch's agree, on every vector the dispatch's own tests generate.
    ///
    /// `deploy/yantrik-os/surface-vectors.json` is `yantrik_ipc_transport::gate::decide` written
    /// out — what every app's dispatch refuses and why — and each vector carries `door`: what a
    /// door that raises cards must do with the same inputs. This table is that door (the shell's
    /// `request_approval` asks it), so it is held to the column. Where the two may differ is
    /// written into the vector as a `note` rather than excused here: plan mode's `standard` runs
    /// on a socket (`SOCKET_FLOOR`) and is refused here, and that is the only place.
    #[test]
    fn mind_mode_agrees_with_the_dispatch_on_every_surface_vector() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../deploy/yantrik-os/surface-vectors.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let doc: serde_json::Value = serde_json::from_str(&text).expect("surface vectors are json");
        let vectors = doc["decide"].as_array().expect("a `decide` list");
        assert!(vectors.len() >= 640, "the file shrank to {} vectors", vectors.len());
        let now = Instant::now();
        let mut drifted = Vec::new();
        for v in vectors {
            let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
            let (app, action) = (s("app"), s("action"));
            let rules: Vec<(&str, &str)> =
                if v["session_rule"] == true { vec![(app.as_str(), action.as_str())] } else { vec![] };
            let mode = Mode::parse(&s("mode")).expect("a mode");
            let modes = modes_for(mode, &rules, now);
            let unrecoverable = v["unrecoverable"] == true;
            // The published sentence, read the way `request_approval` reads it, must agree with
            // the fact the vector carries — one reading of the sentence on the machine.
            assert_eq!(approvals::unrecoverable(&s("purpose")), unrecoverable, "{}", s("id"));
            let got = match modes.decide(&s("grade"), &app, &action, unrecoverable, &s("ceiling"), now) {
                Decision::Run { .. } => "run",
                Decision::Ask => "ask",
                Decision::Refuse { .. } => "refuse",
            };
            if got != s("door") {
                drifted.push(format!("{}: the dispatch says a door should {}, this table says {got}", s("id"), s("door")));
            }
        }
        assert!(drifted.is_empty(), "{} disagreements:\n{}", drifted.len(), drifted.join("\n"));
        let floor = vectors.iter().filter(|v| v.get("note").is_some()).count();
        assert!(floor > 0, "the one documented difference is still in the file, named");
    }

    /// And the checked-in file is what the code produces today.
    ///
    /// This is the half that makes the pair worth having: changing `decide` without
    /// regenerating fails here, and regenerating without changing the bridge fails in
    /// `yos-mcp-selftest.py`. Neither side can move alone.
    #[test]
    fn mind_mode_the_checked_in_vectors_are_what_decide_produces() {
        let path = vectors_path();
        let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e}\n\nGenerate it with:\n  YANTRIK_WRITE_VECTORS=1 cargo test \
                 --offline --profile fast -p yantrik-ui --bin yantrik-ui mind_mode_write_vectors",
                path.display()
            )
        });
        let produced = vectors_document();
        if checked_in == produced {
            return;
        }
        // Say WHICH cell moved. "the file differs" over three hundred vectors is a diff nobody
        // reads; the outcome that changed is the one line somebody has to think about.
        let old: serde_json::Value =
            serde_json::from_str(&checked_in).expect("the checked-in vectors are json");
        let new: serde_json::Value =
            serde_json::from_str(&produced).expect("what the generator made is json");
        let empty = Vec::new();
        let old_v = old["vectors"].as_array().unwrap_or(&empty);
        let new_v = new["vectors"].as_array().unwrap_or(&empty);
        let mut moved: Vec<String> = Vec::new();
        for want in new_v {
            let id = want["id"].as_str().unwrap_or_default();
            match old_v.iter().find(|v| v["id"].as_str() == Some(id)) {
                Some(had) if had["expect"] == want["expect"] => {}
                Some(had) => moved.push(format!(
                    "{id}: was {} and is now {}",
                    had["expect"], want["expect"]
                )),
                None => moved.push(format!("{id}: new")),
            }
        }
        for had in old_v {
            let id = had["id"].as_str().unwrap_or_default();
            if !new_v.iter().any(|v| v["id"].as_str() == Some(id)) {
                moved.push(format!("{id}: gone"));
            }
        }
        panic!(
            "deploy/yantrik-os/mind-mode-vectors.json is not what this code decides any more.\n\
             {}\n\n\
             The table is written twice — here and in deploy/yantrik-os/yos-mcp. If you meant \
             to change it, change BOTH, then regenerate:\n  \
             YANTRIK_WRITE_VECTORS=1 cargo test --offline --profile fast -p yantrik-ui \
             --bin yantrik-ui mind_mode_write_vectors\n  \
             python3 deploy/yantrik-os/yos-mcp-selftest.py",
            if moved.is_empty() {
                "(every outcome is the same; only the file's shape changed)".to_string()
            } else {
                moved.join("\n")
            }
        );
    }
}
