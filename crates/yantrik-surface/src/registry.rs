//! The table of one surface's actions, and the one function every `app.act` to it crosses.

use std::sync::Mutex;

use serde_json::Value;
use yantrik_ipc_contracts::control_surface::{act_json, describe_json, Action, View};
use yantrik_ipc_transport::gate::{self, decide, Authority, LADDER};
use yantrik_ipc_transport::reach::{self, Reach};

use crate::args::{as_declared, check_arguments, declaration_problems};

/// What an app reports about itself, for a surface whose describe closure lives on one thread.
pub type Describer = dyn Fn() -> View;
/// One action's handler, for a surface whose closures live on one thread.
pub type Handler = dyn Fn(&Value) -> Result<Value, String>;
/// What an app reports about itself, for a surface shared across threads (a service).
pub type SharedDescriber = dyn Fn() -> View + Send + Sync;
/// One action's handler, for a surface shared across threads (a service).
pub type SharedHandler = dyn Fn(&Value) -> Result<Value, String> + Send + Sync;

/// A registry whose closures may capture anything — a window's, which live on its UI thread.
pub type LocalRegistry = Registry<Describer, Handler>;
/// A registry whose closures are `Send + Sync` — a service's, dispatched on any worker.
pub type SharedRegistry = Registry<SharedDescriber, SharedHandler>;

/// One surface's actions, their handlers, and how it describes itself.
///
/// Generic over the closure types only so that one dispatch serves both kinds of surface: a
/// window's closures capture Slint handles and must never leave the UI thread, and a service's
/// must be shareable across the socket's workers. Everything else — the order of the checks,
/// every sentence, the revision — is the same code for both.
pub struct Registry<D: ?Sized = Describer, H: ?Sized = Handler> {
    app_id: String,
    describe: Option<Box<D>>,
    actions: Vec<(Action, Box<H>)>,
    /// Grades [`Registry::regrade`] has moved since the surface was published, by action name.
    ///
    /// Its own lock, and that is the whole point. A handler runs from inside the dispatch and
    /// `regrade` is meant to be called from a handler (Studio's `set_backend` raising its
    /// `generate`), so nothing here may be held across the handler: the first version of this,
    /// which borrowed the whole registry mutably, panicked with "RefCell already borrowed" the
    /// first time an action called it and took the app down.
    overrides: Mutex<Vec<(String, &'static str)>>,
}

impl<D: ?Sized, H: ?Sized> Registry<D, H> {
    pub fn new(app_id: &str) -> Self {
        Registry {
            app_id: app_id.to_string(),
            describe: None,
            actions: Vec::new(),
            overrides: Mutex::new(Vec::new()),
        }
    }

    /// The id this surface publishes, and the app a grant for one of its actions is bound to.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// How many actions this surface offers.
    pub fn action_count(&self) -> usize {
        self.actions.len()
    }

    /// What this surface reports when asked. Keep it cheap: it runs on every `describe`, and
    /// again around every `act` (before, when the call carries `expect_revision`, and after).
    pub fn set_describe(&mut self, f: Box<D>) {
        self.describe = Some(f);
    }

    /// One thing this surface can be asked to do. A declaration the dispatch cannot check is
    /// logged here, once, so its author hears about it before a caller does.
    pub fn add(&mut self, spec: Action, f: Box<H>) {
        for problem in declaration_problems(&spec) {
            tracing::warn!(app = %self.app_id, action = %spec.name, "{problem}");
        }
        self.actions.push((spec, f));
    }

    /// What is wrong with the way this surface declares its actions — see
    /// [`crate::declaration_problems`]. An author's test asserts this is empty.
    pub fn problems(&self) -> Vec<String> {
        self.actions.iter().flat_map(|(a, _)| declaration_problems(a)).collect()
    }

    /// The grade an action is published at right now: what it was declared with, unless
    /// [`Registry::regrade`] has moved it.
    ///
    /// Every reader goes through here — the ceiling check in [`Registry::act`],
    /// [`Registry::grade_of`], and the specs [`Registry::describe`] hands out — so the card a
    /// person is shown and the dispatch that enforces it can never be reading two different
    /// numbers.
    fn effective_grade(&self, action: &str, declared: &'static str) -> &'static str {
        self.overrides
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(name, _)| name == action)
            .map(|(_, grade)| *grade)
            .unwrap_or(declared)
    }

    /// The actions as `describe` publishes them: as declared, with any regrade applied.
    pub fn specs(&self) -> Vec<Action> {
        self.actions
            .iter()
            .map(|(a, _)| {
                let mut spec = a.clone();
                spec.permission = self.effective_grade(&spec.name, spec.permission);
                spec
            })
            .collect()
    }

    /// The grade this surface publishes for `name` right now — regrades included — or the
    /// refusal an action it does not have gets.
    ///
    /// Asked before a grant is spent, so a grant is only ever spent on an act whose grade the
    /// ceiling allows (#154). `act` reads the grade the same way.
    pub fn grade_of(&self, name: &str) -> Result<&'static str, String> {
        self.actions
            .iter()
            .find(|(a, _)| a.name == name)
            .map(|(a, _)| self.effective_grade(&a.name, a.permission))
            .ok_or_else(|| self.unknown(name))
    }

