//! What the desk says, and in which colour: the whole of its wording, as plain functions.
//!
//! Nothing here draws. Each takes what the shell sent (`yantrik_ipc_contracts::mind_desk`) and
//! returns words, and each cleans what it was given again with the contract's own functions: the
//! shell has already, and the desk does not rely on it, because a Mind acting in Mind View can read
//! this window, and one end's mistake must not become the other's leak.
//!
//! The rules the wording keeps:
//! - A state is said in words and a colour, never a colour alone.
//! - Amber is for a Mind waiting for the person, red for one that failed, green for one that
//!   finished, teal for one at work, and the rest is quiet.
//! - The desk says the state, or the tool a Mind is running, in its own plain voice. It quotes
//!   nobody: what a Mind says it is doing is withheld from every other Mind, and this window is
//!   one every Mind can read. That line returns with per-Mind attribution (phase 4).
//! - A fact the shell does not have is said as unavailable, not filled in with a comfortable one.

use yantrik_ipc_contracts::mind_desk::{self as wire, Account, Counts, DeskMind, DeskTool, Evidence, MindState, Outcome};

/// The colour a state wears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Quiet,
    Mind,
    Amber,
    Red,
    Green,
}

impl Tone {
    /// The number `ui/desk.slint`'s `DeskTone` reads.
    pub fn index(self) -> i32 {
        match self {
            Tone::Quiet => 0,
            Tone::Mind => 1,
            Tone::Amber => 2,
            Tone::Red => 3,
            Tone::Green => 4,
        }
    }
}

/// The word for a state, as the header says it.
pub fn state_label(state: MindState) -> &'static str {
    match state {
        MindState::Thinking => "Thinking",
        MindState::RunningTool => "Running a tool",
        MindState::WaitingForYou => "Waiting for you",
        MindState::Idle => "Idle",
        MindState::Done => "Done",
        MindState::Failed => "Failed",
        MindState::HarnessGone => "Connection lost",
    }
}

/// The colour for a state. A lost connection is quiet: it is said in words, and red and amber
/// keep their own meanings.
pub fn tone_of(state: MindState) -> Tone {
    match state {
        MindState::Thinking | MindState::RunningTool => Tone::Mind,
        MindState::WaitingForYou => Tone::Amber,
        MindState::Failed => Tone::Red,
        MindState::Done => Tone::Green,
        MindState::Idle | MindState::HarnessGone => Tone::Quiet,
    }
}

/// The small line above the Mind's own: its name and its state, in capitals.
pub fn label(mind: &DeskMind) -> String {
    let name = wire::display_name(&mind.name);
    let name = if name.is_empty() { "Mind".to_string() } else { name };
    format!("{} · {}", name.to_uppercase(), state_label(mind.state).to_uppercase())
}

/// The big line: what the Mind is doing, in the desk's own plain words. The tool it is running is
/// the strip's to name, so this does not say it twice. Never anything the Mind said it is doing:
/// that is the person's, and this window is read by every Mind.
pub fn headline(mind: &DeskMind) -> String {
    match mind.state {
        MindState::RunningTool => "Working on it".to_string(),
        state => state_label(state).to_string(),
    }
}

/// "◆ Verified" (the shell saw it) or "◇ Reported" (the harness said so).
pub fn evidence_mark(evidence: Evidence) -> &'static str {
    match evidence {
        Evidence::Verified => "◆ Verified",
        Evidence::Reported => "◇ Reported",
    }
}

fn outcome_word(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Running => "Running",
        Outcome::Ok => "Ok",
        Outcome::Failed => "Failed",
        Outcome::Interrupted => "Interrupted",
        Outcome::Untold => "No outcome told",
    }
}

/// The tool strip's words: a heading, the tool's name, and who vouches for it and how it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strip {
    pub heading: String,
    pub tool: String,
    pub mark: String,
}

/// The tool the Mind is running, or the last it ran, by name only.
pub fn strip(mind: &DeskMind) -> Strip {
    let named = |t: &DeskTool| wire::tool_name(&t.name);
    match &mind.tool {
        Some(tool) => Strip {
            heading: if tool.outcome == Outcome::Running { "RUNNING TOOL" } else { "LAST TOOL" }.to_string(),
            // A name that is not a plain name was never shown: the call is counted, and said as
            // "a tool".
            tool: named(tool).unwrap_or_else(|| "a tool".to_string()),
            mark: format!("{} · {}", evidence_mark(tool.evidence), outcome_word(tool.outcome)),
        },
        None if mind.counts.calls > 0 => Strip { heading: "LAST TOOL".into(), tool: "a tool".into(), mark: String::new() },
        None => Strip { heading: "NO TOOL YET".into(), tool: String::new(), mark: String::new() },
    }
}

