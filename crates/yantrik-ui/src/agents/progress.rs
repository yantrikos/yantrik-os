//! What an agent at work is doing, from the shell's own record of it (#234).
//!
//! A mind in the middle of a task could only say "still working" (#246), and a task that had
//! stalled said nothing at all, while every call it made, every answer and every refusal had
//! already passed through the shell and into the Agents store. This reads that record: the task,
//! how long it has run, what it did last, when it was last heard from, and whether it is stuck.
//! Nothing here asks the mind; nothing here needs its cooperation.

use super::model::{Agent, CallState, Card, Shown};

/// The same call failing this many times in a row, for the same reason, is a task going round.
pub const STUCK_REPEATS: usize = 3;
/// This long with nothing from the mind, while it waits on nobody, is a task gone quiet.
pub const STUCK_QUIET_SECS: u64 = 90;
/// How many of its last calls a status names.
pub const RECENT_CALLS: usize = 3;

/// One working agent, as the shell sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// What it was asked, as it was asked.
    pub task: String,
    pub elapsed_secs: u64,
    pub calls: usize,
    pub failed: usize,
    /// Seconds since anything at all happened to it.
    pub quiet_secs: u64,
    /// Its last few calls, oldest first, one line each.
    pub recent: Vec<String>,
    /// A call still running, when there is one.
    pub running: Option<String>,
    /// The person owes it an answer: an approval it asked for, a command of its at a prompt, or a
    /// question it asked.
    pub waiting_on_you: bool,
    /// The question it asked the person and still waits on, as it asked it, when there is one.
    pub question: Option<String>,
    /// Why the shell thinks it is stuck, in plain words, when it does.
    pub stuck: Option<String>,
    /// The same, as what kind of stuck: for words that must not quote the mind (a notification).
    pub stuck_kind: Option<Stuck>,
}

/// How a task is stuck, in the shell's own terms.
#[derive(Clone, Debug, PartialEq)]
pub enum Stuck {
    /// The same call failed this many times in a row, for the same reason.
    Repeating { call: String, times: usize },
    /// Nothing heard for this long, while it waits on nobody.
    Quiet { secs: u64 },
}

impl Stuck {
    /// Said in the desktop's own words, with nothing the mind or an app wrote.
    pub fn plain(&self) -> String {
        match self {
            Stuck::Repeating { call, times } => format!("`{call}` has failed the same way {times} times in a row."),
            Stuck::Quiet { secs } => format!("Nothing has been heard from it for {}.", span(*secs)),
        }
    }
}

/// What `agent` is doing right now, or None when it has no turn open.
pub fn of(agent: &Agent, now: u64) -> Option<Progress> {
    let turn = agent.open_turn()?;
    // As shown: a call holding words the person had erased reads with the marker.
    let shown: Vec<Shown<'_>> = turn.cards().map(Card::shown).collect();
    let cards: Vec<&Card> = shown.iter().map(|c| &**c).collect();
    let running = cards.iter().rev().find(|c| c.running()).map(|c| call_line(c));
    // An approval card, a command at a prompt (#182), or a question it asked (#25): either way
    // the person is the one being waited on, and the quiet is theirs, not the task's.
    let waiting_on_you = super::store::waits_on_person(agent);
    let question = agent.waiting_questions().last().map(|q| q.prompt.trim().to_string());
    let quiet_secs = now.saturating_sub(agent.touched);
    let repeating = going_round(&cards);
    // Quiet is only stuck when nothing explains it: not the person owing it an answer, and not
    // a command the shell itself is watching run (a build can be silent for minutes).
    let quiet = quiet_secs >= STUCK_QUIET_SECS && !waiting_on_you && running.is_none();
    let stuck_kind = match &repeating {
        Some((call, times, _)) => Some(Stuck::Repeating { call: call.clone(), times: *times }),
        None if quiet => Some(Stuck::Quiet { secs: quiet_secs }),
        None => None,
    };
    let stuck = match repeating {
        Some((_, _, said)) => Some(said),
        None => quiet.then(|| format!("nothing heard from it for {}", span(quiet_secs))),
    };
    Some(Progress {
        task: turn.prompt.trim().to_string(),
        elapsed_secs: now.saturating_sub(turn.started),
        calls: cards.len(),
        failed: cards.iter().filter(|c| c.state == CallState::Failed).count(),
        quiet_secs,
        recent: cards.iter().rev().take(RECENT_CALLS).rev().map(|c| step_line(c)).collect(),
        running,
        waiting_on_you,
        question,
        stuck,
        stuck_kind,
    })
}

