//! A key from the clipboard into the vault: read once, shaped, checked with its provider's own
//! address, kept, and the clipboard cleared. Never through a model turn, never into a Slint
//! property or a log; what comes back is a sentence that says what happened to the key.

use yantrik_ml::provider::pool::keycheck::{self, check_host, Verdict};
use yantrik_ml::provider::pool::signup::{shape, Misshapen, Signup, Value, SIGNUPS};
use yantrik_ml::provider::pool::tiers::FREE_TIERS;

use super::store::Reply;

/// What a paste came to. Its Debug withholds the account id.
#[derive(PartialEq, Eq)]
pub enum Step {
    /// Kept in the vault; `resting` when the provider is at its limit right now.
    Kept { resting: bool },
    /// Cloudflare's account id, held until its token is checked with it.
    AccountHeld(String),
    /// Not kept, and why, in a sentence that ends by saying what happened to the key.
    NotKept(String),
}

/// The provider and the value a key-store id belongs to.
pub fn value_of(value_id: &str) -> Option<(&'static Signup, &'static Value)> {
    SIGNUPS.iter().find_map(|s| s.values.iter().find(|v| v.id == value_id).map(|v| (s, v)))
}

fn name_of(s: &Signup) -> &'static str {
    FREE_TIERS.iter().find(|t| t.id == s.id).map_or(s.id, |t| t.name)
}

/// Decide what a paste comes to. `check` asks the provider; `store` keeps a value. Both are
/// passed in so this is the whole flow, tested without a network or a vault.
pub fn decide(
    value_id: &str,
    pasted: &str,
    pending_account: Option<&str>,
    check: impl Fn(&Signup, &str, Option<&str>) -> Verdict,
    mut store: impl FnMut(&str, &str) -> Reply,
) -> Step {
    let Some((s, v)) = value_of(value_id) else {
        return Step::NotKept("That is not a value this card asks for. Nothing was sent or saved.".into());
    };
    let name = name_of(s);
    let text = match shape(v, pasted) {
        Ok(t) => t,
        Err(Misshapen::Empty) => {
            return Step::NotKept(format!("The clipboard is empty. Copy the {} on the {name} page, then press Paste.", v.label));
        }
        Err(Misshapen::NotAKey) => {
            return Step::NotKept(format!(
                "That does not look like a {name} {}: they {}. Copy it again, then press Paste. Nothing was sent or saved. \
                 If {name} has changed how its keys look, the provider list needs an update.",
                v.label, v.shape_words
            ));
        }
    };
    // Cloudflare's account id cannot be checked alone: it is held, and checked with the token.
    if v.id == "cloudflare_account" {
        return Step::AccountHeld(text);
    }
    let account = if s.check.url.contains("{account_id}") {
        match pending_account {
            Some(a) => Some(a),
            None => return Step::NotKept(format!("Paste the {name} Account ID first: the token is checked with it. Nothing was sent or saved.")),
        }
    } else {
        None
    };
    let host = check_host(s);
    let verdict = check(s, &text, account);
    let resting = match verdict {
        Verdict::Accepted => false,
        Verdict::AcceptedResting => true,
        Verdict::Refused(code) => {
            return Step::NotKept(format!(
                "{name} refused this key ({code}). Copy it again from the key page; it may have been cut short or revoked. Nothing was saved."
            ));
        }
        Verdict::NoSuchAccount => {
            return Step::NotKept(format!(
                "{name} has no account with that ID, or the token cannot reach it. Copy the Account ID from the Workers AI page again, then the token. Nothing was saved."
            ));
        }
        Verdict::Unreachable => {
            return Step::NotKept(format!("Could not reach {host}. The key was not saved. Check the connection, then press Paste again."));
        }
        Verdict::Unexpected(code) => {
            return Step::NotKept(format!("{host} answered {code}, which this check does not know how to read. Nothing was saved."));
        }
    };
    let mut keep = |id: &str, value: &str| match store(id, value) {
        Reply::Done => Ok(()),
        Reply::Locked => Err("The vault is locked: unlock it, then press Paste again. Nothing was saved.".to_string()),
        Reply::NoAnswer => Err(format!(
            "{name} accepted the key, and the vault did not answer in time: it may still be saved. This row will say Ready if it was."
        )),
        _ => Err("The vault would not keep the key. Nothing was saved.".to_string()),
    };
    if let Some(a) = account {
        if let Err(why) = keep("cloudflare_account", a) {
            return Step::NotKept(why);
        }
    }
    match keep(v.id, &text) {
        Ok(()) => Step::Kept { resting },
        Err(why) => Step::NotKept(why),
    }
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Step::Kept { resting } => write!(f, "Kept {{ resting: {resting} }}"),
            Step::AccountHeld(_) => write!(f, "AccountHeld(<withheld>)"),
            Step::NotKept(why) => write!(f, "NotKept({why:?})"),
        }
    }
}

