//! The free tiers the pool can draw on: what each offers, what it allows, and how it says what is
//! left. Every number is from the provider's own documentation as read on 2026-09-30 (sources in
//! docs/free-tiers.md); a limit a provider does not publish is `None`, and the pool learns it
//! from the provider's refusals instead of guessing it.
//!
//! Two labels decide where a provider may be used at all:
//! - `trains_on_prompts`: the free tier may train on what is sent, or let people read it. Never
//!   used for a private turn.
//! - `may_serve_public`: its terms allow answering people other than the account holder (a
//!   visitor's question on the live stream). Unclear terms count as no.

/// When a window's count starts again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reset {
    /// A rolling window of this many seconds.
    Rolling(u64),
    /// Midnight UTC.
    UtcMidnight,
    /// Midnight in Los Angeles (Gemini's "midnight Pacific time").
    PacificMidnight,
}

/// What a limit counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counts {
    Requests,
    Tokens,
}

/// Who shares a limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Each model has its own.
    PerModel,
    /// Every model of the account (or, with no key, the address) shares one.
    PerAccount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limit {
    pub counts: Counts,
    pub amount: u64,
    pub reset: Reset,
    pub scope: Scope,
}

const fn rpm(n: u64, scope: Scope) -> Limit {
    Limit { counts: Counts::Requests, amount: n, reset: Reset::Rolling(60), scope }
}
const fn rph(n: u64, scope: Scope) -> Limit {
    Limit { counts: Counts::Requests, amount: n, reset: Reset::Rolling(3600), scope }
}
const fn rpd(n: u64, scope: Scope) -> Limit {
    Limit { counts: Counts::Requests, amount: n, reset: Reset::UtcMidnight, scope }
}
const fn tpm(n: u64, scope: Scope) -> Limit {
    Limit { counts: Counts::Tokens, amount: n, reset: Reset::Rolling(60), scope }
}
const fn tpd(n: u64, scope: Scope) -> Limit {
    Limit { counts: Counts::Tokens, amount: n, reset: Reset::UtcMidnight, scope }
}

/// How a provider tells what is left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Headers {
    /// `x-ratelimit-remaining-requests` / `-tokens`, `x-ratelimit-reset-*`, `retry-after`.
    OpenAiStyle,
    /// `ratelimit-remaining`, `ratelimit-reset` (IETF draft), `retry-after`.
    IetfDraft,
    /// Only `retry-after`, if anything, on a refusal.
    RetryAfterOnly,
}

/// One free model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeModel {
    pub id: &'static str,
    pub context: u32,
    pub tools: bool,
    pub json: bool,
    /// How well it writes code, for the pool's choice: 3 strong, 2 good, 1 light.
    pub coding: u8,
}

const fn model(id: &'static str, context: u32, tools: bool, json: bool, coding: u8) -> FreeModel {
    FreeModel { id, context, tools, json, coding }
}

/// One provider's free tier.
#[derive(Clone, Copy, Debug)]
pub struct FreeTier {
    /// The catalogue id (`crate::provider::catalogue`), so the pool and every other surface name
    /// it the same way.
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// A key is needed. Without one, the free models are open to any address.
    pub needs_key: bool,
    pub models: &'static [FreeModel],
    pub limits: &'static [Limit],
    pub headers: Headers,
    pub trains_on_prompts: bool,
    pub may_serve_public: bool,
    /// One line on what to know before switching it on.
    pub note: &'static str,
}

use Scope::{PerAccount, PerModel};

