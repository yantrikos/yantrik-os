//! What the desktop knows about each channel a person can reach it from, how a message from one
//! is answered, and how a card is answered from the phone (design/channels-2026-09-29.md).
//!
//! **Who answers a phone.** Only the built-in companion (Pranab, 29 Sep 2026). It holds its own
//! tools to `Safe` for the turn (`Turn::is_remote`), so a stolen phone is not the person at the
//! keyboard. A mind with a shell of its own — the Mind, Hermes, pi — cannot be held to what a
//! phone may ask, even running as its own account: it could start a command that waits out the
//! hold, or act with another of its agents' tokens. When such a mind is the answering one, the
//! phone is told it answers only at the desk, and nothing is sent to it.
//!
//! **Answered later.** The router asks one message at a time, so a turn from the phone is answered
//! on a thread of its own and its answer sent when it comes ([`Outbox`]): a mind waiting on the
//! person's Allow must not hold up the very message that carries it.
//!
//! **Cards on the phone.** The machinery for answering a card from the phone — a one-time code,
//! `ALLOW 123456` / `DENY 123456` from the same identity in the same conversation, once, before it
//! expires; never for what cannot be undone or runs commands as the person; only on a channel the
//! person trusts with an Allow — is built and tested, and waits for its first caller: a card path
//! for the built-in companion's own tools, which today simply stay read-only from a phone. It is
//! the one place other than the card's own buttons that answers a card, and it answers only for
//! the person: the words come from their identity on the channel, which no mind can write as.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use yantrik_chat::model::ConversationRef;
use yantrik_chat::router::{Asker, Outbox};
use yantrik_harness::protocol::Origin;
use yantrik_harness::{Chunk, Turn};

use crate::agents::model::AgentId;

/// Who besides the person can read a channel: `e2e` when it is end-to-end to this box, else
/// `provider-readable` — the operator can read it, as Telegram can a bot's chats. An unknown
/// channel is the latter: saying a channel is private when it is not is the mistake that matters.
pub fn trust_of(provider: &str) -> &'static str {
    match provider {
        // Only Signal. The paired app (`native`) is not end-to-end yet: plain WebSocket, shared
        // tokens, a client id it names itself (security review, 29 Sep 2026).
        "signal" => "e2e",
        _ => "provider-readable",
    }
}

/// Acts no phone answers, whatever their grade: they run commands as the person, and an Allow
/// for one is a remote shell (security review, 29 Sep 2026). They wait for the machine, as what
/// cannot be undone does.
const NEVER_FROM_A_PHONE: &[(&str, &str)] = &[
    ("shell", "agent_run"),
    ("shell", "agent_input"),
    ("terminal", "*"),
    // Work handed on runs unheld: a phone starts and steers none.
    ("shell", "run_recipe"),
    ("shell", "answer_recipe"),
    ("shell", "resume_recipe"),
    ("shell", "hand_off"),
    ("shell", "new_agent"),
    ("shell", "send_to_agent"),
    // A provider's key moved into a harness's settings: never on a code sent back over a
    // channel the provider itself may be able to read.
    ("shell", "assign_provider"),
    ("shell", "revert_provider"),
];

fn never_from_a_phone(app: &str, action: &str) -> bool {
    NEVER_FROM_A_PHONE.iter().any(|(a, x)| a.eq_ignore_ascii_case(app) && (*x == "*" || *x == action))
}

/// Wrong codes a sender may send before every code waiting for them is burned.
const WRONG_CODES: usize = 5;

/// The most codes waiting at once.
const MOST_CODES: usize = 16;

/// How long a code on the phone answers its card: about the card's own life on the desktop.
const CODE_LIFE: Duration = Duration::from_secs(110);

static OUTBOX: OnceLock<Outbox> = OnceLock::new();
/// The providers whose operator can read them on which the person still turned approvals on.
static APPROVALS_ON: OnceLock<Vec<String>> = OnceLock::new();

/// Where a phone turn came from, while its agent answers it.
#[derive(Clone)]
struct PhoneTurn {
    /// This turn, among others of the same agent.
    id: u64,
    agent: String,
    mind: String,
    provider: String,
    sender_id: String,
    conversation: ConversationRef,
}

