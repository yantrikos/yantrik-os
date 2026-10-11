//! What the free AI card shows, from what is known: the free tiers, the ways in, the person's
//! choices, which values are kept, and what is happening right now. Pure, so every state and
//! every word is tested without a window.

use std::collections::BTreeMap;

use yantrik_ml::provider::pool::keycheck::check_host;
use yantrik_ml::provider::pool::signup::{signup, Signup};
use yantrik_ml::provider::pool::tiers::{FreeTier, FREE_TIERS};

use super::choices::Choices;

/// What is happening right now, not kept across a restart. No Debug: it holds Cloudflare's
/// pasted account id.
#[derive(Clone, Default)]
pub struct Session {
    /// A key being checked, by provider id.
    pub checking: Option<String>,
    /// Why the last key was not kept, by provider id.
    pub rejected: BTreeMap<String, String>,
    /// A key the person asked to remove, waiting for them to confirm.
    pub confirm_remove: Option<String>,
    /// Cloudflare's account id, pasted and waiting for its token to be checked with it.
    pub pending_account: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Value {
    pub id: String,
    pub label: String,
    pub where_: String,
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub state: &'static str,
    pub state_words: String,
    pub offer: String,
    pub needs: String,
    pub detail: String,
    pub trains: bool,
    pub on: bool,
    pub values: Vec<Value>,
    pub opt_out: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub rows: Vec<Row>,
    pub summary: String,
    pub next_line: String,
    pub next_id: String,
    pub next_name: String,
    /// "Paste Groq key", while a key is awaited, and the value it is for.
    pub chip_label: String,
    pub chip_value_id: String,
}

/// The tiers in the card's order: those that never train on prompts first, each group in the
/// pool's own order (most generous first).
fn ordered() -> Vec<&'static FreeTier> {
    let mut tiers: Vec<&FreeTier> = FREE_TIERS.iter().collect();
    tiers.sort_by_key(|t| t.trains_on_prompts);
    tiers
}

/// `kept`: key-store id → last four characters ("" when the vault is shut and cannot say).
pub fn card(choices: &Choices, kept: &BTreeMap<String, String>, session: &Session) -> Card {
    let rows: Vec<Row> = ordered().into_iter().map(|t| row(t, signup(t.id), choices, kept, session)).collect();

    let ready = rows.iter().filter(|r| (r.state == "ready" || r.state == "open") && r.on).count();
    let skipped = rows.iter().filter(|r| r.state == "skipped").count();
    let to_set_up = rows.iter().filter(|r| !matches!(r.state, "ready" | "off" | "open" | "skipped")).count();
    let summary = format!("{ready} of {} ready · {to_set_up} to set up · {skipped} skipped", rows.len());

    let mut card = Card { summary, ..Card::default() };
    if let Some(next) = rows.iter().find(|r| r.state == "not-set-up") {
        card.next_line = format!("Next: {} · {}", next.name, next.needs.trim_start_matches("Needs: "));
        card.next_id = next.id.clone();
        card.next_name = next.name.clone();
    }
    if let Some((r, v)) = rows.iter().filter(|r| r.state == "waiting").find_map(|r| r.values.iter().find(|v| !v.done).map(|v| (r, v))) {
        let what = if r.values.len() > 1 { v.label.clone() } else { "key".to_string() };
        card.chip_label = format!("Paste {} {}", r.name, what);
        card.chip_value_id = v.id.clone();
    }
    card.rows = rows;
    card
}

