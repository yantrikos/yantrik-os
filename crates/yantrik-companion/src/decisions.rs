//! The decision model, shared with every thread that asks it.
//!
//! The companion owns the choice (Settings switches it, `judge_route::build_judge` builds it) and
//! publishes it here. Callers ask here, on their own thread, so a decision never waits behind a
//! chat turn on the companion's. Every answer is a `Verdict` (`docs/decisions.md`), abstentions
//! included: a use switched off, incognito, or no model at all answers "abstain", and the caller
//! decides as it would without one.
//!
//! Who may ask for what is decided here, against the uses (`JUDGE_USES`):
//! - the person's own surfaces ask for any use with a door (`browser_commitment`, `agent`);
//! - an agent asks only for `agent`, and only of a model on this machine or the home network. A
//!   cloud model would carry whatever an agent had read out of the house, past every taint rule
//!   that watches the agent's other ways out.

use std::sync::{Arc, RwLock};

use serde_json::Value;
use yantrik_companion_core::judge_config::{judge_use, JudgeConfig, JUDGE_USES};
use yantrik_ml::judge::{Answer, Judge, JudgeInfo, Locality, OffJudge, Question, Verdict};

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// The person's own software: the shell, the browser service.
    Person,
    /// An agent: a caller with a token, or the mind account.
    Agent,
}

/// The decision model in use, its uses, and whether the desktop is incognito.
#[derive(Clone, Default)]
pub struct Decisions(Arc<RwLock<Desk>>);

struct Desk {
    judge: Option<Arc<dyn Judge>>,
    config: JudgeConfig,
    incognito: bool,
    /// Where the chat model runs (`judge_route::chat_locality`), for Settings to say where a
    /// `chat_model` decision model would send what it judges before one is built.
    chat_at: Locality,
}

impl Default for Desk {
    fn default() -> Self {
        // Nothing published yet: no model, and the chat model assumed to be in the cloud.
        Desk { judge: None, config: JudgeConfig::default(), incognito: false, chat_at: Locality::Cloud }
    }
}

impl Decisions {
    /// What the companion now uses; called whenever the model, its uses or incognito change.
    pub fn publish(&self, judge: Option<Arc<dyn Judge>>, config: &JudgeConfig, incognito: bool, chat_at: Locality) {
        if let Ok(mut desk) = self.0.write() {
            *desk = Desk { judge, config: config.clone(), incognito, chat_at };
        }
    }

    /// Where the chat model runs, as last published.
    pub fn chat_locality(&self) -> Locality {
        self.0.read().map(|d| d.chat_at).unwrap_or(Locality::Cloud)
    }

    /// Who answers now, `None` when no model is set.
    pub fn info(&self) -> Option<JudgeInfo> {
        self.0.read().ok()?.judge.as_ref().map(|j| j.info())
    }

    /// A copy of what is published, taken so no lock is held while a model thinks. A poisoned
    /// lock reads as incognito: nothing is sent.
    fn now(&self) -> (Option<Arc<dyn Judge>>, JudgeConfig, bool) {
        match self.0.read() {
            Ok(desk) => (desk.judge.clone(), desk.config.clone(), desk.incognito),
            Err(_) => (None, JudgeConfig::default(), true),
        }
    }

    /// Put wire-form `questions` about `state` to the model, for the use `purpose`. `Err` is a
    /// refusal or a malformed request, and nothing was sent; an answer the model could not give
    /// is an abstention inside the verdict.
    /// Whether `caller` may ask for `purpose` of the model in use now: every refusal `ask` makes
    /// before anything is sent, and cheap, so a door can decide before it holds a place.
    pub fn admit(&self, purpose: &str, caller: Caller) -> Result<(), String> {
        let door: Vec<&str> = JUDGE_USES.iter().filter(|u| u.door).map(|u| u.id).collect();
        let Some(the_use) = judge_use(purpose).filter(|u| u.door) else {
            return Err(format!("`{purpose}` is not a use of the decision model that can be asked for ({})", door.join(", ")));
        };
        if caller == Caller::Agent && !the_use.for_agents {
            return Err(format!("an agent asks the decision model for `agent`, not `{purpose}`: refused, nothing was sent."));
        }
        if caller == Caller::Agent {
            let (judge, _, _) = self.now();
            if let Some(judge) = judge {
                if !matches!(judge.info().locality, Locality::ThisMachine | Locality::Home) {
                    return Err(AGENT_TO_CLOUD.into());
                }
            }
        }
        Ok(())
    }

