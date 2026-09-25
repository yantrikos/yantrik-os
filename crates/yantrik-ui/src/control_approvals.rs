//! Asking the person — the three actions a caller gets, and the two buttons only a person has.
//!
//! # The shape of it
//!
//! ```text
//!   mind ──request_approval──▶ shell ──▶ a card on screen
//!                                            │
//!                                   a person presses Allow
//!                                            │
//!   mind ──approval_status───▶ shell ────────┘  → "granted"
//!   mind ──consume_approval──▶ shell            → the grant is burned, once
//!   mind ──act on the app────▶ the app          → the action actually runs
//! ```
//!
//! # A mind must not be able to approve itself
//!
//! All three actions are graded `safe`, and they are safe for the same reason: **none of them
//! decides anything.** Raising a request puts a question on screen. Polling reads an answer
//! somebody else gave. Consuming spends a grant that already exists and can only make it worth
//! less. There is deliberately NO action on this surface that grants or denies — the only path
//! to [`crate::approvals::grant`] is the Slint callback a click arrives on, and
//! `published_actions_cannot_grant` below reads every `control*.rs` in this crate and fails if
//! an action whose name reads like approve/grant/allow/deny appears anywhere but here.
//!
//! If you are here to add "auto-approve for trusted callers": that is `tool_permission` in the
//! machine's settings, which is the owner's standing policy, set at the keyboard. It is not a
//! grant, and it belongs in `yantrik-app-runtime::control`, where the machine ceiling already
//! lives.
//!
//! # The machine ceiling is above all of this
//!
//! An approval cannot exceed `tool_permission`. It is not enforced here — it is enforced in
//! `yantrik-app-runtime::control`, in the dispatch every `app.act` crosses, so an approval this
//! module minted for a `dangerous` action on a `standard` machine still gets a `CEILING:`
//! refusal from the app itself. What this module does is publish the ceiling in `describe shell`
//! so the MCP bridge can decline to *ask* a question the machine will refuse to answer. A person
//! asked a pointless question learns that the prompt is noise.

use std::cell::RefCell;
use std::sync::Mutex;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use yantrik_app_runtime::control::{Action, App as ControlSurface, Param};

use crate::approvals::{self, Card, Status};
use crate::App;

/// How often the cards are re-read so an expiry reaches the screen.
///
/// Nothing pushes an expiry: it is a fact about the clock, so something has to look. A second is
/// fine — the sync is skipped entirely when nothing has changed, and with no request waiting
/// that is every tick.
const REFRESH: Duration = Duration::from_secs(1);

// ── What the socket may do ──────────────────────────────────────────

/// Add the three approval actions to the shell's surface.
pub fn actions(surface: ControlSurface, ui: &App) -> ControlSurface {
    let request_ui = ui.as_weak();
    let status_ui = ui.as_weak();
    let consume_ui = ui.as_weak();
    let mode_ui = ui.as_weak();
    let audit_ui = ui.as_weak();
    let audit_view_ui = ui.as_weak();
    let menu_ui = ui.as_weak();

    surface
        .action(
            // `safe`, and the description says what makes it safe: asking is not deciding.
            //
            // The description is also where a mind learns the flow, because it is the only
            // documentation it will ever read. It used to be told, by a model improvising, to
            // "send /approve in the chat panel" — a prompt that did not exist. Saying the three
            // steps here is what stops that being invented again.
            Action::new(
                "request_approval",
                "Ask the person at this machine to allow one action, once. Puts a card on their \
                 screen showing who is asking, what the action does, and every argument. Answer \
                 is `{request_id, status: \"pending\"}`; poll approval_status until it is \
                 granted or denied, then consume_approval before running the action. Asking is \
                 not being allowed: only a person pressing Allow creates a grant, and nothing on \
                 this surface can create one.",
            )
            .risk("safe")
            .arg(Param::text("app").describe("The app the action belongs to, e.g. calendar"))
            .arg(Param::text("action").describe("The action's exact name, e.g. delete_event"))
            .arg(
                Param::text("grade")
                    .describe("The action's permission grade as os_describe reports it: safe, standard, sensitive or dangerous"),
            )
            .arg(
                Param::object("args_json")
                    .optional()
                    .describe("The exact arguments, as a JSON object. The grant is bound to these — a different value later is a different action and will be refused"),
            )
            .arg(
                Param::text("purpose")
                    .optional()
                    .describe("The action's own published description, so the card says what it does in the app's words"),
            )
            .arg(
                Param::text("requester")
                    .optional()
                    .describe("What to call you on the card, e.g. hermes. It is shown as `says \
                               the caller`: nothing checks it, and it grants nothing. Beside it \
                               the card shows the program this machine worked out for itself \
                               from the socket, which is not taken from here"),
            ),
            move |args| {
                let app = required(args, "app")?;
                let action = required(args, "action")?;
                let grade = required(args, "grade")?;
                let parsed = args_value(args.get("args_json"))?;
                let purpose = text(args.get("purpose"));
                let requester = {
                    let given = text(args.get("requester"));
                    // Unattributed is worse than wrong: the person is being asked to trust
                    // something, and "something on this machine" is at least honest about how
                    // much they have been told.
                    if given.is_empty() { "an unnamed caller".to_string() } else { given }
                };

                // Everything the decision table says, not only the plan-mode half of it.
                //
                // This used to consult `decide` and then act on the answer only when the mode
                // was `plan`, which meant a caller that skipped the bridge could still get a
                // card raised for an action graded ABOVE `tool_permission` — a question no
                // answer could satisfy, because the app's own runtime refuses it whatever the
                // person clicks. The rule in both design notes is that nothing above the machine
                // ceiling is ever put in front of a person, and until now only the bridge kept
                // it. The socket is reachable without the bridge, so the shell keeps it too.
                //
                // The grade is checked first, because the grade is the one thing the caller
                // declares that the decision actually turns on.
                let (grade, grade_note, published_purpose, naming) =
                    match settle_grade(&app, &action, &grade) {
                        Ok(settled) => settled,
                        Err(why) => return Err(why),
                    };
                // Bound to the id the app publishes under, not the spelling the caller used. The
                // app's own dispatch spends the grant now (issue #116) and it spends it under that
                // id — so a card asked for `container-manager` would otherwise be allowed and then
                // refused as "approved for app `container-manager`, not `containers`". One app,
                // one name on the card, one name on the grant, one name on a session rule.
                let app = surface_for(&app).unwrap_or(app);

                // ── Agents catalog: an agent held to a role's reach is never shown asking for an
                // act its reach refuses — its door would refuse it whatever the person pressed.
                // Held on the arguments the card would be bound to, since one act is decided by
                // them: a role may open an app its reach names (#195).
                if let Some(Ok(agent)) = crate::control_agent_terminal::calling_agent() {
                    crate::control_agents::within_reach(&agent, &app, &action, &grade, &parsed)?;
                }

                // Whether the app's own sentence says this cannot be taken back. `auto` asks
                // about those exactly as it asks about a `dangerous` action — the defect of
                // 21 September, where `calendar.delete_event` ("It is not recoverable", graded
                // `sensitive`) ran in auto with nobody asked while the mode menu promised the
                // destructive ones still ask.
                //
                // EITHER sentence saying so is enough. The published one is the one that counts
                // and is read from the app; the caller's is kept in the test because it can only
                // ever tighten — a requester who adds "this cannot be undone" to a purpose has
                // asked for a card, which is not a thing worth refusing them — and because the
                // shell surface publishes no description here to read.
                let cannot_be_undone = approvals::unrecoverable(&published_purpose)
                    || approvals::unrecoverable(&purpose);
                // And the card shows the app's own words when the caller sent none, so the red
                // warning line, the session-rule offer and the decision above all read one
                // sentence rather than three.
                let purpose =
                    if purpose.trim().is_empty() { published_purpose } else { purpose };

                match crate::mind_mode::decide(&grade, &app, &action, cannot_be_undone) {
                    // The same sentence the bridge relays, from the same function, so a mind
                    // that reached the shell directly and one that came through the bridge hear
                    // one story rather than two.
                    crate::mind_mode::Decision::Refuse { why } => return Err(why),
                    // Nothing to ask about. Answered plainly rather than with a card: a person
                    // shown a question the machine was going to say yes to anyway learns that
                    // the card is noise, which is the failure this whole design is built around.
                    crate::mind_mode::Decision::Run { .. } => {
                        return Ok(serde_json::json!({
                            "status": "not_needed",
                            "app": app,
                            "action": action,
                            "grade": grade,
                            "mode": crate::mind_mode::current().as_str(),
                            "next": "nobody was asked and nobody needs to be: this desktop's \
                                     current mode runs this without a card. Run the action. If \
                                     it ran unasked, call record_unasked_action afterwards.",
                        }))
                    }
                    crate::mind_mode::Decision::Ask => {}
                }

                let mut verified = who_is_asking(&requester);
                if !grade_note.is_empty() {
                    verified.discrepancies.push(grade_note);
                }
                let agent = verified.agent.clone();

                // And one line beside the arguments saying what their handles are, from the same
                // `describe` the grade came from (#54) — empty for a call that names nothing by
                // handle and for an app that publishes no index. Display only: `parsed` goes to
                // the store exactly as the caller sent it, because the grant is bound to those
                // bytes and this line must never become one more thing approved beside them.
                let target = target_line(&parsed, &naming);
                let asked = approvals::request(
                    &requester, verified, &app, &action, parsed, &grade, &purpose, &target,
                )?;

                // And in the pane of the agent that asked: the same card, under the same request
                // id, so answering it there answers it here (design decision 4). Only for the
                // agent its token named and the kernel vouched for — `agent` is empty otherwise,
                // and nothing the request says can fill it.
                draw_in_pane(&agent, &asked.id, &app, &action);

                // Straight onto the screen. The handler is already on the UI thread — this is
                // the same turn of the event loop that accepted the request — so the card is up
                // before the caller's reply leaves the socket, and a poll that arrives
                // immediately can never see a request the person has not been shown.
                if let Some(ui) = request_ui.upgrade() {
                    sync(&ui);
                }

                // Drawn is not seen. The first time this ran on a real machine, the mind had
                // Calendar open and focused; the card was drawn in the shell's window, top
                // right, and the Calendar window covered it completely — only the card's orange
                // border showed past the edge. The person would never have seen it and the
                // request would have expired on its own. The shell is an ordinary toplevel to
                // labwc, so it has to ask to come forward, exactly as `open_lens` does for the
                // same reason (see its comment in control.rs — the Lens once opened underneath
                // Notes). Off the UI thread: wlrctl is a process.
                //
                // Only for a question the person has not already been shown. A repeat of an
                // identical pending request hands back the card that is already up, and raising
                // the shell again for it would let anything that can call a `safe` action hold
                // somebody's screen by asking the same thing in a loop.
                if asked.fresh {
                    take_the_screen();
                    // And say so where everything else is said. The card is on screen for two
                    // minutes; the notification is what is still there afterwards, so a person
                    // who was away learns that a mind asked for something and got no answer.
                    // Critical, so Do Not Disturb does not swallow a question.
                    crate::wire::notifications::approval_waiting(&requester, &app, &action);
                }

                Ok(serde_json::json!({
                    "request_id": asked.id,
                    "status": asked.status.as_str(),
                    "expires_in_secs": approvals::REQUEST_TTL.as_secs(),
                    "next": "poll approval_status; on `granted` call consume_approval with the \
                             identical app, action and args_json, and run the action only if \
                             that succeeds",
                }))
            },
        )
        .action(
            Action::new(
                "approval_status",
                "Where one approval request stands: pending, granted, denied, expired or \
                 consumed. `denied` is an answer, not a failure — do not ask again unless the \
                 person brings it up. `expired` means nobody answered in time.",
            )
            .risk("safe")
            .arg(Param::text("request_id").describe("The id request_approval answered with")),
            move |args| {
                let id = required(args, "request_id")?;
                let Some(status) = approvals::status(&id) else {
                    return Err(format!(
                        "no approval request `{id}` on this machine. Requests are held in memory, \
                         so a shell restart drops them; ask again."
                    ));
                };
                // Read the same list the card is drawn from, so a status and a screen cannot
                // disagree about the same request.
                let card = approvals::cards().into_iter().find(|c| c.id == id);
                if let Some(ui) = status_ui.upgrade() {
                    sync_if_changed(&ui);
                }
                let mut answer = serde_json::json!({
                    "request_id": id,
                    "status": status.as_str(),
                });
                if let Some(card) = card {
                    answer["age_secs"] = card.age_secs.into();
                    if status == Status::Pending {
                        answer["expires_in_secs"] = approvals::REQUEST_TTL
                            .as_secs()
                            .saturating_sub(card.age_secs)
                            .into();
                    }
                }
                Ok(answer)
            },
        )
        .action(
            Action::new(
                "consume_approval",
                "Spend a grant. Succeeds exactly once, and only if the person granted this \
                 request and the app, action and arguments are byte-for-byte what they were \
                 shown (key order aside). Anything else is refused, and the refusal says which \
                 part differed. Call it immediately before the action, and run the action only \
                 if it succeeded.",
            )
            .risk("safe")
            .arg(Param::text("request_id"))
            .arg(Param::text("app"))
            .arg(Param::text("action"))
            .arg(
                Param::object("args_json")
                    .optional()
                    .describe("The same JSON object the request carried"),
            ),
            move |args| {
                let id = required(args, "request_id")?;
                let app = required(args, "app")?;
                let action = required(args, "action")?;
                let parsed = args_value(args.get("args_json"))?;
                // A grant is for the agent it was asked for: one agent cannot spend another's —
                // a child handed its parent's request id starts with no grants all the same.
                grant_belongs(
                    &id,
                    approvals::agent_of(&id).as_deref().unwrap_or_default(),
                    &crate::control_agent_terminal::calling_agent(),
                )?;
                approvals::consume(&id, &app, &action, &parsed)?;
                if let Some(ui) = consume_ui.upgrade() {
                    sync(&ui);
                }
                Ok(serde_json::json!({
                    "request_id": id,
                    "consumed": true,
                    "authorises": format!("{app}.{action}"),
                    "note": "one action, once. Run it now; this grant is spent.",
                }))
            },
        )
        .action(
            // `safe`, and it is the same argument as the three above: it does not decide
            // anything a person has not already decided. It can only take permission AWAY.
            //
            // Published because a mind putting itself into plan mode is a genuinely useful
            // thing — "check my work before I touch anything" — and harmless by construction.
            // Raising is refused here and the refusal says where a person does it, because a
            // mind told only "no" invents a way: the whole approval card exists because one
            // told somebody to edit an environment variable.
            Action::new(
                "set_mind_mode",
                "Tighten what you may do on this desktop without being asked. Four modes, \
                 loosest first: `bypass` (nothing is asked), `auto` (only destructive actions \
                 are asked about), `ask` (anything that matters is asked about), `plan` (read \
                 only — every change is refused). You can only move DOWN this list. A request to \
                 loosen it is refused: that is the person's decision, made at the keyboard, and \
                 `plan` is the useful one to set yourself before a long piece of work you want \
                 checked first.",
            )
            .risk("safe")
            .arg(
                Param::text("mode")
                    .describe("plan, ask or auto — and only if it is tighter than the current mode"),
            ),
            move |args| {
                let wanted = required(args, "mode")?;
                let settled = crate::mind_mode::lower_from_socket(&wanted)?;
                if let Some(ui) = mode_ui.upgrade() {
                    publish_mode(&ui);
                }
                tracing::info!(mode = settled.as_str(), "a caller tightened the mind mode");
                Ok(serde_json::json!({
                    "mode": settled.as_str(),
                    "means": settled.meaning(),
                    "note": "only the person at this machine can loosen this again.",
                }))
            },
        )
        .action(
            // `safe` for the narrowest possible reason: it writes a line down. It authorises
            // nothing, it unlocks nothing, and a caller that lies to it has lied in a log rather
            // than gained anything — which is why it is the bridge that calls it, immediately
            // after an action that nobody was asked about, rather than the shell trying to
            // observe something it cannot see.
            Action::new(
                "record_unasked_action",
                "Write down one action that ran WITHOUT the person being asked — because the \
                 desktop is in auto or bypass mode, or because a session rule covers it. Call it \
                 straight after the action, with what actually happened. It records; it cannot \
                 authorise anything, and not calling it does not stop anything running. The \
                 person reads these in the mode menu and in ~/.local/share/yantrik/mind-audit.jsonl.",
            )
            .risk("safe")
            .arg(Param::text("app"))
            .arg(Param::text("action"))
            .arg(Param::text("grade").describe("The action's grade, as os_describe reports it"))
            .arg(
                Param::text("mode")
                    .optional()
                    .describe("The mode it ran under: auto, bypass, or rule"),
            )
            .arg(
                Param::object("args_json")
                    .optional()
                    .describe("The exact arguments it ran with, as a JSON object"),
            )
            .arg(
                Param::text("requester")
                    .optional()
                    .describe("What to call yourself in the record. Self-declared; the log also \
                               keeps what this machine established from the socket, separately"),
            )
            .arg(
                Param::text("outcome")
                    .optional()
                    .describe("What happened: ok, failed, or a short phrase"),
            ),
            move |args| {
                let app = required(args, "app")?;
                let action = required(args, "action")?;
                let grade = required(args, "grade")?;
                let parsed = args_value(args.get("args_json"))?;
                let mode = {
                    let given = text(args.get("mode"));
                    if given.is_empty() { crate::mind_mode::current().as_str().to_string() } else { given }
                };
                let requester = {
                    let given = text(args.get("requester"));
                    if given.is_empty() { "an unnamed caller".to_string() } else { given }
                };
                let outcome = {
                    let given = text(args.get("outcome"));
                    // "It ran and nobody said how it went" is worse to read than an honest
                    // blank, so it is named rather than left empty.
                    if given.is_empty() { "not reported".to_string() } else { given }
                };

                let entry = crate::mind_mode::record(
                    &mode, &requester, &who_is_asking(&requester), &app, &action, &parsed,
                    &grade, &outcome,
                );
                if let Some(ui) = audit_ui.upgrade() {
                    publish_mode(&ui);
                }
                tracing::info!(
                    mode = %mode, app = %app, action = %action, outcome = %outcome,
                    "an action ran without the person being asked"
                );
                Ok(serde_json::json!({ "recorded": entry.line() }))
            },
        )
        .action(
            // `safe` for the same reason `record_unasked_action` is: it shows a person something
            // they already own. It changes no mode, mints no rule, decides nothing and reveals
            // nothing the caller could not read from `describe shell`'s `mind_audit_recent`. It
            // opens a list.
            //
            // It is published because a notification's button has to be a real call. The shell
            // presses a button on the sender's behalf by calling the named action on that
            // sender's own control surface — Download Manager's "Open folder" is `open_folder`
            // on Download Manager — and the sender of "Bypass ended" is the shell. Without an
            // action here, that button would be a control that does nothing, which is worse
            // than no button.
            Action::new(
                "show_mind_audit",
                "Put the record of actions that ran WITHOUT the person being asked on their \
                 screen — the same list as the mode chip's \"See what it did without asking\". \
                 It shows what is already written down; it changes nothing, allows nothing, and \
                 does not clear anything. `describe shell` carries the same entries under \
                 `mind_audit_recent` if you only want to read them.",
            )
            .risk("safe"),
            move |_args| {
                let Some(ui) = audit_view_ui.upgrade() else {
                    return Err("the shell is gone".to_string());
                };
                // Suppressed on boot, lock, login and onboarding, which is the same list the
                // approval card and the mode menu use and for the same reason: a list of what
                // this machine did while nobody was watching is readable by whoever happens to
                // be standing in front of a locked screen.
                if MENU_NEVER_ON.contains(&ui.get_current_screen()) {
                    return Err(
                        "this machine is locked, so the record of unasked actions was not put on \
                         screen. It is all still there: unlock it and open the mode chip in the \
                         status bar, or read `mind_audit_recent` in describe shell."
                            .to_string(),
                    );
                }
                ui.set_mind_menu_confirming(false);
                ui.set_mind_menu_audit_open(true);
                ui.set_mind_menu_open(true);
                publish_mode(&ui);
                Ok(serde_json::json!({
                    "showing": "the record of unasked actions",
                    "entries": crate::mind_mode::recent(crate::mind_mode::AUDIT_PUBLISHED).len(),
                    "note": "it is on the person's screen now, over the mode chip; nothing was \
                             changed. `close_mind_menu` puts it away, and so does going to \
                             another screen.",
                }))
            },
        )
        .action(
            // `safe`: it takes something off the screen that `show_mind_audit` — or the person —
            // put there, and nothing else. The mode, its rules and the record are untouched; a
            // bypass confirmation left half-answered is dropped unanswered, which leaves the
            // mode exactly as it was. Closing is what a caller that opened the menu owes the
            // person: before this, the only way to close it was a pointer, and a person away
            // from the desk came back to a menu over their screen (#184).
            Action::new(
                "close_mind_menu",
                "Put away the mode menu — the one `show_mind_audit` opens over the mode chip, with \
                 the record of unasked actions in it. It changes nothing: the mode, its rules and \
                 the record stay as they are. `describe shell` says whether it is open under \
                 `mind_menu`.",
            )
            .risk("safe"),
            move |_args| {
                let Some(ui) = menu_ui.upgrade() else {
                    return Err("the shell is gone".to_string());
                };
                let was = mind_menu_for_describe(
                    ui.get_mind_menu_open(),
                    ui.get_mind_menu_audit_open(),
                    ui.get_mind_menu_confirming(),
                    ui.get_current_screen(),
                );
                ui.set_mind_menu_open(false);
                ui.set_mind_menu_confirming(false);
                ui.set_mind_menu_audit_open(false);
                Ok(serde_json::json!({
                    "closed": was["open"] == true,
                    "was": was,
                    "note": if was["open"] == true {
                        "the mode menu is put away; nothing was changed."
                    } else {
                        "the mode menu was not open; nothing was changed."
                    },
                }))
            },
        )
}

