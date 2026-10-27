//! The agent rail, once (design/app-framework-2026-10-21.md, §5).
//!
//! Every app used to carry its own copy: a `refresh_agent_rail`, a four-second timer, a
//! forty-line ask handler with a thread and a weak window. Here it is one feature with its own
//! model ([`Rail`]), commands ([`RailCommand`]), view adapter ([`rail_window!`]) and refresh.
//! An app supplies `impl Source for Model` (what the context is, which suggestions to offer at
//! this reach, the prompt a suggestion asks, and the proposal an answer becomes), keeps a
//! [`Rail`] in its model, routes [`RailCommand`] through its own command enum (`App::rail`), and
//! has it: the rail refreshes when the model changes (debounced) and when the Mind's reach does,
//! asks the Mind from a job, shows the answer as a proposal card, and applies a proposal through
//! the app's own intents **as an agent**, so the authorised intent path (§3.2) and the app's
//! agent rules hold.
//!
//! The rail redraws only when what it draws changed: a refresh that found nothing new names no
//! part, and its lists are set through [`crate::rows::set`], since a new model is a new frame
//! even when every row in it is the same. There is no fallback tick: everything the rail says comes from the model,
//! which only a command changes, and every command that changes it refreshes the rail. An app
//! whose context says something about time ("updated 3 min ago") runs its own timer for when
//! those words change, and the refresh follows from it (Weather's `UPDATED_TICK`). The
//! four-second tick the rail had cost a frame every four seconds in every app with a rail on
//! screen, with nothing on it changed: on 520's software rasteriser, Weather at rest spent about
//! half of 42–61 ticks per 30 s drawing it (21 Oct).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use yantrik_ipc_transport::gate::Authority;
use yantrik_ipc_transport::privacy;
use yantrik_mind::{AskError, Mind, Reach};

use crate::app::App;
use crate::cx::Cx;
use crate::effects::Effects;
use crate::jobs::{Concurrency, JobKey, Scope};
use crate::refused::Refused;
use crate::timers::TimerId;

/// The part of the model the rail is; `render` redraws it through [`RailView::render_rail`].
pub const PART: &str = "rail";
/// The agent token a proposal is applied under: an agent's call, never the person's own.
pub const AGENT_TOKEN: &str = "rail";
pub use crate::cx::is_person_applying;
/// The refresh after a model change, debounced so a burst of commands is one refresh.
pub const DEBOUNCE: TimerId = TimerId("rail.debounce");
pub const DEBOUNCE_FOR: Duration = Duration::from_millis(300);
/// The sentence for a second press while a card is applied.
const APPLYING: &str = "A change is being applied; a moment.";

/// One row of what the rail knows: where the person is, in the app's own words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextItem {
    pub id: String,
    pub label: String,
    /// The second line: what this is, or why it is here.
    pub detail: String,
    /// Where it came from: `linked`, `memory`, `file`, `calendar`, `mail`. Picks the icon too.
    pub source: String,
}

/// One thing the rail offers to do.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suggestion {
    pub id: String,
    pub label: String,
    /// Why this is being offered.
    pub detail: String,
    /// A keyword the kit resolves: `spark`, `summary`, `reply`.
    pub icon: String,
    /// Pressing this changes something, so it is proposed rather than done.
    pub proposes: bool,
}

/// What a suggestion asks the Mind, and how the card looks while the answer is awaited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub title: String,
    pub source: String,
}

/// What applying a proposal does: one of the app's own intents, with these arguments, run as
/// an agent through the authorised path.
#[derive(Debug, Clone, PartialEq)]
pub struct Apply {
    pub intent: String,
    pub args: Value,
}

/// The card: an answer to read, or a change to apply.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Proposal {
    pub title: String,
    pub body: String,
    /// Where it came from. A card with no source reads as a machine asserting something.
    pub source: String,
    /// What changes if this is applied. Empty: nothing, this is only an answer.
    pub impact: String,
    pub destructive: bool,
    /// The verb on the button; empty falls back to the kit's.
    pub verb: String,
    /// What applying does. `None` for an answer to read.
    pub applies: Option<Apply>,
}

impl Proposal {
    /// The card the rail shows when the Mind did not answer.
    fn unanswered(why: &AskError) -> Proposal {
        Proposal {
            title: "The companion did not answer".into(),
            body: why.to_string(),
            verb: "Close".into(),
            ..Default::default()
        }
    }
}