/// The last calls failing the same way, over and over: the call, how many times, and "`editor.act`
/// failed 4 times in a row: Cannot open: No such file".
fn going_round(cards: &[&Card]) -> Option<(String, usize, String)> {
    let last = cards.last().filter(|c| c.state == CallState::Failed)?;
    let same = |c: &&&Card| c.state == CallState::Failed && c.name == last.name && c.summary == last.summary;
    let times: usize = cards
        .iter()
        .rev()
        .take_while(same)
        .map(|c| (c.repeats as usize).max(1))
        .sum();
    (times >= STUCK_REPEATS).then(|| {
        let why = last.summary.trim();
        let said = if why.is_empty() {
            format!("{} failed {times} times in a row", call_line(last))
        } else {
            format!("{} failed {times} times in a row: {why}", call_line(last))
        };
        (last.name.clone(), times, said)
    })
}

fn call_line(card: &Card) -> String {
    let target = card.target.trim();
    if target.is_empty() {
        card.name.clone()
    } else {
        format!("{} {target}", card.name)
    }
}

fn step_line(card: &Card) -> String {
    let how = match card.state {
        CallState::Running => "running",
        CallState::Ok => "done",
        CallState::Failed => "failed",
        CallState::Interrupted => "interrupted",
        CallState::Untold => "",
    };
    let why = card.summary.trim();
    match (how, why.is_empty() || card.state != CallState::Failed) {
        ("", _) => call_line(card),
        (how, true) => format!("{} — {how}", call_line(card)),
        (how, false) => format!("{} — {how}: {why}", call_line(card)),
    }
}

/// "45 s", "3 min", "1 h 20 min".
pub fn span(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min", secs / 60),
        _ => match (secs % 3600) / 60 {
            0 => format!("{} h", secs / 3600),
            m => format!("{} h {m} min", secs / 3600),
        },
    }
}

impl Progress {
    /// What the desktop says in the chat when a mind at work is spoken to: the shell's own
    /// account, so the person gets an answer now instead of "still working".
    pub fn told(&self, mind: &str) -> String {
        let mut out = format!(
            "{mind} is working on “{}” — {} in, {} call{} so far",
            brief(&self.task, 120),
            span(self.elapsed_secs),
            self.calls,
            if self.calls == 1 { "" } else { "s" },
        );
        if self.failed > 0 {
            out.push_str(&format!(", {} failed", self.failed));
        }
        out.push('.');
        if let Some(why) = &self.stuck {
            out.push_str(&format!("\nIt looks stuck: {why}."));
        } else if self.question.is_some() {
            // Never the question's words: this sentence is copied on — into the Lens, and to another
            // mind in a handover — where an erasure of them could not follow (docs/harness.md).
            out.push_str("\nIt is waiting for your answer to a question in its pane.");
        } else if self.waiting_on_you {
            out.push_str("\nIt is waiting for you to answer an approval card.");
        } else if let Some(call) = &self.running {
            out.push_str(&format!("\nRight now: {call}."));
        } else {
            out.push_str(&format!("\nLast heard from {} ago.", span(self.quiet_secs)));
        }
        if !self.recent.is_empty() {
            out.push_str("\nLatest steps:");
            for step in &self.recent {
                out.push_str(&format!("\n• {}", brief(step, 160)));
            }
        }
        out.push_str("\nAgents has the whole session. /stop stops it.");
        out
    }
}

