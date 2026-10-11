//! The free pool: many providers' free tiers, used as one.
//!
//! The person switches providers on in Settings and gives each its key; the pool decides, call by
//! call, which of them takes a request, keeps inside every provider's limits, and moves on when
//! one refuses. Nothing here sends a request: a caller asks `pick`, makes the call, and reports
//! how it went to `record`. So the same pool serves the desktop's own backend and the gate's
//! proxy, and is tested with no network at all.
//!
//! One account per provider, inside its free limits. Several accounts to stretch a free tier is
//! against nearly every provider's terms, and the pool's reach comes from breadth instead.

pub mod backend;
pub mod headers;
pub mod keycheck;
pub mod keys;
pub mod ledger;
pub mod quota;
pub mod select;
pub mod signup;
pub mod tiers;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod backend_tests;

use std::collections::HashMap;

use quota::{Blocked, Outcome, Quota};
use select::{Candidate, Need};
use tiers::{Counts, FreeModel, FreeTier, Limit, Reset, Scope, FREE_TIERS};

/// What the person chose in Settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Providers switched on, by catalogue id.
    pub enabled: Vec<String>,
    /// Providers that have a key saved (the keys themselves are never handed to the pool).
    pub keyed: Vec<String>,
    /// OpenRouter: the account has bought $10 of credit once, so it allows 1,000 free a day.
    pub openrouter_paid_credit: bool,
}

/// The model to call.
#[derive(Clone, Copy, Debug)]
pub struct Pick {
    pub tier: &'static FreeTier,
    pub model: &'static FreeModel,
}

/// Why nothing can be picked, and when something will be free again.
#[derive(Clone, Debug, PartialEq)]
pub struct Exhausted {
    pub reason: String,
    /// The soonest a provider frees up, if any will.
    pub until: Option<i64>,
}

/// One provider, as Settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TierStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub on: bool,
    /// "Ready", "Needs a key", "Key refused", "Cooling down until …", "Today's free use is spent".
    pub state: String,
    pub used_today: u64,
    /// The day's request cap, when the provider publishes one.
    pub daily_cap: Option<u64>,
    pub note: &'static str,
    pub trains_on_prompts: bool,
}

pub struct Pool {
    settings: Settings,
    quota: Quota,
    sticky: HashMap<String, (&'static str, &'static str)>,
    turn: usize,
    ledger: ledger::Ledger,
}

impl Pool {
    pub fn new(settings: Settings, ledger: ledger::Ledger) -> Pool {
        Pool { settings, quota: Quota::default(), sticky: HashMap::new(), turn: 0, ledger }
    }

    /// The person changed what is switched on, or which providers have a key.
    pub fn set(&mut self, settings: Settings) {
        for id in &settings.keyed {
            if !self.settings.keyed.contains(id) {
                self.quota.forget_refusal(id);
            }
        }
        self.settings = settings;
    }