/// What `describe shell` says under `mind_menu`: whether the mode menu is on the screen, and which
/// part of it — the modes, the record of unasked actions, or a bypass waiting to be confirmed.
///
/// `open` is whether it is drawn, not only whether it was asked for: the menu is never drawn over
/// the boot, lock, login or onboarding screens, so it is not open there whatever the flag says.
pub fn mind_menu_for_describe(open: bool, audit: bool, confirming: bool, screen: i32) -> serde_json::Value {
    let drawn = open && !MENU_NEVER_ON.contains(&screen);
    let showing = match (drawn, confirming, audit) {
        (false, _, _) => serde_json::Value::Null,
        (true, true, _) => "bypass confirmation".into(),
        (true, false, true) => "the record of unasked actions".into(),
        (true, false, false) => "the modes".into(),
    };
    serde_json::json!({
        "open": drawn,
        "showing": showing,
        "close_with": if drawn { "close_mind_menu" } else { "" },
    })
}

/// The screens nothing about the mind's permissions is drawn over: boot, onboarding, lock and
/// login. The same list `show_mind_audit` refuses on and app.slint's `if` for the menu names.
const MENU_NEVER_ON: [i32; 4] = [0, 2, 3, 32];

// ── The grade, which the caller also declares ───────────────────────
//
// `request_approval(app, action, grade, …)` takes the grade as an argument, which makes it the
// same kind of thing as the requester's name: something the caller said. It cannot raise
// privilege — the app re-reads its own grade inside `app.act` and refuses above the ceiling
// regardless — but it decides what this shell does with the request, and an understated grade
// turns "refuse without asking" into a card, or a card into silence. Since the shell is now
// establishing facts about the caller, it establishes this one too.

/// How long the shell will wait for another app to say what one of its actions is graded.
///
/// This runs on the UI thread, inside an action handler, whose own budget is `UI_ROUNDTRIP` =
/// 3s. Half a second leaves room for the rest of the handler and is already ten times what a
/// local `app.describe` costs; a surface slower than that is one the caller should hear about
/// rather than wait on, and `SyncRpcClient`'s breaker makes the second attempt free.
const GRADE_LOOKUP: Duration = Duration::from_millis(500);

/// How much of the "you said X, the app says Y" sentence fits on one elided card row.
const NOTE_CHARS: usize = 62;

/// What an app says its own ids stand for: handle → the thing it names, in the app's words.
///
/// Read from `describe`'s `naming` key, which the calendar publishes for every event it has
/// (#54) and every other app may publish the same way without the shell changing. Empty for an
/// app that publishes none — which is the ordinary case today and simply draws no row.
type Naming = std::collections::BTreeMap<String, String>;

/// The grade to act on, the note the card owes the person if it is not what was declared,
/// the app's own sentence about the action, and what the app says its own ids name.
///
/// Refuses rather than guesses. An app this desktop does not have, an action it does not
/// publish, or a surface that will not say — none of those is a reason to put a card in front of
/// somebody, because there is nothing behind it for them to allow.
///
/// The purpose comes back with the grade because the decision now turns on it too: `auto` asks
/// about an action whose purpose says it cannot be undone. Read from the app rather than taken
/// from the request, for the same reason the grade is — `request_approval` takes a `purpose`
/// argument, and a caller that simply left it out would otherwise have talked the desktop into
/// running `calendar.delete_event` unasked by saying nothing.
fn settle_grade(
    app: &str,
    action: &str,
    claimed: &str,
) -> Result<(String, String, String, Naming), String> {
    let (published, purpose, naming) = published_detail(app, action)?;
    let note = grade_note(claimed, &published);
    Ok((published, note, purpose, naming))
}

/// What the target app itself says one of its actions is graded, what it is for, and — beside
/// that — what the app says its own ids name.
///
/// The purpose and the naming are empty for the shell's own surface: the local registry shortcut
/// below publishes a grade and nothing else, and reaching the description would mean a new
/// function in `yantrik-app-runtime`, which this change does not own. Nothing published by the
/// shell matches the "cannot be undone" wording today — `files_delete` says "Move a file or
/// folder to recoverable Trash" — and the caller ORs this with what the request declared, so a
/// shell action that acquired such a sentence would still be asked about as long as the bridge
/// kept relaying the purpose it reads out of `describe`. And no shell action takes an opaque id
/// today either, so there is nothing for a naming index to resolve.
fn published_detail(app: &str, action: &str) -> Result<(String, String, Naming), String> {
    published_detail_in(
        &yantrik_ipc_transport::server::socket_dir(),
        &crate::apps::Catalogue::shared().get(),
        app,
        action,
    )
}

/// [`published_detail`], against a catalogue and a socket directory the caller names.
fn published_detail_in(
    dir: &std::path::Path,
    installed: &[crate::apps::DesktopEntry],
    app: &str,
    action: &str,
) -> Result<(String, String, Naming), String> {
    let Some(surface) = surface_in(app, installed, dir) else {
        return Err(format!(
            "there is no app called `{app}` on this desktop, so nothing was put in front of the \
             person. `os_apps` lists the names this machine uses."
        ));
    };

    // The shell asking the shell. Over the socket this would be a call the shell's own UI thread
    // has to answer while it is blocked making it — so it is read straight out of the registry
    // that thread already holds. The registry carries grades and nothing else: the shell's own
    // actions take paths, prompts and names, no opaque handle that needs a naming index.
    if surface == "shell" {
        return yantrik_app_runtime::control::published_grade(action)
            .map(|grade| (grade.to_string(), String::new(), Naming::new()))
            .ok_or_else(|| {
                format!(
                    "`shell` publishes no action called `{action}`, so there is nothing to ask \
                     about. Read `os_describe shell` for what it does publish."
                )
            });
    }

    // The window's socket, and when the window is shut, the service's — which publishes the same
    // surface under the same id and meets the same gate (issue #161). System Monitor's
    // `kill_process` is the case: with the window closed, `yos act system-monitor kill_process`
    // reaches `system-monitor.sock`, is refused for want of a grant, and asking for that grant
    // answered "not running … open it first" — so the action could never be approved at all.
    let Some(address) = surface_address(dir, &surface) else {
        return Err(format!(
            "`{app}` is not running — neither its window nor a service answers as `{surface}` — \
             so this machine could not check what `{action}` is graded and did not put a card in \
             front of the person. Open it first."
        ));
    };
    let reply = yantrik_ipc_transport::SyncRpcClient::new(&address)
        .with_timeout(GRADE_LOOKUP)
        .call("app.describe", serde_json::json!({}))
        .map_err(|e| {
            format!(
                "`{app}` did not say what `{action}` is graded ({}), so nothing was put in front \
                 of the person. A grade nobody published is not a grade this machine will act on.",
                e.message
            )
        })?;

    // One lookup for all three facts. Two would be two `app.describe` round trips on the UI
    // thread for one card, and two chances for the grade, the sentence beside it and the names
    // of its ids to come from different revisions of the same app.
    let published = reply["actions"]
        .as_array()
        .and_then(|list| list.iter().find(|a| a["name"].as_str() == Some(action)))
        .and_then(|a| {
            a["permission"].as_str().map(|grade| {
                (grade.to_string(), a["description"].as_str().unwrap_or_default().to_string())
            })
        })
        .ok_or_else(|| {
            format!(
                "`{app}` publishes no action called `{action}`, so there is nothing to ask about \
                 and nothing was put in front of the person."
            )
        })?;
    Ok((published.0, published.1, naming_in(&reply)))
}