/// Every free tier the pool knows, most generous first.
pub const FREE_TIERS: &[FreeTier] = &[
    FreeTier {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        needs_key: true,
        models: &[
            model("openai/gpt-oss-120b", 131_072, true, true, 2),
            model("qwen/qwen3.8-27b", 131_072, true, true, 2),
            model("openai/gpt-oss-20b", 131_072, true, true, 1),
        ],
        limits: &[rpm(30, PerModel), rpd(1_000, PerModel), tpm(8_000, PerModel), tpd(200_000, PerModel)],
        headers: Headers::OpenAiStyle,
        trains_on_prompts: false,
        may_serve_public: true,
        note: "1,000 requests and 200K tokens a day per model. One account per person (its terms).",
    },
    FreeTier {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        needs_key: true,
        models: &[
            model("nvidia/nemotron-3-ultra-550b-a55b:free", 1_000_000, true, false, 3),
            model("qwen/qwen3.8-27b:free", 262_144, true, true, 2),
            model("cohere/north-mini-code:free", 256_000, true, false, 2),
            model("nvidia/nemotron-3-super-120b-a12b:free", 262_144, true, true, 2),
            model("poolside/laguna-s-2.1:free", 262_144, true, false, 2),
        ],
        // 50 a day, or 1,000 once the account has bought $10 of credit (`with_paid_credit`).
        limits: &[rpm(20, PerAccount), rpd(50, PerAccount)],
        headers: Headers::RetryAfterOnly,
        trains_on_prompts: true,
        may_serve_public: false,
        note: "50 free requests a day, 1,000 after a one-time $10 of credit. Some free models' hosts may train on prompts.",
    },
    FreeTier {
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        needs_key: true,
        models: &[model("gemini-3.8-flash", 1_048_576, true, true, 3)],
        // Per-project limits are shown only in AI Studio: learned from refusals.
        limits: &[],
        headers: Headers::RetryAfterOnly,
        trains_on_prompts: true,
        may_serve_public: false,
        note: "Strong at code, 1M context. Its free tier trains on prompts and people may read them.",
    },
    FreeTier {
        id: "cloudflare",
        name: "Cloudflare Workers AI",
        base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1",
        needs_key: true,
        models: &[
            model("@cf/openai/gpt-oss-120b", 131_072, true, false, 2),
            model("@cf/nvidia/nemotron-3-120b-a12b", 262_144, true, false, 2),
        ],
        // 10,000 "neurons" a day, about 150K output tokens of gpt-oss-120b.
        limits: &[rpm(300, PerAccount), tpd(150_000, PerAccount)],
        headers: Headers::RetryAfterOnly,
        trains_on_prompts: false,
        may_serve_public: false,
        note: "About 150K output tokens a day. Needs the account id as well as a token.",
    },
    FreeTier {
        id: "zai",
        name: "Z.ai",
        base_url: "https://api.z.ai/api/paas/v4",
        needs_key: true,
        models: &[model("glm-4.7-flash", 200_000, true, true, 2), model("glm-4.5-flash", 128_000, true, true, 1)],
        // Limited by concurrency, per user tier; no published numbers.
        limits: &[],
        headers: Headers::RetryAfterOnly,
        // Its terms of use let it use what individuals send "for the purpose of developing and
        // improving" its models; only the Team Plan is excluded (docs.z.ai, checked 2026-09-30).
        trains_on_prompts: true,
        may_serve_public: false,
        note: "GLM Flash models are free. One request at a time. Its terms let it train on what is sent.",
    },
    FreeTier {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        needs_key: true,
        models: &[model("mistral-medium-latest", 256_000, true, true, 2)],
        // Free mode's numbers are only on the console's Limits page: learned from refusals.
        limits: &[],
        headers: Headers::OpenAiStyle,
        trains_on_prompts: true,
        may_serve_public: false,
        note: "Free mode may train on prompts unless you opt out in the console.",
    },
    FreeTier {
        id: "kilo",
        name: "Kilo Gateway",
        base_url: "https://api.kilo.ai/api/gateway",
        needs_key: false,
        models: &[
            model("nvidia/nemotron-3-ultra-550b:free", 1_000_000, true, false, 3),
            model("qwen/qwen3.8-27b:free", 262_144, true, false, 2),
            model("cohere/north-mini-code:free", 256_000, true, false, 2),
        ],
        limits: &[rph(200, PerAccount)],
        headers: Headers::RetryAfterOnly,
        trains_on_prompts: true,
        may_serve_public: false,
        note: "No key: 200 requests an hour from this address. Its free models may train on prompts.",
    },
    FreeTier {
        id: "ovh",
        name: "OVHcloud AI Endpoints",
        base_url: "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1",
        needs_key: false,
        models: &[model("gpt-oss-120b", 131_072, true, true, 2)],
        limits: &[rpm(2, PerModel)],
        headers: Headers::IetfDraft,
        trains_on_prompts: false,
        may_serve_public: false,
        note: "No key: 2 requests a minute per model from this address. Never stores or reuses prompts.",
    },
];

/// The tier with this catalogue id.
pub fn tier(id: &str) -> Option<&'static FreeTier> {
    FREE_TIERS.iter().find(|t| t.id == id)
}

