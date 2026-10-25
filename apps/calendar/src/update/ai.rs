//! The agent layer: the header's AI button, the rail's two suggestions, and the card.
//!
//! Calendar had no companion connection at all once: its AI button logged a line and returned.
//! The rail and the card follow the same rule Notes does — every row is something this app or the
//! companion actually holds, and says where it came from. The header's button and the rail's
//! "What does this day look like?" are one question: while the rail can ask, it asks; when it
//! cannot (the Mind's reach is not ready), the button still asks the companion, as it always did,
//! and what came back is drawn on the same card.

use yantrik_app::jobs::{Concurrency, JobKey, Scope};
use yantrik_app::mind::AskError;
use yantrik_app::rail::{self, Proposal, RailCommand};
use yantrik_app::Refused;
use yantrik_ipc_transport::privacy;

use super::Fx;
use crate::command::Command;
use crate::model::{Model, PART_AI};
use crate::Calendar;

/// The rail's suggestion and the header's button.
pub const EXPLAIN: &str = "explain";
/// The rail's "Back to today".
pub const TODAY: &str = "today";

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Explain => explain(m, fx)?,
        Command::DismissAi => {
            // The header's AI button is a toggle and the second press has to put away what the
            // first press put up: the answer is on the card.
            dismiss(m, fx)?;
        }
        Command::Explained(answer) => {
            m.ai.working = false;
            if m.ai.asking {
                m.ai.asking = false;
                m.ai.card = Some(match answer {
                    Ok(text) => answer_card(&m.ai.day, text),
                    Err(e) => {
                        tracing::warn!(error = %e, "Companion call failed");
                        unanswered(&e)
                    }
                });
            }
            fx.changed(PART_AI);
        }
        Command::Rail(command) => rail_command(m, command, fx)?,
        other => unreachable!("not an agent command: {other:?}"),
    }
    Ok(())
}

/// The rail's commands, with the two suggestions that are this app's own buttons routed to them.
fn rail_command(m: &mut Model, command: RailCommand, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        // One code path: the rail presses the same button the toolbar does.
        RailCommand::Activate(id) if id == EXPLAIN => explain(m, fx)?,
        RailCommand::Activate(id) if id == TODAY => super::update(m, Command::Today, fx)?,
        RailCommand::Answered { suggestion, answer } => {
            if suggestion == EXPLAIN {
                m.ai.working = false;
                if let Err(e) = &answer {
                    tracing::warn!(error = %e, "Companion call failed");
                }
                fx.changed(PART_AI);
            }
            rail::update::<Calendar>(m, RailCommand::Answered { suggestion, answer }, fx)?;
        }
        RailCommand::Dismiss => dismiss(m, fx)?,
        other => rail::update::<Calendar>(m, other, fx)?,
    }
    Ok(())
}

/// Ask what the day looks like. What the day holds is read off the model rather than described
/// to the companion second-hand (`rail::day_question`).
fn explain(m: &mut Model, fx: &mut Fx) -> Result<(), Refused> {
    m.ai.day = if m.selected_day > 0 {
        format!("{} {}", m.month_title(), m.selected_day)
    } else {
        m.month_title()
    };
    if m.rail.reach().hint().is_none() && !m.rail.is_applying() {
        rail::update::<Calendar>(m, RailCommand::Activate(EXPLAIN.into()), fx)?;
        m.ai.card = None;
        m.ai.asking = false;
    } else {
        m.ai.card = Some(Proposal {
            title: "Your day".into(),
            source: format!("from {}", m.ai.day),
            ..Default::default()
        });
        m.ai.asking = true;
        let prompt = crate::rail::day_question(m);
        let mind = m.mind.clone();
        fx.spawn_keyed_or(
            JobKey { feature: "ai", scope: Scope::App },
            Concurrency::ReplacePrevious,
            Command::Explained(Err(AskError::Failed("the question did not finish".into()))),
            move |_| {
                // Private mode keeps this desktop from every agent, as the rail's own ask does.
                let answer = if privacy::is_private() {
                    Err(AskError::Failed(privacy::REFUSAL.into()))
                } else {
                    mind.ask(&prompt)
                };
                Command::Explained(answer)
            },
        );
    }
    m.ai.working = true;
    fx.changed(PART_AI);
    Ok(())
}

/// The card goes away, whichever drew it, and a late answer stays away with it; so does the
/// button's spinner.
fn dismiss(m: &mut Model, fx: &mut Fx) -> Result<(), Refused> {
    rail::update::<Calendar>(m, RailCommand::Dismiss, fx)?;
    m.ai.card = None;
    m.ai.asking = false;
    m.ai.working = false;
    fx.changed(PART_AI);
    Ok(())
}

/// The companion's answer about the day. Reading, not changing: the card gives one button.
pub fn answer_card(day: &str, text: String) -> Proposal {
    Proposal {
        title: "Your day".into(),
        body: text,
        source: format!("from {day}"),
        verb: "Close".into(),
        ..Default::default()
    }
}

/// The card when the companion did not answer.
fn unanswered(why: &AskError) -> Proposal {
    Proposal {
        title: "The companion did not answer".into(),
        body: why.to_string(),
        verb: "Close".into(),
        ..Default::default()
    }
}