/// A card sent to a phone: the code that answers it, and who may send it, from where.
struct PhoneCard {
    code: String,
    card_id: String,
    /// The phone turn it was raised in: its code dies with the turn.
    turn: u64,
    provider: String,
    sender_id: String,
    conversation: String,
    until: Instant,
}

static PHONE_TURNS: Mutex<Vec<PhoneTurn>> = Mutex::new(Vec::new());
static PHONE_CARDS: Mutex<Vec<PhoneCard>> = Mutex::new(Vec::new());
/// Wrong codes by `(provider, sender)`.
static WRONG: Mutex<Vec<((String, String), usize)>> = Mutex::new(Vec::new());
/// Senders with a phone turn in flight: one at a time each.
static IN_FLIGHT: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
static NEXT_TURN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// What the shell sends to a channel unasked, and which channels the person trusts with an Allow.
/// Called once the channels have started.
pub fn configure(outbox: Outbox, approvals_on: Vec<String>) {
    let _ = OUTBOX.set(outbox);
    let _ = APPROVALS_ON.set(approvals_on.into_iter().map(|p| p.trim().to_ascii_lowercase()).collect());
}

fn approvals_on(provider: &str) -> bool {
    trust_of(provider) == "e2e" || APPROVALS_ON.get().is_some_and(|on| on.iter().any(|p| p == provider))
}

/// The person wrote on a channel. An answer to a card is answered at once; anything else goes to
/// the mind answering, on a thread of its own, and its answer is sent when it comes.
pub fn from_phone(
    text: &str,
    context: &[String],
    max_reply: Option<usize>,
    asker: &Asker,
    outbox: &Outbox,
) -> Option<String> {
    if let Some(said) = card_answer(text, asker) {
        return Some(said);
    }
    let prompt = if context.is_empty() {
        text.to_string()
    } else {
        let history = context.iter().rev().take(6).rev().cloned().collect::<Vec<_>>().join("\n");
        format!("[Chat context]\n{history}\n\n[Latest message]\n{text}")
    };
    let sender = (asker.provider.clone(), asker.sender_id.clone());
    {
        let mut busy = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        if busy.contains(&sender) {
            return Some("Still working on your last message; this one was not sent.".to_string());
        }
        busy.push(sender.clone());
    }
    let turn_asker = asker.clone();
    let outbox = outbox.clone();
    let spawned = std::thread::Builder::new().name("phone-turn".into()).spawn(move || {
        let _free = InFlight(sender);
        let asker = turn_asker;
        let mut said = ask_from_phone(prompt, &asker);
        if said.trim().is_empty() {
            return;
        }
        if let Some(max) = max_reply.filter(|m| said.len() > *m) {
            let at = said.floor_char_boundary(max.saturating_sub(3));
            said = format!("{}...", &said[..at]);
        }
        if let Err(why) = outbox.send(&asker.provider, &asker.conversation, &said) {
            tracing::warn!(provider = %asker.provider, reason = %why, "an answer to the phone could not be sent");
        }
    });
    if spawned.is_err() {
        let mut busy = IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner());
        busy.retain(|s| !(s.0 == asker.provider && s.1 == asker.sender_id));
        return Some("The desktop could not take that right now; ask again in a moment.".to_string());
    }
    None
}

/// A sender's place in [`IN_FLIGHT`], given back when its turn ends.
struct InFlight((String, String));

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| s != &self.0);
    }
}

/// Whether `text` is an answer to a card (`ALLOW 123456` / `DENY 123456`): never kept in a
/// transcript, so no later turn is given the code as context.
pub fn is_card_answer(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    matches!(words.as_slice(), [verb, code]
        if (verb.eq_ignore_ascii_case("allow") || verb.eq_ignore_ascii_case("deny"))
            && code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()))
}

/// What the phone is told when the answering mind is not the built-in companion.
fn desk_only(name: &str) -> String {
    format!(
        "{name} answers only at the desk. From your phone you talk to the built-in companion: make it          the answering mind, or ask again when you are back."
    )
}

