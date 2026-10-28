//! Settings → AI & Intelligence says only what the code does (#998 and the truth fix after it).
//!
//! The page lost its inert controls (task routing, privacy level, local-only rules, cloud
//! confirmation, usage and budget, auto-fallback, the status card's metrics) and its overclaiming
//! sentences. These tests read the page's sources, as the other source tests in this crate do, so
//! a removed control or an old sentence cannot come back without a test saying so.

const SETTINGS: &str = include_str!("../../../yantrik-ui-slint/ui/settings.slint");
const APP: &str = include_str!("../../../yantrik-ui-slint/ui/app.slint");
const ACCOUNTS_CARD: &str = include_str!("../../../yantrik-ui-slint/ui/components/ai_accounts_card.slint");
const FREE_CARD: &str = include_str!("../../../yantrik-ui-slint/ui/components/free_ai_card.slint");
const ACCOUNTS_RS: &str = include_str!("ai_accounts_card.rs");
const FREE_ROWS_RS: &str = include_str!("free_ai/rows.rs");

/// The AI page: from its category line to the provider overlay that follows its body.
fn ai_page() -> &'static str {
    let from = SETTINGS.find("if root.active-category == 1 :").expect("the AI category");
    let rest = &SETTINGS[from..];
    let to = rest.find("// ===== CATEGORY 2").expect("the next category");
    &rest[..to]
}

/// The page's body: the first `Flickable` up to the provider overlay.
fn ai_body() -> &'static str {
    let page = ai_page();
    let from = page.find("Flickable {").expect("the AI page scrolls");
    let to = page.find("ADD/EDIT PROVIDER OVERLAY").expect("the overlay");
    &page[from..to]
}

/// The text of every double-quoted literal on non-comment lines.
fn literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines().filter(|l| !l.trim_start().starts_with("//")) {
        let mut rest = line;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let mut end = None;
            let mut escaped = false;
            for (i, c) in after.char_indices() {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    end = Some(i);
                    break;
                }
            }
            let Some(end) = end else { break };
            out.push(after[..end].to_string());
            rest = &after[end + 1..];
        }
    }
    out
}

#[test]
fn the_ai_page_scrolls_like_every_other_settings_page() {
    // #998: the Flickable had no viewport, so nothing below the first screenful could be reached.
    // The viewport is the body's own height (taller than the window when the page is), and as wide
    // as the window, as on the other pages.
    let body = ai_body();
    let head = &body[..body.find("VerticalLayout").expect("a body inside the Flickable")];
    assert!(head.contains("viewport-width: self.width;"), "{head}");
    assert!(head.contains("viewport-height: ai-body.preferred-height;"), "{head}");
    assert!(body.contains("ai-body := VerticalLayout {"), "the viewport names the layout it measures");
    assert!(!head.contains("viewport-height: 100%"), "a viewport as tall as the window never scrolls");
}

#[test]
fn the_removed_controls_are_not_on_the_page() {
    let shown: Vec<String> = literals(ai_page()).into_iter().chain(literals(ACCOUNTS_CARD)).chain(literals(FREE_CARD)).collect();
    for gone in [
        "TASK ROUTING",
        "Fast",
        "Balanced",
        "Powerful",
        "PRIVACY LEVEL",
        "Privacy Level",
        "Maximum",
        "Cloud-First",
        "Local-only rules",
        "Never send data to cloud",
        "Cloud escalation confirm",
        "Ask before cloud send",
        "USAGE & COST",
        "Monthly budget",
        "Approaching budget limit",
        "FALLBACK",
        "Auto-fallback",
        "Set Fallback",
        "Latency",
        "Tokens/sec",
        "Today",
        "Running on fallback",
    ] {
        assert!(!shown.iter().any(|s| s.contains(gone)), "the AI page still shows {gone:?}");
    }
    for gone in [
        "privacy-preset",
        "privacy-local-only",
        "privacy-cloud-confirm",
        "route-fast",
        "route-balanced",
        "route-powerful",
        "save-task-routing",
        "save-budget",
        "budget-",
        "usage-",
        "toggle-auto-fallback",
        "set-fallback-provider",
        "ai-set-budget",
        "AiAccountsState.gateway",
    ] {
        assert!(!SETTINGS.contains(gone) && !APP.contains(gone) && !ACCOUNTS_CARD.contains(gone), "{gone} is still declared or wired");
    }
}

#[test]
fn the_real_privacy_controls_are_kept() {
    let page = ai_page();
    assert!(page.contains("Credential vault") && page.contains("vault-set-passphrase"), "the vault card is real");
}