/// The app's own id→name index, from `describe`'s `state.naming`.
///
/// An app publishes it when its actions take handles a person cannot read (#54); an entry whose
/// value is not a string is skipped rather than stringified, because a number the app chose to
/// index under an id is the app confused its own surface, and a card built on that guess would
/// be the shell vouching for a sentence the app never wrote.
fn naming_in(reply: &serde_json::Value) -> Naming {
    reply["state"]["naming"]
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(handle, name)| {
                    name.as_str().map(|name| (handle.clone(), name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How much of the handle itself the naming line echoes before an ellipsis.
///
/// Enough of a uuid7 to tell one event from another on a day's calendar — the line is a pointer
/// into the argument box above it, not a second copy of it, and a full uuid echoed in front of
/// the name would push the name, which is the point of the row, off the end of the elided line.
const TARGET_HANDLE_CHARS: usize = 8;

/// The line that says what a handle in the arguments is, or empty when nothing in the call
/// is one the app has a name for.
///
/// #54: the card for `calendar.delete_event {"id": "01a0c718-…"}` said only the uuid. A person
/// asked "may this be deleted?" cannot answer to a handle — by title and date the same card
/// reads fine; it is the id route, the reliable one the action recommends, that goes opaque.
/// The app knows what its ids stand for and says so on the `describe` this handler already
/// makes one round trip for; this reads the argument values against that index and says what
/// matches: `id 01a0c718… is “Dentist, Fri 25 Sep 13:00”`.
///
/// It is a sentence about the arguments, drawn beside them, never one more thing the grant
/// binds to — the argument box stays byte-for-byte what [`crate::approvals::consume`] compares.
/// Hits are joined in the order the box lists them, so the two rows read top-to-bottom alike,
/// and the row is one line because the card's height is arithmetic.
fn target_line(args: &serde_json::Value, naming: &Naming) -> String {
    let Some(map) = args.as_object() else { return String::new() };
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    keys
        .iter()
        .filter_map(|key| {
            let handle = map[*key].as_str()?;
            let name = naming.get(handle)?;
            let shown: String = handle.chars().take(TARGET_HANDLE_CHARS).collect();
            let head = if shown.chars().count() < handle.chars().count() {
                format!("{shown}\u{2026}")
            } else {
                shown
            };
            Some(format!("{key} {head} is \u{201c}{name}\u{201d}"))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Where a surface answers right now: its window's socket, or else its service's.
///
/// The window first, as every client resolves a name (docs/surface-protocol.md, "Resolving a
/// name"): it is what the person is looking at, and it is the one an act would reach. A socket
/// file nobody listens on is a closed window, not an answer.
fn surface_address(dir: &std::path::Path, surface: &str) -> Option<String> {
    [format!("app-{surface}.sock"), format!("{surface}.sock")]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| {
            #[cfg(unix)]
            {
                std::os::unix::net::UnixStream::connect(path).is_ok()
            }
            #[cfg(not(unix))]
            {
                path.exists()
            }
        })
        .map(|path| path.to_string_lossy().into_owned())
}

/// The control surface an app name answers on.
///
/// The desktop's catalogue first: `shell` and `yantrik` are the desktop, a screen is part of
/// it, and every app that declares a surface in its `.desktop` file answers to its id, its
/// aliases and what it is called — "Downloads" is described as `download-manager`, an approval
/// asked for `container-manager` (the app's name everywhere but on its socket) is bound to
/// `containers`. Then a surface answering under its own name without being in the catalogue at
/// all, which is not a reason to pretend it does not exist: an app's window, or a service.
fn surface_for(app: &str) -> Option<String> {
    surface_in(
        app,
        &crate::apps::Catalogue::shared().get(),
        &yantrik_ipc_transport::server::socket_dir(),
    )
}

/// [`surface_for`], against a catalogue and a socket directory the caller names.
fn surface_in(
    app: &str,
    installed: &[crate::apps::DesktopEntry],
    dir: &std::path::Path,
) -> Option<String> {
    let key = app.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    crate::wire::dock::surface_for(&key, installed).or_else(|| {
        let folded = crate::apps::fold_name(&key);
        (crate::apps::is_surface_name(&folded) && surface_address(dir, &folded).is_some())
            .then_some(folded)
    })
}

/// The sentence the card owes the person when the declared grade is not the published one.
///
/// Both directions are said, because either way the card is about to show a grade the caller did
/// not name and a person comparing the two should not have to wonder. The understated direction
/// is the one that matters — it is how a `dangerous` action would have been asked about as
/// though it were routine — and it is why this is checked at all.
fn grade_note(claimed: &str, published: &str) -> String {
    let claimed = claimed.trim();
    if claimed.eq_ignore_ascii_case(published) {
        return String::new();
    }
    let note = format!("Caller said `{claimed}`; the app publishes `{published}`.");
    if note.chars().count() <= NOTE_CHARS {
        return note;
    }
    let head: String = note.chars().take(NOTE_CHARS).collect();
    format!("{head}\u{2026}")
}

// ── The claim, and the fact beside it ───────────────────────────────

/// What this machine can establish about whoever is on the socket right now.
///
/// **Called from inside an action handler and nowhere else.** The pid comes from a thread-local
/// that `yantrik-app-runtime::control` installs for the duration of one dispatch, so anywhere
/// else it is either empty or — worse — somebody else's request. It is also read *now* rather
/// than when the card is drawn: the direct peer of an MCP-borne request is `python3 yos`, which
/// runs one JSON-RPC call and exits, so a `/proc` walk a second later finds nothing.
///
/// `claimed` is only used to decide whether the two disagree. It never becomes part of the
/// verified answer; that is the entire point of the split.
fn who_is_asking(claimed: &str) -> approvals::Verified {
    let mut verified = who_is_calling(claimed);
    // Which of the person's agents it is for: from the token beside the arguments, checked against
    // the same kernel-verified caller, never from anything the request says (design decision 4).
    let (agent, doubt) = agent_fact(crate::control_agent_terminal::calling_agent());
    verified.agent = agent;
    verified.discrepancies.extend(doubt);
    verified
}

/// The agent a call is for, as a card and a log may show it, and the sentence the card owes the
/// person when a token came and was not believed. Never the token: only the agent it names.
fn agent_fact(calling: Option<Result<crate::agents::AgentId, String>>) -> (String, Option<String>) {
    match calling {
        None => (String::new(), None),
        Some(Ok(agent)) => (agent.to_string(), None),
        Some(Err(why)) => {
            tracing::info!(reason = %why, "an agent token came with a request and was not believed");
            (String::new(), Some(UNBELIEVED_TOKEN.to_string()))
        }
    }
}

/// What the card says about a request whose agent token did not check out. One line, like every
/// discrepancy, and no detail of the token.
const UNBELIEVED_TOKEN: &str = "Its agent token was not issued to it; no agent's pane shows this.";

/// Draw a request in the pane of the agent it is for, when it is for one. `agent` is the verified
/// agent — [`who_is_asking`]'s, from the token — and empty means no pane: a caller that runs as no
/// agent, or one whose token was not believed, is asked in the Lens alone. Returns whether a pane
/// got it.
fn draw_in_pane(agent: &str, request: &str, app: &str, action: &str) -> bool {
    if agent.is_empty() {
        return false;
    }
    crate::agents::store().approval_asked(&crate::agents::AgentId(agent.to_string()), request, &format!("{app}.{action}"));
    true
}

/// May the caller spend request `id`, which was asked for `asked_for` (empty: for no agent)?
///
/// A caller that runs as no agent — the person's own `yos act`, or an app's dispatch spending the
/// grant it was handed (#116) — is not told apart here and is let through, as before. A caller
/// that presented a token is held to it: a token that was not believed spends nothing, and an
/// agent spends only what was asked for it. A child agent handed its parent's request id is
/// refused, because a child starts with no grants.
fn grant_belongs(
    id: &str,
    asked_for: &str,
    calling: &Option<Result<crate::agents::AgentId, String>>,
) -> Result<(), String> {
    match calling {
        None => Ok(()),
        Some(Err(_)) => Err(format!(
            "the agent token that came with this call was not believed, so `{id}` was not spent. \
             Nothing was authorised."
        )),
        Some(Ok(agent)) if agent.0 == asked_for => Ok(()),
        Some(Ok(_)) => Err(format!(
            "`{id}` was asked for {}, not for the agent making this call. A grant is not handed \
             from one agent to another — ask for your own. Nothing was authorised.",
            if asked_for.is_empty() { "by a caller that runs as no agent".to_string() } else { format!("agent `{asked_for}`") }
        )),
    }
}

/// How request `id` came out, for the pane of the agent that asked: `None` while it is waiting.
fn settled_as(id: &str) -> Option<(crate::agents::ApprovalOutcome, String)> {
    use crate::agents::ApprovalOutcome as Pane;
    let (outcome, record) = approvals::outcome(id)?;
    let outcome = match outcome {
        approvals::Outcome::Allowed => Pane::Allowed,
        approvals::Outcome::Denied => Pane::Denied,
        approvals::Outcome::Unanswered => Pane::Expired,
        approvals::Outcome::Withdrawn => Pane::Withdrawn,
    };
    Some((outcome, record))
}

fn who_is_calling(claimed: &str) -> approvals::Verified {
    let Some(caller) = yantrik_app_runtime::control::caller() else {
        // No credentials at all: a TCP connection on the Windows dev build, or a peer that was
        // gone before `SO_PEERCRED` could be read. The card says "could not be identified"
        // rather than falling back to believing the name, which is what it did before.
        return approvals::Verified {
            line: "could not be identified".to_string(),
            ..Default::default()
        };
    };

    // A different uid is worth saying out loud rather than quietly resolving. The socket
    // directory is 0700 today, so this should be unreachable for anyone but root — which makes
    // it exactly the thing to notice if it ever happens.
    if caller.uid != own_uid() {
        tracing::warn!(
            pid = caller.pid,
            uid = caller.uid,
            "a request arrived on the control socket from another user"
        );
    }

    // One read of the harness registry, used twice: resolving reads it to match an ancestor
    // against an attached mind, and the mismatch check reads it to find the mind the claimed
    // name names. `Host::list` locks and reaps, and this runs on the UI thread.
    let minds = crate::caller_identity::attached_minds();
    let identity = crate::caller_identity::resolve_with(caller.pid, &minds);

    approvals::Verified {
        line: identity.line(),
        exe: identity.exe(),
        pid: identity.pid(),
        attached_mind: identity.attached_mind.clone().unwrap_or_default(),
        discrepancies: {
            let said = crate::caller_identity::mismatch(claimed, &identity, &minds);
            if said.is_empty() { Vec::new() } else { vec![said] }
        },
        agent: String::new(),
    }
}

/// This process's own uid, for the comparison above. `libc` is not a dependency of this crate
/// and does not need to become one: the shell's own runtime directory is owned by it.
fn own_uid() -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX);
    }
    #[cfg(not(unix))]
    {
        u32::MAX
    }
}

/// What `describe shell` publishes under `pending_approvals`.
///
/// Every argument is shown. That is deliberate and it is not a leak of anything a caller does
/// not already have: seeing a request tells you what was asked, and consuming it still needs a
/// grant that only a click creates. What it buys is worth more — a second mind, or a test, can
/// see that the machine is waiting on a person rather than hung.
///
/// `requester` and `verified` are two keys and not one, in the same order the card draws them.
/// A caller reading this has to be able to see that the first is a claim and the second is not.
pub fn pending_for_describe() -> serde_json::Value {
    serde_json::Value::Array(
        approvals::pending()
            .into_iter()
            .map(|card| {
                serde_json::json!({
                    "id": card.id,
                    "requester": card.requester,
                    "verified": card.verified.to_json(),
                    "app": card.app,
                    "action": card.action,
                    "grade": card.grade,
                    "age_secs": card.age_secs,
                })
            })
            .collect(),
    )
}

/// What `describe shell` publishes under `mind_mode`.
///
/// The bridge reads this on the same `describe shell` it already reads the ceiling from, and
/// makes the run/ask/refuse decision itself — one read per `os_act` rather than a second round
/// trip to ask the shell to decide. That means the table lives in two places, which is a real
/// cost and is written down in `design/mind-modes-2026-09-21.md`; the Rust one in
/// `mind_mode::Modes::decide` is the definition and the one with the tests.
pub fn mind_mode_for_describe() -> serde_json::Value {
    crate::mind_mode::snapshot()
}

/// The last few things that ran without anybody being asked. See `mind_mode`'s audit section.
pub fn mind_audit_for_describe() -> serde_json::Value {
    crate::mind_mode::recent_for_describe()
}

/// The machine's standing ceiling for callers on the socket, published so the bridge can read it.
///
/// The same value `yantrik-app-runtime::control` enforces, read from the same file by the same
/// function — not `ui.get_settings_tool_permission()`, which is the Settings screen's copy and
/// would be one save behind on a machine where somebody had just tightened it. Publishing the
/// enforced value means a bridge that reads this and a bridge that provokes a `CEILING:` refusal
/// get the same answer.
///
/// It is a file read on the UI thread, which is a thing to be careful about; it is a few hundred
/// bytes, and `describe` already reads the pin list and the app catalogue from disk beside it.
pub fn machine_ceiling() -> String {
    yantrik_app_runtime::control::configured_ceiling()
}

// ── What only a person may do ───────────────────────────────────────

/// Wire the Allow and Deny buttons, and the tick that lets an expiry reach the screen.
///
/// This is the whole of the granting path. Two callbacks, each one line, each reachable only
/// from a `TouchArea` in `intent_lens.slint`. Nothing else in this crate calls
/// `approvals::grant` or `approvals::deny`, and they are `pub(crate)` so nothing outside it can.
pub fn wire(ui: &App) {
    // The apps spend a grant through this shell's `consume_approval` over the socket. This
    // shell's own dispatch cannot — asking itself over its own socket from its own RPC thread is
    // a call that cannot be answered until it returns — so it spends them in-process, through
    // the same store and the same check. Still not a way to grant: `consume` burns what a click
    // created and refuses everything else.
    yantrik_app_runtime::control::spend_grants_with(|id, app, action, args| {
        approvals::consume(id, app, action, args)
    });

    let allow_ui = ui.as_weak();
    ui.on_approval_allow(move |id| {
        let id = id.to_string();
        match approvals::grant(&id) {
            Ok(()) => tracing::info!(request = %id, "a person allowed one action, once"),
            // Not fatal and not silent: the usual cause is a double click, or a card that
            // expired between the paint and the press.
            Err(e) => tracing::info!(request = %id, reason = %e, "Allow did not apply"),
        }
        if let Some(ui) = allow_ui.upgrade() {
            sync(&ui);
        }
    });

    // "Allow for this session" — one click that does two things, in this order.
    //
    // The grant first, because that is what the caller waiting on the socket needs and it is
    // the half that cannot be got any other way. Then the rule, which is what stops the same
    // question coming back. If the rule is refused — the published grade or purpose changed
    // between the paint and the press — the person still got the one action they pressed for,
    // and the refusal is logged rather than silently swallowed.
    let session_ui = ui.as_weak();
    ui.on_approval_allow_session(move |id| {
        let id = id.to_string();
        let card = approvals::card(&id);
        match approvals::grant_for_session(&id) {
            Ok(()) => tracing::info!(request = %id, "a person allowed one action for this session"),
            Err(e) => {
                tracing::info!(request = %id, reason = %e, "Allow for this session did not apply");
                if let Some(ui) = session_ui.upgrade() {
                    sync(&ui);
                }
                return;
            }
        }
        if let Some(card) = card {
            if let Err(e) =
                crate::mind_mode::person_add_rule(&card.app, &card.action, &card.grade, &card.purpose)
            {
                tracing::warn!(request = %id, reason = %e, "no session rule was made for it");
            }
        }
        if let Some(ui) = session_ui.upgrade() {
            sync(&ui);
            publish_mode(&ui);
        }
    });

    let deny_ui = ui.as_weak();
    ui.on_approval_deny(move |id| {
        let id = id.to_string();
        match approvals::deny(&id) {
            Ok(()) => tracing::info!(request = %id, "a person denied one action"),
            Err(e) => tracing::info!(request = %id, reason = %e, "Deny did not apply"),
        }
        if let Some(ui) = deny_ui.upgrade() {
            sync(&ui);
        }
    });

    // ── The mode, and the two things only a person may do to it ──
    //
    // These three callbacks are the ONLY callers of `mind_mode::person_*`, and they are
    // callbacks — a `TouchArea` in `mind_mode_menu.slint`, reached by a pointer. Nothing on the
    // control surface can reach them; `mind_mode_only_a_person_can_raise_the_mode` below reads
    // the source of every `control*.rs` to keep it that way.
    let chosen_ui = ui.as_weak();
    ui.on_mind_mode_chosen(move |mode| {
        let Some(mode) = crate::mind_mode::Mode::parse(&mode) else { return };
        // Bypass has its own callback because it has its own confirmation and its own duration.
        // Letting it arrive here would mean one click could enter it, which is the one mode
        // that must cost a deliberate second answer.
        if mode == crate::mind_mode::Mode::Bypass {
            tracing::warn!("bypass does not arrive through the plain mode chooser");
            return;
        }
        crate::mind_mode::person_set_mode(mode, crate::mind_mode::Bypass::Hour);
        tracing::info!(mode = mode.as_str(), "a person set the mind mode");
        if let Some(ui) = chosen_ui.upgrade() {
            publish_mode(&ui);
        }
    });

    let bypass_ui = ui.as_weak();
    ui.on_mind_bypass_chosen(move |duration| {
        let Some(bypass) = crate::mind_mode::Bypass::parse(&duration) else { return };
        crate::mind_mode::person_set_mode(crate::mind_mode::Mode::Bypass, bypass);
        tracing::warn!(duration = %duration, "a person put this desktop into bypass");
        if let Some(ui) = bypass_ui.upgrade() {
            publish_mode(&ui);
        }
    });

    let revoke_ui = ui.as_weak();
    ui.on_mind_rule_revoked(move |app, action| {
        crate::mind_mode::person_revoke_rule(&app, &action);
        tracing::info!(app = %app, action = %action, "a person revoked a session rule");
        if let Some(ui) = revoke_ui.upgrade() {
            publish_mode(&ui);
        }
    });

    // Where a mind's apps open (#239). A pointer's choice, wired here beside the modes for the
    // same reason they are: nothing on the socket reaches it.
    let mind_view_ui = ui.as_weak();
    ui.on_mind_view_chosen(move |on| {
        if let Err(e) = crate::wire::settings::set_minds_open_in_mind_view(on) {
            tracing::warn!(error = %e, on, "where minds open apps was not saved");
        }
        tracing::info!(on, "a person chose where a mind's apps open");
        if let Some(ui) = mind_view_ui.upgrade() {
            ui.set_mind_view_on(crate::wire::settings::minds_open_in_mind_view());
        }
    });
    ui.set_mind_view_on(crate::wire::settings::minds_open_in_mind_view());

    publish_mode(ui);

    let tick_ui = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, REFRESH, move || {
        if let Some(ui) = tick_ui.upgrade() {
            sync_if_changed(&ui);
            // The countdown on the chip has to move every second while a bypass is running, and
            // a lapsed one has to stop saying "Bypass" — which nothing pushes, because an expiry
            // is a fact about the clock. Republished only when the label actually changes, so a
            // machine in `ask` mode rebuilds nothing on any of these ticks.
            crate::mind_mode::lapse();
            // And say so. The chip changing is not telling anybody: the person this matters
            // most to is the one who chose "1 hour" and left the room, and the chip is the only
            // thing that moved while they were gone. No second timer — the lapse is noticed on
            // the tick that was already looking, and `take_lapse_notice` answers once, so the
            // fifty-nine ticks after it in that minute say nothing.
            if let Some(ended) = crate::mind_mode::take_lapse_notice() {
                tracing::info!(
                    back_to = ended.back_to.as_str(),
                    unasked = ended.unasked,
                    "a bypass ran out on its own"
                );
                crate::wire::notifications::bypass_ended(ended);
            }
            publish_mode_if_changed(&ui);
        }
    });
    // The same keep-alive every timer in `wire::timers` uses: a dropped `Timer` stops.
    std::mem::forget(timer);
}

// ── Getting in front of the person, and getting out of the way again ────
//
// A card the person cannot see is the same as no card: the request expires on its own and they
// are never told anything was asked. So the shell comes forward when a request arrives. The cost
// is that it covers whatever they were using, which is why the window they were in is handed the
// screen back the moment nothing is waiting.

/// The toplevel to hand the screen back to, if it was knowable when the card went up.
///
/// `None` means either nothing is waiting, or the compositor would not say unambiguously which
/// window was in front — in which case the shell stays where it is rather than guessing at a
/// window to throw the person into. See [`crate::windows::front_now`].
static RESTORE_TO: Mutex<Option<String>> = Mutex::new(None);

/// Ask the compositor to bring one toplevel forward. Blocking; call it off the UI thread.
fn focus_toplevel(title: &str) {
    match std::process::Command::new("wlrctl")
        .args(["toplevel", "focus", &format!("title:{title}")])
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => tracing::warn!(
            code = status.code().unwrap_or(-1),
            window = %title,
            "could not bring a window forward for an approval"
        ),
        Err(e) => tracing::warn!(error = %e, "could not run wlrctl for an approval"),
    }
}

/// Note what the person was using, then put the shell in front of it.
///
/// Both halves on one worker thread and in that order, because the reading has to happen before
/// the raise or it reads the shell. It asks the compositor fresh rather than reading the taskbar's
/// cached answer, because that one is up to `COMPOSITOR_TTL` old and the screen is being handed to
/// the window in front NOW. Nothing is recorded if something is already waiting — the
/// shell is already in front by then, so a second reading would capture the shell and the window
/// the person actually came from would be lost.
fn take_the_screen() {
    std::thread::spawn(|| {
        if let Ok(mut slot) = RESTORE_TO.lock() {
            if slot.is_none() {
                *slot = crate::windows::front_now();
            }
        }
        focus_toplevel(crate::windows::SHELL_WINDOW_TITLE);
    });
}

/// Nothing is waiting any more: give the screen back to whatever the person was using.
///
/// Called on every transition to "no pending requests", so it covers a decision and an expiry
/// alike — the person who walked away and came back should find the window they left, not the
/// shell they never answered.
fn give_the_screen_back() {
    let title = match RESTORE_TO.lock() {
        Ok(mut slot) => slot.take(),
        Err(_) => None,
    };
    if let Some(title) = title {
        std::thread::spawn(move || focus_toplevel(&title));
    }
}

// ── Cards onto the screen ───────────────────────────────────────────

thread_local! {
    /// What the screen is showing, so a tick that changes nothing repaints nothing.
    static SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
}

fn fingerprint(cards: &[Card], pane: &str) -> String {
    let cards = cards
        .iter()
        .map(|c| {
            // The age only moves on a card that is still waiting, so a screen with nothing
            // pending settles and stops being rebuilt.
            let age = if c.status == Status::Pending { c.age_secs } else { 0 };
            format!("{}:{}:{age}", c.id, c.status.as_str())
        })
        .collect::<Vec<_>>()
        .join("|");
    // The pane is part of what is shown: walking to the Agents screen, or to another agent
    // there, moves a waiting card between the popup and the pane (#212).
    format!("{cards}|pane:{pane}")
}

/// The agent whose pane is on screen right now, or "" when no pane is: the Agents screen, in
/// its list view, with an agent selected. That pane is where the agent's own card is answered,
/// so the popup must not draw it a second time (#212).
fn pane_agent(ui: &App) -> String {
    if ui.get_current_screen() != crate::wire::agents::SCREEN {
        return String::new();
    }
    let g = ui.global::<crate::AgentsState>();
    if g.get_view() != "list" {
        return String::new();
    }
    g.get_selected().to_string()
}

/// Whether a card is answered in the pane on screen. The pane draws live buttons for exactly
/// the pending cards of the agent it shows (`approval_of` in wire/agents.rs), so this is the
/// same predicate: the verified agent, never anything the request says. A card with no agent
/// — a caller that is no agent, or a token that was not believed — is in no pane and stays on
/// screen.
fn in_the_pane(card: &Card, pane: &str) -> bool {
    !pane.is_empty() && card.verified.agent == pane
}

/// What the screen draws: every decided record, and the oldest pending card that is not being
/// answered in the pane on screen. `cards()` returns the records first and then the pending in
/// order, so the first pending row kept here is the oldest — one card at a time, as before.
fn cards_for_screen<'a>(cards: &'a [Card], pane: &str) -> Vec<&'a Card> {
    let mut out: Vec<&Card> = Vec::new();
    let mut front = false;
    for card in cards {
        if card.status != Status::Pending {
            out.push(card);
        } else if !front && !in_the_pane(card, pane) {
            front = true;
            out.push(card);
        }
    }
    out
}

