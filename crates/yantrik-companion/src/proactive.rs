//! Proactive conversation engine — delivers urge-based messages without the LLM.
//!
//! When an urge reaches sufficient urgency, the engine composes a message
//! from instinct-specific templates and pushes it to the user via the
//! proactive message channel. No LLM required.
//!
//! V15: Now uses the `proactive_templates` engine first (bond-aware templates
//! with data slots), falling back to the legacy `compose_message` for instincts
//! that don't have templates yet.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::bond::BondLevel;
use crate::config::ProactiveConfig;
use crate::proactive_templates::TemplateEngine;
use crate::types::{ProactiveMessage, Urge};
use crate::urges::UrgeQueue;

/// Engine that converts high-urgency urges into proactive messages.
///
/// V15 frequency governor: cooldown scales with bond level, and a question
/// budget ensures we don't ask too many questions (2:1 statement-to-question ratio).
pub struct ProactiveEngine {
    config: ProactiveConfig,
    last_delivery_ts: f64,
    user_name: String,
    templates: TemplateEngine,
    bond_level: BondLevel,
    /// Rolling count of statements delivered (resets every 24h).
    statements_today: u32,
    /// Rolling count of questions delivered (resets every 24h).
    questions_today: u32,
    /// Timestamp of last daily reset.
    daily_reset_ts: f64,
}

impl ProactiveEngine {
    pub fn new(config: ProactiveConfig, user_name: &str) -> Self {
        Self {
            config,
            last_delivery_ts: 0.0,
            user_name: user_name.to_string(),
            templates: TemplateEngine::new(),
            bond_level: BondLevel::Stranger,
            statements_today: 0,
            questions_today: 0,
            daily_reset_ts: 0.0,
        }
    }

    /// Update the bond level used for template rendering and frequency gating.
    pub fn set_bond_level(&mut self, level: BondLevel) {
        self.bond_level = level;
    }

    /// Bond-based cooldown in seconds.
    ///
    /// Stranger: 60 min, Acquaintance: 45 min, Friend: 30 min,
    /// Confidant: 20 min, Partner-in-Crime: 10 min.
    fn effective_cooldown_secs(&self) -> f64 {
        let bond_cooldown: f64 = match self.bond_level {
            BondLevel::Stranger => 120.0 * 60.0,
            BondLevel::Acquaintance => 90.0 * 60.0,
            BondLevel::Friend => 60.0 * 60.0,
            BondLevel::Confidant => 40.0 * 60.0,
            BondLevel::PartnerInCrime => 25.0 * 60.0,
        };
        // Config cooldown is a floor — never go below configured minimum
        let config_cooldown = self.config.cooldown_minutes as f64 * 60.0;
        bond_cooldown.max(config_cooldown)
    }

    /// Check if sending a question is within budget (2:1 statement-to-question ratio).
    fn question_budget_ok(&self, is_question: bool) -> bool {
        if !is_question {
            return true;
        }
        // Allow at least 1 question even with 0 statements
        if self.questions_today == 0 {
            return true;
        }
        // 2:1 ratio — need at least 2 statements per question
        self.statements_today >= self.questions_today * 2
    }

    /// Reset daily counters if a new day has started.
    fn maybe_reset_daily(&mut self, now: f64) {
        if now - self.daily_reset_ts > 86400.0 {
            self.statements_today = 0;
            self.questions_today = 0;
            self.daily_reset_ts = now;
        }
    }