    /// The grade this surface publishes for one of its own actions, or `None` for an action it
    /// does not have — which a caller must not read as "harmless": an unknown action has no
    /// grade, and the honest answer to a question about one is a refusal, not a default.
    pub fn published_grade(&self, name: &str) -> Option<&'static str> {
        self.grade_of(name).ok()
    }

    /// Re-declare the grade this surface publishes for one of its own actions, while it runs.
    ///
    /// A surface whose actions cost different amounts depending on how it is configured needs
    /// this. Studio's `generate` sends a prompt to whatever backend the person chose: to a
    /// ComfyUI on their own LAN that is `standard`, to a hosted service it is `sensitive`,
    /// because the words leave the machine and may cost money doing it. The configuration can
    /// change from an action on this same surface — so without a way to move the grade, a caller
    /// could point the app at a hosted service and generate in the same breath, under the grade
    /// that applied when it was still local. That is the one direction a grade must never be
    /// wrong in.
    ///
    /// Returns the grade now published, so a handler can say what the next call will be asked
    /// for. `Err` leaves the published grade untouched: a typo must not quietly un-grade an
    /// action, nor may one action's regrade touch another.
    pub fn regrade(&self, action: &str, permission: &'static str) -> Result<&'static str, String> {
        check_grade(action, permission)?;
        if !self.actions.iter().any(|(a, _)| a.name == action) {
            let known: Vec<&str> = self.actions.iter().map(|(a, _)| a.name.as_str()).collect();
            return Err(format!("this app has no action `{action}`; it offers: {}", known.join(", ")));
        }
        let mut set = self.overrides.lock().unwrap_or_else(|e| e.into_inner());
        match set.iter_mut().find(|(name, _)| name == action) {
            Some(entry) => entry.1 = permission,
            None => set.push((action.to_string(), permission)),
        }
        Ok(permission)
    }

    /// Hold a call from an agent to its reach (`yantrik_ipc_transport::reach`), on the grade this
    /// surface publishes for the action now and the arguments as sent: an act outside the role's
    /// surfaces, or above its ceiling, is refused with `REACH:` before the machine's ceiling is
    /// asked and before the handler. `None` — no token, or a live agent with no role — holds
    /// nothing. An action this surface does not have is answered as that.
    ///
    /// The arguments matter to one rule: an opening act (`shell.open_app name=notes`) is within a
    /// reach that names the app, whatever the reach's ceiling (`reach::within_call`).
    ///
    /// A second rule beside the gate's, not part of it, so it is called beside [`Registry::act`]
    /// rather than inside it: the gate asks whether the machine and the person allow an act, this
    /// asks whether this agent was given it, and both must say yes.
    pub fn within_reach(&self, reach: Option<&Reach>, name: &str, args: &Value) -> Result<(), String> {
        let Some(reach) = reach else { return Ok(()) };
        reach::within_call(reach, &self.app_id, name, self.grade_of(name)?, args)
    }

    /// Everything about a call that must hold before a person's grant is spent on it: the action
    /// exists and its arguments are right — present, known, and of the declared type or losslessly
    /// converted to it. Answers with the grade the surface publishes for the action now, which is
    /// what the ceiling and the grant are then held to.
    ///
    /// A grant is a person's Allow for one call, bound to its arguments as sent. A call that is
    /// going to be refused for those arguments must be refused before the Allow is used up, or
    /// the person is asked again for something they already said yes to — so the arguments are
    /// checked here, ahead of the ceiling and the spend, and again in [`Registry::act`].
    pub fn check_call(&self, name: &str, args: &Value) -> Result<&'static str, String> {
        let grade = self.grade_of(name)?;
        if let Some((spec, _)) = self.actions.iter().find(|(a, _)| a.name == name) {
            check_arguments(spec, args)?;
        }
        Ok(grade)
    }

    fn unknown(&self, name: &str) -> String {
        let known: Vec<&str> = self.actions.iter().map(|(a, _)| a.name.as_str()).collect();
        format!("unknown action `{name}`; this app offers: {}", known.join(", "))
    }
}

/// A grade [`Registry::regrade`] may move an action to, or the sentence for one it may not.
pub fn check_grade(action: &str, permission: &str) -> Result<(), String> {
    if gate::grade(permission).is_none() {
        return Err(format!(
            "`{permission}` is not a level this OS defines ({}), so `{action}` kept the grade it had",
            LADDER.join(" < ")
        ));
    }
    Ok(())
}