/// What an app tells the rail about itself: four methods, in place of a copy of the rail.
pub trait Source {
    /// Where the person is, from the model.
    fn context(&self) -> Vec<ContextItem>;
    /// What to offer, given whether the Mind can answer. Nothing that asks the Mind is offered
    /// when it cannot; the rail says why instead.
    fn suggestions(&self, reach: Reach) -> Vec<Suggestion>;
    /// The prompt a suggestion asks, or `None` when the suggestion is not on offer now.
    fn prompt(&self, suggestion: &str) -> Option<Prompt>;
    /// The Mind's answer to `suggestion`, as a card: an answer to read, or a change with the
    /// intent that applies it.
    fn proposal(&self, suggestion: &str, answer: &str) -> Proposal;
}

/// Where the model keeps its [`Rail`].
pub trait HasRail {
    fn rail(&self) -> &Rail;
    fn rail_mut(&mut self) -> &mut Rail;
}

/// What the rail is busy with: asking the Mind about a suggestion, or applying the card shown.
/// An enum, not a sentinel string, so a suggestion whose id is `apply` cannot be mistaken for
/// an apply in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Working {
    Asking(String),
    Applying,
}

/// The rail's state, kept in the app's model and changed only by [`update`].
#[derive(Debug, Clone)]
pub struct Rail {
    mind: Arc<dyn Mind>,
    reach: Reach,
    context: Vec<ContextItem>,
    suggestions: Vec<Suggestion>,
    unavailable: &'static str,
    proposal: Option<Proposal>,
    working: Option<Working>,
}

impl Default for Rail {
    /// The live Mind. An app's `model` takes `boot.mind` instead, so a test's fake reaches it.
    fn default() -> Rail {
        Rail::new(yantrik_mind::live())
    }
}

impl Rail {
    pub fn new(mind: Arc<dyn Mind>) -> Rail {
        Rail {
            mind,
            reach: Reach::NoShell,
            context: Vec::new(),
            suggestions: Vec::new(),
            unavailable: "",
            proposal: None,
            working: None,
        }
    }

    pub fn context(&self) -> &[ContextItem] {
        &self.context
    }

    pub fn suggestions(&self) -> &[Suggestion] {
        &self.suggestions
    }

    /// The sentence shown in place of the suggestions when the Mind cannot answer; empty when
    /// it can.
    pub fn unavailable(&self) -> &'static str {
        self.unavailable
    }

    pub fn proposal(&self) -> Option<&Proposal> {
        self.proposal.as_ref()
    }

    /// The suggestion being asked, if one is.
    /// The suggestion being asked, if one is; `None` while a card is applied.
    pub fn working(&self) -> Option<&str> {
        match &self.working {
            Some(Working::Asking(id)) => Some(id),
            _ => None,
        }
    }

    /// A card is being applied: the authority is being read, or the intent runs.
    pub fn is_applying(&self) -> bool {
        matches!(self.working, Some(Working::Applying))
    }

    pub fn is_working(&self) -> bool {
        self.working.is_some()
    }

    pub fn reach(&self) -> Reach {
        self.reach
    }

    /// Everything [`RailView::render_rail`] reads, to tell whether a command changed any of it.
    fn drawn(&self) -> Drawn {
        Drawn {
            context: self.context.clone(),
            suggestions: self.suggestions.clone(),
            unavailable: self.unavailable,
            proposal: self.proposal.clone(),
            working: self.working.clone(),
        }
    }
}

/// What the rail draws, as [`Rail::drawn`] copies it.
#[derive(PartialEq)]
struct Drawn {
    context: Vec<ContextItem>,
    suggestions: Vec<Suggestion>,
    unavailable: &'static str,
    proposal: Option<Proposal>,
    working: Option<Working>,
}

/// What the rail does, routed through the app's own command enum by `App::rail`.
#[derive(Debug, Clone)]
pub enum RailCommand {
    /// Recompute the context and the suggestions from the model.
    Refresh,
    /// The Mind's reach changed (the housekeeping thread heard it).
    Reach(Reach),
    /// A suggestion pressed: ask the Mind from a job.
    Activate(String),
    /// The Mind answered (or did not) a suggestion's prompt.
    Answered {
        suggestion: String,
        answer: Result<String, AskError>,
    },
    /// The card closed.
    Dismiss,
    /// The card's button pressed: read the machine's authority (IO, in a job), then apply.
    Apply,
    /// The authority read; the proposal's intent runs as an agent through the admitted path.
    Authorised { apply: Apply, authority: Authority },
    /// What came of applying.
    Applied(Result<(), String>),
}

/// The rail's `update`: the app's `update` hands it every `Command::Rail(c)`. The rail's part
/// redraws when what it draws changed, and only then (the module's documentation says why).
pub fn update<A: App>(
    m: &mut A::Model,
    command: RailCommand,
    fx: &mut Effects<A>,
) -> Result<(), Refused>
where
    A::Model: Source + HasRail,
{
    let before = m.rail().drawn();
    apply::<A>(m, command, fx)?;
    if m.rail().drawn() == before {
        fx.unchanged();
    } else {
        fx.changed(PART);
    }
    Ok(())
}

