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

/// What `mind`'s own config asks the gateway for: `picked`, or a model named there since. `picked`
/// too when the config cannot say (the gateway is then asked as the mind's pick). Every face reads
/// the answer through [`answering`], so every face follows the same route.
pub(crate) fn asked_for(home: &std::path::Path, mind: &str) -> String {
    crate::provider_handoff::adapter_for(mind).and_then(|a| a.asks_gateway_for(home)).unwrap_or_else(|| yantrik_gateway::PICKED.to_string())
}

/// The model answering `mind`. A refusal is what the gateway would say to its next call.
pub fn answering(mind: &str) -> Result<Answer, Refusal> {
    let asked = asked_for(&crate::wire::harness_provider::home_dir(), mind);
    let planned = crate::gateway::route_for(mind, &asked).map(|r| Plan {
        account_id: r.target.account,
        model: r.target.model,
        lane: r.lane_for.is_some(),
    });
    resolve(crate::gateway::log().last_ok_for(mind), planned, account_name)
}

/// What a refused route is said as, in place of a model, on a mind's detail line.
const REFUSED: &str = "the gateway refuses its calls";

/// A look-alike of the ` - ` separator, which a harness could put in its own words to make two
/// segments read as one.
const LOOKALIKES: &[char] = &['\u{2219}', '\u{2022}', '\u{22c5}', '\u{2027}', '\u{30fb}', '\u{ff65}', '\u{b7}'];

/// A piece of a harness's own words as it may be drawn: the host's one sanitiser (a single line,
/// at most 120 characters, no bidi or control characters, no sign-in in an address), and no
/// separator look-alikes.
fn plain(text: &str) -> String {
    let flat: String = text.chars().map(|c| if LOOKALIKES.contains(&c) { '-' } else { c }).collect();
    yantrik_harness::host::detail::shown_detail(&flat).unwrap_or_default()
}

/// A mind's detail line with the model the shell says answers it (`None`: none does).
///
/// The head is the mind's own name, with its version when it announced one first (Hermes:
/// "Hermes 0.14.0 - <model>"). Harnesses that announce their model first (Pi: "<model> - pi 1.2",
/// DeepSeek: "<model> - <host> - X picks") keep what follows that model, and the model they
/// named is dropped: it may be from before a restart or a pick change, and it is never the head.
/// Everything taken from the mind passes [`plain`].
pub(crate) fn detail_with_model(announced: Option<&str>, name: &str, model: Option<&str>) -> String {
    let parts: Vec<&str> = announced.map(|d| d.split(" \u{b7} ").map(str::trim).filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let word = |s: &str| s.split_whitespace().next().unwrap_or("").to_lowercase();
    let (head, rest): (String, &[&str]) = match parts.split_first() {
        Some((first, _)) if !word(name).is_empty() && word(first) == word(name) => (plain(first), &[]),
        Some((_, rest)) => (plain(name), rest),
        None => (plain(name), &[]),
    };
    let mut line = if head.is_empty() { plain(name) } else { head };
    if let Some(model) = model {
        line = format!("{line} \u{b7} {}", plain(model));
    }
    for part in rest {
        line = format!("{line} \u{b7} {}", plain(part));
    }
    line
}

/// What the host lists as `mind`'s detail (`name` is what it is listed under): for a mind on the
/// gateway, the model `answering` says (the called one, or the route's own before any call). When
/// the gateway would refuse its calls, no model: the refusal, as the panel and the Lens say it,
/// not the mind's own report of the pick. `None` for a mind that is not on the gateway: its own
/// line stands.
pub(crate) fn model_line(mind: &str, name: &str, announced: Option<&str>) -> Option<String> {
    let home = crate::wire::harness_provider::home_dir();
    if !crate::provider_handoff::uses_gateway(&home, mind) {
        return None;
    }
    Some(match answering(mind) {
        Ok(answer) => detail_with_model(announced, name, Some(&answer.model)),
        Err(_) => format!("{} \u{b7} {REFUSED}", detail_with_model(announced, name, None)),
    })
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
