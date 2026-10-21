//! What the companion may ask Email to do by name: the seven actions, worded, argued and graded
//! as they always were, each the command its button runs.
//!
//! An action that reaches the mail server is answered once the server's answer has landed
//! (`Settle::Explicit`): what it reports is what the mailbox says afterwards.
//!
//! Setting an account up with a password is deliberately absent. `docs/app-control.md` is
//! explicit that the transcript a mind works in is readable. An action that accepted a password
//! would put a mail credential in it, in the clear, for every caller of `app.describe`
//! afterwards. `save_account` and `test_connection` are therefore not on this surface and never
//! will be. `begin_google_sign_in` is the exception that proves the rule rather than a hole in
//! it: it accepts nothing, carries nothing back, and the credential it ends in never enters this
//! process at all — the mail service holds the token and the app is told an address.

use std::time::Duration;

use serde_json::{json, Value};
use yantrik_app::control::{Action, Param};
use yantrik_app::intents::{Args, Intent, Intents, Settle};

use crate::command::{Command, Flag, Pick};
use crate::model::Model;
use crate::Email;

/// How long a caller waits for the mail server's answer: two calls of the ten-second budget,
/// the change and the folder read back after it.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(25);

/// An argument that may arrive as a string or as a number.
///
/// `args["which"].as_str().unwrap_or_default()` is where `which=1` became `""`.
fn arg_text(args: &Args<'_>, name: &str) -> String {
    match &args.raw()[name] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// An action answered when its work has landed, with what the model says then.
fn landed(action: Action, parse: impl Fn(&Args<'_>) -> Command + 'static) -> Intent<Email> {
    Intent::new(action, move |a| Ok(parse(a)))
        .settles(Settle::Explicit { within: ANSWER_WITHIN })
        .reply(|m: &Model| m.answer.clone())
}

pub fn table(table: &mut Intents<Email>) {
    table.add(landed(
        // Every action below is graded explicitly, and none of them is `safe`.
        //
        // `safe` in this OS's ladder is for something that changes nothing and stays on the
        // machine. Every action on this surface reaches a mail server across the network —
        // selecting a folder and searching both do, which is not obvious from their names — and
        // opening a message marks it read out there, which is a change someone else can see.
        // So: `standard` throughout, deliberately, rather than by default.
        //
        // Nothing here is above `standard` because nothing here sends or deletes mail. Anything
        // that did would be `sensitive` at least: mail that has gone cannot be taken back, and a
        // deleted message is not on the server any more. That is why `send`, `delete` and
        // `archive` are not published at all, and why `compose` deliberately stops at a draft.
        Action::new("open_message", "Open a message by row number, subject or sender")
            .risk("standard")
            .arg(Param::text("which").describe(
                "The row number from `messages`, counting from 1 \u{2014} or part of the subject, \
                 the sender's name, or their address",
            )),
        // Accepts a number as well as a string. `which=1` used to arrive as a JSON number,
        // `as_str()` gave `""`, and the answer was `nothing in this folder matches ""` — a
        // search for nothing, reported as if it were what was asked.
        |a| Command::Open(Pick::Named(arg_text(a, "which"))),
    ));
    table.add(landed(
        // It looks local and is not: switching mailbox asks the mail server for that folder's
        // messages.
        Action::new("select_folder", "Switch to another mailbox")
            .risk("standard")
            .arg(Param::text("folder").describe("Folder name, e.g. INBOX, Sent, Archive")),
        |a| Command::SelectFolder(arg_text(a, "folder")),
    ));
    table.add(landed(
        // It changes nothing, and it still leaves the machine: the query goes to the mail server
        // as an IMAP SEARCH. A search that could not be run is a refusal: a mail server that
        // refused the query and a mailbox with no matches are not the same answer.
        Action::new("search", "Search the mailbox").risk("standard").arg(Param::text("query")),
        |a| Command::SearchFor(arg_text(a, "query")),
    ));
    table.add(landed(
        Action::new("mark_read", "Mark a message as read").risk("standard").arg(
            Param::text("which").describe("Row number, subject or sender; omit for the open message").optional(),
        ),
        // The flag as the mail server reports it after the change, not the row that was asked
        // about. `{"marked": row}` was true of the request and said nothing about the mailbox.
        |a| Command::Mark { pick: Pick::Named(arg_text(a, "which")), flag: Flag::Read, toggle: false },
    ));
    table.add(landed(
        Action::new("flag", "Flag a message").risk("standard").arg(
            Param::text("which").describe("Row number, subject or sender; omit for the open message").optional(),
        ),
        |a| Command::Mark { pick: Pick::Named(arg_text(a, "which")), flag: Flag::Starred, toggle: false },
    ));
    table.add(
        Intent::new(
            Action::new(
                "compose",
                "Open the composer with a draft filled in. Does NOT send — the user reviews and sends it.",
            )
            .risk("standard")
            .arg(Param::text("to"))
            .arg(Param::text("subject"))
            .arg(Param::text("body")),
            |a| {
                Ok(Command::Compose {
                    to: arg_text(a, "to"),
                    subject: arg_text(a, "subject"),
                    body: arg_text(a, "body"),
                })
            },
        )
        // Sending is not offered on this surface on purpose: a draft can be read before it
        // leaves, and mail that has already gone cannot be taken back.
        .reply(|_| json!({ "drafted": true, "note": "waiting for the user to send it" })),
    );
    table.add(
        Intent::new(
            // The one setup step that can be on this surface, because it is the one that takes
            // no credential: it opens Google's own consent page and stops.
            //
            // `sensitive` rather than `standard`, and the grade is about what it does to the
            // person rather than to the machine. By itself it changes nothing — no account is
            // written, no mailbox is touched, and the flow cannot complete without somebody
            // choosing an account and reading a consent screen that says full access to Gmail.
            // But it puts that screen in front of them, unasked, on their own display, and a
            // consent page a person did not go looking for is exactly the shape of the thing
            // they should not be trained to click through. So it is above `standard`, where a
            // `tool_permission: standard` ceiling refuses it, and the default ceiling allows it.
            //
            // It takes no arguments on purpose. There is nothing to parameterise: the provider
            // is Google because the scope and the servers are Google's, and the address comes
            // from the sign-in rather than from a caller.
            Action::new(
                "begin_google_sign_in",
                "Open Google's sign-in page to add a Gmail account. Cannot finish on its own \u{2014} \
                 the user chooses the account and grants access in their browser.",
            )
            .risk("sensitive"),
            |_| Ok(Command::SignInWithGoogle),
        )
        // What was observed, which is that a browser was asked to open. Not "signed in": whether
        // this works is decided in a browser by a person, minutes from now, and `describe`
        // carries the answer when it arrives.
        .reply(|_| {
            json!({
                "opened": true,
                "note": "Google's consent page was opened. The user has to choose an account \
                         and allow access; ask describe again for the outcome.",
            })
        }),
    );
}
