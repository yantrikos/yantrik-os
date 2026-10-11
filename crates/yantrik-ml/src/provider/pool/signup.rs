//! How a person gets each free tier's key: where to sign up, where the key is made, what it
//! looks like, and how to ask the provider whether it works.
//!
//! The person signs up themselves, one account per provider; the OS opens the right pages in
//! order and takes the key from the clipboard. Every address here was read from the provider's
//! own documentation on 2026-09-30 (`docs/free-tiers.md`). It is data the shell shows and calls,
//! never something a model chooses; it will move into the signed provider registry as it stands.
//!
//! A key goes to one place only: the provider's own host, the same host as its tier's
//! `base_url` (`a_key_is_checked_only_at_its_providers_own_host`).

/// What a key looks like, loosely: enough to refuse what is plainly not a key (a sentence, a
/// password copied earlier) before anything is sent, without locking a person out the day a
/// provider lengthens its keys. A key that fails says how keys look; there is no "send anyway".
#[derive(Clone, Copy, Debug)]
pub struct Format {
    /// One of these must begin it; empty when the provider documents no prefix.
    pub prefixes: &'static [&'static str],
    pub min_len: usize,
    pub max_len: usize,
    /// Characters a key may hold besides ASCII letters and digits.
    pub extra: &'static str,
}

/// One value a provider needs: a key, or (Cloudflare) an account id beside it.
#[derive(Clone, Copy, Debug)]
pub struct Value {
    /// The key-store id: the provider's id for its key, `cloudflare_account` for the account id.
    pub id: &'static str,
    pub label: &'static str,
    /// The page where it is found or made.
    pub url: &'static str,
    /// Where on that page, in a few words.
    pub where_: &'static str,
    pub format: Format,
    /// How it looks, for the sentence that refuses one that does not ("start with gsk_ and are
    /// about 56 characters").
    pub shape_words: &'static str,
}

/// How the key is sent when it is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    /// `Authorization: Bearer <key>`.
    Bearer,
    /// `x-goog-api-key: <key>` (Google's own API).
    GoogApiKey,
}

/// A call that answers whether a key works and spends no free use: a listing, or the key's own
/// record. `{account_id}` is filled in for Cloudflare.
#[derive(Clone, Copy, Debug)]
pub struct Check {
    pub url: &'static str,
    pub auth: Auth,
}

/// One provider's way in.
#[derive(Clone, Copy, Debug)]
pub struct Signup {
    /// The tier's id (`tiers::FREE_TIERS`).
    pub id: &'static str,
    pub signup_url: &'static str,
    /// In order: Cloudflare's account id comes before its token.
    pub values: &'static [Value],
    /// What the provider asks for before a key, shown before anyone starts.
    pub needs: &'static str,
    /// About how long it takes, sign-up to key.
    pub minutes: u8,
    pub check: Check,
    /// Where its free tier's training can be turned off, when it can.
    pub training_opt_out: Option<&'static str>,
}

const ALNUM: &str = "";