/// Ask the mind answering, from the phone `asker` wrote on, and gather its answer. What the phone
/// is told when nothing came is a sentence, never an internal error: the channel's operator can
/// read it.
fn ask_from_phone(prompt: String, asker: &Asker) -> String {
    let Some(host) = crate::wire::harness::host() else {
        return "The desktop is still starting; ask again in a moment.".to_string();
    };
    let origin = Origin {
        channel: asker.provider.clone(),
        remote: true,
        person: asker.sender_name.clone(),
        carries: asker.carries.clone(),
        trust: trust_of(&asker.provider).to_string(),
    };
    let turn = Turn::new(prompt).with_origin(origin);
    let active = host.active_id();
    if active == crate::wire::harness::BUILTIN_ID {
        // By id, not "whoever is active": a switch in between does not redirect it.
        return match host.send_builtin(&active, turn) {
            Some(answer) => gather(answer),
            None => "The desktop's companion is not available right now.".to_string(),
        };
    }
    // Only the built-in companion answers a phone (Pranab, 29 Sep 2026). A mind with a shell of
    // its own — the Mind, Hermes, pi — cannot be held to what a phone may ask: it could start a
    // command that waits out the hold, or act with another of its agents' tokens (security review
    // of the phone cards, N2 and N3). So the phone is told, and nothing is sent to it.
    let name = host.list().into_iter().find(|e| e.id == active).map(|e| e.name).unwrap_or_else(|| active.clone());
    tracing::info!(mind = %active, "a turn from a phone was not sent: only the built-in companion answers a phone");
    desk_only(&name)
}

/// The held turn for a mind answering from the phone: kept for the day a mind can be held whole
/// (its own phone conversation, its processes ended with the turn). Not reached today.
#[allow(dead_code)]
fn ask_held_mind(host: &yantrik_harness::Host, active: &str, name: &str, turn: Turn, asker: &Asker) -> String {
    let agent: AgentId = match host.ensure_main(active) {
        Ok(agent) => agent,
        Err(why) => {
            tracing::warn!(mind = %active, reason = %why, "a turn from a phone found no mind to answer");
            return format!("{name} is not attached right now.");
        }
    };
    let hold = match crate::agents::reaches::hold_remote(host, &agent) {
        Ok(hold) => hold,
        Err(why) => {
            tracing::error!(agent = %agent, reason = %why, "a turn from a phone was not sent");
            return "Nothing was sent: this mind could not be held to what a phone may ask.".to_string();
        }
    };
    let here = PhoneTurnGuard::enter(PhoneTurn {
        id: NEXT_TURN.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        agent: agent.0.clone(),
        mind: name.to_string(),
        provider: asker.provider.clone(),
        sender_id: asker.sender_id.clone(),
        conversation: asker.conversation.clone(),
    });
    let answer = match host.send_to_holding(&agent, turn, |token| hold.holds(token)) {
        Ok(answer) => answer,
        Err(why) => {
            tracing::warn!(agent = %agent, reason = %why, "a turn from a phone was not sent");
            return if why == crate::private_mode::PAUSED { why } else { format!("{name} could not take it right now.") };
        }
    };
    let said = gather(answer);
    drop(here);
    drop(hold);
    said
}

/// A phone turn's place in [`PHONE_TURNS`], taken out when the turn ends, with its codes.
struct PhoneTurnGuard(u64);

impl PhoneTurnGuard {
    fn enter(turn: PhoneTurn) -> PhoneTurnGuard {
        let id = turn.id;
        PHONE_TURNS.lock().unwrap_or_else(|e| e.into_inner()).push(turn);
        PhoneTurnGuard(id)
    }
}

impl Drop for PhoneTurnGuard {
    fn drop(&mut self) {
        // In the order `card_raised` takes them, held together, so no code is added for this turn
        // after it is gone.
        let mut turns = PHONE_TURNS.lock().unwrap_or_else(|e| e.into_inner());
        turns.retain(|t| t.id != self.0);
        PHONE_CARDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| c.turn != self.0);
    }
}