    /// Check if any pending urge qualifies for proactive delivery.
    ///
    /// Called during each think cycle (~60s). Returns a message if
    /// an urge exceeds the urgency threshold and cooldown has elapsed.
    pub fn check(
        &mut self,
        urge_queue: &UrgeQueue,
        conn: &Connection,
    ) -> Option<ProactiveMessage> {
        if !self.config.enabled {
            tracing::info!("Proactive disabled");
            return None;
        }

        let now = now_ts();
        self.maybe_reset_daily(now);

        // V15 frequency governor: bond-based cooldown
        let cooldown_secs = self.effective_cooldown_secs();
        if let Some(elapsed) = cooldown_elapsed(now, self.last_delivery_ts) {
            if elapsed < cooldown_secs {
                // Per-cycle while the cooldown runs, so it is a debug line: a desktop
                // whose backend was down all day filled the log with these (#30).
                tracing::debug!(
                    elapsed_secs = elapsed as u64,
                    cooldown_secs = cooldown_secs as u64,
                    bond = self.bond_level.name(),
                    "Proactive cooldown active (bond-scaled)"
                );
                return None;
            }

            tracing::info!(
                elapsed_secs = elapsed as u64,
                bond = self.bond_level.name(),
                "Proactive cooldown expired, checking urges"
            );
        }

        // Peek at top pending urge
        let pending = urge_queue.get_pending(conn, 1);
        let urge = match pending.first() {
            Some(u) => u,
            None => {
                tracing::info!("Proactive check: no pending urges");
                return None;
            }
        };

        // Must exceed urgency threshold
        if urge.urgency < self.config.delivery_threshold {
            tracing::info!(
                urgency = urge.urgency,
                threshold = self.config.delivery_threshold,
                "Proactive check: urgency below threshold"
            );
            return None;
        }

        // Must have a suggested message (instinct should populate this)
        if urge.suggested_message.is_empty() && urge.reason.is_empty() {
            tracing::info!(
                instinct = urge.instinct_name,
                "Proactive check: no message text"
            );
            return None;
        }

        // Pop it (marks as delivered in the urge queue)
        let delivered = urge_queue.pop_for_interaction(conn, 1);
        let urge = delivered.into_iter().next()?;

        let text = self.compose_message(&urge);

        // Skip delivery if compose returned empty (e.g. humor hint without concrete text)
        if text.is_empty() {
            tracing::info!(
                instinct = urge.instinct_name,
                "Proactive skipped — no composable message"
            );
            return None;
        }

        // Machinery, a raw tool call, or idle thinking that found nothing — none of it is a
        // thought worth saying anywhere. See `judge_proactive`.
        if let Some(why) = must_not_be_said(&text) {
            tracing::warn!(
                instinct = urge.instinct_name,
                reason = why,
                text = text.as_str(),
                "Proactive refused — the composed message is not something to say"
            );
            return None;
        }

        // V15: Question budget — check if this message is a question
        let is_question = text.ends_with('?');
        if !self.question_budget_ok(is_question) {
            tracing::info!(
                statements = self.statements_today,
                questions = self.questions_today,
                "Proactive skipped — question budget exceeded (2:1 ratio)"
            );
            return None;
        }

        self.last_delivery_ts = now;

        // Track statement/question counts
        if is_question {
            self.questions_today += 1;
        } else {
            self.statements_today += 1;
        }

        tracing::info!(
            instinct = urge.instinct_name,
            urgency = urge.urgency,
            is_question,
            statements = self.statements_today,
            questions = self.questions_today,
            "Proactive message delivered"
        );

        Some(ProactiveMessage {
            text,
            urge_ids: vec![urge.urge_id],
            generated_at: now,
        })
    }

    /// Compose a user-facing message from an urge.
    ///
    /// Tries V15 template engine first (bond-aware, data-slot templates).
    /// Falls back to legacy hardcoded patterns for instincts without templates.
    fn compose_message(&mut self, urge: &Urge) -> String {
        let instinct = urge.instinct_name.to_lowercase();

        // Build data slots from the urge's context + standard fields
        let data = self.build_data_slots(urge);

        // Try template engine first
        if let Some(rendered) = self.templates.render(&instinct, &data, self.bond_level) {
            return rendered;
        }

        // Legacy fallback for instincts without templates
        self.compose_legacy(urge)
    }

    /// Build the data slot map from an urge for template rendering.
    fn build_data_slots(&self, urge: &Urge) -> HashMap<String, String> {
        let mut data = HashMap::new();

        // Standard slots available to all templates
        data.insert("user".into(), self.user_name.clone());
        data.insert("reason".into(), urge.reason.clone());
        if !urge.suggested_message.is_empty() {
            data.insert("message".into(), urge.suggested_message.clone());
        }

        // Extract slots from urge context JSON
        if let Some(obj) = urge.context.as_object() {
            for (key, val) in obj {
                let s = match val {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => val.to_string(),
                };
                if !s.is_empty() && s != "null" {
                    data.insert(key.clone(), s);
                }
            }
        }

        data
    }