    /// A provider's key was replaced: a refusal of the old one says nothing about the new one.
    pub fn key_changed(&mut self, provider: &str) {
        self.quota.forget_refusal(provider);
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    fn usable(&self, t: &FreeTier) -> bool {
        self.settings.enabled.iter().any(|e| e == t.id) && (!t.needs_key || self.settings.keyed.iter().any(|k| k == t.id))
    }

    /// A tier's limits, as they apply to this person.
    fn limits(&self, t: &FreeTier) -> Vec<Limit> {
        t.limits
            .iter()
            .map(|l| {
                if t.id == "openrouter" && self.settings.openrouter_paid_credit && l.reset == Reset::UtcMidnight {
                    Limit { amount: tiers::OPENROUTER_PAID_CREDIT_RPD, ..*l }
                } else {
                    *l
                }
            })
            .collect()
    }

    /// The model to send this request to, or why there is none.
    pub fn pick(&mut self, need: &Need, now: i64) -> Result<Pick, Exhausted> {
        let mut fits = Vec::new();
        let mut soonest: Option<i64> = None;
        let mut capable = 0;
        let usable: Vec<&'static FreeTier> = FREE_TIERS.iter().filter(|t| self.usable(t)).collect();
        for t in usable {
            if (!need.shareable && t.trains_on_prompts) || (need.public && !t.may_serve_public) {
                continue;
            }
            let limits = self.limits(t);
            for m in t.models.iter().filter(|m| fits_need(m, need)) {
                capable += 1;
                match self.quota.headroom(t, m, &limits, now) {
                    Ok(room) => fits.push(Candidate { provider: t.id, model: m.id, coding: m.coding, room }),
                    Err(Blocked::Until(u)) => soonest = Some(soonest.map_or(u, |s| s.min(u))),
                    Err(_) => {}
                }
            }
        }
        let sticky = need.sticky.as_ref().and_then(|s| self.sticky.get(s).copied());
        self.turn = self.turn.wrapping_add(1);
        let Some(c) = select::choose(need, &fits, sticky, self.turn) else {
            let reason = if capable == 0 {
                "No switched-on free provider has a model that can do this.".to_string()
            } else {
                "Every free provider that can do this is at its limit right now.".to_string()
            };
            return Err(Exhausted { reason, until: soonest });
        };
        let tier = tiers::tier(c.provider).expect("candidates come from FREE_TIERS");
        let model = tier.models.iter().find(|m| m.id == c.model).expect("candidates come from the tier's models");
        if let Some(s) = &need.sticky {
            self.sticky.insert(s.clone(), (tier.id, model.id));
        }
        Ok(Pick { tier, model })
    }

    /// How the call went. A refusal also lets go of the task's model, so its next step moves on.
    pub fn record(&mut self, pick: Pick, need: &Need, now: i64, out: &Outcome) {
        let limits = self.limits(pick.tier);
        self.quota.record(pick.tier, pick.model, &limits, now, out);
        let ok = (200..300).contains(&out.status);
        if !ok {
            if let Some(s) = &need.sticky {
                self.sticky.remove(s);
            }
        }
        self.ledger.add(&ledger::day_name(now), pick.tier.id, pick.model.id, ok, out.tokens);
        let _ = self.ledger.save();
    }

    /// Every free provider, for Settings: on or off, ready or why not, and today's use.
    pub fn status(&mut self, now: i64) -> Vec<TierStatus> {
        let today = self.ledger.day(&ledger::day_name(now));
        FREE_TIERS
            .iter()
            .map(|t| {
                let on = self.settings.enabled.iter().any(|e| e == t.id);
                let limits = self.limits(t);
                let cap = daily_cap(t, &limits);
                let used = today.get(t.id).map_or(0, |x| x.requests);
                let state = if !on {
                    "Off".to_string()
                } else if t.needs_key && !self.settings.keyed.iter().any(|k| k == t.id) {
                    "Needs a key".to_string()
                } else {
                    let mut blocked_until = None;
                    let mut any_ready = false;
                    let mut key_refused = false;
                    for m in t.models {
                        match self.quota.headroom(t, m, &limits, now) {
                            Ok(_) => any_ready = true,
                            Err(Blocked::KeyRefused) => key_refused = true,
                            Err(Blocked::Until(u)) => blocked_until = Some(blocked_until.map_or(u, |b: i64| b.min(u))),
                            Err(Blocked::ModelGone) => {}
                        }
                    }
                    if key_refused {
                        "Key refused: set it up again".to_string()
                    } else if any_ready {
                        "Ready".to_string()
                    } else if let Some(u) = blocked_until {
                        format!("Resting for {}", wait_words(u - now))
                    } else {
                        "No model available".to_string()
                    }
                };
                TierStatus { id: t.id, name: t.name, on, state, used_today: used, daily_cap: cap, note: t.note, trains_on_prompts: t.trains_on_prompts }
            })
            .collect()
    }
}

/// Whether a model can do what is needed.
fn fits_need(m: &FreeModel, need: &Need) -> bool {
    (!need.tools || m.tools) && (!need.json || m.json) && m.context >= need.min_context && (!need.coding || m.coding >= 2)
}

/// A provider's daily request cap: its per-account one, or its per-model one times its models.
fn daily_cap(t: &FreeTier, limits: &[Limit]) -> Option<u64> {
    let daily = limits.iter().find(|l| l.counts == Counts::Requests && !matches!(l.reset, Reset::Rolling(_)))?;
    Some(match daily.scope {
        Scope::PerAccount => daily.amount,
        Scope::PerModel => daily.amount * t.models.len() as u64,
    })
}

/// "40 s", "12 min", "3 h".
fn wait_words(secs: i64) -> String {
    match secs.max(0) {
        s if s < 90 => format!("{s} s"),
        s if s < 5_400 => format!("{} min", (s + 59) / 60),
        s => format!("{} h", (s + 1_799) / 3_600),
    }
}
