//! Intents: the commands the Mind may raise by name, each with its wire contract
//! (design/app-framework-2026-10-21.md, §3.1). Every intent is a command; not every command is
//! an intent.
//!
//! The surface's existing dispatch stays the admission authority and runs first: the state rule,
//! the argument checks against the declared parameters, the ceiling and the mode, approvals and
//! grants, the revision guard. Then, on the UI thread, the intent's handler runs: the app's own
//! state rule, the agent rule, `parse` into a typed command, the command through `update`, and
//! the reply from the committed model.
//!
//! How the Mind is answered is the intent's [`Settle`]: on return, deferred, or explicitly, when
//! the invocation reaches its one terminal outcome. For the last, the handler leaves the rest
//! of its answer with `answer_later`, and the core settles it when the app calls `fx.settle`,
//! when the invocation's last job lands, panics, is replaced or is cancelled, or when the app
//! shuts down, whichever comes first, within the deadline.

use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;
use yantrik_surface::{answer_later, LocalRegistry};

use crate::app::App;
use crate::control::{agent_is_calling, Action, View};
use crate::cx::Core;
use crate::refused::Refused;

/// One accepted call of an intent, by number. Jobs and `then` chains started under it carry it,
/// so the core knows which invocation a late result or a panic belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvocationId(pub u64);

/// Where a command came from. Carried with it through jobs and continuations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A button, a key, a timer: the person at this window.
    Local,
    /// The control surface: the person through `yos`, or an agent.
    Surface { agent: bool },
}

/// A command being applied under an invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invocation {
    pub id: InvocationId,
    pub origin: Origin,
}

/// The arguments of one call, as the dispatch checked and filled them against the declared
/// parameters: present, of the declared type, defaults in place. These read them typed; they fail
/// only for what the declaration could not say, an empty required text.
pub struct Args<'a>(&'a Value);

impl<'a> Args<'a> {
    pub fn new(value: &'a Value) -> Args<'a> {
        Args(value)
    }

    /// A text argument, non-empty once trimmed.
    pub fn text(&self, name: &str) -> Result<&'a str, Refused> {
        match self.0.get(name).and_then(Value::as_str).map(str::trim) {
            Some(s) if !s.is_empty() => Ok(s),
            _ => Err(Refused::new(
                "missing_argument",
                format!("`{name}` is needed, and must not be empty."),
            )),
        }
    }

    pub fn text_opt(&self, name: &str) -> Option<&'a str> {
        self.0
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub fn integer(&self, name: &str) -> Result<i64, Refused> {
        self.0.get(name).and_then(Value::as_i64).ok_or_else(|| {
            Refused::new(
                "missing_argument",
                format!("`{name}` is needed, as a whole number."),
            )
        })
    }

    pub fn number(&self, name: &str) -> Result<f64, Refused> {
        self.0.get(name).and_then(Value::as_f64).ok_or_else(|| {
            Refused::new(
                "missing_argument",
                format!("`{name}` is needed, as a number."),
            )
        })
    }

    pub fn flag(&self, name: &str) -> Result<bool, Refused> {
        self.0.get(name).and_then(Value::as_bool).ok_or_else(|| {
            Refused::new(
                "missing_argument",
                format!("`{name}` is needed, as true or false."),
            )
        })
    }

    pub fn raw(&self) -> &'a Value {
        self.0
    }
}

/// When the Mind is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// When the command commits: the work is done.
    OnReturn,
    /// When the command commits, with `settled: false`: the work has only started.
    Deferred,
    /// When the invocation reaches its terminal outcome, within the deadline.
    Explicit { within: Duration },
}

