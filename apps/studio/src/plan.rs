//! What a picture is asked for: the ask, the numbers it is held to, and the request each picture
//! of it becomes.
//!
//! Decided once here, so a backend is never handed a width of zero and made to invent a policy
//! about it, and so the window and the control surface cannot disagree about what 1025 pixels
//! means.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::Digest;

use crate::backend::Request;
use crate::said::Said;

/// How many pictures one `generate` will make. A hosted backend charges per picture, so an
/// unbounded `count` is a way to spend somebody's money from one sentence; four is enough to choose
/// from and small enough that asking for more is a decision rather than a typo.
pub const MAX_COUNT: u32 = 4;
pub const MIN_EDGE: u32 = 64;
pub const MAX_EDGE: u32 = 4096;
pub const MAX_STEPS: u32 = 150;
pub const MAX_UPSCALE: u32 = 8;

pub const DEFAULT_NEGATIVE: &str =
    "blurry, low quality, watermark, text, signature, deformed hands, extra limbs";
pub const DEFAULT_WIDTH: u32 = 1024;
pub const DEFAULT_HEIGHT: u32 = 1024;
pub const DEFAULT_STEPS: u32 = 30;
pub const DEFAULT_CFG: f64 = 7.0;

/// What a `generate` asked for, before it becomes one or more requests to a backend.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub prompt: String,
    pub negative: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f64,
    /// `None` means "choose one", and each picture of a multi-picture ask then gets its own.
    pub seed: Option<u64>,
    pub count: u32,
}

impl Plan {
    /// A one-picture ask with the defaults filled in.
    pub fn new(prompt: impl Into<String>) -> Plan {
        Plan {
            prompt: prompt.into(),
            negative: DEFAULT_NEGATIVE.to_string(),
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            steps: DEFAULT_STEPS,
            cfg: DEFAULT_CFG,
            seed: None,
            count: 1,
        }
    }

    /// Check one ask and bring its numbers onto the grid, or say what is wrong with it in words a
    /// caller can act on.
    ///
    /// Clamping rather than refusing, for the numbers: a mind that asks for 1025 pixels wide wants a
    /// picture, not a lecture about the latent grid, and the sidecar records what was actually sent.
    /// Refusing outright is kept for the things that cannot be repaired by picking a nearby value —
    /// an empty prompt, or a count that would spend real money.
    pub fn checked(self) -> Result<Plan, Said> {
        let prompt = self.prompt.trim();
        if prompt.is_empty() {
            return Err(Said::new(
                "a prompt is needed: `generate prompt=\"a lighthouse in fog\"`",
                "a prompt is needed: describe the picture in the prompt box",
            ));
        }
        if prompt.chars().count() > 4000 {
            return Err(Said::both(format!(
                "that prompt is {} characters, which is longer than any model here will read; the first few sentences are enough",
                prompt.chars().count()
            )));
        }
        if self.count == 0 {
            return Err(Said::both("count=0 would make nothing; ask for at least one picture"));
        }
        if self.count > MAX_COUNT {
            return Err(Said::both(format!(
                "count={} is more than the {MAX_COUNT} this app will make in one ask, because a hosted backend charges for each one",
                self.count
            )));
        }
        Ok(Plan {
            prompt: prompt.to_string(),
            // An empty negative is left empty. Some people want no negative prompt, and
            // substituting this app's opinion of one would change their picture without saying so.
            negative: self.negative.trim().to_string(),
            width: clamp_edge(self.width, DEFAULT_WIDTH),
            height: clamp_edge(self.height, DEFAULT_HEIGHT),
            steps: self.steps.clamp(1, MAX_STEPS),
            cfg: self.cfg.clamp(0.0, 30.0),
            seed: self.seed,
            count: self.count,
        })
    }

    /// The request for the `index`th picture of this ask. Seeds run on from the one given, so asking
    /// for four with `seed=7` produces 7, 8, 9 and 10 rather than four of the same thing.
    pub fn request(&self, index: u32) -> Request {
        Request {
            prompt: self.prompt.clone(),
            negative: self.negative.clone(),
            width: self.width,
            height: self.height,
            steps: self.steps,
            cfg: self.cfg,
            seed: self
                .seed
                .map(|seed| seed.wrapping_add(index as u64))
                .unwrap_or_else(random_seed),
        }
    }
}

fn clamp_edge(value: u32, fallback: u32) -> u32 {
    if value == 0 {
        fallback
    } else {
        value.clamp(MIN_EDGE, MAX_EDGE)
    }
}

pub fn random_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut hasher = sha2::Sha256::new();
    hasher.update(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .to_le_bytes(),
    );
    // The process id and a counter, so two asks in the same millisecond — which is what a batch does
    // — still get different seeds. This has to be different every time and reproducible from the
    // sidecar afterwards; it does not have to be unpredictable to an attacker, which is not what a
    // seed is for.
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    u64::from_le_bytes(hasher.finalize()[..8].try_into().unwrap())
}
