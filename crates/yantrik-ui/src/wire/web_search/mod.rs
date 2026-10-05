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

pub mod egress;

use std::sync::Mutex;

use slint::ComponentHandle;
use yantrik_web_search::{client, Service, Target, WebSearch};

use crate::{App, WebSearchState};

/// The last Test: the normalised address and whether it found results. Save reads it, and an edit
/// of the field clears it, so a result never vouches for an address it was not about.
static TESTED: Mutex<Option<(String, bool)>> = Mutex::new(None);

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
/// for an agent's `set_web_search`.
pub fn apply(chosen: Service, url: Option<String>) -> Result<WebSearch, String> {
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
        Target::Builtin => format!("Saved: Built-in (DuckDuckGo) · {when}"),
        Target::Invalid { why, .. } => format!("The saved SearXNG address is not valid ({why}); searches use DuckDuckGo until it is fixed."),
    }
}

/// For `describe shell` → `settings` → `web_search`.
pub fn describe() -> serde_json::Value {
    let ws = crate::wire::settings::web_search();
    let (url, valid) = match ws.target() {
        Target::Searxng(u) => (Some(u.url), true),
        Target::Builtin => (None, true),
        Target::Invalid { .. } => (None, false),
    };
    serde_json::json!({
        "service": ws.service.as_str(),
        "url": url,
        "valid": valid,
        "saved_at": ws.saved_at,
        "change_with": "set_web_search (sensitive: every search would go to the new address)",
    })
}

/// The sentence on an agent's approval card for `set_web_search`: where every search would go.
pub fn explain_set(service: &str, url: &str) -> String {
    let now = match crate::wire::settings::web_search().target() {
        Target::Searxng(u) => format!("the person's SearXNG at {}", u.url),
        _ => "DuckDuckGo".to_string(),
    };
    match service {
        "builtin" => format!("Sends every web search the person's minds and tools make to DuckDuckGo directly, instead of {now}."),
        "searxng" => match yantrik_web_search::check(url) {
            Ok(u) => format!(
                "Sends every web search the person's minds and tools make to {} instead of {now}, after a test search there finds results. Whoever runs that address sees every query and chooses what comes back.",
                u.url
            ),
            Err(why) => format!("This would not go ahead: {why}"),
        },
        other => format!("This would not go ahead: `{other}` is not a service; use builtin or searxng."),
    }
}

/// `set_web_search`, for an agent: the same checks as Save, the Test included. Answers at once;
/// the test and the save happen off the UI thread, and `describe` shows the outcome.
pub fn set_from_agent(service: &str, url: &str, ui: slint::Weak<App>) -> Result<String, String> {
    match service {
        "builtin" => {
            apply(Service::Builtin, None)?;
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui.upgrade() {
                    show_saved(&ui);
                }
            });
            Ok("Saved: built-in (DuckDuckGo).".into())
        }
        "searxng" => {
            let checked = yantrik_web_search::check(url)?;
            let shown = checked.url.clone();
            std::thread::spawn(move || {
                let probe = client::probe(&checked, client::TIMEOUT);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = ui.upgrade() else { return };
                    let g = ui.global::<WebSearchState>();
                    if !probe.ok {
                        tracing::info!(summary = %probe.summary, "an agent's web search address was not saved");
                        g.set_save_error(format!("Not saved (asked by an agent): {}", probe.summary).into());
                        return;
                    }
                    match apply(Service::Searxng, Some(checked.url.clone())) {
                        Ok(_) => {
                            show_saved(&ui);
                            check_egress(&ui, checked);
                        }
                        Err(e) => g.set_save_error(format!("Not saved: {e}").into()),
                    }
                });
            });
            Ok(format!("Testing {shown}; it is saved only if the test search finds results."))
        }
        other => Err(format!("`{other}` is not a service; use builtin or searxng")),
    }
}

/// Put what is saved on screen, as the page shows it after a save or at start.
fn show_saved(ui: &App) {
    let ws = crate::wire::settings::web_search();
    let g = ui.global::<WebSearchState>();
    g.set_service(ws.service.as_str().into());
    if !ws.searxng_url.is_empty() {
        g.set_url(ws.searxng_url.clone().into());
    }
    g.set_saved_line(saved_line(&ws).into());
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

/// Ask the egress proxy, off the UI thread, what minds need to reach `url`.
fn check_egress(ui: &App, url: yantrik_web_search::SearxUrl) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let need = egress::check(&egress::socket(), &url);
        let line = egress::line(&need, &url);
        let offer = matches!(need, egress::Need::Missing(_));
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let g = ui.global::<WebSearchState>();
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
        g.set_service(service);
        g.set_save_error("".into());
        refresh_can_save(&ui);
    });

    let weak = ui.as_weak();
    g.on_url_edited(move |_| {
        let Some(ui) = weak.upgrade() else { return };
        let g = ui.global::<WebSearchState>();
        TESTED.lock().unwrap_or_else(|e| e.into_inner()).take();
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
        // Only ever the saved address: the button is about what minds will search through.
        let Target::Searxng(url) = crate::wire::settings::web_search().target() else { return };
        let g = ui.global::<WebSearchState>();
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

    /// An agent changing where searches go is asked about first, and the card names the address.
    #[test]
    fn an_agent_changing_it_is_sensitive_and_the_card_names_the_address() {
        let src = include_str!("../../control.rs");
        let src = src.split("#[cfg(test)]").next().unwrap();
        let from = src.find("\"set_web_search\"").expect("the shell publishes set_web_search");
        let decl = &src[from..from + src[from..].find(".action(").unwrap()];
        assert!(decl.contains(".risk(\"sensitive\")") || decl.contains(".risk(\"dangerous\")"), "{decl}");
        assert!(!decl.contains(".risk(\"standard\")") && !decl.contains(".risk(\"safe\")"), "{decl}");
        assert!(decl.contains(".explain("), "the card says where searches would go: {decl}");

        let card = explain_set("searxng", "http://192.168.4.42:8888/");
        assert!(card.contains("http://192.168.4.42:8888 instead of") && card.contains("sees every query"), "{card}");
        assert!(explain_set("searxng", "http://search.example.com").starts_with("This would not go ahead"));
        assert!(explain_set("builtin", "").contains("DuckDuckGo directly"));
        assert!(explain_set("google", "").starts_with("This would not go ahead"));
    }

    #[test]
    fn the_saved_line_says_what_and_when() {
        let mut ws = saved(Service::Searxng, "http://192.168.4.42:8888");
        assert_eq!(saved_line(&ws), "");
        ws.saved_at = "2026-10-05T14:02:00+00:00".into();
        let line = saved_line(&ws);
        assert!(line.starts_with("Saved: SearXNG at http://192.168.4.42:8888 · ") && line.contains("2026"), "{line}");
    }
}