    /// Legacy message composition (pre-V15).
    fn compose_legacy(&self, urge: &Urge) -> String {
        let user = &self.user_name;
        let reason = &urge.reason;
        let msg = &urge.suggested_message;

        let instinct = urge.instinct_name.to_lowercase();
        match instinct.as_str() {
            "check_in" => {
                if msg.is_empty() {
                    format!("Hey {}. {}", user, reason)
                } else {
                    msg.clone()
                }
            }
            "reminder" => {
                if msg.is_empty() {
                    format!("Reminder: {}", reason)
                } else {
                    msg.clone()
                }
            }
            "follow_up" => {
                if msg.is_empty() {
                    format!("By the way \u{2014} {}", reason)
                } else {
                    format!("By the way \u{2014} {}", msg)
                }
            }
            "emotional_awareness" => {
                if msg.is_empty() {
                    format!("I noticed {}.", reason)
                } else {
                    format!("I noticed {}. {}", reason, msg)
                }
            }
            "pattern_surfacing" => {
                format!("I've been noticing something: {}", reason)
            }
            "conflict_alerting" | "memoryweaver" => {
                // Internal housekeeping urges — only deliver if instinct provided
                // a concrete suggested_message (e.g. milestone celebrations).
                if msg.is_empty() {
                    return String::new();
                }
                msg.clone()
            }
            "bond_milestone" | "bondmilestone" => {
                if msg.is_empty() {
                    reason.clone()
                } else {
                    msg.clone()
                }
            }
            "scheduler" => {
                if msg.is_empty() {
                    format!("Scheduled: {}", reason)
                } else {
                    msg.clone()
                }
            }
            "emailwatch" => {
                if !msg.is_empty() { msg.clone() }
                else if !reason.is_empty() { format!("Email alert \u{2014} {}", reason) }
                else { return String::new(); }
            }
            "self_awareness" | "selfawareness" => reason.clone(),
            "humor" => {
                // Humor urges are tone hints for conversations, not standalone messages.
                // Only deliver if the instinct provided a concrete suggested_message.
                if msg.is_empty() {
                    return String::new(); // Skip — raw hint, not user-facing
                }
                msg.clone()
            }
            // Natural Communication instincts — all use EXECUTE so they produce
            // suggested_message via LLM. Legacy fallback only if EXECUTE path failed.
            "aftermath" | "questionasking" | "eveningreflection" | "conversationalcallback" | "silencereveal" => {
                if msg.is_empty() {
                    return String::new();
                }
                msg.clone()
            }
            _ => {
                // Unknown instinct — use whatever text is available
                if !msg.is_empty() {
                    msg.clone()
                } else {
                    reason.clone()
                }
            }
        }
    }
}

fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// How long the cooldown has been running: the time since the engine last delivered.
///
/// `None` when it has never delivered. `last_delivery_ts` starts at `0.0`, and reading an
/// unset `0.0` as a real timestamp made `now - 0.0` the seconds since the Unix epoch — the
/// `elapsed_secs=1789671372` a dead backend logged once a minute all day (#30). A cooldown
/// is measured from the engine's own last delivery, so before the first one there is no
/// elapsed to report and nothing to wait out.
fn cooldown_elapsed(now: f64, last_delivery_ts: f64) -> Option<f64> {
    if last_delivery_ts > 0.0 {
        Some(now - last_delivery_ts)
    } else {
        None
    }
}

// ── What is not a thought ───────────────────────────────────────────────────────────────────