/// A card was raised for the first time. When the agent it was raised for is answering a turn
/// from the person's phone, the phone is told what it would do — in the app's own words, with
/// the arguments the grant is bound to — and, where the person may answer it there, given the
/// code that does. `irreversible` and `published` are the request's own reading of the app's
/// sentence, never the caller's. Sent on a thread of its own: this is called while a request is
/// being answered.
pub fn card_raised(card_id: &str, irreversible: bool, published: &str) {
    let Some(card) = crate::approvals::card(card_id) else { return };
    if card.verified.agent.is_empty() {
        return;
    }
    // The newest phone turn of that agent: the one it is answering now.
    let Some(turn) = PHONE_TURNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .rev()
        .find(|t| t.agent == card.verified.agent)
        .cloned()
    else {
        return;
    };
    let Some(outbox) = OUTBOX.get().cloned() else { return };
    // Every line drawn as one line of text: a name or an argument carrying a newline must not draw
    // a line of its own on the phone.
    let one_line = |s: &str| s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>();
    let mut what = format!("{} asks to run {}.{} ({}).", one_line(&turn.mind), one_line(&card.app), one_line(&card.action), one_line(&card.grade));
    if !published.trim().is_empty() {
        what.push_str(&format!("\n{}", one_line(published)));
    }
    if !card.target.trim().is_empty() {
        what.push_str(&format!("\n{}", one_line(card.target.trim())));
    }
    for row in &card.args {
        what.push_str(&format!("\n  {}", one_line(row)));
    }
    // A code only for a card the phone is shown whole: an argument cut short, or more of them
    // than fit, is a grant bound to what the person did not see.
    let whole = card.args.iter().all(|r| !r.ends_with('…') && !r.starts_with('…'));
    let said = if irreversible || never_from_a_phone(&card.app, &card.action) {
        format!("{what}\nThis one waits for you at the machine: it cannot be undone, or it runs commands as you.")
    } else if !whole {
        format!("{what}\nThis one waits for you at the machine: it is too long to show here in full.")
    } else if !approvals_on(&turn.provider) {
        format!(
            "{what}\nIt is waiting on the desktop's screen: approvals from {} are off, since {} can read what is sent here.",
            turn.provider, turn.provider
        )
    } else {
        let Some(code) = fresh_code() else {
            tracing::error!("no randomness for a phone code; the card waits on the desktop");
            return;
        };
        {
            // The turn still live, checked with the codes held: one that ended in between keeps no code.
            let turns = PHONE_TURNS.lock().unwrap_or_else(|e| e.into_inner());
            if !turns.iter().any(|t| t.id == turn.id) {
                return;
            }
            let mut cards = PHONE_CARDS.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            cards.retain(|c| c.until > now && c.card_id != card.id);
            if cards.len() >= MOST_CODES {
                tracing::warn!("too many codes waiting; this card waits on the desktop");
                return;
            }
            cards.push(PhoneCard {
                code: code.clone(),
                card_id: card.id.clone(),
                turn: turn.id,
                provider: turn.provider.clone(),
                sender_id: turn.sender_id.clone(),
                conversation: turn.conversation.id.clone(),
                until: now + CODE_LIFE,
            });
        }
        format!("{what}\nReply ALLOW {code} to let it, or DENY {code}. The code works once, for about two minutes.")
    };
    let _ = std::thread::Builder::new().name("phone-card".into()).spawn(move || {
        // Not kept: a card's code must never become context for a later turn.
        if let Err(why) = outbox.send_unkept(&turn.provider, &turn.conversation, &said) {
            tracing::warn!(provider = %turn.provider, reason = %why, "a card could not be sent to the phone");
        }
    });
}

