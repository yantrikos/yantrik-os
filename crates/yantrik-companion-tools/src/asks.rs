//! Asking the person, on the shell's own card, before the companion changes something (#1311).
//!
//! Text in a mail or a page the companion reads can tell it to delete mail, change a calendar,
//! overwrite a file, forget memories, send a chat message or plan work for later. A refusal that
//! tracks what was read has too many doors; a card at the point of effect has none to track: the
//! person sees exactly what will change and says yes or no. This is the one way every changing
//! tool asks, generalised from the mail send ([`crate::outward`], which is its first caller):
//!
//! 1. raise the shell's card for a named question, with the values shown whole;
//! 2. wait for the person (an unreachable shell, a denial and silence are all a "no");
//! 3. spend the grant, which is bound to exactly these values and good for one call;
//! 4. only then let the tool do the thing, and, where the mode ran it unasked (full bypass),
//!    write that down afterwards ([`Permit::finish`], the shell's `record_unasked_action`).
//!
//! Nothing here decides. Which mode shows a card for which grade is the shell's (`gate::decide`);
//! the four questions are published by it (`control_questions.rs`) and do nothing themselves.
//! No tool builds a card of its own: it names a [`Question`] and fills a [`Card`].
//!
//! # Shown whole, or not asked (decision 4)
//!
//! What a person cannot read they cannot allow. The shell draws each of the four questions
//! whole, by the question and not by reading its sentence (`approval_whole.rs`): every row, each
//! value up to [`WHOLE_VALUE_CHARS`], at most [`WHOLE_ROWS`] rows and [`WHOLE_TOTAL_CHARS`]
//! together, and refuses a request beyond that. This side refuses first ([`Question::fits`]),
//! with the same bounds: the sentence says to do it in the app, and nothing happens.
//!
//! Every free-text value is quoted ([`Card::text`]) so text inside one cannot pose as another
//! field: `subject: "Invoice; to: x"` is one value.

use crate::app_ui::{await_answer, raise_card, Desk};

/// The shell's question surface.
pub(crate) const APP: &str = "shell";

/// How much of one value a card that shows its arguments whole carries
/// (`approvals::WHOLE_VALUE_CHARS`; a shell test keeps the two equal).
pub const WHOLE_VALUE_CHARS: usize = 240;
/// How many rows a question's card shows (`approval_whole::ROWS`).
pub const WHOLE_ROWS: usize = 20;
/// What all rows of a question's card may come to once drawn (`approval_whole::TOTAL_CHARS`).
pub const WHOLE_TOTAL_CHARS: usize = 2400;

/// The four questions the shell publishes. Each is a name, a grade and a sentence there; here it
/// is the name and what the companion owes in return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Question {
    /// Deleting mail or an event, overwriting or editing a file, forgetting a whole topic.
    DeleteForGood,
    /// A chat message leaving the machine.
    SendMessage,
    /// Creating or changing a recipe, a schedule or an automation; queueing a task; starting a recipe.
    PlanWork,
    /// Moving or archiving mail, changing an event, forgetting one memory.
    ChangeItem,
}

impl Question {
    pub const ALL: [Question; 4] = [Question::DeleteForGood, Question::SendMessage, Question::PlanWork, Question::ChangeItem];

    /// The shell's action name.
    pub fn action(self) -> &'static str {
        match self {
            Question::DeleteForGood => "delete_for_good",
            Question::SendMessage => "send_message",
            Question::PlanWork => "plan_work",
            Question::ChangeItem => "change_item",
        }
    }

    /// The grade the shell publishes it at.
    pub fn grade(self) -> &'static str {
        match self {
            Question::ChangeItem => "standard",
            _ => "sensitive",
        }
    }

    /// Whether the shell's card shows its values whole: for all four, by the question
    /// (`approval_whole::is_question`).
    pub fn shown_whole(self) -> bool {
        true
    }

    /// What the card says is being asked, beside the shell's own published sentence.
    fn purpose(self) -> &'static str {
        match self {
            Question::DeleteForGood => "The companion wants to delete or overwrite something of the person's for good.",
            Question::SendMessage => "The companion wants to send a chat message from the person's account.",
            Question::PlanWork => "The companion wants to plan work to be done later or on its own.",
            Question::ChangeItem => "The companion wants to change something of the person's.",
        }
    }

    /// Whether every value of `args` is on the card in full, as the shell draws a card for this
    /// question. `Err` carries the rule that was broken.
    pub fn fits(self, args: &serde_json::Value) -> Result<(), String> {
        let Some(map) = args.as_object() else { return Ok(()) };
        let (per_value, rows) = (WHOLE_VALUE_CHARS, WHOLE_ROWS);
        if map.len() > rows {
            return Err(format!("there are {} fields and a card shows {rows}", map.len()));
        }
        let mut total = 0;
        for (key, value) in map {
            // A field the shell does not publish is refused there; say so here, plainly. A recipe of
            // more than `STEP_FIELDS` steps has a step with no field.
            let known = FIELDS.iter().any(|(k, _)| k == key)
                || key.strip_prefix("step_").and_then(|n| n.parse::<usize>().ok()).is_some_and(|n| (1..=STEP_FIELDS).contains(&n));
            if !known {
                return Err(format!("`{key}` is not a field a card has: a plan lists at most {STEP_FIELDS} steps"));
            }
            let text = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
            let drawn = drawn_len(&text);
            if drawn > per_value {
                return Err(format!("`{key}` is {drawn} characters as the card would draw it and the card shows {per_value}"));
            }
            total += key.chars().count() + 2 + drawn + 2;
        }
        if total > WHOLE_TOTAL_CHARS {
            return Err(format!("the fields come to {total} characters and a card shows {WHOLE_TOTAL_CHARS}"));
        }
        Ok(())
    }
}