/// Openings that mean the text is machinery talking, not the companion.
///
/// Matched against the START of the message only. A thought is allowed to mention an error it
/// found — "Error: query is required" as the first thing said is not a mention, it is the raw
/// string a tool handed back.
const NOT_A_THOUGHT: &[(&str, &str)] = &[
    ("error:", "a tool's error string"),
    ("exception:", "a tool's error string"),
    ("traceback (most recent call last)", "a python traceback"),
    ("panicked at", "a rust panic"),
    ("permission denied:", "the tool registry's refusal"),
    ("unknown tool:", "the tool registry's refusal"),
    ("tool:", "a tool-call transcript line"),
    ("recall failed:", "a tool's error string"),
    ("i'm sorry, i can't", "a model refusal"),
    ("i'm sorry, but i can't", "a model refusal"),
    ("i cannot help with", "a model refusal"),
    ("i can't help with", "a model refusal"),
    ("as an ai language model", "a model refusal"),
];

/// Is this message a tool's error rather than something to say? The reason, if so.
///
/// Observed on 22 September 2026: a MemoryWeaver urge planned a single `recall` step, the plan
/// carried no `query`, the tool answered `Error: query is required`, and the synthesis step —
/// which is told to use only what the tools returned — turned that into
/// *"Error: query is required, so there are no details available to surface a memory
/// connection."* It was posted as notification 68 and sat in the notification centre as one of
/// the machine's own thoughts.
///
/// The recall that could not run is fixed where it was called from. This is the backstop, and
/// it is a separate rule: a synthesis step will narrate whatever it is handed, so any tool
/// failure at all can come back out of the pipeline wearing a sentence. Nothing that opens with
/// one is worth a person's attention, and saying nothing costs nothing — the urge is still in
/// the log, which is where a broken tool call belongs.
pub fn looks_like_tool_error(text: &str) -> Option<&'static str> {
    let start = text
        .trim_start()
        .trim_start_matches(['*', '_', '`', '>', '"', '\'', ' '])
        .to_lowercase()
        // A model writes "can’t" as often as "can't", and the two must not be different rules.
        .replace('\u{2019}', "'");
    NOT_A_THOUGHT
        .iter()
        .find(|(opening, _)| start.starts_with(opening))
        .map(|(_, reason)| *reason)
}

// ── What may become a notification ──────────────────────────────────────────────────────────
//
// Observed on 23 September 2026 (issue #216): the desktop's own companion filed 35 notifications
// in one day, almost all of it chatter rather than anything a person could act on — "coffee's on,
// tech's your jam … [unverified] ☕", "how's your day going?", a raw
// `recall(query=…) → Nothing to surface right now.`, and a toast that read "Nothing actionable
// right now". Notifications are the desktop's one interruptive channel; filling it with small
// talk, internal markers and leaked tool calls teaches people to clear it unread, and then the
// approval, the finished agent or the failed recipe that actually matters is cleared with the
// rest.
//
// The rule is one pure function, [`judge_proactive`], so it can be tested against the messages
// that were really filed. It separates three outcomes: machinery and empty findings must not be
// said anywhere; small talk may reach the Lens, which a person opened on purpose, but must not
// interrupt; and only something actionable — a reminder the person set, a finding about their
// files or calendar with a thing to do, a follow-up they asked for — may become a notification.
// The daily cap on the companion's notifications lives in the shell, next to the store it feeds;
// this is the content half of the same rule.

/// What a proactive thought may become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationVerdict {
    /// Actionable, and cleaned of the markers and emoji a persona adds: it may become a
    /// notification. The cleaned text is carried here.
    Notify(String),
    /// Small talk. It may reach the Lens if the Lens is open, but it must not interrupt.
    LensOnly,
    /// Machinery, or idle thinking that found nothing. It must not be said anywhere. The reason,
    /// for the log.
    Refuse(&'static str),
}

/// The rule for what a proactive companion thought may become. One pure function — see the
/// comment above for why it exists and what it refuses.
pub fn judge_proactive(text: &str) -> NotificationVerdict {
    // Before anything else: a placeholder standing in for "nothing" is not a thought. The model
    // was told to say nothing and wrote the word for nothing (#88 — "[empty]" was posted to the
    // notification centre, with no body, three times).
    if is_contentless(text) {
        return NotificationVerdict::Refuse("a placeholder that says nothing");
    }
    // Machinery first: a tool's own error wearing a sentence.
    if let Some(why) = looks_like_tool_error(text) {
        return NotificationVerdict::Refuse(why);
    }
    // A raw tool call is machinery too, whether or not it errored. `recall(query=…) → …` was
    // filed verbatim as a notification; nobody asked to read the transcript.
    if looks_like_tool_call(text) {
        return NotificationVerdict::Refuse("a raw tool call");
    }
    // Idle thinking that found nothing has nothing to say. "Nothing actionable right now" must
    // never reach a person — saying it is the interruptive channel spending itself on silence.
    if is_empty_finding(text) {
        return NotificationVerdict::Refuse("idle thinking found nothing");
    }
    // Only something a person can act on may interrupt. Everything else — the greetings, the
    // "how's your day", the playful observations — stays in the Lens.
    if !is_actionable(text) {
        return NotificationVerdict::LensOnly;
    }
    NotificationVerdict::Notify(clean_for_person(text))
}