impl<D, H> Registry<D, H>
where
    D: ?Sized + Fn() -> View,
    H: ?Sized + Fn(&Value) -> Result<Value, String>,
{
    /// Read the live view once. Every caller below goes through this, so a revision is never
    /// computed from a different read than the state it is reported beside.
    pub fn snapshot(&self) -> View {
        match &self.describe {
            Some(f) => f(),
            None => View::new(format!("{} (no description published)", self.app_id)),
        }
    }

    /// The reply to `app.describe`: the live view, its revision, and every action with the
    /// grade it is published at now. Takes no authority — reading an app is free.
    pub fn describe(&self) -> Value {
        let view = self.snapshot();
        describe_json(&self.app_id, &view, &self.specs())
    }

    /// Check the arguments, the ceiling and the mode, and the guard; dispatch with the arguments
    /// as the action declared them; and read what came of it — all on the calling thread, with
    /// nothing in between.
    ///
    /// The order, which every door keeps (docs/surface-protocol.md, §5): the action exists; the
    /// calling agent's reach, when it carries a token ([`Registry::within_reach`], called just
    /// before this); the arguments as sent; the ceiling; any grant, spent against the arguments as
    /// sent (before this, see [`crate::ActCall::spend_grant`]); the mode; the revision guard; and
    /// only then the arguments converted to their declared types and the handler.
    ///
    /// These steps are one function because they have to be one turn of whatever serialises the
    /// app. Split across calls, the gap between the check and the dispatch is a window in which
    /// the person can type, and the gap between the dispatch and the read is a window in which
    /// they can undo it. A window calls this on its UI thread, which is the thread that would have
    /// to run anything in between; a service calls it on the worker that took the request.
    ///
    /// `authority` arrives as an argument — the ceiling and the mode already read from their
    /// files, and any grant already spent (see [`crate::ActCall::spend_grant`]) — so the boundary
    /// is enforced in the dispatch itself, the one function every `app.act` crosses, whoever sent
    /// it, while the IO stays wherever the caller wants it and tests can pin the ceiling and the
    /// mode instead of inheriting the developer's.
    pub fn act(
        &self,
        name: &str,
        args: &Value,
        expect_revision: Option<&str>,
        action_id: &str,
        authority: &Authority,
    ) -> Result<Value, String> {
        let Some((spec, run)) = self.actions.iter().find(|(a, _)| a.name == name) else {
            return Err(self.unknown(name));
        };

        // Present, known, and of the declared type or losslessly converted to it — see `args`.
        // First, and before any grant is spent: a malformed call is refused for what is wrong with
        // it, and a person's Allow is not used up on a call that was never going to run.
        check_arguments(spec, args)?;

        // The ceiling and then the mode. The grade read is the one `describe` is showing now (see
        // `regrade`), or the two disagree. With the action's own description beside it: an action
        // this app says cannot be undone is asked about in every mode but bypass, as the shell
        // and the bridge ask.
        let published = self.effective_grade(name, spec.permission);
        decide(authority, &self.app_id, name, published, &spec.description)?;

        // The guard. A caller that read state, decided, and asked for this action gets to say what
        // it was looking at; if the app has moved on, the action does not happen. Refusing is
        // cheap and correctable — acting on a stale premise is neither.
        if let Some(expected) = expect_revision {
            let before = self.snapshot();
            let revision = before.revision();
            if revision != expected {
                return Err(format!(
                    "STALE: this app is at revision {} and you acted on {expected}. \
                     It now reports: {}. Read it again before deciding.",
                    revision, before.summary
                ));
            }
        }

        // The handler reads what it declared: every argument of its declared type, converted where
        // it arrived as something that converts without loss, and every default filled in.
        let result = run(&as_declared(spec, args))?;

        // Read back through the same path a `describe` would take, so a caller never has to make
        // a second round trip to find out what its own action did. `accepted` says the handler
        // ran; `settled` (from the action's own `deferred`) says whether the work finished —
        // never `ok`, never `done`.
        let after = self.snapshot();
        Ok(act_json(&self.app_id, action_id, !spec.deferred, result, &after))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use serde_json::json;
    use yantrik_ipc_contracts::control_surface::Param;
    use yantrik_ipc_transport::gate::Mode;

    use super::*;

    /// A ceiling that binds nothing, for the tests that are about everything *except* the
    /// boundary. The boundary has its own tests below, with the ceiling and the mode pinned per
    /// case rather than inherited from whatever files the machine running them happens to have.
    const OPEN: &str = "dangerous";

    /// Authority that binds nothing: the ceiling and the mode both at the top of the ladder.
    fn open() -> Authority {
        Authority { ceiling: OPEN.into(), mode: Mode::named("bypass"), granted: false }
    }

    /// A machine at `ceiling`, in a mode that asks about nothing under it: the ceiling tests.
    fn under(ceiling: &str) -> Authority {
        Authority { ceiling: ceiling.into(), mode: Mode::named("bypass"), granted: false }
    }

    /// An open ceiling and the mode under test, with or without a grant spent for the call.
    fn in_mode(mode: &str, granted: bool) -> Authority {
        Authority { ceiling: OPEN.into(), mode: Mode::named(mode), granted }
    }

    /// A registry built the way an app builds one.
    fn surface(
        app_id: &str,
        describe: Option<Box<Describer>>,
        actions: Vec<(Action, Box<Handler>)>,
    ) -> Registry {
        let mut reg = Registry::new(app_id);
        if let Some(f) = describe {
            reg.set_describe(f);
        }
        for (spec, f) in actions {
            reg.add(spec, f);
        }
        reg
    }

    /// A registry with one action over a state the test can move underneath it.
    fn notes_at(title: &'static str) -> Registry {
        surface(
            "notes",
            Some(Box::new(move || View::new(format!("Notes \u{2014} {title}")).with("open_note", title))),
            vec![(
                Action::new("rename", "Rename the open note").arg(Param::text("to")),
                Box::new(|args| Ok(json!({ "renamed_to": args["to"].clone() }))),
            )],
        )
    }

    #[test]
    fn an_action_becomes_json_schema() {
        let schema = Action::new("open_note", "Open a note by title")
            .arg(Param::text("title").describe("The note's title"))
            .arg(Param::flag("focus").optional())
            .schema();

        assert_eq!(schema["name"], "open_note");
        // Unstated risk is `standard`: steering someone's window is never free.
        assert_eq!(schema["permission"], "standard");
        assert_eq!(schema["parameters"]["properties"]["title"]["type"], "string");
        assert_eq!(schema["parameters"]["properties"]["focus"]["type"], "boolean");
        // Only the required argument is listed as required.
        assert_eq!(schema["parameters"]["required"], json!(["title"]));
    }

    #[test]
    fn an_action_can_declare_itself_dangerous() {
        let schema = Action::new("kill_process", "End a process").risk("dangerous").schema();
        assert_eq!(schema["permission"], "dangerous");
    }

    #[test]
    fn a_missing_argument_is_named_not_guessed() {
        let reg = surface(
            "notes",
            None,
            vec![(
                Action::new("open_note", "Open a note").arg(Param::text("title")),
                Box::new(|_| Ok(json!("never reached"))),
            )],
        );

        let err = reg.act("open_note", &json!({}), None, "t#1", &open()).unwrap_err();
        assert_eq!(err, "`open_note` needs argument `title`");
    }

    #[test]
    fn an_argument_the_action_never_declared_is_refused_not_dropped() {
        let reg = surface(
            "notes",
            None,
            vec![(
                Action::new("open_note", "Open a note").arg(Param::text("title")),
                Box::new(|_| Ok(json!("ran"))),
            )],
        );

        // The real call that exposed this: a title was passed to an action that does not take
        // one, the argument was dropped, and the caller was told the action succeeded.
        let err = reg
            .act("open_note", &json!({"title": "a", "colour": "red"}), None, "t#1", &open())
            .unwrap_err();
        assert_eq!(err, "`open_note` has no argument `colour`; it takes: title");
    }

    #[test]
    fn an_action_that_takes_nothing_says_so_rather_than_ignoring_you() {
        let reg = surface(
            "notes",
            None,
            vec![(Action::new("new_note", "Start a new note"), Box::new(|_| Ok(json!({"title": "Untitled"}))))],
        );

        // This is verbatim the call made on the deployed VM. It used to answer accepted:true
        // and write a note called "Untitled".
        let err = reg.act("new_note", &json!({"title": "Handover"}), None, "t#1", &open()).unwrap_err();
        assert_eq!(err, "`new_note` takes no arguments, but `title` was given");

        // And the no-argument call it was always meant to accept still works.
        assert!(reg.act("new_note", &json!({}), None, "t#2", &open()).is_ok());
    }

    #[test]
    fn an_unknown_action_lists_the_real_ones() {
        let reg = surface(
            "notes",
            None,
            vec![(Action::new("open_note", "Open a note"), Box::new(|_| Ok(Value::Null)))],
        );

        let err = reg.act("nope", &json!({}), None, "t#1", &open()).unwrap_err();
        assert_eq!(err, "unknown action `nope`; this app offers: open_note", "a wrong guess should be correctable");
    }

    /// Through the dispatch: an argument that is neither of its type nor converts to it without
    /// loss is refused before the handler runs, in a sentence that names the argument, what was
    /// wanted, and what arrived; one that converts reaches the handler as the declared type.
    #[test]
    fn an_argument_of_the_wrong_type_is_refused_and_one_that_converts_is_converted() {
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let reg = surface(
            "system-monitor",
            None,
            vec![(Action::new("kill_process", "End a process").arg(Param::integer("pid")), {
                let seen = seen.clone();
                Box::new(move |args| {
                    seen.borrow_mut().push(args["pid"].clone());
                    Ok(Value::Null)
                })
            })],
        );
        let err = reg.act("kill_process", &json!({"pid": "42x"}), None, "t#1", &open()).unwrap_err();
        assert_eq!(err, "`kill_process` argument `pid` must be an integer, and a string arrived");
        let err = reg.act("kill_process", &json!({"pid": 42.5}), None, "t#2", &open()).unwrap_err();
        assert_eq!(err, "`kill_process` argument `pid` must be an integer, and a number with a fraction arrived");
        assert!(seen.borrow().is_empty(), "the handler must not have run");
        assert!(reg.act("kill_process", &json!({"pid": 4242}), None, "t#3", &open()).is_ok());
        assert!(reg.act("kill_process", &json!({"pid": "4242"}), None, "t#4", &open()).is_ok());
        assert_eq!(*seen.borrow(), vec![json!(4242), json!(4242)], "the handler reads an integer both times");
    }

    /// The arguments are checked first, ahead of the ceiling and the mode: a malformed call is
    /// refused for what is wrong with it before anything about who may make it — and so before a
    /// person's grant could be spent on it. With the arguments right, the grade answers.
    #[test]
    fn the_arguments_are_checked_before_the_grade_is_decided() {
        let reg = surface(
            "system-monitor",
            None,
            vec![(
                Action::new("kill_process", "End a process").risk("dangerous").arg(Param::integer("pid")),
                Box::new(|_| Ok(Value::Null)),
            )],
        );
        let wrong = "`kill_process` argument `pid` must be an integer, and a string arrived";
        let err = reg.act("kill_process", &json!({"pid": "x"}), None, "t#1", &under("sensitive")).unwrap_err();
        assert_eq!(err, wrong);
        let err = reg.act("kill_process", &json!({"pid": "x"}), None, "t#1", &in_mode("ask", false)).unwrap_err();
        assert_eq!(err, wrong);
        let err = reg.act("kill_process", &json!({"pid": 7}), None, "t#1", &under("sensitive")).unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
        let err = reg.act("kill_process", &json!({"pid": 7}), None, "t#1", &in_mode("ask", false)).unwrap_err();
        assert!(err.starts_with("GRANT:"), "{err}");
    }

    /// What must hold before a grant is spent: the action exists and its arguments are right.
    #[test]
    fn a_call_is_checked_before_a_grant_is_spent_on_it() {
        let reg = surface(
            "blender",
            None,
            vec![(
                Action::new("render", "Render").risk("sensitive").arg(Param::integer("samples")),
                Box::new(|_| Ok(Value::Null)),
            )],
        );
        assert_eq!(reg.check_call("render", &json!({"samples": 4})), Ok("sensitive"));
        assert_eq!(reg.check_call("render", &json!({"samples": "4"})), Ok("sensitive"), "converts");
        assert_eq!(
            reg.check_call("render", &json!({"samples": "four"})),
            Err("`render` argument `samples` must be an integer, and a string arrived".into())
        );
        assert_eq!(reg.check_call("render", &json!({})), Err("`render` needs argument `samples`".into()));
        assert!(reg.check_call("bake", &json!({})).unwrap_err().starts_with("unknown action `bake`"));
    }

    /// A declared default is what the handler reads for an argument the caller left out.
    #[test]
    fn the_handler_reads_a_default_for_an_argument_left_out() {
        let reg = surface(
            "counter",
            None,
            vec![(
                Action::new("increment", "Add to the counter").arg(Param::integer("by").default(1)),
                Box::new(|args| Ok(json!({ "by": args["by"].clone() }))),
            )],
        );
        let answer = reg.act("increment", &json!({}), None, "c#1", &open()).unwrap();
        assert_eq!(answer["result"]["by"], 1);
        let answer = reg.act("increment", &json!({"by": 5}), None, "c#2", &open()).unwrap();
        assert_eq!(answer["result"]["by"], 5);
        let answer = reg.act("increment", &json!({"by": "5"}), None, "c#3", &open()).unwrap();
        assert_eq!(answer["result"]["by"], 5, "a string that is exactly an integer is one");
        let err = reg.act("increment", &json!({"by": "five"}), None, "c#4", &open()).unwrap_err();
        assert!(err.contains("must be an integer"), "{err}");
    }

    #[test]
    fn describe_falls_back_when_the_app_published_nothing() {
        let reg = surface("notes", None, Vec::new());
        let out = reg.describe();
        assert_eq!(out["app"], "notes");
        assert_eq!(out["summary"], "notes (no description published)");
        assert_eq!(out["actions"], json!([]));
    }

    #[test]
    fn describe_publishes_the_richer_types_and_the_expected_duration() {
        let reg = surface(
            "studio",
            Some(Box::new(|| View::new("Studio"))),
            vec![(
                Action::new("generate", "Make a picture")
                    .defers()
                    .expected_seconds(45)
                    .arg(Param::one_of("size", &["512", "1024"]).default("1024"))
                    .arg(Param::array("prompts", "string")),
                Box::new(|_| Ok(Value::Null)),
            )],
        );
        let described = reg.describe();
        let action = &described["actions"][0];
        assert_eq!(action["expected_seconds"], 45);
        assert_eq!(action["settles"], "later");
        assert_eq!(action["parameters"]["properties"]["size"]["enum"], json!(["512", "1024"]));
        assert_eq!(action["parameters"]["properties"]["size"]["default"], "1024");
        assert_eq!(action["parameters"]["properties"]["prompts"]["items"], json!({"type": "string"}));
        assert_eq!(action["parameters"]["required"], json!(["prompts"]));
        assert!(reg.problems().is_empty(), "{:?}", reg.problems());
    }

    // ── Accepted is not done ──

    #[test]
    fn acting_never_answers_with_a_bare_success() {
        let answer = notes_at("Kernel asks")
            .act("rename", &json!({ "to": "Kernel answers" }), None, "notes#1", &open())
            .unwrap();

        // The three things a caller has to be able to tell apart.
        assert_eq!(answer["accepted"], true);
        assert_eq!(answer["action_id"], "notes#1");
        assert_eq!(answer["settled"], true);
        assert_eq!(answer["result"]["renamed_to"], "Kernel answers");
        // And the state afterwards, so nobody has to make a second call to find out what they did.
        assert!(answer["summary"].as_str().unwrap().contains("Kernel asks"));
        assert!(answer["revision"].as_str().is_some_and(|r| r.len() == 16));
    }

    #[test]
    fn an_action_that_only_starts_the_work_says_so() {
        // The build case. Returning `accepted: true` with nothing else would let a caller report a
        // compile as finished the instant it was started.
        let reg = surface(
            "builder",
            None,
            vec![(Action::new("build", "Start a build").defers(), Box::new(|_| Ok(json!({ "job": 83 }))))],
        );

        let answer = reg.act("build", &json!({}), None, "builder#1", &open()).unwrap();
        assert_eq!(answer["accepted"], true);
        assert_eq!(answer["settled"], false, "a dispatched build has not built anything yet");

        // And the schema says it in advance, so a caller can plan to watch rather than discover
        // afterwards that it has to.
        let schema = Action::new("build", "Start a build").defers().schema();
        assert_eq!(schema["settles"], "later");
        assert_eq!(Action::new("open_note", "Open").schema()["settles"], "on return");
    }

    // ── The guard ──

    #[test]
    fn an_action_decided_on_a_state_the_app_has_left_is_refused() {
        // The race this exists for. A caller reads "Kernel asks", decides to rename it, and by the
        // time the call lands the user has opened something else. Renaming now renames the wrong
        // note, and the caller would report success.
        let stale = View::new("Notes \u{2014} Kernel asks").with("open_note", "Kernel asks").revision();
        let now = View::new("Notes \u{2014} Shopping list").with("open_note", "Shopping list").revision();

        let err = notes_at("Shopping list")
            .act("rename", &json!({ "to": "x" }), Some(&stale), "notes#1", &open())
            .unwrap_err();

        assert_eq!(
            err,
            format!(
                "STALE: this app is at revision {now} and you acted on {stale}. \
                 It now reports: Notes \u{2014} Shopping list. Read it again before deciding."
            )
        );
    }

    #[test]
    fn a_guard_that_matches_lets_the_action_through() {
        let current = View::new("Notes \u{2014} Kernel asks").with("open_note", "Kernel asks").revision();

        let answer = notes_at("Kernel asks")
            .act("rename", &json!({ "to": "ok" }), Some(&current), "notes#1", &open())
            .unwrap();
        assert_eq!(answer["accepted"], true);
        assert_eq!(answer["result"]["renamed_to"], "ok");
    }

    #[test]
    fn an_action_with_no_guard_still_runs() {
        // Most calls are the caller's own initiative and have nothing to compare against.
        // Requiring a revision would only teach callers to echo back whatever they last saw,
        // which is a guard that always passes.
        let answer = notes_at("Kernel asks").act("rename", &json!({ "to": "ok" }), None, "notes#1", &open()).unwrap();
        assert_eq!(answer["accepted"], true);
    }

    #[test]
    fn the_guard_is_checked_before_the_arguments_are_used() {
        // Ordering that matters: a stale guard must refuse without the handler having run. If the
        // rename happened and *then* we noticed the state had moved, the refusal would be a lie.
        let ran = Rc::new(Cell::new(false));
        let reg = surface(
            "notes",
            Some(Box::new(|| View::new("Notes \u{2014} now"))),
            vec![(Action::new("go", "Go"), {
                let ran = ran.clone();
                Box::new(move |_| {
                    ran.set(true);
                    Ok(Value::Null)
                })
            })],
        );

        let err = reg.act("go", &json!({}), Some("0000000000000000"), "n#1", &open());
        assert!(err.is_err());
        assert!(!ran.get(), "the handler must not have run");
    }

    // ── The ceiling ──

    /// The shape the bug was measured in: `files_delete`, graded `dangerous`, on a machine
    /// whose ceiling is `sensitive`. The bridge refused it; the dispatch waved it through, and
    /// the file was gone. These tests live beside the dispatch, so the boundary fails loudly if
    /// anyone moves the check back out to a caller.
    fn delete_surface(ran: Rc<Cell<bool>>) -> Registry {
        surface(
            "shell",
            Some(Box::new(|| View::new("Shell \u{2014} Files"))),
            vec![(
                Action::new("files_delete", "Delete a file").risk("dangerous").arg(Param::text("name")),
                Box::new(move |args| {
                    ran.set(true);
                    Ok(json!({ "deleted": args["name"].clone() }))
                }),
            )],
        )
    }

    #[test]
    fn an_action_above_the_ceiling_is_refused_by_the_dispatch_itself() {
        let ran = Rc::new(Cell::new(false));
        let reg = delete_surface(ran.clone());

        let err = reg
            .act("files_delete", &json!({"name": "x"}), None, "shell#1", &under("sensitive"))
            .unwrap_err();

        assert!(err.starts_with("CEILING:"), "a caller has to be able to branch on this: {err}");
        assert!(err.contains("dangerous"), "the refusal names the grade: {err}");
        assert!(err.contains("sensitive"), "and the ceiling it was over: {err}");
        assert!(err.contains("tool_permission"), "and where that ceiling is set: {err}");
        assert!(!ran.get(), "the handler must not have run");
    }

    #[test]
    fn the_ceiling_refuses_a_well_formed_call_on_the_grade_alone() {
        // The arguments come first (a malformed call is answered for its arguments, and no grant
        // is spent on it); a well-formed call over the ceiling is refused on the grade, before the
        // mode, the guard or the handler.
        let reg = delete_surface(Rc::new(Cell::new(false)));

        let err = reg.act("files_delete", &json!({}), None, "shell#1", &under("sensitive")).unwrap_err();
        assert_eq!(err, "`files_delete` needs argument `name`");
        let err = reg
            .act("files_delete", &json!({"name": "x"}), Some("0000000000000000"), "shell#1", &under("sensitive"))
            .unwrap_err();
        assert!(err.starts_with("CEILING:"), "not the guard's refusal: {err}");
    }

    #[test]
    fn an_action_at_or_below_the_ceiling_runs() {
        // "At or below" is the whole contract; the ceiling is not a blanket refusal.
        let ran = Rc::new(Cell::new(false));
        let reg = delete_surface(ran.clone());
        let answer = reg.act("files_delete", &json!({"name": "x"}), None, "shell#1", &under("dangerous")).unwrap();
        assert_eq!(answer["accepted"], true);
        assert!(ran.get());

        // And the everyday case: a `standard` action under the shipped `sensitive` default.
        let answer = notes_at("Kernel asks")
            .act("rename", &json!({ "to": "ok" }), None, "notes#1", &under("sensitive"))
            .unwrap();
        assert_eq!(answer["accepted"], true);
    }

    #[test]
    fn a_ceiling_tightened_to_safe_binds_the_default_actions_too() {
        // The setting has to actually tighten, not only refuse the graded-dangerous few: every
        // action floors at `standard`, so `safe` closes the door to programmatic callers entirely.
        let err = notes_at("Kernel asks")
            .act("rename", &json!({ "to": "ok" }), None, "notes#1", &under("safe"))
            .unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
        assert!(err.contains("standard"), "the refusal names the action's own grade: {err}");
    }

    #[test]
    fn an_action_graded_off_the_ladder_is_refused_not_waved_through() {
        // An ungradeable action is not "safe". A typo in a `.risk(...)` must fail closed, or the
        // typo silently becomes an exemption.
        let reg = surface(
            "notes",
            None,
            vec![(Action::new("nuke", "Typo'd grade").risk("catastrophic"), Box::new(|_| Ok(json!("never reached"))))],
        );

        let err = reg.act("nuke", &json!({}), None, "notes#1", &open()).unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
        assert!(err.contains("not a level this OS defines"), "{err}");
        assert!(!reg.problems().is_empty(), "and its author is told");
    }

    // ── Regrading ──

    #[test]
    fn an_action_can_be_regraded_while_the_app_runs_and_the_ceiling_follows() {
        // Studio's shape: `generate` is graded when the surface is published, and the backend it
        // sends prompts to can be changed afterwards by another action on the same surface.
        let reg = surface(
            "studio",
            None,
            vec![(Action::new("generate", "Make a picture from a sentence"), Box::new(|_| Ok(json!({"queued": 1}))))],
        );
        assert!(reg.act("generate", &json!({}), None, "studio#1", &under("standard")).is_ok());

        assert_eq!(reg.regrade("generate", "sensitive").unwrap(), "sensitive");
        assert_eq!(reg.published_grade("generate"), Some("sensitive"), "the two readers disagree");
        let err = reg.act("generate", &json!({}), None, "studio#2", &under("standard")).unwrap_err();
        assert!(err.starts_with("CEILING:") && err.contains("graded `sensitive`"), "{err}");
        assert!(reg.act("generate", &json!({}), None, "studio#3", &under("sensitive")).is_ok());
        // And `describe` — what a caller reads before deciding — reports the new grade.
        assert_eq!(reg.describe()["actions"][0]["permission"], json!("sensitive"));

        assert_eq!(reg.regrade("generate", "standard").unwrap(), "standard");
        assert!(reg.act("generate", &json!({}), None, "studio#4", &under("standard")).is_ok());
    }

    #[test]
    fn a_grade_this_os_does_not_define_leaves_the_action_at_the_one_it_had() {
        let reg = surface(
            "studio",
            None,
            vec![
                (Action::new("generate", "Make a picture").risk("standard"), Box::new(|_| Ok(Value::Null))),
                (Action::new("refresh", "Read the gallery again"), Box::new(|_| Ok(Value::Null))),
            ],
        );

        let err = reg.regrade("generate", "catastrophic").unwrap_err();
        assert_eq!(
            err,
            "`catastrophic` is not a level this OS defines (safe < standard < sensitive < dangerous), \
             so `generate` kept the grade it had"
        );
        assert_eq!(reg.published_grade("generate"), Some("standard"), "the grade moved anyway");

        let err = reg.regrade("no_such_action", "sensitive").unwrap_err();
        assert_eq!(err, "this app has no action `no_such_action`; it offers: generate, refresh");
        assert_eq!(reg.published_grade("refresh"), Some("standard"), "an unknown action regraded a known one");
    }

    /// The only way `regrade` is ever actually used: from inside a handler, while the dispatch is
    /// running it. Nothing may be held across the handler that `regrade` needs.
    #[test]
    fn a_handler_can_regrade_from_inside_its_own_dispatch() {
        let reg: Rc<std::cell::OnceCell<Registry>> = Rc::new(std::cell::OnceCell::new());
        let inner = reg.clone();
        let built = surface(
            "studio",
            None,
            vec![
                (Action::new("generate", "Make a picture from a sentence"), Box::new(|_| Ok(json!({"queued": 1})))),
                (
                    Action::new("set_backend", "Choose where pictures are made").risk("sensitive"),
                    Box::new(move |_| {
                        let now = inner.get().expect("installed").regrade("generate", "sensitive")?;
                        Ok(json!({ "generate_is_graded": now }))
                    }),
                ),
            ],
        );
        let _ = reg.set(built);
        let reg = reg.get().unwrap();

        let answered = reg
            .act("set_backend", &json!({}), None, "studio#1", &under("sensitive"))
            .expect("set_backend must not take the app down");
        assert_eq!(answered["result"]["generate_is_graded"], json!("sensitive"));
        let err = reg.act("generate", &json!({}), None, "studio#2", &under("standard")).unwrap_err();
        assert!(err.starts_with("CEILING:") && err.contains("graded `sensitive`"), "{err}");
    }

    // ── The mode, and the grant ──

    /// Blender's `render`, graded `sensitive`, over a flag that says whether it ran — the action
    /// the account from inside VM 520 found running through `yos act` with no card.
    fn render_surface(ran: Rc<Cell<bool>>) -> Registry {
        surface(
            "blender",
            Some(Box::new(|| View::new("Blender \u{2014} cube.blend"))),
            vec![(
                Action::new("render", "Render the scene").risk("sensitive").arg(Param::text("out")),
                Box::new(move |args| {
                    ran.set(true);
                    Ok(json!({ "rendered_to": args["out"].clone() }))
                }),
            )],
        )
    }

    /// The defect of #116 and #49: `blender.render` is `sensitive`, the machine was in `ask`,
    /// and through `yos act` it ran with no card and no record. The dispatch is the one function
    /// every door crosses, so it is where the refusal has to live — and the refusal has to say how
    /// to get a grant, or a program reading it goes looking for another door.
    #[test]
    fn a_sensitive_act_without_a_grant_is_refused_in_ask_mode() {
        let ran = Rc::new(Cell::new(false));
        let err = render_surface(ran.clone())
            .act("render", &json!({"out": "x.png"}), None, "blender#1", &in_mode("ask", false))
            .unwrap_err();

        assert!(err.starts_with("GRANT:"), "a caller has to be able to branch on this: {err}");
        assert!(err.contains("graded `sensitive`"), "the refusal names the grade: {err}");
        assert!(err.contains("ask mode"), "and the mode it was over: {err}");
        assert!(err.contains("request_approval") && err.contains("press Allow"), "and how to get a grant: {err}");
        assert!(!ran.get(), "the handler must not have run");
    }

    #[test]
    fn the_mode_is_asked_after_the_arguments_and_before_the_guard() {
        let reg = render_surface(Rc::new(Cell::new(false)));
        let err = reg.act("render", &json!({}), None, "blender#1", &in_mode("ask", false)).unwrap_err();
        assert_eq!(err, "`render` needs argument `out`");
        let err = reg
            .act("render", &json!({"out": "x.png"}), Some("0000000000000000"), "blender#1", &in_mode("ask", false))
            .unwrap_err();
        assert!(err.starts_with("GRANT:"), "not the guard's refusal: {err}");
    }

    #[test]
    fn a_sensitive_act_runs_in_auto_mode() {
        for mode in ["auto", "bypass"] {
            let ran = Rc::new(Cell::new(false));
            let answer = render_surface(ran.clone())
                .act("render", &json!({"out": "x.png"}), None, "blender#1", &in_mode(mode, false))
                .unwrap_or_else(|e| panic!("{mode}: {e}"));
            assert_eq!(answer["accepted"], true, "{mode}");
            assert!(ran.get(), "{mode}: the handler ran");
        }
    }

    #[test]
    fn a_grant_lets_a_sensitive_act_run_in_any_mode() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            let ran = Rc::new(Cell::new(false));
            let answer = render_surface(ran.clone())
                .act("render", &json!({"out": "x.png"}), None, "blender#1", &in_mode(mode, true))
                .unwrap_or_else(|e| panic!("{mode}: {e}"));
            assert_eq!(answer["accepted"], true, "{mode}");
            assert!(ran.get(), "{mode}: the handler ran");
        }
    }

    #[test]
    fn a_standard_act_needs_no_grant_in_any_mode() {
        for mode in ["plan", "ask", "auto", "bypass"] {
            let answer = notes_at("Kernel asks")
                .act("rename", &json!({ "to": "ok" }), None, "notes#1", &in_mode(mode, false))
                .unwrap_or_else(|e| panic!("{mode}: {e}"));
            assert_eq!(answer["accepted"], true, "{mode}");
        }
    }

    #[test]
    fn plan_mode_refuses_a_sensitive_act_and_says_no_card_is_coming() {
        let ran = Rc::new(Cell::new(false));
        let err = render_surface(ran.clone())
            .act("render", &json!({"out": "x.png"}), None, "blender#1", &in_mode("plan", false))
            .unwrap_err();
        assert!(err.starts_with("GRANT:") && err.contains("plan mode"), "{err}");
        assert!(err.contains("raises no card"), "plan must not promise a card: {err}");
        assert!(!ran.get());
    }

    #[test]
    fn a_session_rule_covers_the_action_it_names_and_no_other() {
        let with_rule = |app: &str, action: &str| {
            let mut mode = Mode::named("ask");
            mode.session_rules.push((app.to_string(), action.to_string()));
            Authority { ceiling: OPEN.into(), mode, granted: false }
        };
        let args = json!({"out": "anything.png"});

        let ran = Rc::new(Cell::new(false));
        render_surface(ran.clone()).act("render", &args, None, "b#1", &with_rule("blender", "render")).unwrap();
        assert!(ran.get());

        for (app, action) in [("blender", "bake"), ("studio", "render")] {
            let ran = Rc::new(Cell::new(false));
            let err = render_surface(ran.clone()).act("render", &args, None, "b#2", &with_rule(app, action)).unwrap_err();
            assert!(err.starts_with("GRANT:"), "a rule for {app}.{action} is not a rule for blender.render: {err}");
            assert!(!ran.get());
        }
    }

    #[test]
    fn the_ceiling_still_refuses_dangerous_whatever_the_grant_or_mode() {
        for (mode, granted) in [("bypass", false), ("ask", true), ("bypass", true)] {
            let ran = Rc::new(Cell::new(false));
            let authority = Authority { ceiling: "sensitive".into(), mode: Mode::named(mode), granted };
            let err = delete_surface(ran.clone())
                .act("files_delete", &json!({"name": "x"}), None, "shell#1", &authority)
                .unwrap_err();
            assert!(err.starts_with("CEILING:"), "{mode}, granted={granted}: {err}");
            assert!(!ran.get(), "{mode}, granted={granted}: the handler must not have run");
        }
    }

    /// `describe` takes no authority — the signature is the proof — and it reports the grade a
    /// call would be asked for, so a caller can see the cost before paying it.
    #[test]
    fn describe_needs_nothing() {
        let described = render_surface(Rc::new(Cell::new(false))).describe();
        assert_eq!(described["actions"][0]["name"], json!("render"));
        assert_eq!(described["actions"][0]["permission"], json!("sensitive"));
    }

    /// `deploy/yantrik-os/surface-vectors.json` is `gate::decide` written out for every grade,
    /// ceiling, mode, session rule, grant and purpose. This dispatch calls the gate rather than
    /// copying it, and this replays every vector through the dispatch itself — an action graded and
    /// described as the vector says, acted on with no arguments under the vector's authority — so
    /// the way this crate calls the gate (with the published grade and description, the arguments
    /// already answered — here, none are declared and none are sent) is held to the same table
    /// every other implementation replays: allowed means the handler ran, refused means the exact
    /// sentence. What the dispatch does around the gate is `dispatch-vectors.json` (`vectors.rs`).
    #[test]
    fn every_policy_vector_is_what_this_dispatch_decides() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/yantrik-os/surface-vectors.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let vectors: Value = serde_json::from_str(&text).expect("the vectors are json");
        let decisions = vectors["decide"].as_array().expect("decide vectors");
        assert!(decisions.len() >= 600, "the table shrank to {}", decisions.len());

        let mut outcomes = std::collections::BTreeMap::<String, usize>::new();
        for v in decisions {
            let text = |key: &str| v[key].as_str().unwrap_or_else(|| panic!("{key} in {v}")).to_string();
            let (app, action, grade) = (text("app"), text("action"), text("grade"));
            let ran = Rc::new(Cell::new(false));
            let reg = surface(
                &app,
                Some(Box::new(|| View::new("vector"))),
                vec![(
                    // A grade off the ladder (`spicy`) has to reach the dispatch as declared.
                    Action::new(&action, &text("purpose")).risk(Box::leak(grade.into_boxed_str())),
                    {
                        let ran = ran.clone();
                        Box::new(move |_| {
                            ran.set(true);
                            Ok(Value::Null)
                        })
                    },
                )],
            );
            let mut mode = Mode::named(&text("mode"));
            if v["session_rule"] == true {
                mode.session_rules.push((app.clone(), action.clone()));
            }
            let authority = Authority { ceiling: text("ceiling"), mode, granted: v["grant"] == true };

            let answer = reg.act(&action, &json!({}), None, "vector#1", &authority);
            match v["outcome"].as_str() {
                Some("allow") => {
                    assert!(answer.is_ok(), "{}: refused {:?}", v["id"], answer);
                    assert!(ran.get(), "{}: allowed, and the handler did not run", v["id"]);
                }
                Some(outcome) => {
                    assert_eq!(answer.as_ref().err(), v["refusal"].as_str().map(str::to_string).as_ref(), "{}", v["id"]);
                    assert!(answer.unwrap_err().starts_with(&format!("{outcome}:")), "{}", v["id"]);
                    assert!(!ran.get(), "{}: refused, and the handler ran anyway", v["id"]);
                }
                None => panic!("no outcome in {v}"),
            }
            *outcomes.entry(v["outcome"].as_str().unwrap().to_string()).or_default() += 1;
        }
        assert_eq!(outcomes.keys().cloned().collect::<Vec<_>>(), ["CEILING", "GRANT", "allow"], "{outcomes:?}");

        // And the revision every envelope this dispatch builds carries.
        for key in ["revision", "revision_float_edges"] {
            for v in vectors[key].as_array().unwrap_or(&Vec::new()) {
                let view = View::new(v["summary"].as_str().unwrap()).state(v["state"].clone());
                assert_eq!(view.revision(), v["revision"].as_str().unwrap(), "{key}: {v}");
            }
        }
    }

    // ── An agent's reach ──

    /// #195, through the dispatch every door runs. The Planner — `calendar, notes · safe` — opens
    /// Notes and Calendar and brings Notes forward, acts graded `standard`, above its ceiling; it
    /// opens no other app and no screen; and once Notes is open it reads there and writes nothing.
    /// The reach only narrows: the machine's ceiling still decides the opening.
    #[test]
    fn a_role_opens_the_apps_its_reach_names_and_is_held_inside_them() {
        // What the shell's `open_app` opens for a name, told to this process as the shell tells
        // itself (`agents::reaches::opened_app` there): apps by the ids they publish, an alias by
        // its app's, and a screen, the launcher and the browser as no app at all.
        reach::resolve_opened_apps_with(|name| match name.to_lowercase().as_str() {
            app @ ("notes" | "calendar" | "terminal") => Some(app.to_string()),
            "text-editor" | "editor" => Some("editor".to_string()),
            _ => None,
        });
        let planner = Reach {
            agent: "deepseek:c-p1an".into(),
            role: "planner".into(),
            name: "Planner".into(),
            surfaces: vec!["calendar".into(), "notes".into()],
            ceiling: "safe".into(),
        };
        let opened = Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let shell = surface(
            "shell",
            None,
            vec![
                (Action::new("open_app", "Launch an app, or focus it").arg(Param::text("name")).defers(), {
                    let opened = opened.clone();
                    Box::new(move |args| {
                        opened.borrow_mut().push(args["name"].as_str().unwrap_or_default().to_string());
                        Ok(json!({ "launching": args["name"].clone() }))
                    })
                }),
                (
                    Action::new("show_app", "Bring an open app to the front").arg(Param::text("name")).defers(),
                    Box::new(|args| Ok(json!({ "showing": args["name"].clone() }))),
                ),
                (Action::new("show_screen", "Show a screen").arg(Param::text("screen")), Box::new(|_| Ok(Value::Null))),
            ],
        );
        let notes = surface(
            "notes",
            None,
            vec![
                (Action::new("list_notes", "List the notes").risk("safe"), Box::new(|_| Ok(json!([])))),
                (Action::new("new_note", "Start a new note"), Box::new(|_| Ok(Value::Null))),
            ],
        );
        let act = |reg: &Registry, name: &str, args: Value, authority: &Authority| {
            reg.within_reach(Some(&planner), name, &args).and_then(|()| reg.act(name, &args, None, "t#1", authority))
        };
        let ask = in_mode("ask", false);

        for app in ["notes", "calendar"] {
            let answer = act(&shell, "open_app", json!({ "name": app }), &ask).unwrap_or_else(|e| panic!("{app}: {e}"));
            assert_eq!(answer["accepted"], true);
        }
        assert!(act(&shell, "show_app", json!({ "name": "notes" }), &ask).is_ok());
        for name in ["terminal", "settings", "problems", "browser", "launchpad", "text-editor"] {
            let err = act(&shell, "open_app", json!({ "name": name }), &ask).unwrap_err();
            assert!(err.starts_with(&format!("REACH: shell.open_app `{name}` is outside the Planner's reach")), "{err}");
            assert!(err.contains("and may open calendar and notes"), "{err}");
        }
        let err = act(&shell, "show_screen", json!({ "screen": "settings" }), &ask).unwrap_err();
        assert!(err.starts_with("REACH: shell.show_screen is outside the Planner's reach"), "{err}");
        assert_eq!(*opened.borrow(), ["notes", "calendar"], "only the apps its reach names were opened");

        // Open, Notes is held to the reach's ceiling like anything else.
        assert!(act(&notes, "list_notes", json!({}), &ask).is_ok());
        let err = act(&notes, "new_note", json!({}), &ask).unwrap_err();
        assert!(err.starts_with("REACH: notes.new_note is graded `standard`, above the Planner's `safe` ceiling"), "{err}");

        // The machine's ceiling still refuses the opening; the reach stepped aside, nothing more.
        let err = act(&shell, "open_app", json!({ "name": "notes" }), &under("safe")).unwrap_err();
        assert!(err.starts_with("CEILING:"), "{err}");
        assert_eq!(opened.borrow().len(), 2, "the ceiling's refusal opened nothing");
    }

    /// A service's registry is the same dispatch with shareable closures: same sentences.
    #[test]
    fn a_shared_registry_refuses_in_the_same_words() {
        let mut shared: SharedRegistry = Registry::new("notes");
        shared.add(Action::new("open_note", "Open a note").arg(Param::text("title")), Box::new(|_| Ok(Value::Null)));
        let local = surface(
            "notes",
            None,
            vec![(Action::new("open_note", "Open a note").arg(Param::text("title")), Box::new(|_| Ok(Value::Null)))],
        );
        for args in [json!({}), json!({"title": 1}), json!({"title": "a", "x": 1})] {
            assert_eq!(
                shared.act("open_note", &args, None, "n#1", &open()),
                local.act("open_note", &args, None, "n#1", &open()),
                "{args}"
            );
        }
        assert_eq!(shared.act("nope", &json!({}), None, "n#1", &open()), local.act("nope", &json!({}), None, "n#1", &open()));
    }
}