/// OpenRouter's daily cap after the account has bought $10 of credit, once (its limits page).
pub const OPENROUTER_PAID_CREDIT_RPD: u64 = 1_000;

/// A tier is tight when it allows at most this many requests a minute...
pub const TIGHT_PER_MINUTE: u64 = 5;
/// ...or at most this many a day. What the companion starts by itself is not sent to a tight
/// account: it is kept for what the person asks (`crate::asked`). The one place that decides it.
pub const TIGHT_PER_DAY: u64 = 100;

impl FreeTier {
    /// The request limit that makes this tier tight: the per-minute one when it is, else the
    /// per-day one, else `None`. A limit that is not published is not a tight one (the pool learns
    /// it from refusals), and neither is one that counts tokens or an hour.
    pub fn tight_limit(&self) -> Option<&Limit> {
        let per = |reset: fn(&Reset) -> bool, most: u64| {
            self.limits.iter().find(|l| l.counts == Counts::Requests && reset(&l.reset) && l.amount <= most)
        };
        per(|r| *r == Reset::Rolling(60), TIGHT_PER_MINUTE)
            .or_else(|| per(|r| matches!(r, Reset::UtcMidnight | Reset::PacificMidnight), TIGHT_PER_DAY))
    }

    pub fn is_tight(&self) -> bool {
        self.tight_limit().is_some()
    }

    /// The most tokens a minute the tier takes (`tpm`), when it publishes one. One request
    /// larger than this can never be taken, however long it waits: the gateway fits it to this
    /// (`yantrik_gateway::fit`).
    pub fn tokens_per_minute(&self) -> Option<u64> {
        self.limits.iter().find(|l| l.counts == Counts::Tokens && l.reset == Reset::Rolling(60)).map(|l| l.amount)
    }

    /// The context window, in tokens, of one of this tier's models, when it is listed.
    pub fn context_of(&self, model: &str) -> Option<u32> {
        self.models.iter().find(|m| m.id == model).map(|m| m.context)
    }
}

/// What an account's model takes in one request. Each is `None` where it is not known, and a call
/// is then not held to it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Room {
    /// Tokens a minute (`tpm`): one call larger than this can never be taken.
    pub per_minute: Option<u64>,
    /// The model's context window, in tokens.
    pub context: Option<u64>,
}

impl Room {
    /// The most tokens one request may carry: the smaller of what is known.
    pub fn per_request(&self) -> Option<u64> {
        match (self.per_minute, self.context) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Room {
    /// The most prompt tokens a call asking for `answer` more may carry: the per-minute allowance
    /// as it is, and the context window less the answer.
    pub fn prompt_limit(&self, answer: u64) -> Option<u64> {
        let context = self.context.map(|c| c.saturating_sub(answer));
        match (self.per_minute, context) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

impl Room {
    /// What one free-tier model takes: the tier's tokens a minute and the model's context window
    /// (0 is unknown). No tier (a paid or local account) is unknown on both counts.
    pub fn of_tier(tier: Option<&FreeTier>, model: &str) -> Room {
        Room {
            per_minute: tier.and_then(FreeTier::tokens_per_minute),
            context: tier.and_then(|t| t.context_of(model)).map(u64::from).filter(|c| *c > 0),
        }
    }

    /// The most tokens an answer may ask for in this room: a quarter of what one request holds,
    /// at most `ceiling`, so what is said and what is asked of the model both fit it.
    pub fn answer_room(&self, ceiling: u64) -> u64 {
        self.per_request().map_or(ceiling, |r| (r / 4).min(ceiling))
    }
}

/// What a tight limit allows, as the person reads it: "2 requests a minute", "50 requests a day".
/// `None` for a limit that is not a per-minute or per-day request count.
pub fn allows_words(limit: &Limit) -> Option<String> {
    let per = match limit.reset {
        Reset::Rolling(60) => "a minute",
        Reset::UtcMidnight | Reset::PacificMidnight => "a day",
        Reset::Rolling(_) => return None,
    };
    (limit.counts == Counts::Requests).then(|| format!("{} {} {per}", limit.amount, if limit.amount == 1 { "request" } else { "requests" }))
}

#[cfg(test)]
mod tight_tests;

#[cfg(test)]
mod room_tests;