/// The [`NotificationVerdict::Refuse`] case on its own, for the gates that drop a message
/// everywhere — the transcript included — rather than only withholding a notification. This is a
/// projection of the one rule, not a second rule: `ProactiveEngine::check` and the bridge's
/// delivery join ask it so machinery and empty findings are never composed into a message at all.
pub fn must_not_be_said(text: &str) -> Option<&'static str> {
    match judge_proactive(text) {
        NotificationVerdict::Refuse(why) => Some(why),
        _ => None,
    }
}

/// Does the text open as a raw tool call — `name(args)`, with or without a `→ result` after it?
///
/// Matched at the start, or by a result arrow, so a sentence that merely mentions a call in
/// passing ("I ran the backup check") is not caught. The transcript line the companion leaked read
/// `recall(query="…") → Nothing to surface right now.` and starts with the call.
fn looks_like_tool_call(text: &str) -> bool {
    let t = text
        .trim_start()
        .trim_start_matches(['*', '_', '`', '>', '"', '\'', ' ']);
    // A result arrow just after a closing paren: `recall(…) → Nothing to surface right now.`
    if t.contains(") →") || t.contains(")->") || t.contains(") ->") {
        return true;
    }
    // Or the message simply opens with a call: an identifier, no space, then `(`.
    let Some(paren) = t.find('(') else {
        return false;
    };
    paren > 0
        && !t[..paren].contains(' ')
        && t[..paren]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

/// Openings that mean idle thinking found nothing and there is nothing to say.
const EMPTY_FINDING: &[&str] = &[
    "nothing actionable",
    "nothing to surface",
    "nothing to report",
    "nothing to show",
    "nothing to tell",
    "nothing to flag",
    "nothing worth",
    "nothing pending",
    "nothing new",
    "nothing interesting",
    "no pending",
    "all clear",
    "all quiet",
];

/// Is this idle thinking that found nothing? Matched at the start, the way `looks_like_tool_error`
/// is: a thought may mention that something is clear, but opening with it means the turn produced
/// nothing, and "nothing" is never worth an interruption.
fn is_empty_finding(text: &str) -> bool {
    let start = text
        .trim_start()
        .trim_start_matches(['*', '_', '`', '>', '"', '\'', ' '])
        .to_lowercase();
    EMPTY_FINDING.iter().any(|opening| start.starts_with(opening))
}

/// Whole-message placeholders a model writes when it was told to say nothing (#88).
///
/// Observed on the VM, filed on 21 September 2026: notifications 7, 28 and 41 carried the titles
/// "[empty]", "[empty]" and "(empty)" with an empty body. Matched against the WHOLE message —
/// unlike the opening rules above — so a real sentence that happens to contain one of these words
/// ("the inbox was empty") is not caught, only a message that is nothing but the placeholder.
const CONTENTLESS: &[&str] = &[
    "", "[empty]", "(empty)", "empty", "[]", "()", "[none]", "(none)", "none", "null",
    "[nothing]", "(nothing)", "nothing", "[blank]", "(blank)", "blank", "no content",
    "[no content]", "(no content)", "n/a", "na", "-", "—",
];

/// Is the whole message a placeholder standing in for "nothing"?
fn is_contentless(text: &str) -> bool {
    let whole = text
        .trim()
        .trim_matches(['*', '_', '`', '>', '"', '\'', ' '])
        .trim_end_matches(['.', '!', '…'])
        .trim()
        .to_lowercase()
        .replace('\u{2019}', "'");
    CONTENTLESS.contains(&whole.as_str())
}

/// Words and phrases that mark a thought as something a person can act on: a reminder, a finding
/// about their files, calendar, mail or money with a thing to do, a step that needs a decision, or
/// a follow-up they asked for. A heuristic, not a parser — a proactive thought that misses every
/// cue is treated as small talk and kept out of the notification store, and the shell's daily cap
/// is the backstop for volume. Chosen so the chatty messages filed on 23 September match none of
/// them; the test table below holds those verbatim.
const ACTIONABLE: &[&str] = &[
    // A reminder, or something on the clock.
    "reminder", "remind", "don't forget", "do not forget", "remember to", "time to",
    "due", "deadline", "overdue", "scheduled", "schedule", "appointment", "meeting",
    "standup", "stand-up", "interview", "starts in", "starts at", "minutes",
    // A follow-up or a commitment — something the person asked for or owes.
    "you asked", "you wanted", "you said", "you mentioned", "you requested", "as requested",
    "follow up", "follow-up", "followup", "commitment", "committed", "todo", "to-do", "task",
    // A finding about the person's files, calendar, mail or money.
    "email", "e-mail", "inbox", "unread", "attachment", "file", "folder", "document",
    "calendar", "backup", "invoice", "bill", "payment", "receipt", "renewal", "delivery",
    "shipment", "expires", "expiring", "expired",
    // Something went wrong, or needs a decision.
    "needs attention", "action needed", "action required", "requires", "approval", "approve",
    "waiting for", "pending approval", "failed", "failure", "error", "warning", "alert",
    "critical", "blocked", "rejected", "denied", "unpaid", "running low", "low battery",
    "low storage", "disk full", "almost full",
    // An explicit nudge.
    "you might want", "you may want", "consider", "heads up", "heads-up", "just so you know",
    "wanted to let you know", "letting you know", "fyi",
];

/// Is this something a person can act on? A case-insensitive match against [`ACTIONABLE`].
fn is_actionable(text: &str) -> bool {
    let lower = text.to_lowercase();
    ACTIONABLE.iter().any(|cue| lower.contains(cue))
}

/// Markers the machine writes for itself that must never reach a person's text. These are the
/// exact forms the hallucination firewall and the recipe executor emit.
const INTERNAL_MARKERS: &[&str] = &[
    "[unverified]",
    "_(unverified)_",
    "(unverified)",
    "_(limited source)_",
    "[limited source]",
    "[verified]",
    "[citation needed]",
];

/// Take the machine's own decorations off a message bound for a person: the internal markers the
/// firewall and recipe executor add, and the emoji the persona adds. What is left is the sentence.
fn clean_for_person(text: &str) -> String {
    let mut cleaned = text.to_string();
    for marker in INTERNAL_MARKERS {
        // With the space before it first, so removing the marker does not strand a double space.
        cleaned = cleaned.replace(&format!(" {marker}"), "");
        cleaned = cleaned.replace(marker, "");
    }
    let cleaned: String = cleaned.chars().filter(|c| !is_emoji(*c)).collect();
    tidy_spaces(&cleaned)
}

/// Is this character a picture rather than a word — an emoji, a dingbat, a variation selector or a
/// zero-width joiner? The punctuation a sentence needs (— … ' " ·) is deliberately kept.
fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF  // pictographs, emoticons, transport, supplemental symbols
        | 0x2600..=0x27BF  // misc symbols and dingbats — ☕ ✨ ✅
        | 0x2B00..=0x2BFF  // misc symbols and arrows — ⭐
        | 0xFE00..=0xFE0F  // variation selectors
        | 0x200D           // zero-width joiner
        | 0x20E3           // combining enclosing keycap
    )
}