fn apply<A: App>(m: &mut A::Model, command: RailCommand, fx: &mut Effects<A>) -> Result<(), Refused>
where
    A::Model: Source + HasRail,
{
    match command {
        RailCommand::Refresh => refresh(m),
        RailCommand::Reach(reach) => {
            m.rail_mut().reach = reach;
            refresh(m);
        }
        RailCommand::Activate(id) => {
            if m.rail().is_applying() {
                return Err(Refused::new("applying", APPLYING));
            }
            let Some(prompt) = m.prompt(&id) else {
                return Err(Refused::new(
                    "no_suggestion",
                    "That suggestion is not on offer now.",
                ));
            };
            let rail = m.rail_mut();
            if let Some(hint) = rail.reach.hint() {
                return Err(Refused::new("no_mind", hint));
            }
            rail.working = Some(Working::Asking(id.clone()));
            rail.proposal = Some(Proposal {
                title: prompt.title,
                source: prompt.source,
                ..Default::default()
            });
            let mind = rail.mind.clone();
            // One ask at a time; a second suggestion replaces the first, whose answer is dropped.
            fx.spawn_keyed(
                JobKey {
                    feature: "rail",
                    scope: Scope::App,
                },
                Concurrency::ReplacePrevious,
                move |_| {
                    // While the person has Private mode on, no agent sees anything of this
                    // desktop: the app's context is not sent to the Mind (design §3.2). The
                    // file is read here, off the UI thread, as the transport reads it: absent
                    // is not private, unreadable or without a home is.
                    let answer = if privacy::is_private() {
                        Err(AskError::Failed(privacy::REFUSAL.into()))
                    } else {
                        mind.ask(&prompt.text)
                    };
                    routed::<A>(RailCommand::Answered {
                        suggestion: id,
                        answer,
                    })
                },
            );
        }
        RailCommand::Answered { suggestion, answer } => {
            if m.rail().working != Some(Working::Asking(suggestion.clone())) {
                // A replaced ask's late answer, or a dismissal meanwhile.
                return Ok(());
            }
            let card = match &answer {
                Ok(text) => m.proposal(&suggestion, text),
                Err(e) => Proposal::unanswered(e),
            };
            let rail = m.rail_mut();
            rail.working = None;
            rail.proposal = Some(card);
        }
        RailCommand::Dismiss => {
            let rail = m.rail_mut();
            rail.proposal = None;
            rail.working = None;
        }
        RailCommand::Apply => {
            if m.rail().is_applying() {
                return Err(Refused::new("applying", APPLYING));
            }
            let Some(apply) = m.rail().proposal.as_ref().and_then(|p| p.applies.clone()) else {
                return Err(Refused::new(
                    "nothing_to_apply",
                    "This proposal is an answer to read; there is nothing to apply.",
                ));
            };
            m.rail_mut().working = Some(Working::Applying);
            // Private mode, the ceiling and the mode are files: read off the UI thread, as the
            // surface's own RPC thread reads them, then applied on it. Private mode is the
            // person's switch over every agent act, and the rail's apply is one (design §3.2).
            // Keyed: a press replaces the read of an earlier press, whose late result is
            // dropped, so a card dismissed since is never applied on a read made for it; a job
            // the pool could not run reports, so no card is left applying.
            fx.spawn_keyed_or(
                JobKey {
                    feature: "rail.apply",
                    scope: Scope::App,
                },
                Concurrency::ReplacePrevious,
                routed::<A>(RailCommand::Applied(Err(
                    "The machine's authority could not be read; nothing was applied.".into(),
                ))),
                move |_| {
                    if privacy::is_private() {
                        return routed::<A>(RailCommand::Applied(Err(privacy::REFUSAL.into())));
                    }
                    routed::<A>(RailCommand::Authorised {
                        apply,
                        authority: Authority::now(),
                    })
                },
            );
        }
        RailCommand::Authorised { apply, authority } => {
            // Only the card the person pressed Apply on: an authority read for a card dismissed
            // or replaced since is dropped — the apply must be the card shown, and a press must
            // be in flight.
            let shown = m.rail().proposal.as_ref().and_then(|p| p.applies.as_ref());
            if !m.rail().is_applying() || shown != Some(&apply) {
                return Ok(());
            }
            fx.apply_as_agent(apply, authority);
        }
        RailCommand::Applied(result) => {
            if !m.rail().is_applying() {
                return Ok(());
            }
            let rail = m.rail_mut();
            rail.working = None;
            match result {
                Ok(()) => rail.proposal = None,
                Err(why) => {
                    if let Some(card) = rail.proposal.as_mut() {
                        card.body = why;
                        card.verb = "Close".into();
                        card.impact.clear();
                        card.applies = None;
                    }
                }
            }
        }
    }
    Ok(())
}