/// How long the card draws a value: its backslashes doubled, controls and other characters it can
/// only show as escapes counted at their escaped length. Over-counts rather than under-counts.
fn drawn_len(text: &str) -> usize {
    text.chars()
        .map(|c| match c {
            '\\' => 2,
            c if c.is_ascii() && !c.is_ascii_control() => 1,
            c if c.is_control() => 9,
            c if c.is_ascii() || c.is_alphanumeric() || c.is_whitespace() => 1,
            _ => 9,
        })
        .sum()
}

/// One free-text value as the card draws it: a JSON string, so it is visibly one value whatever
/// it holds. A `"` inside is escaped and cannot end it early; `;` and `key:` inside stay inside.
pub fn quoted(text: &str) -> serde_json::Value {
    serde_json::Value::String(serde_json::Value::String(text.to_string()).to_string())
}

/// Every field a card of these questions can carry, with what it means: the vocabulary the
/// shell publishes for the four questions (`control_questions.rs`) and the only keys a [`Card`]
/// may use (the scenarios check it). The shell refuses a request naming any other field.
pub const FIELDS: &[(&str, &str)] = &[
    ("tool", "The companion tool that is asking"),
    ("account", "The account the thing belongs to"),
    ("from", "Who a mail is from, quoted"),
    ("subject", "A mail's whole subject, quoted"),
    ("date", "When a mail was sent"),
    ("folder", "The folder a mail is in, quoted"),
    ("to_folder", "The folder a mail goes to, quoted"),
    ("change", "What the change does, in words"),
    ("title", "A title, quoted; for a change, before and after"),
    ("event", "The calendar event being changed, quoted"),
    ("start", "When an event starts; for a change, before and after"),
    ("end", "When an event ends; for a change, before and after"),
    ("people", "The people on an event, quoted; for a change, before and after"),
    ("location", "Where an event is, quoted; for a change, before and after"),
    ("description", "A description, quoted; for a change, before and after"),
    ("all_day", "Whether an event lasts all day, before and after"),
    ("reminder_minutes", "How long before an event it is announced, before and after"),
    ("also", "What else goes with it"),
    ("path", "A file's path, quoted"),
    ("first_line", "The first line of what replaces a file's text or an edit, quoted"),
    ("last_line", "The last line of what replaces a file's text or an edit, quoted"),
    ("removes_first", "The first line of the text an edit removes, quoted"),
    ("removes_last", "The last line of the text an edit removes, quoted"),
    ("memory", "A memory's id"),
    ("text", "A memory's or a message's whole text, quoted"),
    ("to", "Who a message goes to, quoted"),
    ("topic", "A topic to forget, quoted"),
    ("memories", "The memories that go, each quoted"),
    ("entities", "The things it knows about that are deleted, each quoted"),
    ("schedules", "The schedules that are cancelled, each quoted"),
    ("name", "A plan's name, quoted"),
    ("trigger", "When a plan runs, in words"),
    ("variables", "What a recipe starts with, quoted"),
    ("label", "A schedule's name, quoted"),
    ("schedule", "When a schedule fires, in words"),
    ("next", "When it fires next"),
    ("does", "What the companion does when it fires, quoted"),
    ("repeats", "How many times it fires"),
    ("task", "The schedule being changed, quoted"),
    ("status", "A schedule's state, before and after"),
    ("cron", "A schedule's timing, before and after"),
    ("next_invoke", "When a schedule fires next, before and after"),
    ("urgency", "How urgent a schedule is, before and after"),
    ("interval_seconds", "How often a schedule fires, before and after"),
    ("instructions", "The instructions a plan or a task follows, quoted"),
    ("condition", "What must hold first, quoted"),
    ("automation", "The automation being switched, quoted"),
    ("priority", "A task's priority"),
];