    pub fn ask(&self, state: &Value, questions: &Value, purpose: &str, caller: Caller) -> Result<Verdict, String> {
        self.admit(purpose, caller)?;
        let parsed = yantrik_ml::judge::questions_from_json(questions)?;
        let asked: Vec<(&str, Question)> = parsed.iter().map(|(id, q)| (id.as_str(), q.clone())).collect();
        let (judge, config, incognito) = self.now();
        if incognito {
            return Ok(abstaining(state, &asked, "incognito: nothing is sent to a decision model"));
        }
        let Some(judge) = judge else {
            return Ok(abstaining(state, &asked, "no decision model is set"));
        };
        // The same judge the locality is read from and the question is put to: one snapshot.
        let at = judge.info().locality;
        if caller == Caller::Agent && !matches!(at, Locality::ThisMachine | Locality::Home) {
            return Err(AGENT_TO_CLOUD.into());
        }
        if !config.use_on_where(purpose, at == Locality::Cloud) {
            return Ok(abstaining(state, &asked, "this use of the decision model is switched off in Settings"));
        }
        Ok(judge.decide(state, &asked))
    }

    /// `ask`, answered in the wire form.
    pub fn ask_json(&self, state: &Value, questions: &Value, purpose: &str, caller: Caller) -> Result<Value, String> {
        self.ask(state, questions, purpose, caller).map(|v| v.to_json())
    }

    /// One known question, for Settings' Test button. The answer is known (pressing "Place your
    /// order" on a checkout spends money), so a result far from yes says the model is
    /// misconfigured, not only that it answered. Asked even when a use is off: it tests the model.
    pub fn test(&self) -> Result<Verdict, String> {
        let (judge, _, _) = self.now();
        let judge = judge.ok_or("no decision model is set")?;
        let state = serde_json::json!({
            "control": {"role": "button", "label": "Place your order"},
            "page": {"title": "Checkout", "heading": "Review your order"},
            "nearby_text": "Order total: $42.17",
        });
        let verdict = judge.decide(&state, &[("commit", Question::Noul {
            instructions: "Would pressing this control spend money or place an order?".into(),
        })]);
        match verdict.get("commit") {
            Some(Answer::Abstain { reason }) => Err(reason.clone()),
            _ => Ok(verdict),
        }
    }
}

const AGENT_TO_CLOUD: &str = "the decision model in use is not on this machine or the home network, and an agent's \
                              questions go only to one that is: refused, nothing was sent.";

