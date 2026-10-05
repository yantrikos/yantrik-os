//! Settings → Network → Web search: which service the person's minds and tools search through.
//!
//! One setting, the `web_search` section of settings.yaml (crates/yantrik-web-search). Saving it
//! does three things and nothing else: writes the file (the companion's tools read it per search),
//! tells the harness host (every attached harness hears it on its next poll), and asks the egress
//! proxy whether minds can reach the address (`egress`), offering a button when they cannot.
//!
//! A SearXNG is saved only after a Test of that exact address found at least one result, from
//! Settings and from an agent alike: an address that answers nothing would turn every search into
//! a silent fallback.

pub mod agent;
pub mod egress;

use std::sync::Mutex;

use slint::ComponentHandle;
use yantrik_web_search::{client, SearxUrl, Service, Target, WebSearch};

use crate::{App, WebSearchState};

pub use agent::{describe, explain_set, set_from_agent};

/// The last Test: the normalised address and whether it found results. Save reads it. An edit of
/// the field, a switch of service and every save clear it, so a result never vouches for an
/// address it was not about, nor for the same address later.
static TESTED: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// The address the "Allow minds to reach this search service" button on screen was offered for.
/// The press adds a rule only for this, and only while it is still what is saved.
static OFFERED: Mutex<Option<String>> = Mutex::new(None);

fn forget_test() {
    TESTED.lock().unwrap_or_else(|e| e.into_inner()).take();
}

fn set_offer(url: Option<String>) {
    *OFFERED.lock().unwrap_or_else(|e| e.into_inner()) = url;
}

/// What the harness protocol carries for `ws`. An address that fails the check is never sent:
/// that harness is told built-in, as the companion's tools fall back to it.
pub fn to_protocol(ws: &WebSearch) -> yantrik_harness::protocol::WebSearch {
    match ws.target() {
        Target::Searxng(url) => yantrik_harness::protocol::WebSearch::searxng(url.url),
        Target::Builtin | Target::Invalid { .. } => yantrik_harness::protocol::WebSearch::builtin(),
    }
}

/// Whether Save may be pressed: something differs from what is saved, and a SearXNG address was
/// tested as it is now and found results.
pub fn can_save(chosen: Service, field: &str, saved: &WebSearch, tested: Option<&(String, bool)>) -> bool {
    match chosen {
        Service::Builtin => saved.service != Service::Builtin,
        Service::Searxng => match yantrik_web_search::check(field) {
            Ok(url) => {
                tested.is_some_and(|(t, ok)| *ok && *t == url.url)
                    && !(saved.service == Service::Searxng && saved.searxng_url == url.url)
            }
            Err(_) => false,
        },
    }
}

/// Write the setting and tell every harness. The one place a change is applied, for Settings and
/// for an agent's `set_web_search`. The last Test is spent here, whatever it was about.
pub fn apply(chosen: Service, url: Option<String>) -> Result<WebSearch, String> {
    forget_test();
    let saved = crate::wire::settings::web_search();
    let web_search = WebSearch {
        service: chosen,
        // Built-in keeps the last SearXNG address, so switching back does not mean typing it.
        searxng_url: url.unwrap_or(saved.searxng_url),
        saved_at: chrono::Local::now().to_rfc3339(),
    };
    crate::wire::settings::set_web_search(web_search.clone())?;
    if let Some(host) = crate::wire::harness::host() {
        host.set_web_search(to_protocol(&web_search));
    }
    tracing::info!(service = chosen.as_str(), "Web search service saved");
    Ok(web_search)
}

