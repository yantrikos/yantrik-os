//! Which model is answering, said once (#1114). The mind panel, the chat bubble, the Lens and
//! `describe` all read it here, so they cannot name different models.
//!
//! The newest call the gateway answered for a mind is what answered it; before any call, the
//! gateway's own routing says where the next one goes (`gateway::route_for`). The stored pick
//! (`picker.json`) and the harness's own `model: picked` are never read to say it: a file names
//! what was asked for, and a pick that had gone silently became another model on 520.
//!
//! An account is always named by its display name, never by its slug.

use yantrik_gateway::log::Call;
use yantrik_gateway::Refusal;

/// The model answering a mind, or that will.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub account_id: String,
    /// The account as the person reads it ("Ollama Cloud").
    pub account: String,
    /// The provider's own name for the model.
    pub model: String,
    /// The private lane answered (or will), in place of the account the pick names.
    pub lane: bool,
    /// When the gateway last answered a call with it; `None` before any.
    pub called_at: Option<i64>,
}

impl Answer {
    /// `<account>/<model>`, as the gateway and the picker name a model.
    pub fn id(&self) -> String {
        format!("{}/{}", self.account_id, self.model)
    }

    /// "answering · Ollama Cloud · deepseek-v4.1-flash" after a call, "will send to …" before one.
    pub fn words(&self) -> String {
        let lane = if self.lane { " \u{b7} via your lane" } else { "" };
        match self.called_at {
            Some(_) => format!("answering \u{b7} {} \u{b7} {}{lane}", self.account, self.model),
            None => format!("will send to {} \u{b7} {}{lane}", self.account, self.model),
        }
    }
}

/// Where the gateway would send the next call.
struct Plan {
    account_id: String,
    model: String,
    lane: bool,
}

/// The model answering `mind`. A refusal is what the gateway would say to its next call.
pub fn answering(mind: &str) -> Result<Answer, Refusal> {
    answering_asking(mind, yantrik_gateway::PICKED)
}

/// [`answering`] for a mind whose own config asks the gateway for `asked` (`picked`, or a model
/// named there).
pub(crate) fn answering_asking(mind: &str, asked: &str) -> Result<Answer, Refusal> {
    let planned = crate::gateway::route_for(mind, asked).map(|r| Plan {
        account_id: r.target.account,
        model: r.target.model,
        lane: r.lane_for.is_some(),
    });
    resolve(crate::gateway::log().last_ok_for(mind), planned, account_name)
}

/// The call that was answered wins over the plan while the plan still points where it went; a plan
/// that is refused wins over both, because the next call would be. A plan that points elsewhere
/// means the pick (or its route) changed after that call, so the new route is said ("will send
/// to") until a call to it succeeds.
fn resolve(last_ok: Option<Call>, planned: Result<Plan, Refusal>, name_of: impl Fn(&str) -> String) -> Result<Answer, Refusal> {
    let planned = planned?;
    let last_ok = last_ok.filter(|c| c.account == planned.account_id && c.model == planned.model);
    Ok(match last_ok {
        Some(c) => Answer { account: name_of(&c.account), account_id: c.account, model: c.model, lane: c.lane_for.is_some(), called_at: Some(c.at) },
        None => Answer { account: name_of(&planned.account_id), account_id: planned.account_id, model: planned.model, lane: planned.lane, called_at: None },
    })
}

fn account_name(id: &str) -> String {
    crate::ai_accounts::accounts().into_iter().find(|a| a.id == id).map(|a| a.name).unwrap_or_else(|| "an account that is gone".to_string())
}

/// "Ollama Cloud · deepseek-v4.1-flash" for `<account>/<model>`; the model alone when no account
/// by that id exists, since an id that matches nothing is a slug the person never saw.
pub(crate) fn display_model(id: &str) -> String {
    let (account, model) = id.split_once('/').unwrap_or(("", id));
    match crate::ai_accounts::accounts().into_iter().find(|a| a.id == account) {
        Some(a) => format!("{} \u{b7} {model}", a.name),
        None => model.to_string(),
    }
}

/// The name a mind goes by on screen.
pub(crate) fn mind_name(id: &str) -> String {
    crate::wire::harness::host()
        .and_then(|h| h.list().into_iter().find(|e| e.id == id).map(|e| e.name))
        .unwrap_or_else(|| id.to_string())
}

/// The sentence for a pick that is offered by no account any more. `using` is the default pick that
/// answers instead; `None` when there is none and the mind waits.
pub(crate) fn gone_words(mind: &str, asked: &str, using: Option<&str>) -> String {
    let gone = format!("Your pick, {}, isn't offered by any of your accounts any more.", display_model(asked));
    match using {
        Some(default) => format!("{gone} {mind} is using your default, {}.", display_model(default)),
        None => format!("{gone} You have no default pick, so {mind} waits until you pick again."),
    }
}

#[cfg(test)]
mod tests;