/// `ALLOW 123456` or `DENY 123456`, from the person the card was sent to, answers it once. `None`
/// for a message that is not such an answer, which goes to the mind as usual.
fn card_answer(text: &str, asker: &Asker) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let [verb, code] = words.as_slice() else { return None };
    let allow = match verb.to_ascii_lowercase().as_str() {
        "allow" => true,
        "deny" => false,
        _ => return None,
    };
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let sender = (asker.provider.clone(), asker.sender_id.clone());
    let found = {
        let mut cards = PHONE_CARDS.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        cards.retain(|c| c.until > now);
        let at = cards.iter().position(|c| {
            c.code == *code && c.provider == asker.provider && c.sender_id == asker.sender_id && c.conversation == asker.conversation.id
        });
        at.map(|at| cards.remove(at))
    };
    let Some(card) = found else {
        // A wrong code, counted: after a few, every code waiting for this sender is burned, so
        // six digits cannot be walked.
        let mut wrong = WRONG.lock().unwrap_or_else(|e| e.into_inner());
        let count = match wrong.iter_mut().find(|(s, _)| s == &sender) {
            Some(entry) => {
                entry.1 += 1;
                entry.1
            }
            None => {
                wrong.push((sender.clone(), 1));
                1
            }
        };
        if count >= WRONG_CODES {
            PHONE_CARDS.lock().unwrap_or_else(|e| e.into_inner()).retain(|c| !(c.provider == sender.0 && c.sender_id == sender.1));
            wrong.retain(|(s, _)| s != &sender);
            tracing::warn!(provider = %sender.0, "too many wrong codes; every code waiting for this sender was burned");
            return Some("Too many wrong codes: every card waiting for you here is now answered only at the machine.".to_string());
        }
        return Some("No card is waiting for that code: it was answered, or it expired.".to_string());
    };
    WRONG.lock().unwrap_or_else(|e| e.into_inner()).retain(|(s, _)| s != &sender);
    let decided = if allow { crate::approvals::grant(&card.card_id) } else { crate::approvals::deny(&card.card_id) };
    Some(match decided {
        Ok(()) => {
            tracing::warn!(card = %card.card_id, provider = %card.provider, allow, "a card was answered from the phone");
            if allow { "Allowed.".to_string() } else { "Denied.".to_string() }
        }
        Err(why) => {
            tracing::info!(card = %card.card_id, reason = %why, "a phone's answer did not apply");
            "That card was already answered or has gone.".to_string()
        }
    })
}

/// Six digits from the operating system's randomness (`yantrik_harness::random_hex`, the one
/// source of the desktop's secrets), or `None` without it.
fn fresh_code() -> Option<String> {
    let hex = yantrik_harness::random_hex(4).ok()?;
    Some(format!("{:06}", u32::from_str_radix(&hex, 16).ok()? % 1_000_000))
}