/// "4:32", or "1:04:32" past the hour.
pub fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The run's counts, and its age as of when the shell made the snapshot, never as a clock that
/// ticks: "4:32 elapsed at 14:32 · 12 calls · 1 failed · 1 interrupted". Nothing for a Mind that
/// has not run.
pub fn stats(mind: &DeskMind, as_of: &str) -> String {
    let Counts { calls, failed, interrupted } = mind.counts;
    let Some(secs) = mind.elapsed_secs else { return String::new() };
    let at = wire::one_line(as_of, 8);
    let age = if at.is_empty() { format!("{} elapsed", clock(secs)) } else { format!("{} elapsed at {at}", clock(secs)) };
    format!("{age} · {} · {failed} failed · {interrupted} interrupted", plural(calls, "call", "calls"))
}

/// The footer's safety line, from the kind of account the shell named for the Mind's harness:
/// the kernel's word at attach, and not a list of names.
///
/// "OS actions ask first" is the OS-mediated path only: it says nothing of what a harness running
/// as the person can do to the person's files by itself, which is why that line says so.
pub fn safety_line(account: Account) -> &'static str {
    match account {
        Account::Own => "Own account · Network checked · OS actions ask first",
        Account::Person => "Runs as you · OS actions ask first · Not isolated (planned)",
        Account::Unknown => "Account unavailable",
    }
}

/// The model the harness said it runs, and the mark that says whose word that is; or nothing at
/// all. "Unavailable" would claim a model call failed, and a Mind that is answering has no such
/// failure: a Mind on its own account may simply not name its model, and then the line is empty.
pub fn model_line(model: Option<&str>) -> (String, &'static str) {
    match model.and_then(wire::model_label) {
        Some(model) => (model, "◇ Harness-reported model"),
        None => (String::new(), ""),
    }
}