/// "Saved: SearXNG at http://… · 5 Oct, 14:02".
pub fn saved_line(ws: &WebSearch) -> String {
    if ws.saved_at.is_empty() {
        return String::new();
    }
    let when = chrono::DateTime::parse_from_rfc3339(&ws.saved_at)
        .map(|t| t.with_timezone(&chrono::Local).format("%-d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|_| ws.saved_at.clone());
    match ws.target() {
        Target::Searxng(url) => format!("Saved: SearXNG at {} · {when}", url.url),
        Target::Builtin => format!("Saved: Built-in (DuckDuckGo's HTML search, nothing else) · {when}"),
        Target::Invalid { why, .. } => format!("The saved SearXNG address is not valid ({why}); searches use DuckDuckGo until it is fixed."),
    }
}

/// Put what is saved on screen, as the page shows it after a save or at start.
fn show_saved(ui: &App) {
    set_offer(None);
    let ws = crate::wire::settings::web_search();
    let g = ui.global::<WebSearchState>();
    g.set_service(ws.service.as_str().into());
    if !ws.searxng_url.is_empty() {
        g.set_url(ws.searxng_url.clone().into());
    }
    g.set_saved_line(saved_line(&ws).into());
    // A save spent the last Test (`apply`): its line would vouch for nothing now.
    g.set_test_line("".into());
    g.set_test_ok(false);
    g.set_save_error("".into());
    g.set_can_save(false);
    g.set_egress_line("".into());
    g.set_egress_offer(false);
}

fn refresh_can_save(ui: &App) {
    let g = ui.global::<WebSearchState>();
    let chosen = if g.get_service() == "searxng" { Service::Searxng } else { Service::Builtin };
    let tested = TESTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    g.set_can_save(can_save(chosen, &g.get_url(), &crate::wire::settings::web_search(), tested.as_ref()));
}

/// The address the egress button may add a rule for at its press: the one it was offered for, and
/// only while that is still the saved address.
pub fn offered_url(offered: Option<&str>, saved: Target) -> Result<SearxUrl, String> {
    let Some(offered) = offered else {
        return Err("Nothing is offered now; save the address again to be asked.".into());
    };
    match saved {
        Target::Searxng(url) if url.url == offered => Ok(url),
        _ => Err("The saved web search address changed after this was offered, so no rule was added. Save it again to be asked about the one saved now.".into()),
    }
}

/// Ask the egress proxy, off the UI thread, what minds need to reach `url`.
fn check_egress(ui: &App, url: SearxUrl) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let need = egress::check(&egress::socket(), &url);
        let line = egress::line(&need, &url);
        let offer = matches!(need, egress::Need::Missing(_));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<WebSearchState>();
            set_offer(offer.then(|| url.url.clone()));
            g.set_egress_line(line.into());
            g.set_egress_offer(offer);
        });
    });
}