/// Every free tier that needs a key, in the order the setup offers them.
pub const SIGNUPS: &[Signup] = &[
    Signup {
        id: "groq",
        signup_url: "https://console.groq.com/login",
        values: &[Value {
            id: "groq",
            label: "API key",
            url: "https://console.groq.com/keys",
            where_: "API Keys, then Create API Key",
            format: Format { prefixes: &["gsk_"], min_len: 40, max_len: 100, extra: ALNUM },
            shape_words: "start with gsk_ and are about 56 characters",
        }],
        needs: "an account",
        minutes: 3,
        check: Check { url: "https://api.groq.com/openai/v1/models", auth: Auth::Bearer },
        training_opt_out: None,
    },
    Signup {
        id: "cloudflare",
        signup_url: "https://dash.cloudflare.com/sign-up/workers-and-pages",
        values: &[
            Value {
                id: "cloudflare_account",
                label: "Account ID",
                url: "https://dash.cloudflare.com/?to=/:account/ai/workers-ai",
                where_: "Workers AI, then Use REST API: the Account ID",
                format: Format { prefixes: &[], min_len: 32, max_len: 32, extra: ALNUM },
                shape_words: "are 32 letters and digits",
            },
            Value {
                id: "cloudflare",
                label: "API token",
                url: "https://dash.cloudflare.com/?to=/:account/ai/workers-ai",
                where_: "Workers AI, then Use REST API, then Create a Workers AI API Token",
                // `cfut_`/`cfat_` since 2026; 40 characters with no prefix before.
                format: Format { prefixes: &[], min_len: 40, max_len: 100, extra: "_" },
                shape_words: "start with cfut_ or cfat_, or are 40 letters and digits",
            },
        ],
        needs: "a Cloudflare account, and two values from it: its account ID and a token",
        minutes: 8,
        check: Check { url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/models/search", auth: Auth::Bearer },
        training_opt_out: None,
    },
    Signup {
        id: "openrouter",
        signup_url: "https://openrouter.ai/sign-up",
        values: &[Value {
            id: "openrouter",
            label: "API key",
            url: "https://openrouter.ai/keys",
            where_: "Keys, then Create Key",
            format: Format { prefixes: &["sk-or-v1-"], min_len: 40, max_len: 120, extra: "-" },
            shape_words: "start with sk-or-v1-",
        }],
        needs: "an account; no card",
        minutes: 3,
        // `/models` answers without a key, so it would say nothing; the key's own record does.
        check: Check { url: "https://openrouter.ai/api/v1/key", auth: Auth::Bearer },
        training_opt_out: Some("https://openrouter.ai/settings/privacy"),
    },
    Signup {
        id: "gemini",
        signup_url: "https://aistudio.google.com/apikey",
        values: &[Value {
            id: "gemini",
            label: "API key",
            url: "https://aistudio.google.com/apikey",
            where_: "Create API key",
            // `AIza…` (39 characters) was what AI Studio issued; by Oct 2026 it issues `AQ.…` keys,
            // dots included (Pranab's key, 1 Oct 2026, refused by the old shape before anything was
            // sent). Both are taken; the check with Google says whether one works.
            format: Format { prefixes: &["AIza", "AQ."], min_len: 30, max_len: 200, extra: "-_." },
            shape_words: "start with AIza or AQ.",
        }],
        needs: "a Google account, in a country where the Gemini API is offered",
        minutes: 3,
        check: Check { url: "https://generativelanguage.googleapis.com/v1beta/models", auth: Auth::GoogApiKey },
        training_opt_out: None,
    },
    Signup {
        id: "zai",
        signup_url: "https://z.ai/model-api",
        values: &[Value {
            id: "zai",
            label: "API key",
            url: "https://z.ai/manage-apikey/apikey-list",
            where_: "API Keys, then Create a new API key",
            // `<id>.<secret>`, as its own JWT example splits it.
            format: Format { prefixes: &[], min_len: 40, max_len: 120, extra: "." },
            shape_words: "are two parts joined by a dot",
        }],
        needs: "an account",
        minutes: 4,
        // Not documented as a check; it answers 401 without a key (seen 2026-09-30).
        check: Check { url: "https://api.z.ai/api/paas/v4/models", auth: Auth::Bearer },
        training_opt_out: None,
    },
    Signup {
        id: "mistral",
        signup_url: "https://console.mistral.ai",
        values: &[Value {
            id: "mistral",
            label: "API key",
            url: "https://console.mistral.ai/home?profile_dialog=api-keys",
            where_: "API Keys, then Create new key",
            format: Format { prefixes: &[], min_len: 24, max_len: 80, extra: ALNUM },
            shape_words: "are about 32 letters and digits",
        }],
        needs: "an account; no card",
        minutes: 4,
        check: Check { url: "https://api.mistral.ai/v1/models", auth: Auth::Bearer },
        training_opt_out: Some("https://admin.mistral.ai/organization/privacy"),
    },
];

/// The way in for a tier, when it needs a key.
pub fn signup(id: &str) -> Option<&'static Signup> {
    SIGNUPS.iter().find(|s| s.id == id)
}