/// How many recipe steps a card lists, one field each: `step_1` to `step_12`.
pub const STEP_FIELDS: usize = 12;

/// [`FIELDS`] and the step fields, as the shell publishes them.
pub fn fields() -> Vec<(String, String)> {
    let steps = (1..=STEP_FIELDS).map(|i| (format!("step_{i}"), "One step of a recipe, in words, quoted".to_string()));
    FIELDS.iter().map(|(k, d)| (k.to_string(), d.to_string())).chain(steps).collect()
}

/// What a card shows: the tool that asks and the values it will act on, in the order given.
pub struct Card {
    args: serde_json::Map<String, serde_json::Value>,
}

impl Card {
    /// A card for `tool`. The tool's name is always on it.
    pub fn new(tool: &str) -> Card {
        let mut args = serde_json::Map::new();
        args.insert("tool".into(), serde_json::Value::String(tool.to_string()));
        Card { args }
    }

    /// A value that is not free text (an address already validated, a size, a time), shown as is.
    pub fn plain(mut self, key: &str, value: impl std::fmt::Display) -> Card {
        self.args.insert(key.into(), serde_json::Value::String(value.to_string()));
        self
    }

    /// A free-text value, quoted so it cannot pose as another field.
    pub fn text(mut self, key: &str, value: &str) -> Card {
        self.args.insert(key.into(), quoted(value));
        self
    }

    /// The arguments the card shows and the grant is bound to.
    pub fn args(&self) -> serde_json::Value {
        serde_json::Value::Object(self.args.clone())
    }

    /// The tool that is asking.
    pub fn tool(&self) -> &str {
        self.args.get("tool").and_then(|v| v.as_str()).unwrap_or("")
    }
}

/// What the person is asked and how the answers read back, for one kind of ask. The mail send
/// has its own wording; the four questions share one.
pub(crate) struct Spec<'a> {
    pub action: &'a str,
    pub grade: &'a str,
    pub purpose: &'a str,
    /// "this email": what the card is about.
    pub thing: &'a str,
    /// "sending this email": what the person said no to.
    pub doing: &'a str,
    /// "sending": what a spent Allow still needs.
    pub needs: &'a str,
    /// "send": what the model wanted to do, for the retry sentence.
    pub to_do: &'a str,
    /// "nothing was sent".
    pub nothing: &'a str,
    /// The whole sentence for a request the card cannot show whole.
    pub too_long: String,
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Whether the shell refused to put a request up because the card could not show it whole.
pub(crate) fn is_too_long(why: &str) -> bool {
    why.contains("too long to put in front of a person") || why.contains("shown whole or not asked about")
}

/// What an allowed change owes the record afterwards, if the mode let it run unasked.
pub struct Permit {
    unasked: Option<Unasked>,
    answered: Answered,
}

/// How the shell says the answer was reached, in its own field `answered` (set by the shell and
/// by nothing the companion sends). A caller that wants a person's press, and not the mode's
/// or a session rule's, asks [`Permit::pressed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answered {
    /// A person pressed Allow on a card for this call.
    Pressed,
    /// The mode ran it with no card.
    Mode,
    /// A session rule the person made earlier answered.
    SessionRule,
    /// An administrator's standing rule answered.
    StandingRule,
    /// The shell did not say. Never counts as pressed.
    Unsaid,
}

impl Answered {
    fn from_shell(word: &serde_json::Value) -> Answered {
        match word.as_str() {
            Some("pressed") => Answered::Pressed,
            Some("mode") => Answered::Mode,
            Some("session_rule") => Answered::SessionRule,
            Some("standing_rule") => Answered::StandingRule,
            _ => Answered::Unsaid,
        }
    }
}

struct Unasked {
    desk: Desk,
    args: serde_json::Value,
    mode: String,
    action: String,
    grade: String,
}

impl std::fmt::Debug for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permit(owes_a_record: {})", self.unasked.is_some())
    }
}

impl Permit {
    /// Nothing to record: the person was asked, or this is a test.
    pub fn asked() -> Permit {
        Permit { unasked: None, answered: Answered::Pressed }
    }

    /// How the shell says this was answered.
    pub fn answered(&self) -> Answered {
        self.answered
    }

    /// Whether a person pressed Allow on a card for this call: what a follow-up that refuses
    /// changes made after reading someone else's text will require.
    pub fn pressed(&self) -> bool {
        self.answered == Answered::Pressed
    }

    /// Say how the change went. In the person's full bypass nobody was asked, and the shell's mode
    /// promises it is "written down instead": this is that, by the shell's own
    /// `record_unasked_action`. A failure to record cannot undo the change and is not an error.
    pub fn finish(self, done: bool) {
        let Some(u) = self.unasked else { return };
        let _ = u.desk.shell_call(
            "app.act",
            serde_json::json!({ "action": "record_unasked_action", "args": {
                "app": APP, "action": u.action, "grade": u.grade, "mode": u.mode, "args_json": u.args,
                "requester": "companion", "outcome": if done { "ok" } else { "failed" },
            }}),
        );
    }
}