pub fn wire(ui: &App) {
    show_saved(ui);
    let g = ui.global::<WebSearchState>();

    let weak = ui.as_weak();
    g.on_choose(move |service| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        if g.get_service() != service {
            forget_test();
            g.set_test_line("".into());
            g.set_test_ok(false);
        }
        g.set_service(service);
        g.set_save_error("".into());
        refresh_can_save(&ui);
    });

    let weak = ui.as_weak();
    g.on_url_edited(move |_| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        forget_test();
        g.set_test_line("".into());
        g.set_test_ok(false);
        g.set_url_error("".into());
        g.set_can_save(false);
    });

    let weak = ui.as_weak();
    g.on_test(move || {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        let url = match yantrik_web_search::check(&g.get_url()) {
            Ok(url) => url,
            Err(why) => {
                g.set_url_error(why.into());
                g.set_test_line("".into());
                return;
            }
        };
        g.set_url_error("".into());
        g.set_testing(true);
        g.set_test_line("".into());
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let probe = client::probe(&url, client::TIMEOUT);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                let g = ui.global::<WebSearchState>();
                g.set_testing(false);
                // Edited while the test ran: this result is about an address no longer there.
                if yantrik_web_search::check(&g.get_url()).map(|u| u.url) != Ok(url.url.clone()) {
                    return;
                }
                *TESTED.lock().unwrap_or_else(|e| e.into_inner()) = Some((url.url.clone(), probe.ok));
                g.set_test_line(probe.summary.into());
                g.set_test_ok(probe.ok);
                refresh_can_save(&ui);
            });
        });
    });

    let weak = ui.as_weak();
    g.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        let chosen = if g.get_service() == "searxng" { Service::Searxng } else { Service::Builtin };
        let tested = TESTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // The button is disabled otherwise; checked again here, where it counts.
        if !can_save(chosen, &g.get_url(), &crate::wire::settings::web_search(), tested.as_ref()) {
            g.set_save_error("Test the address first: it is saved only after a test search finds results.".into());
            return;
        }
        let url = match chosen {
            Service::Searxng => yantrik_web_search::check(&g.get_url()).ok(),
            Service::Builtin => None,
        };
        match apply(chosen, url.as_ref().map(|u| u.url.clone())) {
            Ok(_) => {
                show_saved(&ui);
                if let Some(url) = url {
                    check_egress(&ui, url);
                }
            }
            Err(e) => g.set_save_error(format!("Not saved: {e}").into()),
        }
    });

    let weak = ui.as_weak();
    g.on_allow_egress(move || {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        // Only the address the button was offered for, and only while it is the saved one.
        let offered = OFFERED.lock().unwrap_or_else(|e| e.into_inner()).take();
        let url = match offered_url(offered.as_deref(), crate::wire::settings::web_search().target()) {
            Ok(url) => url,
            Err(why) => {
                g.set_egress_offer(false);
                g.set_egress_line(why.into());
                return;
            }
        };
        g.set_egress_busy(true);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let outcome = egress::allow(&egress::socket(), &url);
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                let g = ui.global::<WebSearchState>();
                g.set_egress_busy(false);
                let line = match outcome {
                    Ok(egress::Pressed::Added) => {
                        tracing::info!(host = %url.host, port = url.port, "egress rule added for the web search service");
                        g.set_egress_offer(false);
                        format!("Added a rule: minds may reach {}:{}.", url.host, url.port)
                    }
                    Ok(egress::Pressed::NotNeeded(need)) => {
                        g.set_egress_offer(false);
                        egress::line(&need, &url)
                    }
                    Err(e) => format!("The rule was not added: {e}"),
                };
                g.set_egress_line(line.into());
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(service: Service, url: &str) -> WebSearch {
        WebSearch { service, searxng_url: url.into(), saved_at: String::new() }
    }

    #[test]
    fn save_needs_a_test_of_this_exact_address_that_found_results() {
        let none = saved(Service::Builtin, "");
        let field = "http://192.168.4.42:8888/";
        assert!(!can_save(Service::Searxng, field, &none, None), "untested");
        let failed = ("http://192.168.4.42:8888".to_string(), false);
        assert!(!can_save(Service::Searxng, field, &none, Some(&failed)), "tested, no results");
        let good = ("http://192.168.4.42:8888".to_string(), true);
        assert!(can_save(Service::Searxng, field, &none, Some(&good)), "tested as normalised, with results");
        assert!(!can_save(Service::Searxng, "http://192.168.4.43:8888", &none, Some(&good)), "a result vouches only for its address");
        assert!(!can_save(Service::Searxng, "http://search.example.com", &none, Some(&good)), "an address that fails the check");
        let same = saved(Service::Searxng, "http://192.168.4.42:8888");
        assert!(!can_save(Service::Searxng, field, &same, Some(&good)), "nothing to change");
        assert!(can_save(Service::Builtin, "", &same, None), "built-in needs no test");
        assert!(!can_save(Service::Builtin, "", &none, None), "already built-in");
    }

    #[test]
    fn harnesses_are_told_a_checked_address_or_builtin() {
        let ws = saved(Service::Searxng, "http://192.168.4.42:8888/");
        assert_eq!(to_protocol(&ws), yantrik_harness::protocol::WebSearch::searxng("http://192.168.4.42:8888"));
        assert_eq!(to_protocol(&saved(Service::Searxng, "http://me:pw@10.0.0.2")), yantrik_harness::protocol::WebSearch::builtin());
        assert_eq!(to_protocol(&saved(Service::Builtin, "http://10.0.0.2")), yantrik_harness::protocol::WebSearch::builtin());
    }

    #[test]
    fn the_saved_line_says_what_and_when() {
        let mut ws = saved(Service::Searxng, "http://192.168.4.42:8888");
        assert_eq!(saved_line(&ws), "");
        ws.saved_at = "2026-10-05T14:02:00+00:00".into();
        let line = saved_line(&ws);
        assert!(line.starts_with("Saved: SearXNG at http://192.168.4.42:8888 · ") && line.contains("2026"), "{line}");
        let mut builtin = saved(Service::Builtin, "");
        builtin.saved_at = ws.saved_at.clone();
        assert!(saved_line(&builtin).starts_with("Saved: Built-in (DuckDuckGo's HTML search, nothing else) · "));
    }

    /// Security review of #656, finding 7: a passing Test is spent by a save and by a switch of
    /// service, so it cannot let Save through later for an address nobody tested since.
    #[test]
    fn a_test_result_is_spent_by_a_save_and_a_switch() {
        let src = include_str!("mod.rs");
        let src = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        let apply = &src[src.find("pub fn apply(").unwrap()..];
        assert!(apply[..apply.find("crate::wire::settings::set_web_search").unwrap()].contains("forget_test();"), "apply spends the test first");
        let choose = &src[src.find("g.on_choose(").unwrap()..src.find("g.on_url_edited(").unwrap()];
        assert!(choose.contains("forget_test();"), "a switch of service spends it:\n{choose}");
        let edited = &src[src.find("g.on_url_edited(").unwrap()..src.find("g.on_test(").unwrap()];
        assert!(edited.contains("forget_test();"));

        // And what that buys: with the result gone, Save is not offered for the same address.
        let field = "http://192.168.4.42:8888";
        let good = (field.to_string(), true);
        let builtin = saved(Service::Builtin, field);
        assert!(can_save(Service::Searxng, field, &builtin, Some(&good)));
        *TESTED.lock().unwrap() = Some(good);
        forget_test();
        assert!(!can_save(Service::Searxng, field, &builtin, TESTED.lock().unwrap().as_ref()));
    }

    /// Security review of #656, finding 8: the egress button adds a rule only for the address it
    /// was offered for, and refuses if the saved address is another by the press.
    #[test]
    fn the_egress_offer_is_bound_to_its_address() {
        let saved_now = |url: &str| saved(Service::Searxng, url).target();
        let lan = "http://192.168.4.42:8888";
        assert_eq!(offered_url(Some(lan), saved_now(lan)).unwrap().url, lan);
        let moved = offered_url(Some(lan), saved_now("https://search.example.com")).unwrap_err();
        assert!(moved.contains("changed after this was offered"), "{moved}");
        assert!(offered_url(Some(lan), Target::Builtin).is_err());
        assert!(offered_url(None, saved_now(lan)).is_err(), "nothing offered, nothing added");

        let src = include_str!("mod.rs");
        let press = &src[src.find("g.on_allow_egress(").unwrap()..];
        let press = &press[..press.find("egress::allow(").unwrap()];
        assert!(press.contains("OFFERED.lock()") && press.contains(".take()") && press.contains("offered_url("), "{press}");
    }

    /// Security review of #656, finding 2: Save, Test and the egress button answer a pointer and
    /// nothing else, so no process in the session can press them through the accessibility
    /// default action. YButton's own gate is tested in yantrik-ui-kit.
    #[test]
    fn the_cards_buttons_are_pointer_only() {
        let src = include_str!("../../../../yantrik-ui-slint/ui/components/web_search_card.slint");
        let button_with = |needle: &str| {
            let at = src.find(needle).unwrap_or_else(|| panic!("`{needle}` is no longer on the card"));
            let open = src[..at].rfind("YButton {").expect("a YButton opens before it");
            src[open..open + src[open..].find('}').unwrap()].to_string()
        };
        for label in ["label: \"Save\";", "label: WebSearchState.testing ? \"Testing…\" : \"Test\";", "label: \"Allow minds to reach this search service\";"] {
            let b = button_with(label);
            assert!(b.contains("pointer-only: true;"), "{label} lost pointer-only:\n{b}");
        }
        assert_eq!(src.matches("YButton {").count(), 3, "a fourth button needs pointer-only and a line here");
        assert_eq!(src.matches("pointer-only: true;").count(), 3);
        for banned in ["accessible-action", "forward-focus", ".focus()"] {
            assert!(!src.contains(banned), "the card must not use `{banned}`");
        }
    }
}
