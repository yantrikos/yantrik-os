//! What the companion is for: summarising a message, drafting from an instruction, suggesting a
//! reply, rewriting a draft, and sorting one message into a bucket. Five things a model is
//! genuinely better at than a rule. Each one reads the message that is open, or the composer,
//! and the companion is asked from a job.
//!
//! Mail is someone else's words. It goes to the companion as the message to work on, under an
//! instruction that says what to do with it, and what comes back is put in the composer unsent or
//! on the card it is about; it is never acted on.

use yantrik_app::effects::Effects;

use super::{clear_notice, say};
use crate::command::{Ask, Command, Work};
use crate::model::{part, Model};
use crate::{rows, Email};

/// The labels the screen can draw. A model that answers something else is reported, not
/// rendered as a category nobody defined.
const KNOWN: [&str; 4] = ["action-needed", "fyi", "meeting", "newsletter"];

pub fn ask(m: &mut Model, ask: Ask, fx: &mut Effects<Email>) {
    let open = m.open.as_ref().filter(|o| !o.detail.body.is_empty());
    let prompt = match &ask {
        Ask::Summarize | Ask::SuggestReply | Ask::Classify => {
            let Some(o) = open else {
                let what = if ask == Ask::Classify { "classify" } else { "read" };
                return say(m, fx, format!("There is no message open to {what}."));
            };
            let d = &o.detail;
            match ask {
                Ask::Summarize => format!(
                    "Summarise this email in at most three short lines. Say what is being asked of \
                     me, if anything. Use only what it says.\n\nFrom: {}\nSubject: {}\n\n{}",
                    d.from_name, d.subject, d.body
                ),
                Ask::SuggestReply => format!(
                    "Draft a short reply to this email. Match its register. Reply with the body \
                     only.\n\nFrom: {}\nSubject: {}\n\n{}",
                    d.from_name, d.subject, d.body
                ),
                _ => format!(
                    "Put this email in exactly one of these categories: action-needed, fyi, \
                     meeting, newsletter. Reply with the category and nothing else.\n\nFrom: \
                     {}\nSubject: {}\n\n{}",
                    d.from_name, d.subject, d.body
                ),
            }
        }
        Ask::Draft(instruction) => {
            if instruction.trim().is_empty() {
                return say(m, fx, "Say what the message should be about.");
            }
            format!(
                "Draft an email for this instruction. Reply with the body only, no subject \
                 line and no commentary.\n\n{instruction}"
            )
        }
        Ask::Enhance(style) => {
            let body = m.composer.text.body.clone();
            if body.trim().is_empty() {
                return say(m, fx, "There is nothing in the composer to rewrite.");
            }
            format!(
                "Rewrite this email to be {style}. Keep every fact and every commitment. \
                 Reply with the body only.\n\n{body}"
            )
        }
    };
    // An answer about a message belongs on that message: it is dropped if another one has been
    // opened by the time it comes back.
    let about = match ask {
        Ask::Summarize | Ask::SuggestReply | Ask::Classify => m.open.as_ref().map(|o| (o.id.clone(), o.folder.clone())),
        Ask::Draft(_) | Ask::Enhance(_) => None,
    };
    m.ai_working = true;
    fx.changed(part::STATUS);
    let mind = m.mind.clone();
    fx.spawn_or(Command::Unfinished(Work::Ai), move |_| {
        let outcome = mind.ask(&prompt).map_err(|e| e.to_string());
        Command::Answered { ask, about, outcome }
    });
}

pub fn answered(m: &mut Model, ask: Ask, about: Option<(String, String)>, outcome: Result<String, String>, fx: &mut Effects<Email>) {
    m.ai_working = false;
    fx.changed(part::STATUS);
    let still_open = |m: &Model| match &about {
        Some((id, folder)) => m.open.as_ref().is_some_and(|o| o.id == *id && o.folder == *folder),
        None => true,
    };
    if !still_open(m) {
        return;
    }
    let text = match outcome {
        Ok(text) => text,
        // Said out loud, not logged. A model that could not be reached left the card exactly as
        // it was and the person waiting at it with no idea why.
        Err(e) => return say(m, fx, e),
    };
    match ask {
        // The summary belongs on the message it is about.
        Ask::Summarize => {
            if let Some(open) = m.open.as_mut() {
                open.detail.ai_summary = text;
                fx.changed(part::OPEN);
            }
            clear_notice(m, fx);
        }
        // A suggested reply belongs in the composer, unsent.
        Ask::SuggestReply => {
            if let Some(open) = m.open.as_ref() {
                m.composer.open = true;
                m.composer.text = rows::reply(&open.detail, false);
            }
            m.composer.text.body = text;
            fx.changed(part::COMPOSE_TEXT);
            clear_notice(m, fx);
        }
        Ask::Draft(_) | Ask::Enhance(_) => {
            m.composer.text.body = text;
            fx.changed(part::COMPOSE_TEXT);
            clear_notice(m, fx);
        }
        Ask::Classify => {
            let label = text.trim().trim_matches('.').to_lowercase();
            if KNOWN.contains(&label.as_str()) {
                m.ai_intent = label;
                clear_notice(m, fx);
            } else {
                m.ai_intent.clear();
                say(
                    m,
                    fx,
                    format!(
                        "The companion answered \u{201c}{}\u{201d}, which is not one of the four categories this screen knows.",
                        text.trim()
                    ),
                );
            }
        }
    }
}