/// Who may raise this intent.
pub enum Agents<A: App> {
    /// Anyone.
    Allowed,
    /// Not an agent, with this sentence.
    Refused(&'static str),
    /// An agent, if the rule allows this call.
    Rule(AgentRule<A>),
}

/// A rule read from the model that says whether an agent may raise an intent with these arguments.
pub type AgentRule<A> = Box<dyn Fn(&<A as App>::Model, &Args<'_>) -> Result<(), Refused>>;
/// A rule read from the model that every intent of the app is held to.
pub type StateRule<A> = Rc<dyn Fn(&<A as App>::Model) -> Result<(), Refused>>;
type Parse<A> = Box<dyn Fn(&Args<'_>) -> Result<<A as App>::Command, Refused>>;
pub(crate) type Reply<A> = Rc<dyn Fn(&<A as App>::Model) -> Value>;

/// A command with its wire contract.
pub struct Intent<A: App> {
    action: Action,
    parse: Parse<A>,
    reply: Reply<A>,
    agents: Agents<A>,
    settle: Settle,
}

impl<A: App> Intent<A> {
    /// `action` declares the name, the parameters, the grade and the description, as the surface
    /// publishes them; `parse` makes the command from the checked arguments.
    pub fn new(
        action: Action,
        parse: impl Fn(&Args<'_>) -> Result<A::Command, Refused> + 'static,
    ) -> Intent<A> {
        Intent {
            action,
            parse: Box::new(parse),
            reply: Rc::new(|_| Value::Object(Default::default())),
            agents: Agents::Allowed,
            settle: Settle::OnReturn,
        }
    }

    /// What the call answers, read from the committed model. Default: `{}`.
    pub fn reply(mut self, f: impl Fn(&A::Model) -> Value + 'static) -> Intent<A> {
        self.reply = Rc::new(f);
        self
    }

    pub fn agents(mut self, agents: Agents<A>) -> Intent<A> {
        self.agents = agents;
        self
    }

    pub fn settles(mut self, settle: Settle) -> Intent<A> {
        self.settle = settle;
        if matches!(settle, Settle::Deferred) {
            self.action = std::mem::replace(&mut self.action, Action::new("", "")).defers();
        }
        self
    }

    pub fn name(&self) -> &str {
        &self.action.name
    }
}

/// The app's intents, declared in `App::intents`.
pub struct Intents<A: App> {
    intents: Vec<Intent<A>>,
    state_rule: Option<StateRule<A>>,
    /// The `describe` keys kept in Private mode (`shown_in_private`). None unless listed.
    shown_in_private: Vec<&'static str>,
}

impl<A: App> Default for Intents<A> {
    fn default() -> Self {
        Intents {
            intents: Vec::new(),
            state_rule: None,
            shown_in_private: Vec::new(),
        }
    }
}

impl<A: App> Intents<A> {
    pub fn new() -> Intents<A> {
        Intents::default()
    }

    pub fn add(&mut self, intent: Intent<A>) {
        // The framework's own actions: an app's intent of that name would hide the hand-over.
        assert!(
            !RESERVED.contains(&intent.action.name.as_str()),
            "`{}` is the framework's action; an app cannot declare an intent of that name",
            intent.action.name
        );
        self.intents.push(intent);
    }

    /// The declarations, as the surface publishes them.
    pub fn specs(&self) -> Vec<Action> {
        self.intents.iter().map(|i| i.action.clone()).collect()
    }

    /// A rule about the app's own state that every call is held to before it is parsed, in the
    /// app's own words (the shell's is the locked desktop).
    pub fn state_rule(&mut self, rule: impl Fn(&A::Model) -> Result<(), Refused> + 'static) {
        self.state_rule = Some(Rc::new(rule));
    }

    /// Keys of this app's `describe` state that say nothing about the person, kept in its answer
    /// while they are in Private mode (#924, `control::private_mode`). Everything else of the
    /// app's state is left out, so the default is nothing: list a key only when its whole value
    /// is not personal (a unit, a CPU figure), never a title, a name, a path or a place. A key
    /// the private view writes itself (`open`, `windows_open`, `private`) is logged and not kept.
    pub fn shown_in_private(&mut self, keys: &[&'static str]) {
        for key in keys {
            if !self.shown_in_private.contains(key) {
                self.shown_in_private.push(key);
            }
        }
    }

    /// The keys [`Intents::shown_in_private`] listed, in that order.
    pub fn shown_in_private_keys(&self) -> &[&'static str] {
        &self.shown_in_private
    }

    pub fn len(&self) -> usize {
        self.intents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.intents.iter().map(|i| i.action.name.clone()).collect()
    }
}

/// The surface's registry for `table`, with every intent's handler bound to `core`. Serving it is
/// the runner's (`control::App::from_registry`); a test acts on it directly.
/// The most an intent may keep the Mind waiting for an explicit settlement, whatever it declares.
pub const MAX_SETTLE_WAIT: Duration = Duration::from_secs(30);

/// Action names an app may not declare: the framework registers these itself.
pub const RESERVED: [&str; 2] = ["launch", "present"];

/// A deep link's intent, admitted: its name is one the app declares; its arguments are given
/// the declared defaults, checked and typed by the declared parameters, as the surface does for
/// a socket call; and, when the launch came by hand-over, it is graded standard or below. Any
/// process of the uid may call `launch`, which is graded safe, so an intent that writes
/// something lasting (#48) cannot come in that way; the person's own command line may name any
/// intent, it is their input. What comes back is what `Event::Launch` carries.
pub fn admit(
    specs: &[Action],
    intent: crate::launch::LaunchIntent,
    handed_over: bool,
) -> Result<crate::launch::LaunchIntent, String> {
    use yantrik_ipc_transport::gate::grade;
    let spec = specs
        .iter()
        .find(|s| s.name == intent.name)
        .ok_or_else(|| format!("this app has no intent `{}`", intent.name))?;
    let args = yantrik_surface::with_defaults(spec, &intent.args);
    yantrik_surface::check_arguments(spec, &args)?;
    let args = yantrik_surface::as_declared(spec, &args).into_owned();
    // A grade off the ladder is refused, never read as "below standard".
    let level = grade(spec.permission)
        .ok_or_else(|| format!("`{}` is graded `{}`, which is not a level this OS defines", spec.name, spec.permission))?;
    if handed_over && level > grade("standard").unwrap_or(1) {
        return Err(format!(
            "a launch handed over may raise an intent graded safe or standard; `{}` is {}",
            spec.name, spec.permission
        ));
    }
    Ok(crate::launch::LaunchIntent { name: intent.name, args })
}

pub fn registry<A: App>(core: &Rc<Core<A>>, table: Intents<A>) -> LocalRegistry {
    let mut registry = LocalRegistry::new(A::ID);
    let specs = table.specs();
    // An intent an agent may never raise is refused by the dispatch before it decides anything,
    // so a call that cannot run is never shown as an approval card and spends no grant. The
    // model-reading rules (`Agents::Rule`, the app's state rule) cannot run there: they are the
    // UI thread's, so they run in the handler, after a grant was spent, and fail closed.
    let never_for_agents: std::collections::HashMap<String, &'static str> = table
        .intents
        .iter()
        .filter_map(|i| match i.agents {
            Agents::Refused(sentence) => Some((i.action.name.clone(), sentence)),
            _ => None,
        })
        .collect();
    registry.set_state_rule(Box::new(move |action: &str| {
        match never_for_agents.get(action) {
            Some(sentence) if agent_is_calling() => Err((*sentence).to_string()),
            _ => Ok(()),
        }
    }));
    let weak = Rc::downgrade(core);
    let describe = weak.clone();
    registry.set_describe(Box::new(move || match describe.upgrade() {
        Some(core) => core.read(A::describe),
        None => View::new(format!("{} (closing)", A::ID)),
    }));
    let shown = &table.shown_in_private;
    crate::control::private_mode::wire(&mut registry, weak.clone(), shown, core.paths());
    for intent in table.intents {
        let Intent {
            action,
            parse,
            reply,
            agents,
            settle,
        } = intent;
        let core = weak.clone();
        let state_rule = table.state_rule.clone();
        let handler = move |args: &Value| -> Result<Value, String> {
            let core = core.upgrade().ok_or("the app is closing")?;
            let args = Args(args);
            if let Some(rule) = &state_rule {
                core.read(|m| rule(m)).map_err(|r| r.sentence)?;
            }
            let agent = agent_is_calling();
            if agent {
                match &agents {
                    Agents::Allowed => {}
                    Agents::Refused(sentence) => return Err(sentence.to_string()),
                    Agents::Rule(rule) => core.read(|m| rule(m, &args)).map_err(|r| r.sentence)?,
                }
            }
            let command = parse(&args).map_err(|r| r.sentence)?;
            let origin = Origin::Surface { agent };
            match settle {
                Settle::OnReturn | Settle::Deferred => {
                    core.run_invocation(origin, command, None)
                        .map_err(|r| r.sentence)?;
                    Ok(core.read(|m| reply(m)))
                }
                Settle::Explicit { within } => {
                    // The wait holds an RPC worker; an app's deadline is capped so two hung
                    // jobs cannot pin both workers until shutdown.
                    let within = within.min(MAX_SETTLE_WAIT);
                    let (tx, rx) = mpsc::sync_channel(1);
                    let id = core.open_settlement(tx, reply.clone());
                    if let Err(refused) = core.run_invocation(origin, command, Some(id)) {
                        core.drop_settlement(id);
                        return Err(refused.sentence);
                    }
                    let wait = move || {
                        rx.recv_timeout(within).map_err(|_| {
                            format!("the app did not settle this within {} s", within.as_secs())
                        })?
                    };
                    // The rest of the answer, off the UI thread. Handed back only when no
                    // dispatch is running (a handler called directly), and then run here.
                    match answer_later(wait) {
                        Ok(()) => Ok(Value::Null),
                        Err(wait) => wait(),
                    }
                }
            }
        };
        registry.add(action, Box::new(handler));
    }
    registry.add(crate::launch::action(), Box::new(launch_handler(weak, specs)));
    registry
}

/// The `launch` action: a second copy's launch, applied once and answered as `present` is.
///
/// Request ids seen are remembered (the last [`crate::launch::REMEMBERED`]), so a retry of a
/// hand-over that was answered late opens nothing twice; it still brings the window forward. A
/// launch for another profile is refused: that is another instance's. An agent's call is refused
/// as `present`'s is: the desktop's hand-over is not a way to open files past the intents.
fn launch_handler<A: App>(
    core: std::rc::Weak<Core<A>>,
    specs: Vec<Action>,
) -> impl Fn(&Value) -> Result<Value, String> + 'static {
    let seen: std::cell::RefCell<std::collections::VecDeque<String>> =
        std::cell::RefCell::new(std::collections::VecDeque::new());
    move |args: &Value| {
        if agent_is_calling() {
            return Err("launch is the desktop's own hand-over between two launches of an app; an \
                        agent drives the app through its other actions"
                .into());
        }
        let core = core.upgrade().ok_or("the app is closing")?;
        // An app already ending (a backstop passed, an exit asked for) takes no launch: nothing
        // is applied to it, and the second copy is told, as `present` would tell it.
        if core.exit_reason().is_some() {
            return Err("the app is ending, so it takes no launch; launch it again once it has".into());
        }
        let mut launch = crate::launch::Launch::from_value(&args["launch"])?;
        let profile = core.paths().profile().to_string();
        if launch.profile != profile {
            return Err(format!(
                "that launch is for profile `{}`; this instance runs profile `{profile}`",
                launch.profile
            ));
        }
        if let Some(intent) = launch.intent.take() {
            launch.intent = Some(admit(&specs, intent, true)?);
        }
        let fresh = {
            let mut seen = seen.borrow_mut();
            if seen.contains(&launch.request) {
                false
            } else {
                if seen.len() >= crate::launch::REMEMBERED {
                    seen.pop_front();
                }
                seen.push_back(launch.request.clone());
                true
            }
        };
        if fresh {
            core.event(crate::app::Event::Launch(launch));
        } else {
            tracing::info!(app = A::ID, "a launch handed over twice; applied once");
        }
        // Shown through this core, as the runner's presenter does: the window open brought
        // forward, or the first opened again during a linger, and `Event::Presented` raised.
        crate::present::answer_with(Some(&|| core.present()), args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Param;
    use serde_json::json;

    /// A deep link's intent is admitted by the table: the name must be declared, the arguments
    /// are typed by the declared parameters, and by hand-over only safe and standard pass.
    #[test]
    fn a_deep_links_intent_is_admitted_by_the_table() {
        let specs = vec![
            Action::new("find", "Find a note").risk("safe").arg(Param::integer("n")),
            Action::new("set_step", "Set the step").risk("sensitive").arg(Param::integer("step")),
        ];
        let intent = |name: &str, args: Value| crate::launch::LaunchIntent { name: name.into(), args };
        assert!(admit(&specs, intent("nope", json!({})), true).unwrap_err().contains("no intent `nope`"));
        let typed = admit(&specs, intent("find", json!({ "n": "3" })), true).unwrap();
        assert_eq!(typed.args, json!({ "n": 3 }), "text typed as the integer it is");
        assert!(admit(&specs, intent("find", json!({ "n": "three" })), true).is_err());
        let e = admit(&specs, intent("set_step", json!({ "step": "5" })), true).unwrap_err();
        assert!(e.contains("safe or standard") && e.contains("sensitive"), "{e}");
        assert!(admit(&specs, intent("set_step", json!({ "step": "5" })), false).is_ok(), "the person's own command line");
        let odd = vec![Action::new("odd", "Off the ladder").risk("whatever")];
        assert!(admit(&odd, intent("odd", json!({})), false).unwrap_err().contains("not a level"));
        let defaults = vec![Action::new("list", "List").risk("safe").arg(Param::integer("limit").optional().default(10))];
        assert_eq!(admit(&defaults, intent("list", json!({})), true).unwrap().args, json!({ "limit": 10 }));
    }


    #[test]
    fn args_read_what_the_dispatch_filled_and_refuse_an_empty_text() {
        let v =
            serde_json::json!({"name": "  Dallas ", "n": 3, "x": 1.5, "on": true, "blank": "  "});
        let a = Args::new(&v);
        assert_eq!(a.text("name").unwrap(), "Dallas");
        assert_eq!(a.integer("n").unwrap(), 3);
        assert_eq!(a.number("x").unwrap(), 1.5);
        assert!(a.flag("on").unwrap());
        assert_eq!(a.text_opt("blank"), None);
        let e = a.text("blank").unwrap_err();
        assert_eq!(e.kind, "missing_argument");
        assert!(a.integer("name").is_err());
    }
}