/// The answer as one message, read the way the Lens reads a stream: the built-in companion's
/// `__DONE__`, `__REPLACE__` and run marks are conventions, not text. A failure is said as a
/// sentence; its detail goes to the log, not to a channel someone else may read.
fn gather(answer: yantrik_harness::Answer) -> String {
    let mut said = String::new();
    let mut replacing = false;
    while let Ok(chunk) = answer.recv() {
        match chunk {
            Chunk::Text(token) => {
                if token == "__DONE__" {
                    break;
                }
                if token.starts_with(crate::streaming::RUN_MARK) {
                    continue;
                }
                if let Some(rest) = token.strip_prefix("__REPLACE__") {
                    said.clear();
                    if rest.is_empty() {
                        replacing = true;
                    } else {
                        said.push_str(rest);
                    }
                    continue;
                }
                if replacing {
                    said = token;
                    replacing = false;
                } else {
                    said.push_str(&token);
                }
            }
            Chunk::Failed(why) => {
                if why == crate::private_mode::PAUSED {
                    return why;
                }
                tracing::warn!(reason = %why, "a turn from a phone failed");
                return "The answer did not come; it is on the desktop's log.".to_string();
            }
            Chunk::Event(_) => {}
        }
    }
    said
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(tokens: &[&str]) -> yantrik_harness::Answer {
        let (tx, rx) = std::sync::mpsc::channel();
        for t in tokens {
            tx.send(Chunk::Text((*t).to_string())).unwrap();
        }
        rx
    }

    fn asker(provider: &str, sender: &str) -> Asker {
        Asker {
            provider: provider.into(),
            sender_name: "Pranab".into(),
            sender_id: sender.into(),
            carries: vec!["text".into()],
            conversation: ConversationRef::direct(provider, sender),
        }
    }

    #[test]
    fn the_companions_stream_is_read_as_the_lens_reads_it() {
        assert_eq!(gather(stream(&["Hello", ", Pranab", "__DONE__", "ignored"])), "Hello, Pranab");
        assert_eq!(gather(stream(&["draft", "__REPLACE__", "final", "__DONE__"])), "final");
        assert_eq!(gather(stream(&["x", "__REPLACE__Private mode is on", "__DONE__"])), "Private mode is on");
        assert_eq!(gather(stream(&["done", "__RUN__:pi:main#3", "__DONE__"])), "done");
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Chunk::Failed("/home/pranab/.config/secret path failed".into())).unwrap();
        assert!(!gather(rx).contains("/home"), "no internal detail goes to a channel");
    }

    #[test]
    fn a_code_answers_only_its_card_only_from_its_person_and_only_once() {
        PHONE_CARDS.lock().unwrap().push(PhoneCard {
            code: "314159".into(),
            card_id: "appr-not-in-the-store".into(),
            turn: 0,
            provider: "signal".into(),
            sender_id: "+15550001".into(),
            conversation: "+15550001".into(),
            until: Instant::now() + CODE_LIFE,
        });
        assert!(card_answer("what is on my screen", &asker("signal", "+15550001")).is_none(), "not an answer: to the mind");
        assert!(card_answer("allow 31415", &asker("signal", "+15550001")).is_none(), "five digits is not a code");
        let stranger = card_answer("ALLOW 314159", &asker("signal", "+15559999")).unwrap();
        assert!(stranger.starts_with("No card is waiting"), "{stranger}");
        let other_channel = card_answer("allow 314159", &asker("telegram", "+15550001")).unwrap();
        assert!(other_channel.starts_with("No card is waiting"), "{other_channel}");
        let first = card_answer("allow 314159", &asker("signal", "+15550001")).unwrap();
        assert_eq!(first, "That card was already answered or has gone.", "the code was taken; the card itself is not in this test's store");
        let again = card_answer("allow 314159", &asker("signal", "+15550001")).unwrap();
        assert!(again.starts_with("No card is waiting"), "once: {again}");
    }

    #[test]
    fn an_expired_code_answers_nothing() {
        PHONE_CARDS.lock().unwrap().push(PhoneCard {
            code: "271828".into(),
            card_id: "appr-x".into(),
            turn: 0,
            provider: "signal".into(),
            sender_id: "+15550002".into(),
            conversation: "+15550002".into(),
            until: Instant::now() - Duration::from_secs(1),
        });
        let said = card_answer("allow 271828", &asker("signal", "+15550002")).unwrap();
        assert!(said.starts_with("No card is waiting"), "{said}");
    }

    #[test]
    fn wrong_codes_burn_every_code_waiting_for_their_sender() {
        PHONE_CARDS.lock().unwrap().push(PhoneCard {
            code: "161803".into(),
            card_id: "appr-y".into(),
            turn: 0,
            provider: "signal".into(),
            sender_id: "+15550003".into(),
            conversation: "+15550003".into(),
            until: Instant::now() + CODE_LIFE,
        });
        let me = asker("signal", "+15550003");
        for guess in ["000001", "000002", "000003", "000004"] {
            assert!(card_answer(&format!("allow {guess}"), &me).unwrap().starts_with("No card"));
        }
        let fifth = card_answer("allow 000005", &me).unwrap();
        assert!(fifth.starts_with("Too many wrong codes"), "{fifth}");
        assert!(card_answer("allow 161803", &me).unwrap().starts_with("No card"), "the right code is burned too");
    }

    #[test]
    fn what_runs_commands_as_the_person_never_takes_a_code() {
        assert!(never_from_a_phone("shell", "agent_run") && never_from_a_phone("Terminal", "type"));
        assert!(!never_from_a_phone("notes", "new_note") && !never_from_a_phone("shell", "open_app"));
        assert!(is_card_answer("ALLOW 123456") && is_card_answer("deny 000000"));
        assert!(!is_card_answer("allow me to explain") && !is_card_answer("allow 12345"));
    }

    #[test]
    fn a_code_is_six_digits_from_the_kernel() {
        let code = fresh_code().expect("urandom");
        assert!(code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()), "{code}");
    }

    #[test]
    fn only_end_to_end_channels_say_so() {
        assert_eq!(trust_of("signal"), "e2e");
        assert_eq!(trust_of("native"), "provider-readable", "the paired app is not end-to-end yet");
        assert_eq!(trust_of("telegram"), "provider-readable");
        assert_eq!(trust_of("whatsapp"), "provider-readable", "the Cloud API is Meta-readable");
        assert_eq!(trust_of("carrier-pigeon"), "provider-readable");
    }
}