/// Why pasted text is not a key of this shape, or `Ok` with it trimmed.
pub fn shape(value: &Value, pasted: &str) -> Result<String, Misshapen> {
    let text = pasted.trim();
    if text.is_empty() {
        return Err(Misshapen::Empty);
    }
    let f = &value.format;
    let allowed = |c: char| c.is_ascii_alphanumeric() || f.extra.contains(c);
    // The prefix is matched as it is (its own `_` or `-`); what follows it is held to the charset.
    let rest = if f.prefixes.is_empty() {
        Some(text)
    } else {
        f.prefixes.iter().find_map(|p| text.strip_prefix(p))
    };
    let fits = rest.is_some_and(|r| !r.is_empty() && r.chars().all(allowed))
        && (f.min_len..=f.max_len).contains(&text.len());
    // Cloudflare's account id is hex; its token's prefix is optional but its shape is not.
    let hex_ok = value.id != "cloudflare_account" || text.chars().all(|c| c.is_ascii_hexdigit());
    let dot_ok = value.id != "zai" || text.matches('.').count() == 1;
    if fits && hex_ok && dot_ok {
        Ok(text.to_string())
    } else {
        Err(Misshapen::NotAKey)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Misshapen {
    /// Nothing on the clipboard.
    Empty,
    /// Something, but not shaped like this provider's keys.
    NotAKey,
}

/// Whether text looks like a key of any provider here, for what must never be kept (clipboard
/// history): the prefixed shapes only, which cannot be mistaken for ordinary words.
pub fn looks_like_a_provider_key(text: &str) -> bool {
    let t = text.trim();
    SIGNUPS
        .iter()
        .flat_map(|s| s.values)
        .filter(|v| !v.format.prefixes.is_empty())
        .any(|v| shape(v, t).is_ok())
        || (["cfut_", "cfat_"].iter().any(|p| t.starts_with(p)) && t.len() >= 40 && !t.contains(char::is_whitespace))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::pool::tiers::FREE_TIERS;

    fn host(url: &str) -> &str {
        url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or("")
    }

    /// A key is sent to its provider's own host and nowhere else, and only over HTTPS.
    #[test]
    fn a_key_is_checked_only_at_its_providers_own_host() {
        for s in SIGNUPS {
            let tier = FREE_TIERS.iter().find(|t| t.id == s.id).unwrap_or_else(|| panic!("{} is no tier", s.id));
            assert!(tier.needs_key, "{} needs no key", s.id);
            assert!(s.check.url.starts_with("https://"), "{}", s.check.url);
            assert_eq!(host(s.check.url), host(tier.base_url), "{}: the check goes elsewhere than the tier", s.id);
            for page in std::iter::once(s.signup_url).chain(s.values.iter().map(|v| v.url)) {
                assert!(page.starts_with("https://"), "{page}");
            }
        }
        // Every tier that needs a key has a way in, and no keyless tier is offered one.
        for t in FREE_TIERS {
            assert_eq!(signup(t.id).is_some(), t.needs_key, "{}", t.id);
        }
    }

    #[test]
    fn a_pasted_key_is_taken_only_in_its_providers_shape() {
        let groq = &signup("groq").unwrap().values[0];
        let key = format!("gsk_{}", "A1b2".repeat(13));
        assert_eq!(shape(groq, &format!("  {key}\n")).unwrap(), key, "trimmed");
        assert_eq!(shape(groq, "   "), Err(Misshapen::Empty));
        for not_a_key in ["my password is hunter2", "sk-or-v1-abcdefabcdefabcdefabcdefabcdefabcdef", &format!("gsk_{}!", "a".repeat(50))] {
            assert_eq!(shape(groq, not_a_key), Err(Misshapen::NotAKey), "{not_a_key}");
        }
        let account = &signup("cloudflare").unwrap().values[0];
        assert!(shape(account, "0123456789abcdef0123456789abcdef").is_ok());
        assert!(shape(account, "0123456789abcdef0123456789abcdeg").is_err(), "not hex");
        // Google's current keys begin `AQ.` and carry dots; its older ones `AIza`.
        let gemini = &signup("gemini").unwrap().values[0];
        assert!(shape(gemini, &format!("AQ.Ab8RN{}", "x_y-Z.9".repeat(8))).is_ok(), "the AQ. shape AI Studio issues now");
        assert!(shape(gemini, &format!("AIza{}", "Sy_x".repeat(9))).is_ok(), "the older AIza shape");
        assert!(shape(gemini, "AQ. my password").is_err());
        let zai = &signup("zai").unwrap().values[0];
        assert!(shape(zai, &format!("{}.{}", "a".repeat(32), "B".repeat(16))).is_ok());
        assert!(shape(zai, &"a".repeat(48)).is_err(), "no dot");
    }

    #[test]
    fn what_looks_like_a_key_is_recognised_and_ordinary_text_is_not() {
        assert!(looks_like_a_provider_key(&format!("gsk_{}", "x9".repeat(26))));
        assert!(looks_like_a_provider_key(&format!("sk-or-v1-{}", "ab12".repeat(16))));
        assert!(looks_like_a_provider_key(&format!("AIza{}", "Sy_x".repeat(9))));
        assert!(looks_like_a_provider_key(&format!("cfut_{}", "k".repeat(44))));
        for ordinary in ["hello world", "https://example.com/page", "the quick brown fox jumps over the lazy dog again", "gsk_ is a prefix"] {
            assert!(!looks_like_a_provider_key(ordinary), "{ordinary}");
        }
    }
}