/// `text` cut to `max` characters on a word, with an ellipsis when it was cut.
pub fn brief(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    let at = cut.rfind(' ').filter(|&i| i > max / 2).unwrap_or(cut.len());
    format!("{}…", cut[..at].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{AgentMeta, Item, Provenance, State, Turn, Usage};
    use yantrik_harness::AgentId;

    fn card(name: &str, state: CallState, summary: &str, at: u64) -> Card {
        let mut c = Card::new(&format!("c{at}"), name, "", serde_json::json!({}), Provenance::Reported, at);
        c.state = state;
        c.summary = summary.to_string();
        if state != CallState::Running {
            c.ended = Some(at + 1);
        }
        c
    }

    fn agent_with(cards: Vec<Card>, started: u64, touched: u64) -> Agent {
        Agent {
            meta: AgentMeta::new(AgentId::new("hermes", AgentId::MAIN), "Hermes"),
            state: State::RunningTool,
            since: started,
            status: String::new(),
            turns: vec![Turn {
                n: 1,
                prompt: "Build a small game in ~/Documents/Ridge-Runners".into(),
                started,
                ended: None,
                ok: None,
                lost: false, origin: Default::default(),
                items: cards.into_iter().map(Item::Card).collect(),
                events: true,
                trail_seq: 0,
            }],
            usage: Usage::default(),
            refused: 0,
            refusals: Vec::new(),
            refusals_shown: None,
            erasures: Vec::new(),
            approvals_asked: 0,
            approvals_answered: 0,
            pending_approvals: Vec::new(),
            job_waits: false,
            seq: 0,
            touched,
            next_turn: 2,
        }
    }

    /// The 23 September game, as the journal had it: the editor refused four times for the same
    /// reason. The shell saw every refusal; now it says so.
    #[test]
    fn the_same_refusal_over_and_over_is_stuck_and_says_why() {
        let refused = "Cannot open: No such file";
        let agent = agent_with(
            vec![
                card("os_act", CallState::Ok, "", 10),
                card("editor.act", CallState::Failed, refused, 20),
                card("editor.act", CallState::Failed, refused, 30),
                card("editor.act", CallState::Failed, refused, 40),
                card("editor.act", CallState::Failed, refused, 50),
            ],
            0,
            50,
        );
        let p = of(&agent, 60).expect("a turn is open");
        assert_eq!(p.stuck.as_deref(), Some("editor.act failed 4 times in a row: Cannot open: No such file"));
        assert_eq!(p.stuck_kind, Some(Stuck::Repeating { call: "editor.act".into(), times: 4 }));
        assert_eq!(
            p.stuck_kind.as_ref().unwrap().plain(),
            "`editor.act` has failed the same way 4 times in a row.",
            "the notification's words quote no refusal text"
        );
        assert_eq!((p.calls, p.failed, p.elapsed_secs), (5, 4, 60));
        let told = p.told("Hermes");
        assert!(told.starts_with("Hermes is working on “Build a small game"), "{told}");
        assert!(told.contains("It looks stuck: editor.act failed 4 times"), "{told}");
        assert!(told.contains("• editor.act — failed: Cannot open"), "{told}");
    }

    /// Two different failures are a task trying things, not one going round.
    #[test]
    fn different_failures_are_not_stuck() {
        let agent = agent_with(
            vec![
                card("editor.act", CallState::Failed, "Cannot open: No such file", 10),
                card("editor.act", CallState::Failed, "This appears to be a binary file", 20),
                card("web_go", CallState::Failed, "no browser this desktop can drive is open", 30),
            ],
            0,
            30,
        );
        assert_eq!(of(&agent, 40).unwrap().stuck, None);
    }

    /// Quiet is stuck only when nothing explains it: past the limit and waiting on nobody.
    #[test]
    fn quiet_is_stuck_unless_it_waits_on_the_person_or_a_command_runs() {
        let quiet = agent_with(vec![card("os_act", CallState::Ok, "", 10)], 0, 10);
        let p = of(&quiet, 10 + STUCK_QUIET_SECS).unwrap();
        assert_eq!(p.stuck.as_deref(), Some("nothing heard from it for 1 min"));
        assert_eq!(p.stuck_kind, Some(Stuck::Quiet { secs: STUCK_QUIET_SECS }));

        let mut asking = agent_with(vec![card("os_act", CallState::Ok, "", 10)], 0, 10);
        asking.pending_approvals.push("appr-7".into());
        let p = of(&asking, 10 + STUCK_QUIET_SECS).unwrap();
        assert_eq!(p.stuck, None);
        assert!(p.told("Hermes").contains("waiting for you to answer an approval card"));

        let mut prompting = agent_with(vec![card("os_act", CallState::Ok, "", 10)], 0, 10);
        prompting.job_waits = true;
        assert_eq!(of(&prompting, 10 + STUCK_QUIET_SECS).unwrap().stuck, None, "a command at a prompt waits on the person");

        let building = agent_with(vec![card("agent_run", CallState::Running, "", 10)], 0, 10);
        let p = of(&building, 10 + 600).unwrap();
        assert_eq!(p.stuck, None, "a long command is work, not silence");
        assert!(p.told("Hermes").contains("Right now: agent_run."));

        assert_eq!(of(&quiet, 10 + STUCK_QUIET_SECS - 1).unwrap().stuck, None);
    }

    /// A question to the person (#25) is the person's quiet, like an approval: a mind that asks
    /// "Keep or Erase?" and waits two minutes is waiting on them, not stuck. Once answered, it no
    /// longer excuses anything.
    #[test]
    fn a_question_waiting_on_the_person_is_not_stuck_until_it_is_answered() {
        let asked = |answer: &str| {
            let mut a = agent_with(vec![card("os_act", CallState::Ok, "", 10)], 0, 10);
            a.turns[0].items.push(Item::Question(crate::agents::model::Question {
                request: "q-1".into(),
                prompt: "Keep or Erase the three memories about the old address?".into(),
                options: vec!["Keep".into(), "Erase".into()],
                answer: answer.into(),
                closed: String::new(),
                asked: 10,
            }));
            a
        };

        let p = of(&asked(""), 10 + 120).unwrap();
        assert_eq!((p.stuck.as_deref(), p.stuck_kind.as_ref()), (None, None), "a question waiting is not stuck");
        assert!(p.waiting_on_you, "it waits on the person");
        assert_eq!(p.question.as_deref(), Some("Keep or Erase the three memories about the old address?"));
        assert!(p.told("Hermes").contains("waiting for your answer to a question in its pane"), "{}", p.told("Hermes"));
        assert!(!p.told("Hermes").contains("old address"), "the question's words are not copied on");

        let silent = agent_with(vec![card("os_act", CallState::Ok, "", 10)], 0, 10);
        let p = of(&silent, 10 + 120).unwrap();
        assert_eq!(p.stuck_kind, Some(Stuck::Quiet { secs: 120 }), "no question: quiet is stuck, as before");
        assert!(!p.waiting_on_you);

        let p = of(&asked("Keep"), 10 + 120).unwrap();
        assert_eq!(p.stuck_kind, Some(Stuck::Quiet { secs: 120 }), "an answered question no longer excuses the quiet");
        assert_eq!((p.waiting_on_you, p.question), (false, None));
    }

    #[test]
    fn an_agent_with_no_open_turn_has_no_progress() {
        let mut agent = agent_with(vec![], 0, 0);
        agent.turns[0].ended = Some(5);
        assert_eq!(of(&agent, 10), None);
    }

    #[test]
    fn spans_and_briefs_read_as_a_person_would_say_them() {
        assert_eq!(span(45), "45 s");
        assert_eq!(span(180), "3 min");
        assert_eq!(span(4800), "1 h 20 min");
        assert_eq!(span(7200), "2 h");
        assert_eq!(brief("one two three four", 9), "one two…");
        assert_eq!(brief("short", 9), "short");
    }
}