/// The clipboard's text, read once: at most 4 KiB (no key is longer), text only, within three
/// seconds. `None` when there is more, or nothing came in time.
pub fn read_clipboard() -> Option<String> {
    yantrik_companion::tools::provider_keys::read_clipboard(4096, std::time::Duration::from_secs(3))
}

/// Empty the clipboard, once its key is in the vault.
pub fn clear_clipboard() {
    if let Err(e) = std::process::Command::new("wl-copy").arg("--clear").status() {
        tracing::warn!(error = %e, "the clipboard could not be cleared after a key was kept");
    }
}

/// Ask the provider (the real check).
pub fn check(s: &Signup, key: &str, account: Option<&str>) -> Verdict {
    keycheck::check(s, key, account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const GROQ: &str = "gsk_0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJ";
    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

    fn stored() -> RefCell<Vec<(String, String)>> {
        RefCell::new(Vec::new())
    }

    #[test]
    fn a_good_key_is_checked_then_kept() {
        let kept = stored();
        let asked = RefCell::new(None);
        let step = decide("groq", &format!(" {GROQ}\n"), None, |s, k, _| { *asked.borrow_mut() = Some((s.id, k.to_string())); Verdict::Accepted }, |id, v| { kept.borrow_mut().push((id.into(), v.into())); Reply::Done });
        assert_eq!(step, Step::Kept { resting: false });
        assert_eq!(asked.into_inner(), Some(("groq", GROQ.to_string())), "checked with its own provider, trimmed");
        assert_eq!(kept.into_inner(), vec![("groq".to_string(), GROQ.to_string())]);
    }

    #[test]
    fn nothing_is_sent_for_what_is_not_a_key() {
        for pasted in ["", "   ", "hunter2", "my password is gsk_abc"] {
            let step = decide("groq", pasted, None, |_, _, _| panic!("a check was sent for {pasted:?}"), |_, _| panic!("stored"));
            let Step::NotKept(why) = step else { panic!("kept {pasted:?}") };
            assert!(why.contains("Paste"), "{why}");
            assert!(!why.contains(pasted.trim()) || pasted.trim().is_empty(), "the refusal repeats what was pasted: {why}");
        }
    }

    #[test]
    fn a_refused_or_unreachable_key_is_not_kept_and_says_so() {
        for (verdict, words) in [(Verdict::Refused(401), "refused this key (401)"), (Verdict::Unreachable, "Could not reach api.groq.com"), (Verdict::Unexpected(500), "answered 500")] {
            let v = verdict.clone();
            let step = decide("groq", GROQ, None, move |_, _, _| v.clone(), |_, _| panic!("stored a key the provider did not accept"));
            let Step::NotKept(why) = step else { panic!("{verdict:?} was kept") };
            assert!(why.contains(words) && why.contains("not saved") || why.contains("Nothing was saved"), "{why}");
            assert!(!why.contains(GROQ), "the key is in the sentence");
        }
        assert_eq!(decide("groq", GROQ, None, |_, _, _| Verdict::AcceptedResting, |_, _| Reply::Done), Step::Kept { resting: true });
        let locked = decide("groq", GROQ, None, |_, _, _| Verdict::Accepted, |_, _| Reply::Locked);
        assert!(matches!(locked, Step::NotKept(w) if w.contains("vault is locked")));
    }

    #[test]
    fn cloudflare_holds_its_account_and_checks_the_token_with_it() {
        assert_eq!(decide("cloudflare_account", ACCOUNT, None, |_, _, _| panic!("checked alone"), |_, _| panic!("stored alone")), Step::AccountHeld(ACCOUNT.into()));
        let token = format!("cfut_{}", "a".repeat(40));
        let no_account = decide("cloudflare", &token, None, |_, _, _| panic!("checked without its account"), |_, _| panic!());
        assert!(matches!(no_account, Step::NotKept(w) if w.contains("Account ID first")));
        let kept = stored();
        let step = decide("cloudflare", &token, Some(ACCOUNT), |_, _, a| { assert_eq!(a, Some(ACCOUNT)); Verdict::Accepted }, |id, v| { kept.borrow_mut().push((id.into(), v.into())); Reply::Done });
        assert_eq!(step, Step::Kept { resting: false });
        assert_eq!(kept.into_inner().iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["cloudflare_account", "cloudflare"]);
    }
}