fn refresh<M: Source + HasRail>(m: &mut M) {
    let reach = m.rail().reach;
    let context = m.context();
    let suggestions = m.suggestions(reach);
    let rail = m.rail_mut();
    rail.context = context;
    rail.suggestions = suggestions;
    rail.unavailable = reach.hint().unwrap_or("");
}

/// A rail command as the app's own. Only an app with a rail routes the rail's commands, and only
/// such an app runs [`update`]; the framework's own uses (`App::rail`) check for `None`.
fn routed<A: App>(command: RailCommand) -> A::Command {
    A::rail(command).expect("an app with a rail turns every RailCommand into its own command")
}

/// What a window with a rail can do: draw it, and send its presses as commands. Implemented by
/// [`rail_window!`] for a generated component that exposes the kit's rail and proposal card.
pub trait RailView {
    fn render_rail(&self, rail: &Rail);
    fn wire_rail<A: App<Ui = Self>>(&self, cx: &Cx<A>);
}

/// Binds the window's rail callbacks to the app's rail commands. From `App::wire`.
pub fn wire<A: App>(cx: &Cx<A>)
where
    A::Ui: RailView,
{
    cx.ui().wire_rail(cx);
}

/// Implements [`RailView`] for a generated root component whose `.slint` exposes the kit's rail
/// and proposal card as `agent-context`, `agent-suggestions`, `agent-unavailable`, `proposal`,
/// `proposal-working`, and the callbacks `agent-suggestion-activated(string)`,
/// `proposal-applied()` and `proposal-dismissed()`, and exports `AgentContextItem`,
/// `AgentSuggestion` and `AgentProposal`. `slint::include_modules!()` must be in scope.
#[macro_export]
macro_rules! rail_window {
    ($ui:ident) => {
        impl $crate::rail::RailView for $ui {
            fn render_rail(&self, rail: &$crate::rail::Rail) {
                // Lists through `rows::set`: a render of every part (the first, or a command
                // that names none) leaves rows that did not change alone.
                use $crate::slint::ComponentHandle as _;
                let context: Vec<AgentContextItem> = rail
                    .context()
                    .iter()
                    .map(|c| AgentContextItem {
                        id: c.id.as_str().into(),
                        label: c.label.as_str().into(),
                        detail: c.detail.as_str().into(),
                        source: c.source.as_str().into(),
                    })
                    .collect();
                $crate::rows::set(self.get_agent_context(), context, |list| self.set_agent_context(list));
                let working = rail.working();
                let suggestions: Vec<AgentSuggestion> = rail
                    .suggestions()
                    .iter()
                    .map(|s| AgentSuggestion {
                        id: s.id.as_str().into(),
                        label: s.label.as_str().into(),
                        detail: s.detail.as_str().into(),
                        icon: s.icon.as_str().into(),
                        running: working == Some(s.id.as_str()),
                        proposes: s.proposes,
                    })
                    .collect();
                $crate::rows::set(self.get_agent_suggestions(), suggestions, |list| {
                    self.set_agent_suggestions(list)
                });
                self.set_agent_unavailable(rail.unavailable().into());
                self.set_proposal(match rail.proposal() {
                    Some(p) => AgentProposal {
                        title: p.title.as_str().into(),
                        body: p.body.as_str().into(),
                        source: p.source.as_str().into(),
                        impact: p.impact.as_str().into(),
                        destructive: p.destructive,
                        verb: p.verb.as_str().into(),
                    },
                    None => AgentProposal::default(),
                });
                self.set_proposal_working(rail.is_working());
            }

            fn wire_rail<A: $crate::App<Ui = Self>>(&self, cx: &$crate::Cx<A>) {
                use $crate::slint::ComponentHandle as _;
                let no_rail = || $crate::Refused::new("no_rail", "This app has no agent rail.");
                self.on_agent_suggestion_activated(cx.try_cmd1(
                    move |id: $crate::slint::SharedString| {
                        A::rail($crate::rail::RailCommand::Activate(id.to_string()))
                            .ok_or_else(no_rail)
                    },
                ));
                self.on_proposal_dismissed(cx.cmd(|| {
                    A::rail($crate::rail::RailCommand::Dismiss)
                        .expect("an app that wires a rail routes its commands")
                }));
                self.on_proposal_applied(cx.cmd(|| {
                    A::rail($crate::rail::RailCommand::Apply)
                        .expect("an app that wires a rail routes its commands")
                }));
            }
        }
    };
}