/// A verdict that abstains on every question, saying why.
fn abstaining(state: &Value, asked: &[(&str, Question)], why: &str) -> Verdict {
    let mut verdict = OffJudge.decide(state, asked);
    for answer in verdict.answers.values_mut() {
        *answer = Answer::Abstain { reason: why.to_string() };
    }
    verdict
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    /// A model that says yes to everything, from wherever it is told it runs, and counts the
    /// times it was asked.
    struct Yes(Locality, AtomicUsize);

    impl Judge for Yes {
        fn name(&self) -> &str {
            "yes"
        }
        fn info(&self) -> JudgeInfo {
            JudgeInfo { adapter: "systemone", provider: "yes".into(), model: "yes".into(), locality: self.0, calibrated: true }
        }
        fn ask(&self, _: &Value, questions: &[(&str, Question)]) -> anyhow::Result<HashMap<String, Answer>> {
            self.1.fetch_add(1, Ordering::SeqCst);
            Ok(questions.iter().map(|(id, _)| ((*id).to_string(), Answer::Noul(0.9))).collect())
        }
    }

    fn desk(at: Locality) -> (Decisions, Arc<Yes>) {
        let yes = Arc::new(Yes(at, AtomicUsize::new(0)));
        let d = Decisions::default();
        d.publish(Some(yes.clone()), &JudgeConfig::default(), false, Locality::ThisMachine);
        (d, yes)
    }

    fn q() -> Value {
        json!({"q": {"type": "noul", "instructions": "Is it?"}})
    }

    fn abstained(v: &Verdict) -> bool {
        v.answers.values().all(Answer::is_abstain)
    }

    #[test]
    fn a_surface_asks_for_any_use_with_a_door() {
        let (d, _) = desk(Locality::Home);
        for purpose in ["browser_commitment", "agent"] {
            let v = d.ask(&json!({}), &q(), purpose, Caller::Person).unwrap();
            assert_eq!(v.get("q"), Some(&Answer::Noul(0.9)), "{purpose}");
        }
        assert!(d.ask(&json!({}), &q(), "route_tools", Caller::Person).is_err(), "tool choice is the companion's own");
        assert!(d.ask(&json!({}), &q(), "wire_money", Caller::Person).is_err());
    }

    #[test]
    fn a_cloud_model_is_sent_pages_only_once_the_person_switches_that_on() {
        let (d, yes) = desk(Locality::Cloud);
        assert!(abstained(&d.ask(&json!({}), &q(), "browser_commitment", Caller::Person).unwrap()));
        assert_eq!(yes.1.load(Ordering::SeqCst), 0, "nothing went to the cloud");
        let mut config = JudgeConfig::default();
        config.set_use("browser_commitment", true);
        d.publish(Some(yes.clone()), &config, false, Locality::Cloud);
        assert!(!abstained(&d.ask(&json!({}), &q(), "browser_commitment", Caller::Person).unwrap()));
    }

    #[test]
    fn an_agent_asks_only_for_agent_and_only_a_model_in_the_house() {
        for at in [Locality::ThisMachine, Locality::Home] {
            let (d, _) = desk(at);
            assert!(!abstained(&d.ask(&json!({}), &q(), "agent", Caller::Agent).unwrap()), "{at:?}");
            assert!(d.ask(&json!({}), &q(), "browser_commitment", Caller::Agent).is_err());
        }
        let (d, yes) = desk(Locality::Cloud);
        let refused = d.ask(&json!({"secret": "x"}), &q(), "agent", Caller::Agent).unwrap_err();
        assert!(refused.contains("nothing was sent"), "{refused}");
        assert_eq!(yes.1.load(Ordering::SeqCst), 0, "the cloud model was never asked");
    }

    #[test]
    fn off_incognito_and_no_model_abstain_without_asking() {
        let (d, yes) = desk(Locality::ThisMachine);
        let mut config = JudgeConfig::default();
        config.set_use("browser_commitment", false);
        d.publish(Some(yes.clone()), &config, false, Locality::ThisMachine);
        let v = d.ask(&json!({}), &q(), "browser_commitment", Caller::Person).unwrap();
        assert!(abstained(&v));
        assert!(!abstained(&d.ask(&json!({}), &q(), "agent", Caller::Person).unwrap()), "the other uses are still on");
        d.publish(Some(yes.clone()), &JudgeConfig::default(), true, Locality::ThisMachine);
        assert!(abstained(&d.ask(&json!({}), &q(), "agent", Caller::Agent).unwrap()));
        assert_eq!(yes.1.load(Ordering::SeqCst), 1, "only the use that was on asked the model");
        let none = Decisions::default();
        assert!(abstained(&none.ask(&json!({}), &q(), "browser_commitment", Caller::Person).unwrap()));
        assert!(none.test().is_err());
    }

    #[test]
    fn a_malformed_question_is_refused_before_anything_is_asked() {
        let (d, yes) = desk(Locality::ThisMachine);
        assert!(d.ask(&json!({}), &json!({"q": {"type": "essay"}}), "agent", Caller::Person).is_err());
        assert_eq!(yes.1.load(Ordering::SeqCst), 0);
    }
}
