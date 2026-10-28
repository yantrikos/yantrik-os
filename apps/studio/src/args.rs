//! The arguments of a call, read the way a Mind sends them.
//!
//! The dispatch checks that a required argument is present and of the declared type, or can be
//! converted to it. What it cannot say is what the app means by a blank one or by `-1`, so the
//! readings here are tolerant of the shapes a mind sends (`width: "1024"` about as often as
//! `width: 1024`), and each action turns "not given" into its own default.

use serde_json::Value;

use crate::command::{BackendAsk, Which};
use crate::plan::{self, Plan};

/// An optional text argument. Absent and empty mean the same thing here: a blank `base_url` asks
/// for the kind's own default, which is what `set_backend` does with it.
pub fn given(args: &Value, name: &str) -> String {
    args.get(name).and_then(|v| v.as_str()).unwrap_or_default().to_string()
}

/// A number a caller might send either way.
///
/// The runtime checks that a required argument is present, not what type it arrived as, and a mind
/// writes `width: "1024"` about as often as `width: 1024`. Refusing the string form would be a
/// refusal about JSON rather than about the picture. Zero means "not given", and each action turns
/// that into its own default.
pub fn number(args: &Value, name: &str) -> u32 {
    wide(args, name).unwrap_or(0).min(u32::MAX as u64) as u32
}

/// The same reading, without the cap and without a zero standing in for "absent".
pub fn wide(args: &Value, name: &str) -> Option<u64> {
    let value = args.get(name)?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse::<u64>().ok()))
}

/// `count`, with zero (not given) meaning `default`.
pub fn count_or(args: &Value, default: u32) -> u32 {
    match number(args, "count") {
        0 => default,
        n => n,
    }
}

/// `factor` of an upscale: zero (not given) means 2.
pub fn factor_of(args: &Value) -> u32 {
    match number(args, "factor") {
        0 => 2,
        n => n,
    }
}

/// The ask a `generate` call made.
pub fn plan_of(args: &Value) -> Plan {
    let mut plan = Plan::new(given(args, "prompt"));
    // Present and empty means "no negative prompt", which is a choice; absent means "yours",
    // which is the default. Telling those apart is the whole reason this is not `given`.
    if let Some(negative) = args.get("negative").and_then(|v| v.as_str()) {
        plan.negative = negative.to_string();
    }
    plan.width = number(args, "width");
    plan.height = number(args, "height");
    let steps = number(args, "steps");
    plan.steps = if steps == 0 { plan::DEFAULT_STEPS } else { steps };
    plan.count = count_or(args, 1);
    // Read separately from the other numbers, because `number` caps at u32, which is right for a
    // size and wrong for a seed. Truncating 4294967296 to 0 would still make a picture, and the
    // record beside it would then name a seed that does not reproduce it.
    plan.seed = wide(args, "seed");
    plan
}

/// What `set_backend` was given.
pub fn backend_ask(args: &Value) -> BackendAsk {
    BackendAsk {
        kind: given(args, "kind"),
        base_url: given(args, "base_url"),
        model: given(args, "model"),
        api_key_env: given(args, "api_key_env"),
        workflow: given(args, "workflow"),
    }
}

/// Which job a `cancel` named.
pub fn which(args: &Value) -> Which {
    match job_id(args) {
        Some(id) => Which::One(id),
        None => Which::Unusable { given: args.get("job").is_some() },
    }
}

/// A job id the way the queue takes it: a whole number of 1 or more, whether it arrived as a
/// number or as its digits. Anything else is `None`, and the caller is told about `cancel_all`.
///
/// Deliberately not `wide()`, which casts through a float in which `-1` saturates to `0`: for a
/// width that is a rounding, but for a job id it turned "cancel the job before the first one" into
/// "cancel everything".
fn job_id(args: &Value) -> Option<i32> {
    let value = args.get("job")?;
    let whole = value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|number| number.is_finite() && number.fract() == 0.0)
                .map(|number| number as i64)
        })
        .or_else(|| value.as_str().and_then(|text| text.trim().parse::<i64>().ok()))?;
    i32::try_from(whole).ok().filter(|id| *id >= 1)
}