/// The pane to leave the card to, asked only while something is waiting: with nothing pending
/// there is no card to place, and moving about the desktop must not repaint the records.
fn pane_now(ui: &App, cards: &[Card]) -> String {
    if cards.iter().any(|c| c.status == Status::Pending) {
        pane_agent(ui)
    } else {
        String::new()
    }
}

fn sync_if_changed(ui: &App) {
    let cards = approvals::cards();
    let pane = pane_now(ui, &cards);
    let now = fingerprint(&cards, &pane);
    let changed = SHOWN.with(|shown| {
        if *shown.borrow() == now {
            false
        } else {
            *shown.borrow_mut() = now;
            true
        }
    });
    if changed {
        publish(ui, cards, &pane);
    }
}

fn sync(ui: &App) {
    let cards = approvals::cards();
    let pane = pane_now(ui, &cards);
    SHOWN.with(|shown| *shown.borrow_mut() = fingerprint(&cards, &pane));
    publish(ui, cards, &pane);
}

pub(crate) fn row_for(card: Card) -> crate::ApprovalRequest {
    crate::ApprovalRequest {
        id: card.id.into(),
        // Which of the person's agents asked — from its token, never its words — so the card
        // names it wherever it is drawn (design decision 4). Empty for a caller that is no agent.
        agent: card.verified.agent.clone().into(),
        // Filled by whoever draws it, from the agents' store: `publish` for the Lens, the pane
        // from the agent it draws. Never from anything the request says.
        on_behalf: slint::SharedString::new(),
        requester: card.requester.into(),
        // Never blank. An empty line where the verified fact should be reads as "nothing to
        // report", which is the opposite of what an unidentifiable caller means — and the card
        // would silently lose a row, which is the height defect this design already had once.
        verified: if card.verified.line.is_empty() {
            "could not be identified".into()
        } else {
            card.verified.line.into()
        },
        // One model entry per sentence, one single-line `Text` per entry, for the same reason
        // the arguments are a list: the card's height has to be arithmetic.
        discrepancies: ModelRc::new(VecModel::from(
            card.verified
                .discrepancies
                .into_iter()
                .map(slint::SharedString::from)
                .collect::<Vec<_>>(),
        )),
        app: card.app.into(),
        action: card.action.into(),
        grade: card.grade.into(),
        // An action with nothing published about it is the case commit d73760d was about.
        // Say so rather than leaving a blank line where the reason should be.
        purpose: if card.purpose.is_empty() {
            "(the app publishes no description for this action)".into()
        } else {
            card.purpose.into()
        },
        // The first sentence of that description, and the line the card leads with (#218):
        // the whole paragraph is for the person who wants it, under "show more", not the first
        // thing everybody has to read. Empty when the app publishes nothing — the card hides
        // the row and the purpose block above says so instead.
        summary: card.summary.into(),
        // One model entry per argument, one single-line `Text` per entry on the card. A
        // newline-joined string was the first shape of this and it is what made the card's
        // height something the layout had to discover by measuring wrapped text.
        args: ModelRc::new(VecModel::from(
            card.args.into_iter().map(slint::SharedString::from).collect::<Vec<_>>(),
        )),
        // One elided line beside the box, or nothing: the card hides the row when an app
        // publishes no naming index (#54), so this is a pass-through, not a second fallback.
        target: card.target.into(),
        warning: card.warning.into(),
        can_session: card.can_session,
        decision: match card.status {
            Status::Pending => "",
            Status::Granted | Status::Consumed => "allowed",
            Status::Denied => "denied",
            Status::Expired => "expired",
        }
        .into(),
        record: card.record.into(),
        age_text: if card.status == Status::Pending {
            let left = approvals::REQUEST_TTL.as_secs().saturating_sub(card.age_secs);
            format!("{left}s left").into()
        } else {
            slint::SharedString::new()
        },
    }
}

