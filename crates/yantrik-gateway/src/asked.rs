//! Who started a call, and the one rule that reads it: a tight account is kept for what somebody
//! asked.
//!
//! The built-in companion marks the calls it starts by itself (`yantrik_ml::asked`, the header
//! [`yantrik_ml::asked::HEADER`]). A call from the companion's own grant that carries the mark, to
//! an account whose allowance is tight ([`Target::tight`]), is refused here, before anything is
//! sent: 429, [`yantrik_ml::asked::KEPT_CODE`]. The default is asked: a call without the mark is
//! never refused by this rule, and a harness other than the companion that sends the mark changes
//! nothing for itself.

use yantrik_ml::asked::{BY_ITSELF, HEADER, KEPT_CODE};

use crate::http::Request;
use crate::policy::{Refusal, Target};
use crate::tokens::Grant;

/// The harness id of the built-in companion (`wire::harness::BUILTIN_ID` in the shell, which a
/// test there keeps equal to this).
pub const COMPANION: &str = "companion";

/// `self` when the request carries the mark of a call the companion started itself.
pub fn mark(req: &Request) -> Option<&'static str> {
    req.header(HEADER).filter(|v| v.trim().eq_ignore_ascii_case(BY_ITSELF)).map(|_| BY_ITSELF)
}

/// The refusal for a marked call from the companion to a tight account, else `None`. The one place
/// the rule is decided; `mark` is [`mark`]'s answer for the request.
pub fn kept_for_asked(grant: &Grant, mark: Option<&str>, target: &Target) -> Option<Refusal> {
    if mark.is_none() || grant.harness != COMPANION || !target.tight {
        return None;
    }
    Some(Refusal::new(
        429,
        KEPT_CODE,
        format!(
            "{} allows few requests, so this account is kept for what you ask. The companion started this call by itself, so it was not sent.",
            target.provider
        ),
    ))
}

/// The refusal for a marked call from the companion to an account an asked call just met a 429
/// from, else `None`. The window is the one [`crate::hold::Holds`] keeps.
pub fn held_for_asked(grant: &Grant, mark: Option<&str>, target: &Target, holds: &crate::hold::Holds) -> Option<Refusal> {
    if mark.is_none() || grant.harness != COMPANION {
        return None;
    }
    let left = holds.remaining(&target.account)?;
    Some(Refusal::new(
        429,
        KEPT_CODE,
        format!(
            "{} told your own request to wait (too many requests), so for about {} more seconds it is kept for what you ask. The companion started this call by itself, so it was not sent.",
            target.provider,
            left.as_secs() + 1
        ),
    ))
}
