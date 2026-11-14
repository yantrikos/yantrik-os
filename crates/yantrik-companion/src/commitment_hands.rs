//! The one door the commitment executor's tool calls go through (#1184), and what a failure at
//! it means.
//!
//! Every call is `ToolRegistry::execute`, so the permission, taint and reach gates run and the
//! call is written to the audit. A refusal or an error comes back as a sentence meant for a
//! model; it is sorted here into "try again later" and "final", and said in the person's words.
//! The raw sentence is never kept or shown.
//!
//! The whole preparation has a time budget: it runs in the think loop, beside the chat worker, so
//! it stops starting new calls once the budget is spent and picks up later.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_companion_core::tools::{ToolContext, ToolRegistry};

/// A step that did not happen, and whether it is worth trying again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// Not now: an app still starting, Email's window briefly down, out of time. The person's words.
    Later(String),
    /// Not at all, as things stand. The person's words.
    Final(String),
}

impl Failed {
    pub fn words(&self) -> &str {
        match self {
            Failed::Later(w) | Failed::Final(w) => w,
        }
    }

    pub fn is_later(&self) -> bool {
        matches!(self, Failed::Later(_))
    }
}

/// The tools a preparation may use, and the budget they share.
pub struct Hands<'a> {
    registry: &'a ToolRegistry,
    ctx: &'a ToolContext<'a>,
    started: Instant,
    budget: Duration,
}

/// What an app is called to the person.
pub fn app_name(app: &str) -> &str {
    match app {
        "presentation" => "Presentation",
        "spreadsheet" => "Spreadsheet",
        "document-editor" => "Document Editor",
        "email" => "Email",
        other => other,
    }
}

impl<'a> Hands<'a> {
    pub fn new(registry: &'a ToolRegistry, ctx: &'a ToolContext<'a>, budget: Duration) -> Self {
        Self { registry, ctx, started: Instant::now(), budget }
    }

    pub fn out_of_time(&self) -> bool {
        self.started.elapsed() >= self.budget
    }

    /// One tool call. Nothing of what it said is kept.
    pub fn call(&mut self, tool: &str, args: Value) -> String {
        self.registry.execute(self.ctx, tool, &args)
    }

    /// `app_action`: what the app answered, parsed, when it took the action; otherwise why not, in
    /// the person's words.
    pub fn act(&mut self, app: &str, action: &str, args: Value) -> Result<Value, Failed> {
        self.act_through(app, action, args, false)
    }

    /// `act` through the door a model turn cannot use (`ToolRegistry::execute_housekeeping`): for
    /// the companion's own upkeep, such as clearing the draft of a promise that closed.
    pub fn act_unlisted(&mut self, app: &str, action: &str, args: Value) -> Result<Value, Failed> {
        self.act_through(app, action, args, true)
    }

    fn act_through(&mut self, app: &str, action: &str, args: Value, unlisted: bool) -> Result<Value, Failed> {
        if self.out_of_time() {
            return Err(Failed::Later("I ran out of time and will pick this up again".into()));
        }
        let call = json!({ "app": app, "action": action, "args": args });
        let said = if unlisted {
            self.registry.execute_housekeeping(self.ctx, "app_action", &call)
        } else {
            self.call("app_action", call)
        };
        let prefix = format!("{app}.{action} \u{2192}");
        match said.strip_prefix(&prefix) {
            Some(answer) => Ok(serde_json::from_str(answer.trim()).unwrap_or(Value::Null)),
            None if said.starts_with(&format!("{app}.{action} accepted")) => Ok(Value::Null),
            None => Err(classify(app, &said)),
        }
    }
}

/// What a refusing or failing answer means, from its sentence, in the person's words.
pub fn classify(app: &str, said: &str) -> Failed {
    let lower = said.to_lowercase();
    let name = app_name(app);
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if has(&["unsent message of your own"]) {
        return Failed::Later("you have an email of your own that isn't sent yet, so I left mine for later".into());
    }
    if has(&["is starting", "send it again", "still starting", "may still be starting", "try again", "timed out", "no window of it was listed"])
        || (has(&["mind view"]) && has(&["not available", "could not be started", "is down", "not running"]))
    {
        return Failed::Later(format!("{name} wasn't ready, so I'll try again"));
    }
    // Email's own refusals say what they are: nothing here is a gate that would ask the person.
    if has(&["file to attach", "only files in your home", "protected place", "no home folder"]) || lower.contains("cannot_attach") {
        return Failed::Final(format!("{name} wouldn't take that file, so I didn't attach it"));
    }
    if has(&["your own saved draft could not be read"]) {
        return Failed::Final(format!("{name} couldn't read the draft you saved, so I left it alone and didn't make mine"));
    }
    if has(&["could not be read, so it was not replaced", "could not be kept"]) {
        return Failed::Final(format!("{name} couldn't keep the draft, so I didn't make it"));
    }
    if lower.starts_with("refused") {
        return Failed::Final(format!("I'm not allowed to do that in {name} while you're away"));
    }
    Failed::Final(format!("{name} couldn't do that"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_start_is_for_later_and_is_said_in_the_persons_words() {
        let f = classify("email", "Email is starting; nothing was done \u{2014} send it again in a moment.");
        assert_eq!(f, Failed::Later("Email wasn't ready, so I'll try again".into()));
        assert!(!f.words().contains("nothing was done"));
    }

    #[test]
    fn mind_view_being_down_is_for_later() {
        assert!(classify("shell", "`presentation` was not started: Mind View is not available (no labwc). It was not opened on the person's desktop.").is_later());
    }

    #[test]
    fn a_refusal_is_final_and_does_not_leak_the_gate_sentence() {
        let f = classify("email", "Refused: `app_action` reaches the internet and research is off.");
        assert_eq!(f, Failed::Final("I'm not allowed to do that in Email while you're away".into()));
    }

    #[test]
    fn email_own_refusals_get_their_own_words_not_the_reach_sentence() {
        let f = classify("email", "The file to attach is not there.");
        assert_eq!(f, Failed::Final("Email wouldn't take that file, so I didn't attach it".into()));
        let f = classify("email", "The draft Yantrik prepared earlier could not be read, so it was not replaced: x");
        assert_eq!(f, Failed::Final("Email couldn't keep the draft, so I didn't make it".into()));
        assert!(!f.words().contains("while you're away"));
    }

    #[test]
    fn an_unreadable_saved_draft_of_the_persons_has_its_own_sentence() {
        let f = classify("email", "Your own saved draft could not be read, so a draft from Yantrik was not started: bad json");
        assert_eq!(f, Failed::Final("Email couldn't read the draft you saved, so I left it alone and didn't make mine".into()));
        assert!(!f.words().contains("while you're away"));
    }

    #[test]
    fn anything_else_is_final() {
        assert_eq!(classify("presentation", "no such action"), Failed::Final("Presentation couldn't do that".into()));
    }
}