fn publish(ui: &App, cards: Vec<Card>, pane: &str) {
    let waiting = cards.iter().filter(|c| c.status == Status::Pending).count();

    // The same answers, in the pane of the agent each request was for: one request id, so
    // answering in either place settles both, and an expiry or a withdrawal reaches the pane on
    // the same turn it reaches the Lens.
    crate::agents::settle_approvals(settled_as);

    // One card at a time, even though up to three requests can be waiting — and never the one
    // the pane on screen is answering: that card the person reads beside the session it belongs
    // to, with the same buttons, and seeing it twice is seeing it nowhere (#212). The card was
    // drawn in one place or the other, never neither: the pane shows buttons for exactly the
    // cards `cards_for_screen` leaves out.
    //
    // Three cards stacked is 780px on an 800px screen: the third one's buttons land under the
    // taskbar, unreachable. It is also the wrong thing to show — a person facing a stack reads
    // none of them properly, which is the approval-fatigue failure the whole design is trying to
    // avoid. So the oldest is the one on screen and the rest wait behind a count.
    let mut shown: Vec<crate::ApprovalRequest> = Vec::new();
    let mut in_front: Vec<crate::ApprovalRequest> = Vec::new();
    for card in cards_for_screen(&cards, pane) {
        let pending = card.status == Status::Pending;
        let mut row = row_for(card.clone());
        // Who the agent works for — "Council recipe → Reviewer" — from how the shell started it.
        // Read here, where no other lock is held, never inside `row_for`, which the Agents pane
        // calls while it holds the agents' store.
        if !row.agent.is_empty() {
            let agent = crate::agents::AgentId(row.agent.to_string());
            row.on_behalf = crate::agents::store().read(|s| s.agent(&agent).map(|a| a.meta.on_behalf())).unwrap_or_default().into();
        }
        if pending {
            in_front.push(row.clone());
        }
        shown.push(row);
    }

    // Two models from one list. The Lens draws the whole conversation — the records of what was
    // decided as well as the one card waiting — and the overlay over the other screens draws
    // only the card, because a record is a thing to read later, not a thing to put in front of
    // somebody who is doing something else.
    ui.set_pending_approvals(ModelRc::new(VecModel::from(in_front)));
    ui.set_approvals(ModelRc::new(VecModel::from(shown)));
    ui.set_approvals_waiting(waiting.saturating_sub(1) as i32);

    // Nothing is waiting any more — by a decision, or because it expired unanswered. Either way
    // the shell was pushed in front of whatever the person was using and now owes it back.
    if waiting == 0 {
        give_the_screen_back();
    }
}

// ── The mode onto the screen ────────────────────────────────────────

thread_local! {
    /// What the chip and the menu are showing, so a tick that changes nothing repaints nothing.
    static MODE_SHOWN: RefCell<String> = const { RefCell::new(String::new()) };
}

fn mode_fingerprint() -> String {
    // Three cheap reads, deliberately not `snapshot()`: that one reads the machine ceiling off
    // disk, and this is asked once a second whether or not the menu is open.
    //
    // The chip label carries the countdown, so a bypass rebuilds once a second and nothing else
    // ever does. The audit's length is enough: entries are append-only.
    format!(
        "{}|{}|{}",
        crate::mind_mode::chip_label(),
        crate::mind_mode::rules_summary(),
        crate::mind_mode::recent(crate::mind_mode::AUDIT_PUBLISHED).len(),
    )
}

fn publish_mode_if_changed(ui: &App) {
    let now = mode_fingerprint();
    let changed = MODE_SHOWN.with(|shown| {
        if *shown.borrow() == now {
            false
        } else {
            *shown.borrow_mut() = now;
            true
        }
    });
    if changed {
        publish_mode(ui);
    }
}