#[test]
fn the_sentences_say_what_the_code_does() {
    // Every gateway call is treated as private, so a shared-nothing choice has no "everything else".
    // The choice binds the gateway, so the lines are about Minds that use Yantrik models; the
    // built-in companion on a saved primary talks to its provider directly and the row says so.
    assert!(ACCOUNTS_RS.contains("Requests from your Minds that use Yantrik models to {} go to your private model {} instead."));
    assert!(ACCOUNTS_RS.contains("The built-in companion talks to {} directly; this choice applies to your other Minds."));
    assert!(!ACCOUNTS_RS.contains("Your Minds' requests"));
    assert!(!ACCOUNTS_RS.contains("everything else goes to"));
    let code = ACCOUNTS_RS.split("#[cfg(test)]").next().unwrap_or(ACCOUNTS_RS);
    assert!(!code.contains("kept on this computer by") && !code.contains("Personal things are kept"), "the lane's place comes from its address");
    // Share and the lane header speak of the gateway's Minds and the lane, not of every Mind.
    assert!(code.contains("Requests from your Minds that use Yantrik models go to {} with what they know about you."));
    assert!(!code.contains("Your minds share what they know"));
    assert!(code.contains("Your private model: {account} · {model}, {}."));
    assert!(ACCOUNTS_RS.contains("\"On this computer or your network.\""));
    assert!(!ACCOUNTS_RS.contains("never leaves it"));
    // The vault holds the free tiers' keys and nothing else.
    assert!(ACCOUNTS_CARD.contains("Free-tier keys are kept in the vault."));
    assert!(!ACCOUNTS_CARD.contains("a mind is never given one"));
    // The pool never gets a gateway turn, because every turn is private.
    assert!(FREE_CARD.contains("Requests from your Minds that use Yantrik models don't go to these providers: Yantrik treats every such request as private."));
    assert!(!FREE_CARD.contains("nothing private in them"));
    // The sign-up page is a separate profile, not a sealed one.
    assert!(FREE_ROWS_RS.contains("in a separate browser profile, apart from your other browsing."));
    assert!(!FREE_ROWS_RS.contains("nothing on this machine can read"));
    // The training labels carry the date of the table they come from.
    assert!(FREE_CARD.contains("NEVER TRAINS ON YOUR PROMPTS, AS OF 30 SEP 2026"));
    assert!(FREE_CARD.contains("TRAINS ON YOUR PROMPTS, AS OF 30 SEP 2026"));
}

#[test]
fn the_training_date_is_the_tables_date() {
    // The labels come from yantrik-ml's curated table; the page's date is that table's.
    let table = include_str!("../../../yantrik-ml/src/provider/pool/tiers.rs");
    assert!(table.contains("2026-09-30"), "the curated table is no longer dated 30 Sep 2026; update the page's label");
}

#[test]
fn no_sentence_on_the_page_carries_a_run_of_spaces() {
    // A long string split across lines ends each line with a backslash. When one is lost, the next
    // line's indentation lands inside the sentence.
    for (name, src) in [("ai_accounts_card.rs", ACCOUNTS_RS), ("free_ai/rows.rs", FREE_ROWS_RS), ("ai_accounts_card.slint", ACCOUNTS_CARD), ("free_ai_card.slint", FREE_CARD)] {
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        for s in literals(code) {
            assert!(!s.contains("   "), "{name} has a sentence with a run of spaces: {s:?}");
        }
    }
}

#[test]
fn no_saved_provider_row_shows_a_fallback_badge() {
    // Nothing builds a backend from a provider's stored `is_fallback`; the flag stays in the file
    // and the row neither says it nor colours its border for it. config.yaml's `llm.fallback` is
    // the real one, and the map of what runs on what names only that one.
    let page = ai_page();
    assert!(!page.contains("provider.is-fallback"), "a provider row reads the stored fallback flag");
    assert!(!literals(page).iter().any(|s| s.contains("Fallback")), "the AI page shows a Fallback label");
}

#[test]
fn nothing_declares_a_privacy_mode_chip_that_nothing_draws() {
    // `ai-privacy-mode` was set from "any saved account" and read by no element; it is gone.
    assert!(!APP.contains("ai-privacy-mode") && !SETTINGS.contains("ai-privacy-mode"));
}

#[test]
fn nothing_declares_a_fallback_state_that_nothing_sets() {
    // `using-fallback`, the "fallback" status and `ai-fallback-banner` were never set by anything.
    for src in [SETTINGS, APP] {
        assert!(!src.contains("using-fallback") && !src.contains("ai-fallback-banner"));
        assert!(!src.contains("status == \"fallback\""));
    }
    assert!(!include_str!("../../../yantrik-ui-slint/ui/desktop.slint").contains("ai-fallback-banner"));
    assert!(!include_str!("ai_status.rs").contains("using_fallback"));
}

#[test]
fn the_lane_button_does_not_say_where_the_lane_is() {
    // A lane on the network is not "on this computer": the labels name the role, not the place.
    assert!(ACCOUNTS_CARD.contains("Use as your private model"));
    assert!(!ACCOUNTS_CARD.contains("personal things on this computer") && !ACCOUNTS_CARD.contains("personal things here"));
}