/// Collapse the double spaces a removal left behind and trim, without flattening line breaks —
/// `headline_and_rest` in the shell still reads them to split a title from a body.
fn tidy_spaces(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let mut collapsed = String::with_capacity(line.len());
            let mut prev_space = false;
            for ch in line.chars() {
                if ch == ' ' {
                    if prev_space {
                        continue;
                    }
                    prev_space = true;
                } else {
                    prev_space = false;
                }
                collapsed.push(ch);
            }
            // A space stranded before punctuation by a removal.
            collapsed
                .replace(" .", ".")
                .replace(" ,", ",")
                .replace(" ;", ";")
                .replace(" :", ":")
                .replace(" !", "!")
                .replace(" ?", "?")
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_error_is_never_said_out_loud() {
        // Notification 68, verbatim. The synthesis step was handed `Error: query is required`
        // and wrote a sentence around it; the desktop posted it as a thought.
        assert_eq!(
            looks_like_tool_error(
                "Error: query is required, so there are no details available to surface a \
                 memory connection."
            ),
            Some("a tool's error string")
        );
        // The same thing with the markdown a model tends to put round it.
        assert_eq!(
            looks_like_tool_error("**Error:** the recall returned nothing"),
            Some("a tool's error string")
        );
        assert!(looks_like_tool_error("Traceback (most recent call last):\n  File \"x.py\"")
            .is_some());
        assert!(looks_like_tool_error("Permission denied: 'run_command' requires Dangerous")
            .is_some());
        assert!(looks_like_tool_error("Tool: recall() → Error: query is required").is_some());
        assert!(looks_like_tool_error("I'm sorry, I can't help with that.").is_some());
        assert!(looks_like_tool_error("I\u{2019}m sorry, I can\u{2019}t help with that.").is_some());
    }

    #[test]
    fn an_ordinary_thought_still_gets_through() {
        // Notification 61, verbatim — the one that was worth reading.
        assert_eq!(
            looks_like_tool_error(
                "One thing that stood out: your memory graph shows you've set up both a morning \
                 brief and a preference for warm, concise end-of-day reflections."
            ),
            None
        );
        assert_eq!(looks_like_tool_error("Hey — how's your morning shaping up?"), None);
        // A thought is allowed to be ABOUT an error; it just may not open as one.
        assert_eq!(
            looks_like_tool_error("The backup job hit an error: the disk is full."),
            None
        );
        assert_eq!(looks_like_tool_error(""), None);
    }

    #[test]
    fn only_something_actionable_may_become_a_notification() {
        use NotificationVerdict::*;

        // The day the companion filed 35 notifications, verbatim. Not one of these is something a
        // person could act on, so not one may interrupt — they are Lens-only at most.
        for small_talk in [
            "Hey Pranab — coffee's on, tech's your jam, and I'm here to keep things rolling \
             smoothly today [unverified]. ☕",
            "Hey Pranab—how's your day going? Coffee still in hand? ☕",
            "Browser's off its leash — zero tabs, no processes humming. Not that I blame it; \
             probably just wants a vacation from your debugging sessions.",
            "You've got that tech-debugging itch again, don't you? Remember the WebGL session \
             earlier—how'd it go down?",
            "Hey, what's new on your end? Still hacking away at tech stuff or just chilling today?",
        ] {
            assert_eq!(
                judge_proactive(small_talk),
                LensOnly,
                "small talk must not become a notification: {small_talk}"
            );
        }

        // A raw tool call, and idle thinking that found nothing, are refused outright — never said
        // anywhere, let alone filed.
        assert_eq!(
            judge_proactive(
                "recall(query=\"interesting memory connection past conversation event\") → \
                 Nothing to surface right now."
            ),
            Refuse("a raw tool call")
        );
        assert_eq!(
            judge_proactive("Nothing actionable right now — no pending tasks or reminders."),
            Refuse("idle thinking found nothing")
        );
        // A tool's error is still machinery, by the older rule this one is built on.
        assert_eq!(
            judge_proactive("Error: query is required, so there is nothing to surface."),
            Refuse("a tool's error string")
        );

        // What the channel is for: a reminder, a finding with a thing to do, a follow-up asked for.
        for actionable in [
            "Your meeting starts in 15 minutes.",
            "Reminder: standup moved to 10:30.",
            "You have 2 unread emails from Sam.",
            "The nightly backup failed — the disk is almost full.",
            "You asked me to follow up on the report; it is ready to review.",
        ] {
            assert!(
                matches!(judge_proactive(actionable), Notify(_)),
                "an actionable thought may become a notification: {actionable}"
            );
        }
    }

    #[test]
    fn a_notification_reaches_the_person_without_markers_or_emoji() {
        use NotificationVerdict::*;

        // The internal marker the firewall adds and the emoji the persona adds are stripped; the
        // sentence a person can act on survives whole.
        assert_eq!(
            judge_proactive("The nightly backup failed [unverified]. ☕"),
            Notify("The nightly backup failed.".to_string())
        );
        assert_eq!(
            judge_proactive("Heads up — your invoice is due today 🎉 _(limited source)_"),
            Notify("Heads up — your invoice is due today".to_string())
        );
        // Punctuation a sentence needs is kept; only the pictures go.
        assert_eq!(
            judge_proactive("Your meeting starts in 15 minutes…"),
            Notify("Your meeting starts in 15 minutes…".to_string())
        );
    }

    #[test]
    fn a_proactive_turn_that_ends_in_a_tool_call_files_nothing() {
        // The other half of the same fault: an idle turn planned a `recall`, the tool came back
        // with nothing to surface, and the composed message was the raw call. `check` is the gate
        // every urge passes through, so it must file nothing at all here.
        let conn = Connection::open_in_memory().unwrap();
        let queue = UrgeQueue::new(
            &conn,
            crate::config::UrgeQueueConfig {
                expiry_hours: 48.0,
                max_pending: 20,
                boost_increment: 0.1,
            },
        );
        // An instinct with no template, so `compose_message` falls through to the legacy path and
        // returns the suggested message verbatim — the raw tool call.
        let spec = crate::types::UrgeSpec::new("recall_probe", "a recall that found nothing", 0.95)
            .with_message(
                "recall(query=\"interesting memory connection past conversation event\") → \
                 Nothing to surface right now.",
            )
            .with_cooldown("recall_probe:test");
        assert!(queue.push(&conn, &spec).is_some(), "the urge should queue");

        let mut engine = ProactiveEngine::new(
            ProactiveConfig {
                enabled: true,
                delivery_threshold: 0.4,
                cooldown_minutes: 0,
            },
            "Pranab",
        );
        assert!(
            engine.check(&queue, &conn).is_none(),
            "a turn that ends in a raw tool call must file nothing"
        );
    }

    #[test]
    fn a_placeholder_that_says_nothing_is_never_posted() {
        use NotificationVerdict::*;

        // Notifications 7, 28 and 41 on the VM, verbatim: titles "[empty]", "[empty]" and
        // "(empty)" with no body (#88). The instruction had been "say nothing if nothing stands
        // out", and the model wrote the word for nothing. That is not a thought, and this is its
        // own rule — not an entry in the tool-error list, which is about machinery talking.
        for placeholder in ["[empty]", "(empty)", "", "   ", "[none]", "None.", "*[empty]*", "\"(empty)\"", "n/a", "-"] {
            assert_eq!(
                judge_proactive(placeholder),
                Refuse("a placeholder that says nothing"),
                "a message that is nothing but a placeholder must not be said: {placeholder:?}"
            );
        }

        // The rule matches the whole message: a real sentence that happens to contain one of
        // these words is a real thought and is judged on its own merits.
        assert_eq!(judge_proactive("The room was empty when I checked."), LensOnly);
        assert!(matches!(
            judge_proactive("The backup failed — the disk is almost full."),
            Notify(_)
        ));
    }

    #[test]
    fn the_cooldown_measures_from_the_engines_own_last_delivery() {
        // #30: a desktop whose backend never answered logged `elapsed_secs=1789671372`
        // once a minute — the engine's delivery clock was never set, and an unset clock
        // read as the seconds since the Unix epoch instead of "nothing delivered yet".
        let now = 1_790_000_000.0; // a plausible "now" (September 2026)
        assert_eq!(
            cooldown_elapsed(now, 0.0),
            None,
            "a delivery clock that was never set has no elapsed to report"
        );
        assert_eq!(
            cooldown_elapsed(now, now - 90.0),
            Some(90.0),
            "once something was delivered, the cooldown measures from that delivery"
        );
    }
}