fn row(t: &FreeTier, s: Option<&Signup>, choices: &Choices, kept: &BTreeMap<String, String>, session: &Session) -> Row {
    let on = !choices.off.contains(t.id);
    let mut r = Row {
        id: t.id.to_string(),
        name: t.name.to_string(),
        state: "not-set-up",
        state_words: String::new(),
        offer: t.note.to_string(),
        needs: String::new(),
        detail: String::new(),
        trains: t.trains_on_prompts,
        on,
        values: Vec::new(),
        opt_out: String::new(),
    };
    let Some(s) = s else {
        // No account needed: the pool may use it as it is, unless switched off.
        r.state = "open";
        r.state_words = "no account needed".into();
        return r;
    };
    r.needs = format!("Needs: {} · about {} min", s.needs, s.minutes);
    if let Some(url) = s.training_opt_out {
        r.opt_out = format!("Its training can be turned off in its own settings ({url}); the label stays either way.");
    }
    r.values = s
        .values
        .iter()
        .map(|v| Value {
            id: v.id.to_string(),
            label: v.label.to_string(),
            where_: v.where_.to_string(),
            done: kept.contains_key(v.id) || (v.id == "cloudflare_account" && session.pending_account.is_some()),
        })
        .collect();
    let all_kept = s.values.iter().all(|v| kept.contains_key(v.id));
    // The key's tail: the provider's own value, which is the last of its values.
    let tail = s.values.last().and_then(|v| kept.get(v.id)).cloned().unwrap_or_default();
    let ends = if tail.is_empty() { "kept in the vault".to_string() } else { format!("key ends in {tail}") };

    let id = t.id;
    if session.confirm_remove.as_deref() == Some(id) {
        r.state = "confirm-remove";
        r.state_words = "remove the key?".into();
        r.detail = format!("Remove the {0} key? The pool stops using {0}. Your {0} account is untouched.", t.name);
    } else if session.checking.as_deref() == Some(id) {
        r.state = "checking";
        r.state_words = "checking".into();
        r.detail = format!("Checking with {}…", check_host(s));
    } else if all_kept && on {
        r.state = "ready";
        r.state_words = "ready".into();
        r.detail = format!("Ready · {ends}");
    } else if all_kept {
        r.state = "off";
        r.state_words = "off".into();
        r.detail = format!("Off · {ends} · the pool will not use it until it is switched on");
    } else if choices.skipped.contains(id) {
        r.state = "skipped";
    } else if let Some(why) = session.rejected.get(id) {
        r.state = "rejected";
        r.state_words = "key rejected".into();
        r.detail = why.clone();
    } else if choices.stage.get(id).map(String::as_str) == Some("waiting") {
        r.state = "waiting";
        r.state_words = if s.values.len() > 1 { format!("waiting for {} values", s.values.len()) } else { "waiting for key".into() };
        r.detail = if s.values.len() > 1 {
            "Paste each value from the page, in order. Both are checked together once both are in.".into()
        } else {
            format!("Copy the key on that page ({}), then press Paste. It goes to the vault and is not kept in clipboard history.", s.values[0].where_)
        };
    } else if choices.stage.get(id).map(String::as_str) == Some("sign-up-opened") {
        r.state = "sign-up-opened";
        r.state_words = "sign-up opened".into();
        r.detail = "The sign-up page is open in a browser window of its own, which nothing on this machine can read. When the account exists, open the key page, or paste the key if you have already copied it.".into();
    } else {
        r.state_words = "not set up".into();
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn find<'a>(c: &'a Card, id: &str) -> &'a Row {
        c.rows.iter().find(|r| r.id == id).unwrap()
    }

    #[test]
    fn a_fresh_machine_offers_the_first_provider_and_counts_honestly() {
        let c = card(&Choices::default(), &BTreeMap::new(), &Session::default());
        assert_eq!(c.rows.len(), FREE_TIERS.len());
        // Those that never train first; Kilo and OVH need no account and are ready as they are.
        assert!(!c.rows[0].trains && c.rows.last().unwrap().trains);
        assert_eq!(find(&c, "ovh").state, "open");
        assert_eq!(find(&c, "kilo").state, "open");
        assert_eq!(c.summary, "2 of 8 ready · 6 to set up · 0 skipped");
        assert_eq!(c.next_id, "groq");
        assert!(c.next_line.starts_with("Next: Groq · an account · about 3 min"), "{}", c.next_line);
        assert_eq!(c.chip_label, "", "no chip while nothing is awaited");
    }

    #[test]
    fn every_state_reads_as_itself() {
        let mut choices = Choices::default();
        choices.stage.insert("groq".into(), "waiting".into());
        choices.stage.insert("mistral".into(), "sign-up-opened".into());
        choices.skipped.insert("gemini".into());
        choices.off.insert("zai".into());
        let session = Session { rejected: [("openrouter".to_string(), "OpenRouter refused this key (401).".to_string())].into(), ..Session::default() };
        let c = card(&choices, &kept(&[("zai", "9q2x")]), &session);

        let groq = find(&c, "groq");
        assert_eq!((groq.state, groq.state_words.as_str()), ("waiting", "waiting for key"));
        assert_eq!(c.chip_label, "Paste Groq key");
        assert_eq!(c.chip_value_id, "groq");
        assert_eq!(find(&c, "mistral").state, "sign-up-opened");
        assert_eq!(find(&c, "gemini").state, "skipped");
        let zai = find(&c, "zai");
        assert_eq!(zai.state, "off");
        assert!(zai.detail.contains("ends in 9q2x"), "{}", zai.detail);
        assert_eq!(find(&c, "openrouter").state, "rejected");
        assert!(find(&c, "openrouter").detail.contains("401"));
        assert!(c.summary.ends_with("1 skipped"), "{}", c.summary);
        // Skipped rows are not offered as next; Cloudflare is the first left.
        assert_eq!(c.next_id, "cloudflare");
    }

    #[test]
    fn cloudflare_asks_for_two_values_in_order() {
        let mut choices = Choices::default();
        choices.stage.insert("cloudflare".into(), "waiting".into());
        let c = card(&choices, &BTreeMap::new(), &Session::default());
        let cf = find(&c, "cloudflare");
        assert_eq!(cf.state_words, "waiting for 2 values");
        assert_eq!(cf.values.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["cloudflare_account", "cloudflare"]);
        assert_eq!(c.chip_label, "Paste Cloudflare Workers AI Account ID");
        let session = Session { pending_account: Some("0123456789abcdef0123456789abcdef".into()), ..Session::default() };
        let c = card(&choices, &BTreeMap::new(), &session);
        assert!(find(&c, "cloudflare").values[0].done);
        assert_eq!(c.chip_value_id, "cloudflare", "the token is next");
    }

    #[test]
    fn a_kept_key_is_ready_and_shows_only_its_tail() {
        let c = card(&Choices::default(), &kept(&[("groq", "7f3k")]), &Session::default());
        let groq = find(&c, "groq");
        assert_eq!(groq.state, "ready");
        assert_eq!(groq.detail, "Ready · key ends in 7f3k");
        let asking = Session { confirm_remove: Some("groq".into()), ..Session::default() };
        let c = card(&Choices::default(), &kept(&[("groq", "7f3k")]), &asking);
        assert!(find(&c, "groq").detail.starts_with("Remove the Groq key?"));
    }

    /// Words a person reads on this card never say ACTIVE or connected (Pranab: no ambiguity).
    #[test]
    fn no_row_says_active_or_connected() {
        let mut choices = Choices::default();
        for t in FREE_TIERS {
            choices.stage.insert(t.id.into(), "waiting".into());
        }
        for c in [card(&Choices::default(), &BTreeMap::new(), &Session::default()), card(&choices, &kept(&[("groq", "abcd")]), &Session::default())] {
            for r in &c.rows {
                for text in [&r.state_words, &r.detail, &r.offer, &r.needs] {
                    let lower = text.to_lowercase();
                    assert!(!lower.contains("active") && !lower.contains("connected"), "{}: {text}", r.id);
                }
            }
        }
    }
}