/// The one ask: raise the card, wait, spend. `Ok` only when the person allowed exactly `args` (or
/// the shell says its mode asks nobody); `Err` is a sentence for the model and nothing was done.
pub(crate) fn ask_with(desk: &Desk, spec: &Spec<'_>, args: &serde_json::Value) -> Result<Permit, String> {
    let nothing = capital(spec.nothing);
    let asked = raise_card(desk, APP, spec.action, spec.grade, args, spec.purpose)
        .map_err(|why| if is_too_long(&why) { spec.too_long.clone() } else { format!("{why} {nothing}.") })?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        let mode = answer["mode"].as_str().unwrap_or("bypass_all").to_string();
        let (action, grade) = (spec.action.to_string(), spec.grade.to_string());
        let answered = Answered::from_shell(&answer["answered"]);
        return Ok(Permit { unasked: Some(Unasked { desk: desk.clone(), args: args.clone(), mode, action, grade }), answered });
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err(format!(
            "The desktop was asked to put a card up for {} and did not say which request it was, so {}.",
            spec.thing, spec.nothing
        ));
    };
    match await_answer(desk, &request_id).as_str() {
        "granted" => spend(desk, spec, &request_id, args).map(|said| Permit { unasked: None, answered: said }),
        "denied" => Err(format!(
            "The person at the machine said no to {}. {nothing}. That is an answer, not an error: do not ask again \
             unless they bring it up.",
            spec.doing
        )),
        "consumed" => Err(format!(
            "The Allow for {} had already been spent, so {}. Ask again if it still needs {}.",
            spec.thing, spec.nothing, spec.needs
        )),
        _ => Err(format!(
            "Nobody answered the card for {} within {} s, so {}. They were probably away from the keyboard; say what you \
             wanted to {} and try again when they are back.",
            spec.thing,
            desk.wait.as_secs(),
            spec.nothing,
            spec.to_do
        )),
    }
}

/// Burn the grant: it authorises this one call, once.
fn spend(desk: &Desk, spec: &Spec<'_>, request_id: &str, args: &serde_json::Value) -> Result<Answered, String> {
    desk.shell_call(
        "app.act",
        serde_json::json!({ "action": "consume_approval", "args": {
            "request_id": request_id, "app": APP, "action": spec.action, "args_json": args,
        }}),
    )
    .map(|spent| Answered::from_shell(&spent["result"]["answered"]))
    .map_err(|e| {
        format!(
            "The person allowed {}, but the Allow could not be spent, so {}: {}",
            spec.thing, spec.nothing, e.message
        )
    })
}

/// The sentence for a change the card cannot show in full.
fn too_long(card: &Card, rule: &str) -> String {
    format!(
        "This change ({}) is too long to put in front of the person in full, and what they cannot read they cannot \
         allow, so it was not asked about ({rule}). Nothing was changed. Do it in the app, where the person can see \
         all of it.",
        card.tool()
    )
}

fn spec_for<'a>(q: Question, card: &Card) -> Spec<'a> {
    Spec {
        action: q.action(),
        grade: q.grade(),
        purpose: q.purpose(),
        thing: "this change",
        doing: "this change",
        needs: "doing",
        to_do: "do",
        nothing: "nothing was changed",
        too_long: too_long(card, "see the card's limits"),
    }
}

/// Ask the person, on `desk`'s card, whether `card` may go ahead as question `q`.
pub fn ask_on(desk: &Desk, q: Question, card: &Card) -> Result<Permit, String> {
    let args = card.args();
    if let Err(rule) = q.fits(&args) {
        return Err(too_long(card, &rule));
    }
    ask_with(desk, &spec_for(q, card), &args)
}

/// Ask on the session's own desk.
pub fn ask(q: Question, card: &Card) -> Result<Permit, String> {
    ask_on(&Desk::session(), q, card)
}

/// Ask, and only on an Allow run `effect`, once; then write down a run the mode let go unasked.
/// The tool calls this immediately before the thing it does, after its arguments are validated:
/// `Err` is the person's no (or the reason nobody was asked) or the effect's own failure, and in
/// the first case `effect` never ran.
pub fn act<T>(q: Question, card: &Card, effect: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    act_on(&Desk::session(), q, card, effect)
}

/// [`act`] on a given desk.
pub fn act_on<T>(desk: &Desk, q: Question, card: &Card, effect: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let permit = ask_on(desk, q, card)?;
    let result = effect();
    permit.finish(result.is_ok());
    result
}