/// The apps open in Mind View, as a person reads them.
pub fn apps_line(apps: &[String]) -> String {
    let names: Vec<String> = apps.iter().map(|a| wire::display_name(a)).filter(|a| !a.is_empty()).collect();
    if names.is_empty() {
        "No apps open".to_string()
    } else {
        names.join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wire::StopStatus;

    fn mind(state: MindState) -> DeskMind {
        DeskMind {
            agent: "hermes:main".into(),
            name: "Hermes".into(),
            state,
            since: 0,
            stop: None,
            tool: None,
            counts: Counts::default(),
            elapsed_secs: None,
            model: None,
            account: Account::Person,
        }
    }

    const ALL: [MindState; 7] = [
        MindState::Thinking,
        MindState::RunningTool,
        MindState::WaitingForYou,
        MindState::Idle,
        MindState::Done,
        MindState::Failed,
        MindState::HarnessGone,
    ];

    #[test]
    fn every_state_has_its_word() {
        let said: Vec<&str> = ALL.iter().map(|s| state_label(*s)).collect();
        assert_eq!(said, ["Thinking", "Running a tool", "Waiting for you", "Idle", "Done", "Failed", "Connection lost"]);
    }

    #[test]
    fn amber_is_only_for_waiting_red_only_for_failed_and_green_only_for_done() {
        let tones: Vec<(MindState, Tone)> = ALL.iter().map(|s| (*s, tone_of(*s))).collect();
        for (state, tone) in &tones {
            assert_eq!(*tone == Tone::Amber, *state == MindState::WaitingForYou, "{state:?}");
            assert_eq!(*tone == Tone::Red, *state == MindState::Failed, "{state:?}");
            assert_eq!(*tone == Tone::Green, *state == MindState::Done, "{state:?}");
            assert_eq!(*tone == Tone::Mind, matches!(state, MindState::Thinking | MindState::RunningTool), "{state:?}");
        }
        // A lost connection is said in words and wears no alarm colour.
        assert_eq!(tone_of(MindState::HarnessGone), Tone::Quiet);
        // And the numbers the Slint global reads are distinct, so no two tones look alike.
        let mut seen: Vec<i32> = [Tone::Quiet, Tone::Mind, Tone::Amber, Tone::Red, Tone::Green].iter().map(|t| t.index()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 5);
    }

    #[test]
    fn the_label_names_the_mind_and_its_state() {
        assert_eq!(label(&mind(MindState::RunningTool)), "HERMES · RUNNING A TOOL");
        assert_eq!(label(&mind(MindState::HarnessGone)), "HERMES · CONNECTION LOST");
        let mut nameless = mind(MindState::Idle);
        nameless.name = String::new();
        assert_eq!(label(&nameless), "MIND · IDLE");
    }

    /// The desk says what the Mind is doing in its own voice, quotes nobody, and leaves the tool's
    /// name to the strip, whatever the tool is.
    #[test]
    fn the_desk_says_what_the_mind_is_doing_in_its_own_voice() {
        for state in ALL {
            let said = headline(&mind(state));
            let want = if state == MindState::RunningTool { "Working on it" } else { state_label(state) };
            assert_eq!(said, want, "{state:?}");
            assert!(!said.contains('“'), "nobody is quoted");
        }
        let mut running = mind(MindState::RunningTool);
        running.tool = Some(DeskTool { name: "describe".into(), evidence: Evidence::Reported, outcome: Outcome::Running });
        assert_eq!(headline(&running), "Working on it");
        assert!(!headline(&running).contains("describe"), "the strip names the tool");
    }

    #[test]
    fn a_stop_status_is_the_shells_word() {
        // The three things a snapshot can say of a stop are all the desk has to say of one.
        let all = [StopStatus::Asked, StopStatus::Stopped, StopStatus::NotStopped];
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn the_strip_names_the_tool_and_who_vouches_and_how_it_went() {
        let mut m = mind(MindState::RunningTool);
        m.tool = Some(DeskTool { name: "blender.add_object".into(), evidence: Evidence::Reported, outcome: Outcome::Running });
        assert_eq!(
            strip(&m),
            Strip { heading: "RUNNING TOOL".into(), tool: "blender.add_object".into(), mark: "◇ Reported · Running".into() }
        );
        m.tool = Some(DeskTool { name: "agent_run".into(), evidence: Evidence::Verified, outcome: Outcome::Failed });
        assert_eq!(strip(&m), Strip { heading: "LAST TOOL".into(), tool: "agent_run".into(), mark: "◆ Verified · Failed".into() });
        m.tool = Some(DeskTool { name: "agent_run".into(), evidence: Evidence::Reported, outcome: Outcome::Untold });
        assert_eq!(strip(&m).mark, "◇ Reported · No outcome told");
        // Nothing yet, and a call whose name was not fit to show.
        m.tool = None;
        assert_eq!(strip(&m).heading, "NO TOOL YET");
        m.counts.calls = 2;
        assert_eq!((strip(&m).heading.as_str(), strip(&m).tool.as_str()), ("LAST TOOL", "a tool"));
    }

    #[test]
    fn a_command_line_is_never_a_tool_name_on_the_strip() {
        let mut m = mind(MindState::RunningTool);
        m.tool = Some(DeskTool { name: "cat ~/.ssh/id_ed25519".into(), evidence: Evidence::Verified, outcome: Outcome::Running });
        let s = strip(&m);
        assert_eq!(s.tool, "a tool");
        assert!(!format!("{s:?}{}", headline(&m)).contains("ssh"));
    }

    #[test]
    fn the_runs_counts_and_its_age_as_of_the_snapshot() {
        let mut m = mind(MindState::RunningTool);
        assert_eq!(stats(&m, "14:32"), "", "nothing for a Mind that has not run");
        m.elapsed_secs = Some(272);
        m.counts = Counts { calls: 12, failed: 1, interrupted: 1 };
        assert_eq!(stats(&m, "14:32"), "4:32 elapsed at 14:32 · 12 calls · 1 failed · 1 interrupted");
        m.counts = Counts { calls: 1, failed: 0, interrupted: 0 };
        assert_eq!(stats(&m, "14:32"), "4:32 elapsed at 14:32 · 1 call · 0 failed · 0 interrupted");
        m.elapsed_secs = Some(3_872);
        assert_eq!(stats(&m, ""), "1:04:32 elapsed · 1 call · 0 failed · 0 interrupted");
    }

    /// The safety line is chosen from the account the shell named, which the kernel decided: the
    /// built-in Mind's account says what it says, a harness that runs as the person says that, and
    /// what is not known is not given either claim.
    #[test]
    fn the_safety_line_follows_the_account_and_never_flatters() {
        assert_eq!(safety_line(Account::Own), "Own account · Network checked · OS actions ask first");
        assert_eq!(safety_line(Account::Person), "Runs as you · OS actions ask first · Not isolated (planned)");
        assert_eq!(safety_line(Account::Unknown), "Account unavailable");
        for claim in ["Own account", "Network checked"] {
            assert!(!safety_line(Account::Person).contains(claim) && !safety_line(Account::Unknown).contains(claim));
        }
        assert!(!safety_line(Account::Own).contains("Not isolated") && !safety_line(Account::Own).contains("Runs as you"));
    }

    /// No Mind's name is written down to decide it: the same account gives the same line to a
    /// harness called anything.
    #[test]
    fn the_line_does_not_depend_on_the_minds_name() {
        let line = |name: &str, account| {
            let mut m = mind(MindState::Idle);
            m.name = name.into();
            m.account = account;
            safety_line(m.account)
        };
        for name in ["Hermes", "Pi", "OpenClaw", "DeepSeek", "Yantrik Mind", "Anything else"] {
            assert_eq!(line(name, Account::Person), "Runs as you · OS actions ask first · Not isolated (planned)");
            assert_eq!(line(name, Account::Own), "Own account · Network checked · OS actions ask first");
        }
    }

    #[test]
    fn the_model_is_the_harnesss_word_or_nothing() {
        assert_eq!(model_line(Some("deepseek-v4.1-flash via ollama-cloud")), ("deepseek-v4.1-flash via ollama-cloud".to_string(), "◇ Harness-reported model"));
        assert_eq!(model_line(None), (String::new(), ""));
        assert_eq!(model_line(Some("/home/me/private.gguf")), (String::new(), ""), "a path is not a model");
    }

    #[test]
    fn the_apps_are_read_as_names() {
        assert_eq!(apps_line(&[]), "No apps open");
        assert_eq!(apps_line(&["Blender".into(), "Terminal".into(), "Files".into()]), "Blender / Terminal / Files");
        assert_eq!(apps_line(&["/usr/bin/x".into(), "Notes".into()]), "Notes");
    }
}