fn publish_mode(ui: &App) {
    MODE_SHOWN.with(|shown| *shown.borrow_mut() = mode_fingerprint());
    // And to the apps, which enforce it (issue #116). Every change of mode or rule comes
    // through here, so this is the one place the file has to be kept true; it rewrites
    // nothing when nothing it says has changed.
    crate::mind_mode::publish_policy_file();

    let mode = crate::mind_mode::current();
    ui.set_mind_mode(mode.as_str().into());
    ui.set_mind_mode_label(crate::mind_mode::chip_label().into());
    ui.set_mind_mode_means(mode.meaning().into());
    ui.set_mind_ceiling(machine_ceiling().into());

    let snapshot = crate::mind_mode::snapshot();
    let rules: Vec<crate::MindRule> = snapshot["session_rules"]
        .as_array()
        .map(|list| {
            list.iter()
                .map(|r| {
                    let app = r["app"].as_str().unwrap_or_default().to_string();
                    let action = r["action"].as_str().unwrap_or_default().to_string();
                    crate::MindRule {
                        label: format!("{app}.{action}").into(),
                        app: app.into(),
                        action: action.into(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    ui.set_mind_session_rules(ModelRc::new(VecModel::from(rules)));

    // Newest first on screen. The list answers "what has it just done", and a person scanning it
    // reads from the top — which is the opposite of the transcript order the approval records
    // use, where the newest belongs nearest the thing waiting on you.
    let mut audit: Vec<crate::MindAuditEntry> = crate::mind_mode::recent(
        crate::mind_mode::AUDIT_PUBLISHED,
    )
    .into_iter()
    .map(|e| crate::MindAuditEntry {
        at: e.at.into(),
        what: format!("{}.{}", e.app, e.action).into(),
        // One line, already bounded the way the card bounds them. Joined with two spaces rather
        // than newlines because this is a single elided `Text` in a menu, not a card. The grade
        // and the mode it ran under stay in `describe shell` and in the file; see the struct.
        args: e.args.join("  ").into(),
        outcome: e.outcome.into(),
    })
    .collect();
    audit.reverse();
    ui.set_mind_audit(ModelRc::new(VecModel::from(audit)));
}

// ── Arguments as they actually arrive ───────────────────────────────

fn required(args: &serde_json::Value, key: &str) -> Result<String, String> {
    let value = text(args.get(key));
    if value.is_empty() {
        return Err(format!("`{key}` is required and was empty"));
    }
    Ok(value)
}

/// One argument as text, whatever shape the transport left it in.
///
/// `yos act` builds its arguments by running `json.loads` over every `key=value` pair, so a
/// purpose of `"120"` arrives as the number 120 and a requester of `"true"` arrives as a
/// boolean. Those are not errors worth a round trip — the caller meant the text — so they are
/// rendered rather than refused.
fn text(value: Option<&serde_json::Value>) -> String {
    match value {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(other) => other.to_string(),
    }
}

/// The arguments the grant will be bound to.
///
/// `args_json` is published as an `object`, because an object is what every caller that spends a
/// grant sends: the dispatch's own `gate::spend_grant`, the Python SDK's, `yos act` asking on a
/// caller's behalf, and `yos act shell request_approval args_json={...}`, which binds a value by
/// its published type. It was published as `text` until the dispatch began checking types, and
/// the check found it: every grant spent from outside the shell arrived as an object for an
/// argument that said it was a string. The dispatch now refuses a string before this runs.
///
/// Both shapes are still read here, because this function does not know which door its value
/// came through, and the two must produce the same canonical form or a grant requested one way
/// and consumed the other would never match — so the canonicalisation is done once, in Rust, on
/// the parsed value.
///
/// An `agent_token` among them is taken out and not used. A token is not an argument: these are
/// what the card draws, what `record_unasked_action` writes to the audit log and what the grant is
/// bound to — and the dispatch that spends the grant takes the same key out of the action's
/// arguments first (`gate::agent_token_of`), so a grant bound without it is the grant that
/// matches. Which agent is asking rides beside the arguments and is read by `who_is_asking`.
fn args_value(raw: Option<&serde_json::Value>) -> Result<serde_json::Value, String> {
    let mut value = args_object(raw)?;
    if let Some(map) = value.as_object_mut() {
        if map.remove(yantrik_ipc_transport::gate::AGENT_TOKEN).is_some() {
            tracing::warn!("an agent token arrived inside `args_json`; it was removed and not used");
        }
    }
    Ok(value)
}

fn args_object(raw: Option<&serde_json::Value>) -> Result<serde_json::Value, String> {
    match raw {
        None | Some(serde_json::Value::Null) => Ok(serde_json::json!({})),
        Some(serde_json::Value::Object(map)) => Ok(serde_json::Value::Object(map.clone())),
        Some(serde_json::Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                return Ok(serde_json::json!({}));
            }
            let parsed: serde_json::Value = serde_json::from_str(s).map_err(|e| {
                format!("`args_json` is not JSON: {e}. Send the action's arguments as an object, \
                         e.g. {{\"id\": \"evt-3\"}}.")
            })?;
            if !parsed.is_object() {
                return Err(format!(
                    "`args_json` parsed as {}, not an object. The grant is bound to the action's \
                     named arguments, so it has to be an object.",
                    kind_of(&parsed)
                ));
            }
            Ok(parsed)
        }
        Some(other) => Err(format!(
            "`args_json` arrived as {}, not an object or a JSON string.",
            kind_of(other)
        )),
    }
}

fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod control_approvals_tests {
    use std::path::{Path, PathBuf};

    /// The words that would be a granting action if one existed.
    const DECIDING: &[&str] = &["approve", "grant", "allow", "deny"];

    /// The three that may carry one, and why each is not a way to decide anything:
    /// `request_approval` asks, `approval_status` reads, `consume_approval` spends.
    const PERMITTED: &[&str] = &["request_approval", "approval_status", "consume_approval"];

    fn control_sources() -> Vec<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("control") && n.ends_with(".rs"))
                    .unwrap_or(false)
            })
            .collect();
        found.sort();
        found
    }

    /// Every `Action::new("name"` in the shell's control modules, with the file it came from.
    fn published_actions() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for path in control_sources() {
            let src = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            for (index, _) in src.match_indices("Action::new(") {
                let rest = &src[index + "Action::new(".len()..];
                // The name is the first string literal after the paren, possibly on the next
                // line. Anything else is not an action declaration and is skipped.
                let Some(open) = rest.find('"') else { continue };
                if rest[..open].chars().any(|c| !c.is_whitespace()) {
                    continue;
                }
                let Some(close) = rest[open + 1..].find('"') else { continue };
                out.push((rest[open + 1..open + 1 + close].to_string(), file.clone()));
            }
        }
        out
    }

    /// The surface has no way to approve anything.
    ///
    /// This is the security property of the whole feature, and it is exactly the kind that
    /// erodes: somebody adds `allow_action` to unblock a demo, it ships, and a mind can wave
    /// its own requests through. So the check is mechanical and reads the source of every
    /// `control*.rs`, not a list somebody maintains beside them.
    #[test]
    fn approvals_published_actions_cannot_grant() {
        let actions = published_actions();
        assert!(
            actions.len() > 10,
            "only {} actions were found — the scan is not reading the control modules any more, \
             which would make this test pass by seeing nothing. Found: {actions:?}",
            actions.len()
        );

        let offenders: Vec<String> = actions
            .iter()
            .filter(|(name, _)| {
                let lower = name.to_ascii_lowercase();
                DECIDING.iter().any(|w| lower.contains(w)) && !PERMITTED.contains(&name.as_str())
            })
            .map(|(name, file)| format!("{name} (in {file})"))
            .collect();

        assert!(
            offenders.is_empty(),
            "the shell publishes an action that reads as a decision about permission: {}\n\n\
             A caller on the socket must not be able to approve, grant, allow or deny anything — \
             that is the one thing that makes an approval card mean something. Granting is a UI \
             callback (`on_approval_allow` in control_approvals.rs) and nothing else. If this \
             action genuinely does not decide, rename it so it does not read like it does.",
            offenders.join(", ")
        );
    }

    /// And the three that are allowed are actually there.
    ///
    /// Without this, deleting the feature would make the test above pass, which is the usual
    /// way an invariant test becomes a decoration.
    #[test]
    fn approvals_the_three_asking_actions_are_published() {
        let names: Vec<String> = published_actions().into_iter().map(|(n, _)| n).collect();
        for wanted in PERMITTED {
            assert!(
                names.iter().any(|n| n == wanted),
                "`{wanted}` is not published any more; the approval flow is broken. Published: {}",
                names.join(", ")
            );
        }
    }

    /// The words a secret would arrive under, in an action name or in one of its parameters.
    const SECRET_WORDS: &[&str] =
        &["passphrase", "password", "passwd", "pin", "secret", "credential", "unlock"];

    /// Actions whose *name* may contain one of the words above, and why each is not a way in.
    ///
    /// `pin_app` pins an app tile to START. It matches because "pin" is in `SECRET_WORDS` and
    /// "pin" is what this vault's passphrase used to be called, which is exactly why the word is
    /// still watched. Kept as an explicit list of two-word justifications so that adding a
    /// genuine `unlock_vault` has to come past this constant and a reader, rather than past a
    /// regex somebody loosened to make a build go green.
    const SECRET_PERMITTED: &[&str] = &["pin_app"];

    /// Arguments whose names may contain one of those words. `pinned` is `pin_app`'s flag.
    const SECRET_PARAM_PERMITTED: &[&str] = &["pinned"];

    /// Only a person's keystrokes can supply a vault passphrase.
    ///
    /// The security property of the vault work, checked the way `approvals_published_actions_
    /// cannot_grant` checks its own: mechanically, over the source of every `control*.rs`, rather
    /// than against a list somebody remembers to update. The failure it exists to stop is the
    /// ordinary one — somebody adds `vault_unlock(passphrase=…)` so a script can bring a machine
    /// up unattended, it ships, and from then on anything that can open the shell's socket can
    /// hand the vault a guess. At that point the Argon2id wrapping is protecting a file against
    /// an attacker who is no longer reading the file.
    ///
    /// Two halves, because a passphrase could arrive as an action or as an argument to one.
    #[test]
    fn no_published_action_can_carry_a_passphrase() {
        let actions = published_actions();
        assert!(
            actions.len() > 10,
            "only {} actions were found — the scan is not reading the control modules any more, \
             which would make this test pass by seeing nothing",
            actions.len()
        );

        let offenders: Vec<String> = actions
            .iter()
            .filter(|(name, _)| {
                let lower = name.to_ascii_lowercase();
                SECRET_WORDS.iter().any(|w| lower.contains(w))
                    && !SECRET_PERMITTED.contains(&name.as_str())
            })
            .map(|(name, file)| format!("{name} (in {file})"))
            .collect();
        assert!(
            offenders.is_empty(),
            "the shell publishes an action that reads as a way to supply or handle a secret: {}\n\n\
             A caller on the socket must not be able to unlock the vault, set its passphrase, or \
             pass one to anything. The passphrase is typed into the card in `intent_lens.slint` \
             and reaches `vault_unlock::adopt` through `wire::vault`, and there is no other way \
             in. If this action genuinely carries no secret, rename it so it does not read like \
             it does.",
            offenders.join(", ")
        );

        // And no argument of any published action is named like one either. An action called
        // `configure` taking `passphrase` would pass the half above and be exactly the hole.
        let mut param_offenders: Vec<String> = Vec::new();
        for path in control_sources() {
            let whole = std::fs::read_to_string(&path).unwrap();
            let src = whole.split("#[cfg(test)]").next().unwrap_or("").to_string();
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            // `Param::text("name")`, `Param::flag("name")`, and any other constructor on Param.
            for (index, _) in src.match_indices("Param::") {
                let rest = &src[index..];
                let Some(open) = rest.find('"') else { continue };
                // Only the string literal that opens this Param, not one further down the file.
                if open > 40 {
                    continue;
                }
                let Some(close) = rest[open + 1..].find('"') else { continue };
                let name = &rest[open + 1..open + 1 + close];
                let lower = name.to_ascii_lowercase();
                if SECRET_WORDS.iter().any(|w| lower.contains(w))
                    && !SECRET_PARAM_PERMITTED.contains(&name)
                {
                    param_offenders.push(format!("{name} (in {file})"));
                }
            }
        }
        assert!(
            param_offenders.is_empty(),
            "a published action takes an argument named like a secret: {}\n\n\
             Nothing on the shell's socket may carry a passphrase, a PIN or a password, whatever \
             the action around it is called.",
            param_offenders.join(", ")
        );
    }

    /// The vault's tools do not ask a mind to relay the passphrase either.
    ///
    /// A different surface from the one above and the same rule. These tools used to take a `pin`
    /// argument whose description told the model to ask the user for it — which put the secret
    /// that protects every credential on the machine into a transcript, a context window, and
    /// whatever the answering provider keeps. Checked from the source for the same reason: this
    /// is the kind of argument somebody adds back to unblock something.
    #[test]
    fn the_vault_tools_do_not_ask_a_mind_for_the_passphrase() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yantrik-companion-tools/src/vault.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        // The tool definitions are JSON literals; a parameter is a quoted key in them.
        for banned in ["\"pin\":", "\"new_pin\":", "\"current_pin\":", "\"passphrase\":"] {
            assert!(
                !src.contains(banned),
                "{} declares a {banned} parameter again.\n\n\
                 A vault passphrase must not travel through a tool call. A locked vault answers \
                 LOCKED_ANSWER and raises the desktop's own prompt; the model is told, in that \
                 answer, that it cannot carry the secret and must not ask for it.",
                path.display()
            );
        }

        // And the answer it gives instead is the recognisable one, not a generic error.
        assert!(
            src.contains("VAULT_LOCKED:"),
            "the locked-vault answer is gone from {}; a mind would be back to reading a generic \
             failure it has learned to retry",
            path.display()
        );
    }

    /// The words that would be a way to loosen the mode, or mint a session rule, if one existed.
    const MODE_WORDS: &[&str] = &["mode", "rule", "bypass", "permission", "ceiling"];

    /// The one action allowed to carry them, and why it is not a way to loosen anything:
    /// `set_mind_mode` refuses every request that would make the desktop more permissive.
    const MODE_PERMITTED: &[&str] = &["set_mind_mode"];

    /// The functions in `mind_mode` that a person's click reaches, and nothing else may.
    const PERSON_ONLY: &[&str] = &["person_set_mode", "person_add_rule", "person_revoke_rule"];

    /// The function those callbacks are wired in. Anything else naming them is the bug.
    const CALLBACK_HOME: &str = "wire";

    /// The top-level function each line of a source file belongs to.
    ///
    /// Line-based and deliberately dumb: a top-level `fn` in this crate starts at column zero
    /// (optionally behind `pub` or `pub(crate)`), and a closure inside one never does. Brace
    /// matching would be the "proper" way and would trip over the braces inside the string
    /// literals these files are full of — `{\"id\": \"evt-3\"}` and friends.
    fn enclosing_fns(src: &str) -> Vec<String> {
        let mut current = String::from("(top level)");
        let mut out = Vec::new();
        for line in src.lines() {
            let head = line
                .strip_prefix("pub(crate) ")
                .or_else(|| line.strip_prefix("pub "))
                .unwrap_or(line);
            if let Some(rest) = head.strip_prefix("fn ") {
                let name: String =
                    rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                if !name.is_empty() {
                    current = name;
                }
            }
            out.push(current.clone());
        }
        out
    }

    /// Only a person can make this desktop more permissive.
    ///
    /// The same property as `approvals_published_actions_cannot_grant` and the same reason for
    /// checking it mechanically: somebody adds `allow_mode` or `add_session_rule` to unblock a
    /// demo, it ships, and a mind can put the machine into bypass and then do as it likes. Two
    /// halves, because there are two ways in — publishing an action that loosens it, and calling
    /// the person-only functions from somewhere a caller on the socket can reach.
    #[test]
    fn mind_mode_only_a_person_can_raise_the_mode() {
        let actions = published_actions();
        assert!(
            actions.len() > 10,
            "only {} actions were found — the scan is not reading the control modules any more",
            actions.len()
        );

        let offenders: Vec<String> = actions
            .iter()
            .filter(|(name, _)| {
                let lower = name.to_ascii_lowercase();
                MODE_WORDS.iter().any(|w| lower.contains(w))
                    && !MODE_PERMITTED.contains(&name.as_str())
            })
            .map(|(name, file)| format!("{name} (in {file})"))
            .collect();
        assert!(
            offenders.is_empty(),
            "the shell publishes an action that reads as a change to what the mind may do \
             unasked: {}\n\n\
             A caller on the socket must not be able to loosen the mode or mint a session rule. \
             `set_mind_mode` is the only published action about modes and it can only TIGHTEN. \
             If this action genuinely cannot loosen anything, rename it so it does not read like \
             it can.",
            offenders.join(", ")
        );

        // And the person-only functions are called from exactly one place: the callback wiring.
        for path in control_sources() {
            let whole = std::fs::read_to_string(&path).unwrap();
            // The published surface is the code, not the tests. This module's own test block
            // names these functions in a constant, and a test asserting about a name is not a
            // caller of it.
            let src = whole.split("#[cfg(test)]").next().unwrap_or("").to_string();
            let file = path.file_name().unwrap().to_string_lossy().to_string();
            let owners = enclosing_fns(&src);
            for (index, line) in src.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for wanted in PERSON_ONLY {
                    if !line.contains(wanted) {
                        continue;
                    }
                    assert_eq!(
                        owners[index], CALLBACK_HOME,
                        "{file}:{} calls `{wanted}` from `{}`. It may only be called from \
                         `{CALLBACK_HOME}`, where the callers are Slint callbacks a person's \
                         click arrives on. Anything reachable from an action handler makes the \
                         mode chip decorative.",
                        index + 1,
                        owners[index],
                    );
                }
            }
        }
    }

    /// And `set_mind_mode` is actually published, so deleting it cannot make the scan pass.
    #[test]
    fn mind_mode_the_tightening_action_is_published() {
        let names: Vec<String> = published_actions().into_iter().map(|(n, _)| n).collect();
        // `show_mind_audit` is here because it is what the "See what it did" button on the
        // "Bypass ended" notification calls. The shell presses that button on the sender's own
        // control surface, so deleting the action would leave a button that silently does
        // nothing — and a dead control on a notification about permissions is worse than none.
        for wanted in ["set_mind_mode", "record_unasked_action", "show_mind_audit"] {
            assert!(
                names.iter().any(|n| n == wanted),
                "`{wanted}` is not published any more. Published: {}",
                names.join(", ")
            );
        }
    }

    /// #184: what `show_mind_audit` opens, a caller can put away — `close_mind_menu`, graded
    /// `safe` like the action that opened it — and read whether it is still there.
    #[test]
    fn what_show_mind_audit_opens_a_caller_can_close_and_see() {
        let names: Vec<String> = published_actions().into_iter().map(|(n, _)| n).collect();
        assert!(names.iter().any(|n| n == "close_mind_menu"), "close_mind_menu is published: {}", names.join(", "));

        // Its grade is `safe`: the declaration's own `.risk`, before the next action begins.
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_approvals.rs")).unwrap();
        let src = src.split("#[cfg(test)]").next().unwrap();
        let at = src.find("\"close_mind_menu\",").expect("the declaration");
        let own = &src[at..];
        // To the next action, or to the end of `actions` when it is the last.
        let own = &own[..own.find("Action::new(").or_else(|| own.find("\n}\n")).unwrap_or(own.len())];
        assert!(own.contains(".risk(\"safe\")"), "close_mind_menu is graded safe");
        // It closes all of it, the two sub-panels with it, and opens nothing.
        for set in ["set_mind_menu_open(false)", "set_mind_menu_confirming(false)", "set_mind_menu_audit_open(false)"] {
            assert!(own.contains(set), "close_mind_menu does not {set}");
        }
        assert!(!own.contains("(true)"), "close_mind_menu opens nothing");

        // `describe shell` says whether it is open, and which part of it is showing.
        let control = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control.rs")).unwrap();
        let control: String = control.split("#[cfg(test)]").next().unwrap().split_whitespace().collect();
        assert!(
            control.contains(".with(\"mind_menu\",crate::control_approvals::mind_menu_for_describe("),
            "describe shell publishes `mind_menu`"
        );
    }

    #[test]
    fn describe_says_whether_the_mode_menu_is_open_and_what_it_shows() {
        use super::mind_menu_for_describe as menu;
        // As `show_mind_audit` leaves it, over the desktop.
        let audit = menu(true, true, false, 1);
        assert_eq!(audit["open"], true);
        assert_eq!(audit["showing"], "the record of unasked actions");
        assert_eq!(audit["close_with"], "close_mind_menu");
        assert_eq!(menu(true, false, false, 34)["showing"], "the modes");
        assert_eq!(menu(true, true, true, 7)["showing"], "bypass confirmation");
        // Put away.
        let closed = menu(false, false, false, 1);
        assert_eq!((closed["open"].clone(), closed["showing"].clone()), (serde_json::json!(false), serde_json::Value::Null));
        // Never drawn over boot, onboarding, lock or login, so never open there.
        for screen in [0, 2, 3, 32] {
            assert_eq!(menu(true, true, false, screen)["open"], false, "screen {screen}");
        }
    }

    /// The menu belongs to the screen it was opened over: changing screen closes it and its
    /// sub-panels, whatever moved the screen. The behaviour itself is exercised on the real
    /// component by tests/ui-preview (`verify-mind-panel`); this pins where it lives.
    #[test]
    fn changing_screen_puts_the_mode_menu_away() {
        let app = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../yantrik-ui-slint/ui/app.slint")).unwrap();
        let at = app.find("changed current-screen =>").expect("app.slint closes the menu when the screen changes");
        let body = &app[at..at + app[at..].find('}').unwrap()];
        for set in ["root.mind-menu-open = false;", "root.mind-menu-confirming = false;", "root.mind-menu-audit-open = false;"] {
            assert!(body.contains(set), "the screen change does not `{set}`: {body}");
        }
    }

    /// The card always has a verified row, and it is never empty.
    ///
    /// A blank here would read as "nothing to report", which is the opposite of what an
    /// unidentifiable caller means — and the row would collapse, which is how this card lost its
    /// header off the top of the screen the first time it ran on a real machine.
    /// The grade a caller declares is checked against the one the app publishes.
    #[test]
    fn approvals_an_understated_grade_is_corrected_and_said_out_loud() {
        use super::grade_note;

        // The case this exists for: a `dangerous` action declared as something routine. The
        // decision below runs on the published grade, and the card says the caller lied about it.
        let said = grade_note("standard", "dangerous");
        assert!(said.contains("standard"), "{said}");
        assert!(said.contains("dangerous"), "{said}");
        assert!(said.chars().count() <= super::NOTE_CHARS + 1, "{said}");
        assert!(!said.contains('\n'), "one card row: {said}");

        // Over-declaring is said too — the card is about to show a grade the caller did not name
        // and a person comparing the two should not have to wonder which is which.
        assert!(!grade_note("dangerous", "standard").is_empty());

        // Agreement is silent, in either spelling. The bridge sends the app's own grade, so this
        // is the ordinary path and it must add nothing to the card.
        assert_eq!(grade_note("sensitive", "sensitive"), "");
        assert_eq!(grade_note(" Sensitive ", "sensitive"), "");
    }

    /// `request_approval` acts on EVERY outcome of the decision table, not only plan mode.
    ///
    /// The gap this closes: a caller that skipped the bridge used to get a card raised for an
    /// action graded above `tool_permission` — a question no answer could satisfy, because the
    /// app's own runtime refuses it whatever the person clicks. The refusals here are the ones
    /// `mind_mode::decide` makes, relayed verbatim, so a mind that came through the bridge and
    /// one that came straight to the socket hear one story.
    #[test]
    fn approvals_the_shell_asks_only_what_the_decision_table_says_to_ask() {
        use crate::mind_mode::{Decision, Mode, Modes};
        use std::time::Instant;

        // Every outcome the shared vectors name (`deploy/yantrik-os/mind-mode-vectors.json`),
        // at least once each: what the mode decides, and what this handler does about it.
        let cases: [(&str, Mode, &str, &str, &str); 6] = [
            // outcome         mode         ceiling      published grade  what the shell does
            ("run", Mode::Auto, "dangerous", "standard", "no card"),
            // `auto` runs a `sensitive` action without asking, and `ask` mode would have raised
            // a card for it — which is exactly what `run_logged` means and what the audit is
            // for. Not driven through `bypass` here because `Modes::new` refuses to construct
            // one: a machine must never come up in bypass, so only a person's click enters it.
            ("run_logged", Mode::Auto, "dangerous", "sensitive", "no card"),
            ("ask", Mode::Ask, "dangerous", "sensitive", "card"),
            ("refuse_grade", Mode::Ask, "dangerous", "catastrophic", "refused"),
            ("refuse_ceiling", Mode::Auto, "standard", "dangerous", "refused"),
            ("refuse_mode", Mode::Plan, "dangerous", "standard", "refused"),
        ];

        for (outcome, mode, ceiling, published, expected) in cases {
            let modes = Modes::new(mode);
            // `false`: these cases are about the grade. The app's own sentence about undoing
            // gets its own assertion below, because it is what decides `calendar.delete_event`
            // on a real machine.
            let decision =
                modes.decide(published, "calendar", "delete_event", false, ceiling, Instant::now());

            // The shape `request_approval` branches on. Kept beside the table it is derived from
            // so a fourth outcome cannot be added to `decide` without this failing to classify.
            let did = match &decision {
                Decision::Refuse { .. } => "refused",
                Decision::Run { .. } => "no card",
                Decision::Ask => "card",
            };
            assert_eq!(
                did, expected,
                "`{outcome}` (mode {mode:?}, ceiling {ceiling}, graded {published}) must be \
                 {expected}, and the handler branches on exactly these three variants"
            );

            // And a refusal is relayed word for word, never reworded into something a mind
            // would read as a transport failure worth retrying.
            if let Decision::Refuse { why } = &decision {
                assert!(!why.is_empty());
                assert!(
                    why.contains("not a level this OS defines")
                        || why.contains("tool_permission")
                        || why.contains("plan mode"),
                    "a refusal this handler relays has to be one of the three the table makes: \
                     {why}"
                );
            }
        }

        // The whole point of looking the grade up: a `dangerous` action declared `standard` is
        // decided as `dangerous`. Declared, it would have been run without a card in auto mode;
        // published, the same machine refuses it outright under a `standard` ceiling.
        let auto = Modes::new(Mode::Auto);
        let claimed = auto.decide("standard", "files", "delete", false, "standard", Instant::now());
        let published =
            auto.decide("dangerous", "files", "delete", false, "standard", Instant::now());
        assert!(matches!(claimed, Decision::Run { .. }), "what the lie would have bought");
        assert!(
            matches!(&published, Decision::Refuse { why } if why.contains("tool_permission")),
            "and what the published grade actually decides: {published:?}"
        );
        assert!(!super::grade_note("standard", "dangerous").is_empty(), "and the card says so");

        // And the same argument for the sentence beside the grade. `request_approval` takes a
        // `purpose` argument; a caller that omitted it used to talk this handler into answering
        // `not_needed` for `calendar.delete_event` on a desktop in `auto` — which is the whole
        // of the defect, reachable from the socket without the bridge. The handler reads the
        // purpose the app publishes, so leaving it out changes nothing.
        let unsaid = auto.decide("sensitive", "calendar", "delete_event", false, "dangerous", Instant::now());
        let published = auto.decide("sensitive", "calendar", "delete_event", true, "dangerous", Instant::now());
        assert!(matches!(unsaid, Decision::Run { .. }), "what saying nothing would have bought");
        assert_eq!(published, Decision::Ask, "and what the app's own sentence decides");
        assert!(
            crate::approvals::unrecoverable("Take an event off the calendar. It is not recoverable"),
            "which is the sentence Calendar actually publishes today"
        );
    }

    #[test]
    fn approvals_an_unidentified_caller_never_renders_a_blank_row() {
        use crate::approvals::{Card, Status, Verified};
        use slint::Model;

        let card = |verified: Verified| {
            let purpose = "Delete a file. It is not recoverable.";
            Card {
                id: "appr-1".into(),
                requester: "an unnamed caller".into(),
                verified,
                app: "files".into(),
                action: "delete".into(),
                grade: "dangerous".into(),
                purpose: purpose.into(),
                summary: crate::approvals::summary_of(purpose),
                args: vec!["name: taxes.pdf".into()],
                // Files names no handle here — `name: taxes.pdf` is already the thing itself —
                // so the naming row is empty, and this is the ordinary path on the card (#54).
                target: String::new(),
                warning: "The app says this cannot be undone.".into(),
                can_session: false,
                status: Status::Pending,
                record: String::new(),
                age_secs: 3,
            }
        };

        let nothing = super::row_for(card(Verified::default()));
        assert_eq!(nothing.verified, "could not be identified");
        assert_eq!(nothing.discrepancies.row_count(), 0);

        let known = super::row_for(card(Verified {
            line: "hermes_cli gateway (pid 696) \u{b7} the attached mind".into(),
            exe: "/home/pranab/hermes-agent/venv/bin/python".into(),
            pid: 696,
            attached_mind: "Hermes Agent".into(),
            discrepancies: vec![
                "\u{201c}Hermes Agent\u{201d} is attached here \u{2014} this is not it.".into(),
                "The caller called this `standard`; the app publishes `dangerous`.".into(),
            ],
            agent: String::new(),
        }));
        assert!(known.verified.contains("pid 696"), "{}", known.verified);
        // Both disagreements survive. Concatenating them into one elided row would have shown
        // the first and silently dropped the one that changes what the machine does.
        assert_eq!(known.discrepancies.row_count(), 2);
        // Every row is one line: the card's height is arithmetic, not a measurement.
        assert!(!known.verified.contains('\n'));
        for i in 0..known.discrepancies.row_count() {
            let row = known.discrepancies.row_data(i).unwrap();
            assert!(!row.contains('\n'), "{row}");
        }
    }

    /// The row the card draws carries the app's whole sentence, and no warning the shell cannot
    /// stand behind.
    ///
    /// The two cards of 22 September, rebuilt from the store outward: `studio.set_backend` with
    /// `kind: fake` and with `kind: openai-images`. Both showed "…the sentences typed into this
    /// app will leav… (585 characters in full)" — cut mid-word at the clause that said why the
    /// grade is `sensitive`, and identical in the direction that stops anything leaving. What
    /// reaches the Slint row now is the sentence as Studio published it, and the two rows differ
    /// where they truly differ: in the argument box.
    #[test]
    fn approvals_the_row_says_the_whole_purpose_and_differs_by_argument() {
        use crate::approvals::{Store, Verified};
        use slint::Model;
        use std::time::Instant;

        // Verbatim from `yos describe studio`; 585 characters.
        let purpose = "Choose where pictures are made from now on, and write that choice down in \
            the configuration file. Graded `sensitive` because it decides where every later \
            prompt goes: naming a hosted service means the sentences typed into this app will \
            leave this machine and may cost money. `generate` and `variations` are regraded the \
            moment this lands, so a caller cannot point Studio at a service and generate in the \
            same breath under the old, local grade. No key is taken here — only the NAME of an \
            environment variable that holds one, which is read at call time and never stored, \
            logged or shown.";
        assert_eq!(purpose.chars().count(), 585);

        let mut store = Store::new();
        let now = Instant::now();
        let mut row = |args: serde_json::Value| {
            let id = store
                .request("claude-code 2.1.276", Verified::default(), "studio", "set_backend",
                    args, "sensitive", purpose, "", now, "19:32")
                .unwrap()
                .id;
            let card = store.pending(now).into_iter().find(|c| c.id == id).unwrap();
            super::row_for(card)
        };
        let back = row(serde_json::json!({"kind": "fake"}));
        let away = row(serde_json::json!({
            "kind": "openai-images", "model": "gpt-image-1", "api_key_env": "OPENAI_API_KEY"
        }));

        for shown in [&back, &away] {
            assert_eq!(shown.purpose.as_str(), purpose, "the sentence, whole");
            assert!(!shown.purpose.contains("characters in full"), "{}", shown.purpose);
            assert_eq!(
                shown.summary.as_str(),
                "Choose where pictures are made from now on, and write that choice down in the \
                 configuration file.",
                "the row also carries the one line the card leads with (#218)"
            );
            // Nothing in red. The shell knows the grade and the app's sentence; it does not know
            // what `fake` or `openai-images` means to Studio, so a hosted-service warning of its
            // own would be the OS vouching for something it has not established — in the safe
            // direction as much as the other one.
            assert_eq!(shown.warning.as_str(), "");
        }
        assert_eq!(back.args.row_count(), 1);
        assert_eq!(back.args.row_data(0).unwrap().as_str(), "kind: fake");
        assert_eq!(away.args.row_count(), 3);
        assert_eq!(away.args.row_data(1).unwrap().as_str(), "kind: openai-images");
    }

    /// The arguments a grant binds to survive both ways they can arrive.
    #[test]
    fn approvals_args_json_is_accepted_as_object_or_string() {
        use super::args_value;
        let as_object = serde_json::json!({"id": "evt-3", "confirm": true});
        let as_string = serde_json::Value::String(r#"{"confirm":true,"id":"evt-3"}"#.into());

        let from_object = args_value(Some(&as_object)).expect("an object is what yos act sends");
        let from_string = args_value(Some(&as_string)).expect("a string is what raw JSON-RPC sends");
        assert_eq!(
            crate::approvals::canonical(&from_object),
            crate::approvals::canonical(&from_string),
            "the two transports have to bind to the same grant"
        );

        assert_eq!(args_value(None).unwrap(), serde_json::json!({}));
        assert_eq!(
            args_value(Some(&serde_json::Value::String(String::new()))).unwrap(),
            serde_json::json!({})
        );

        let err = args_value(Some(&serde_json::Value::String("[1,2]".into())))
            .expect_err("a list is not a set of named arguments");
        assert!(err.contains("not an object"), "{err}");

        let err = args_value(Some(&serde_json::Value::String("id=evt-3".into())))
            .expect_err("key=value is not JSON");
        assert!(err.contains("not JSON"), "{err}");
    }

    /// A token is not an argument: one put inside `args_json` is taken out before the card draws
    /// the arguments, the audit writes them or the grant is bound to them — and the grant then
    /// matches the dispatch, which takes the same key out of the action's arguments.
    #[test]
    fn approvals_a_token_inside_the_arguments_never_reaches_the_card_the_log_or_the_grant() {
        use super::args_value;
        let token = "0123456789abcdef0123456789abcdef";
        for raw in [
            serde_json::json!({"command": "ls", "agent_token": token}),
            serde_json::Value::String(format!(r#"{{"command":"ls","agent_token":"{token}"}}"#)),
        ] {
            let bound = args_value(Some(&raw)).unwrap();
            assert_eq!(bound, serde_json::json!({"command": "ls"}));
            let rows = crate::approvals::args_rows(&bound);
            assert!(!rows.join(" ").contains(token), "{rows:?}");
        }
    }

    /// The agent on a card comes from the token, and only a believed one: a token that did not
    /// check out puts the card in no pane and says so, in a line that carries nothing of it.
    #[test]
    fn approvals_the_agent_on_a_card_is_the_one_its_token_names_or_none() {
        use super::{agent_fact, UNBELIEVED_TOKEN};
        use crate::agents::AgentId;
        assert_eq!(agent_fact(None), (String::new(), None), "no token: the Lens alone, as always");
        assert_eq!(agent_fact(Some(Ok(AgentId("pi:c-7f3a91".into())))), ("pi:c-7f3a91".to_string(), None));
        let (agent, doubt) = agent_fact(Some(Err("token 0123abcd was not issued to the process 4242".into())));
        assert_eq!(agent, "", "a token that was not believed names no agent");
        assert_eq!(doubt.as_deref(), Some(UNBELIEVED_TOKEN));
        assert!(!UNBELIEVED_TOKEN.contains("0123") && UNBELIEVED_TOKEN.chars().count() < 70, "one line, nothing of the token");
        // Drawn in a pane only for a named agent.
        assert!(!super::draw_in_pane("", "appr-1", "shell", "agent_run"));
    }

    /// A child starts with no grants: a request id asked for one agent spends for nobody else.
    #[test]
    fn approvals_one_agent_cannot_spend_anothers_grant() {
        use super::grant_belongs;
        use crate::agents::AgentId;
        let parent = Some(Ok(AgentId("pi:c-parent".into())));
        let child = Some(Ok(AgentId("pi:c-child1".into())));
        assert!(grant_belongs("appr-1", "pi:c-parent", &parent).is_ok(), "the agent that asked spends it");
        let err = grant_belongs("appr-1", "pi:c-parent", &child).unwrap_err();
        assert!(err.contains("not handed from one agent to another") && err.contains("pi:c-parent"), "{err}");
        assert!(grant_belongs("appr-2", "", &child).is_err(), "nor one the person asked for");
        assert!(grant_belongs("appr-1", "pi:c-parent", &Some(Err("no".into()))).is_err(), "a token not believed spends nothing");
        assert!(grant_belongs("appr-1", "pi:c-parent", &None).is_ok(), "an app's own dispatch, as before");
    }

    /// #212: a card shown twice — once in the pane, once in the floating popup — was a card
    /// nobody could tell was one question or two. One place answers it now: the pane when the
    /// agent's own pane is on screen, the popup otherwise, and never neither.
    #[test]
    fn approvals_a_card_the_pane_answers_is_not_in_the_popup_too() {
        use crate::approvals::{Card, Status, Verified};

        fn card(id: &str, status: Status, agent: &str) -> Card {
            Card {
                id: id.into(),
                requester: "pi 0.87".into(),
                verified: Verified { agent: agent.into(), ..Verified::default() },
                app: "files".into(),
                action: "delete".into(),
                grade: "sensitive".into(),
                purpose: "Delete a file. It is not recoverable.".into(),
                summary: crate::approvals::summary_of("Delete a file. It is not recoverable."),
                args: vec![],
                target: String::new(),
                warning: String::new(),
                can_session: false,
                status,
                record: String::new(),
                age_secs: 3,
            }
        }
        // As `cards()` returns them: the decided records first, then the pending, oldest first.
        let cards = vec![
            card("appr-0", Status::Granted, "pi:c-1"),
            card("appr-1", Status::Pending, "pi:c-1"),
            card("appr-2", Status::Pending, "deepseek:c-2"),
        ];
        let ids = |pane: &str| {
            super::cards_for_screen(&cards, pane).into_iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
        };

        // No pane on screen: the oldest pending is in front, exactly as before.
        assert_eq!(ids(""), ["appr-0", "appr-1"]);
        // The pane is pi's agent: its own card is answered there, and the popup takes the next
        // one — a second request is never left without a place on screen.
        assert_eq!(ids("pi:c-1"), ["appr-0", "appr-2"]);
        // A pane for another agent changes nothing about pi's card.
        assert_eq!(ids("deepseek:c-2"), ["appr-0", "appr-1"]);
        // The only card waiting is the pane's own: the popup keeps the records and no card.
        let alone = vec![card("appr-9", Status::Pending, "pi:c-1")];
        assert!(super::cards_for_screen(&alone, "pi:c-1").is_empty());
        assert_eq!(super::cards_for_screen(&alone, "").len(), 1);
        // A card with no verified agent is in no pane, whatever is on screen.
        let nobody = vec![card("appr-8", Status::Pending, "")];
        assert_eq!(super::cards_for_screen(&nobody, "pi:c-1").len(), 1);

        // Walking to or from the pane is a change the once-a-second tick has to republish.
        assert_ne!(
            super::fingerprint(&cards, ""),
            super::fingerprint(&cards, "pi:c-1"),
            "the same cards with a different pane must not look unchanged"
        );
        assert_eq!(super::fingerprint(&cards, "pi:c-1"), super::fingerprint(&cards, "pi:c-1"));

        // And the two halves stay in step: the popup leaves out exactly the cards the pane draws
        // live buttons for, so a request is never hidden from both places at once.
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_approvals.rs")).unwrap();
        let src = src.split("#[cfg(test)]").next().unwrap();
        assert!(src.contains("!pane.is_empty() && card.verified.agent == pane"), "the popup matches the pane by the verified agent alone");
        assert!(src.contains("ui.get_current_screen() != crate::wire::agents::SCREEN"), "only the Agents screen has a pane");
        assert!(src.contains("g.get_view() != \"list\""), "the map is not a pane: the session is not on screen");
        let wire = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/wire/agents.rs")).unwrap();
        let wire = wire.split("#[cfg(test)]").next().unwrap();
        assert!(wire.contains("c.verified.agent == a.meta.id.0"), "the pane's live buttons are the same predicate");
    }
}

/// Approvals for a surface whose window is shut: its service answers for it (issue #161).
#[cfg(all(test, unix))]
mod service_surface_approval_tests {
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};

    use super::{published_detail_in, surface_in, Naming};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yantrik-approvals-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A socket that answers `app.describe` with `describe`, as a service's own dispatch does.
    fn serve(path: &Path, describe: serde_json::Value) {
        let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue; // somebody asking whether anything is here
                }
                let asked: serde_json::Value = serde_json::from_str(&line).unwrap();
                let reply = serde_json::json!({ "jsonrpc": "2.0", "id": asked["id"], "result": describe });
                let mut stream = stream;
                let _ = stream.write_all(format!("{reply}\n").as_bytes());
            }
        });
    }

    fn sysmon(grade: &str, description: &str) -> serde_json::Value {
        serde_json::json!({
            "app": "system-monitor",
            "summary": "System — CPU 3%",
            "state": {},
            "actions": [{
                "name": "kill_process",
                "description": description,
                "permission": grade,
                "parameters": { "type": "object", "properties": {}, "required": [] },
            }],
        })
    }

    #[test]
    fn a_service_action_can_be_approved_with_its_window_shut() {
        let dir = scratch("service");
        let installed = crate::surfaces::shipped_catalogue();
        // Nothing answers at all: refused, and the sentence says both places were looked in.
        let err = published_detail_in(&dir, &installed, "system-monitor", "kill_process").unwrap_err();
        assert!(err.contains("neither its window nor a service"), "{err}");

        // The window was open once and crashed: its socket file is still there, nobody listens.
        drop(std::os::unix::net::UnixListener::bind(dir.join("app-system-monitor.sock")).unwrap());
        serve(&dir.join("system-monitor.sock"), sysmon("dangerous", "End a running process by PID"));

        for name in ["system-monitor", "sysmonitor", "System Monitor"] {
            assert_eq!(
                published_detail_in(&dir, &installed, name, "kill_process"),
                Ok((
                    "dangerous".to_string(),
                    "End a running process by PID".to_string(),
                    Naming::new(),
                )),
                "`{name}`, with the window shut, is graded by its service"
            );
        }
        let err = published_detail_in(&dir, &installed, "system-monitor", "no_such").unwrap_err();
        assert!(err.contains("publishes no action called `no_such`"), "{err}");

        // With the window open, the window is what an act reaches, so it is what is asked.
        std::fs::remove_file(dir.join("app-system-monitor.sock")).unwrap();
        serve(&dir.join("app-system-monitor.sock"), sysmon("sensitive", "the window's own account"));
        assert_eq!(
            published_detail_in(&dir, &installed, "sysmonitor", "kill_process"),
            Ok((
                "sensitive".to_string(),
                "the window's own account".to_string(),
                Naming::new(),
            ))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the app says its own ids stand for rides the same `describe` as its grade (#54) —
    /// one round trip, so the sentence beside a handle cannot come from a different revision
    /// of the app than the grade the card was graded by.
    #[test]
    fn an_app_that_names_its_ids_has_them_read_off_the_same_describe() {
        let dir = scratch("naming");
        let installed = crate::surfaces::shipped_catalogue();
        // The window is shut; the service answers, and its surface names an event handle.
        drop(std::os::unix::net::UnixListener::bind(dir.join("app-system-monitor.sock")).unwrap());
        let mut describe = sysmon("dangerous", "End a running process by PID");
        describe["state"]["naming"] = serde_json::json!({
            "01a0c718-3931-7342-b9c7-8de36140ddb0": "Dentist, Fri 25 Sep 13:00",
            // An app that indexed a number under an id confused its own surface; a card built on
            // stringifying that guess would be the shell vouching for a sentence the app wrote.
            "not-a-name": 7,
        });
        serve(&dir.join("system-monitor.sock"), describe);
        let (grade, purpose, naming) =
            published_detail_in(&dir, &installed, "system-monitor", "kill_process").unwrap();
        assert_eq!(grade, "dangerous");
        assert_eq!(purpose, "End a running process by PID");
        assert_eq!(
            naming.get("01a0c718-3931-7342-b9c7-8de36140ddb0").map(String::as_str),
            Some("Dentist, Fri 25 Sep 13:00"),
            "the handle resolves off the wire, beside the grade"
        );
        assert!(!naming.contains_key("not-a-name"), "a value that is not a name is skipped");
        assert_eq!(
            super::target_line(
                &serde_json::json!({"id": "01a0c718-3931-7342-b9c7-8de36140ddb0"}),
                &naming
            ),
            "id 01a0c718\u{2026} is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}",
            "and the arguments resolve against it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A surface nothing declares is still found while it answers under its own name — a
    /// service, or an app somebody started without a .desktop file.
    #[test]
    fn a_surface_answering_under_its_own_name_is_found_without_a_desktop_file() {
        let dir = scratch("undeclared");
        let installed = crate::surfaces::shipped_catalogue();
        assert_eq!(surface_in("hello-service", &installed, &dir), None);
        serve(&dir.join("hello-service.sock"), serde_json::json!({ "app": "hello-service", "actions": [] }));
        assert_eq!(surface_in("Hello Service", &installed, &dir).as_deref(), Some("hello-service"));
        // Declared names are still the catalogue's, whatever answers in the directory.
        assert_eq!(surface_in("container-manager", &installed, &dir).as_deref(), Some("containers"));
        assert_eq!(surface_in("yantrik", &installed, &dir).as_deref(), Some("shell"));
        assert_eq!(surface_in("../etc", &installed, &dir), None, "not a name, whatever is on the disk");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The line that names what a handle in the arguments stands for (#54).
#[cfg(test)]
mod target_line_tests {
    use super::{naming_in, target_line, Naming};

    fn index(entries: &[(&str, &str)]) -> Naming {
        entries.iter().map(|(handle, name)| (handle.to_string(), name.to_string())).collect()
    }

    const DENTIST: &str = "01a0c718-3931-7342-b9c7-8de36140ddb0";
    const STANDUP: &str = "01a0c718-3931-7e0a-8a1b-0f2d4c8a91b2";

    /// The card the person on 22 September could not answer: a delete asked by id.
    #[test]
    fn a_handle_reads_as_the_thing_it_stands_for() {
        let naming = index(&[(DENTIST, "Dentist, Fri 25 Sep 13:00")]);
        let line = target_line(&serde_json::json!({"id": DENTIST}), &naming);
        assert_eq!(line, "id 01a0c718\u{2026} is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}");
        assert!(!line.contains("b9c7-8de36140ddb0"), "the tail of the handle is not the point");
        assert!(!line.contains('\n'), "one line: the card's height is arithmetic");
        assert_eq!(target_line(&serde_json::json!({"id": STANDUP}), &naming), "", "what the app does not name draws nothing");
    }

    /// A handle no longer than the echo is shown whole — the ellipsis is for what it cuts.
    #[test]
    fn a_short_handle_wears_no_ellipsis() {
        let naming = index(&[("evt-3", "Gym, Sat 26 Sep 08:00")]);
        assert_eq!(
            target_line(&serde_json::json!({"id": "evt-3"}), &naming),
            "id evt-3 is \u{201c}Gym, Sat 26 Sep 08:00\u{201d}"
        );
    }

    /// Two handles — an event and the one to merge it into — read in the order the box above
    /// lists them, which is sorted, so the rows line up.
    #[test]
    fn two_handles_join_in_the_order_the_box_lists_them() {
        let naming = index(&[(DENTIST, "Dentist, Fri 25 Sep 13:00"), (STANDUP, "Standup, Tue 22 Sep 09:00")]);
        let line = target_line(
            &serde_json::json!({"id": STANDUP, "into": DENTIST}),
            &naming,
        );
        assert_eq!(
            line,
            "id 01a0c718\u{2026} is \u{201c}Standup, Tue 22 Sep 09:00\u{201d}; \
             into 01a0c718\u{2026} is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}"
        );
    }

    /// The rule of the row: it says what the app says. Not an object, no string value, an app
    /// that publishes no index — every way there is nothing to vouch for draws nothing.
    #[test]
    fn what_nothing_names_draws_nothing() {
        let naming = index(&[(DENTIST, "Dentist, Fri 25 Sep 13:00")]);
        for args in [
            serde_json::json!(null),
            serde_json::json!("01a0c718-3931-7342-b9c7-8de36140ddb0"),
            serde_json::json!({"id": 7184}),
            serde_json::json!({"pid": 7184, "id": "not-in-the-index"}),
        ] {
            assert_eq!(target_line(&args, &naming), "", "{args}");
        }
        assert_eq!(target_line(&serde_json::json!({"id": DENTIST}), &Naming::new()), "", "an app that names nothing");
    }

    /// The index as `describe` answers it: an id→name map the app wrote, and an entry that is
    /// not a name — which is skipped, not stringified (see `naming_in`).
    #[test]
    fn the_index_is_read_off_the_state_the_app_publishes() {
        let mut entries = serde_json::Map::new();
        entries.insert(DENTIST.into(), serde_json::json!("Dentist, Fri 25 Sep 13:00"));
        entries.insert("not-a-name".into(), serde_json::json!(7));
        let reply = serde_json::json!({
            "app": "calendar",
            "state": { "naming": serde_json::Value::Object(entries) },
        });
        let naming = naming_in(&reply);
        assert_eq!(naming.get(DENTIST).map(String::as_str), Some("Dentist, Fri 25 Sep 13:00"));
        assert_eq!(naming.len(), 1, "the number is not a name: {naming:?}");
        assert!(naming_in(&serde_json::json!({"state": {}})).is_empty(), "an app that publishes none");
        assert!(naming_in(&serde_json::Value::Null).is_empty(), "an app that answered nothing at all");
    }

    /// The string reaches the Slint row as the store gave it: `row_for` is a pass-through, so
    /// the card cannot be the place a name goes missing.
    #[test]
    fn the_row_carries_the_line_out_of_the_shell_untouched() {
        use crate::approvals::{Card, Status, Verified};
        let line = format!("id 01a0c718\u{2026} is \u{201c}Dentist, Fri 25 Sep 13:00\u{201d}");
        let row = super::row_for(Card {
            id: "appr-9".into(),
            requester: "pi 0.87".into(),
            verified: Verified::default(),
            app: "calendar".into(),
            action: "delete_event".into(),
            grade: "sensitive".into(),
            purpose: "Take an event off the calendar.".into(),
            summary: "Take an event off the calendar.".into(),
            args: vec![format!("id: {DENTIST}")],
            target: line.clone(),
            warning: String::new(),
            can_session: false,
            status: Status::Pending,
            record: String::new(),
            age_secs: 12,
        });
        assert_eq!(row.target.as_str(), line.as_str());
    }
}
