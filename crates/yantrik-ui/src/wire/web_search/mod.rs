//! Settings → Network → Web search: the one web search setting (#1439).
//!
//! SearXNG on: the person's SearXNG answers every search, whoever asks. Off: the OS's own browser
//! does (`yantrik_companion_tools`). The setting is `~/.config/yantrik/web_search.yaml`
//! (crates/yantrik-web-search), written here and read by the companion's tools on every search.
//!
//! A SearXNG is saved only after a Test of that exact address found at least one result, from
//! Settings and from an agent alike: an address that answers nothing would turn every search into
//! an error.

pub mod agent;

use std::sync::Mutex;

use slint::ComponentHandle;
use yantrik_web_search::{client, Service, Target, WebSearch};

use crate::{App, WebSearchState};

pub use agent::{describe, explain_set, set_from_agent};

/// The last Test: the normalised address and whether it found results. Save reads it. An edit of
/// the field, a switch and every save clear it, so a result never vouches for an address it was
/// not about, nor for the same address later.
static TESTED: Mutex<Option<(String, bool)>> = Mutex::new(None);

fn forget_test() {
    TESTED.lock().unwrap_or_else(|e| e.into_inner()).take();
}

/// Whether Save may be pressed: something differs from what is saved, and a SearXNG address was
/// tested as it is now and found results.
pub fn can_save(chosen: Service, field: &str, saved: &WebSearch, tested: Option<&(String, bool)>) -> bool {
    match chosen {
        Service::Off => saved.service != Service::Off,
        Service::Searxng => match yantrik_web_search::check(field) {
            Ok(url) => {
                tested.is_some_and(|(t, ok)| *ok && *t == url.url) && !(saved.service == Service::Searxng && saved.searxng_url == url.url)
            }
            Err(_) => false,
        },
    }
}

/// Write the setting. The one place a change is applied, for Settings and for an agent's
/// `set_web_search`. The last Test is spent here, whatever it was about.
pub fn apply(chosen: Service, url: Option<String>) -> Result<WebSearch, String> {
    forget_test();
    let saved = yantrik_web_search::load();
    let web_search = WebSearch {
        service: chosen,
        // Off keeps the last SearXNG address, so switching it on again does not mean typing it.
        searxng_url: url.unwrap_or(saved.searxng_url),
        saved_at: chrono::Local::now().to_rfc3339(),
    };
    yantrik_web_search::save(&web_search)?;
    tracing::info!(service = chosen.as_str(), "Web search setting saved");
    Ok(web_search)
}

/// "Saved: your SearXNG · 5 Oct 2026, 14:02".
pub fn saved_line(ws: &WebSearch) -> String {
    if ws.saved_at.is_empty() {
        return String::new();
    }
    let when = chrono::DateTime::parse_from_rfc3339(&ws.saved_at)
        .map(|t| t.with_timezone(&chrono::Local).format("%-d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|_| ws.saved_at.clone());
    match ws.target() {
        Target::Searxng(url) => format!("Saved: your SearXNG at {} · {when}", url.url),
        Target::Browser => format!("Saved: SearXNG off, this desktop's own browser searches · {when}"),
        Target::Invalid { why } => format!("The saved SearXNG address is not valid ({why}); searches fail until it is fixed."),
    }
}

/// Put what is saved on screen, as the page shows it after a save or at start.
fn show_saved(ui: &App) {
    show_saved_keeping(ui, false);
}

/// [`show_saved`], for a save the person did not make (an agent's): an address they are typing
/// stays in the field rather than being replaced under their hands.
pub(super) fn show_saved_keeping(ui: &App, keep_typed: bool) {
    let ws = yantrik_web_search::load();
    let g = ui.global::<WebSearchState>();
    g.set_service(ws.service.as_str().into());
    let typing = keep_typed && !g.get_url().is_empty();
    if !ws.searxng_url.is_empty() && !typing {
        g.set_url(ws.searxng_url.clone().into());
    }
    g.set_saved_line(saved_line(&ws).into());
    // A save spent the last Test (`apply`): its line would vouch for nothing now.
    g.set_test_line("".into());
    g.set_test_ok(false);
    g.set_save_error("".into());
    g.set_can_save(false);
}

fn chosen_service(ui: &App) -> Service {
    if ui.global::<WebSearchState>().get_service() == "searxng" { Service::Searxng } else { Service::Off }
}

fn refresh_can_save(ui: &App) {
    let g = ui.global::<WebSearchState>();
    let tested = TESTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    g.set_can_save(can_save(chosen_service(ui), &g.get_url(), &yantrik_web_search::load(), tested.as_ref()));
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
        let chosen = chosen_service(&ui);
        let tested = TESTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        // The button is disabled otherwise; checked again here, where it counts.
        if !can_save(chosen, &g.get_url(), &yantrik_web_search::load(), tested.as_ref()) {
            g.set_save_error("Test the address first: it is saved only after a test search finds results.".into());
            return;
        }
        let url = match chosen {
            Service::Searxng => yantrik_web_search::check(&g.get_url()).ok().map(|u| u.url),
            Service::Off => None,
        };
        match apply(chosen, url) {
            Ok(_) => show_saved(&ui),
            Err(e) => g.set_save_error(format!("Not saved: {e}").into()),
        }
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
        let none = saved(Service::Off, "");
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
        assert!(can_save(Service::Off, "", &same, None), "off needs no test");
        assert!(!can_save(Service::Off, "", &none, None), "already off");
    }

    #[test]
    fn the_saved_line_says_what_and_when() {
        let mut ws = saved(Service::Searxng, "http://192.168.4.42:8888");
        assert_eq!(saved_line(&ws), "");
        ws.saved_at = "2026-10-05T14:02:00+00:00".into();
        let line = saved_line(&ws);
        assert!(line.starts_with("Saved: your SearXNG at http://192.168.4.42:8888 · ") && line.contains("2026"), "{line}");
        let mut off = saved(Service::Off, "");
        off.saved_at = ws.saved_at.clone();
        assert!(saved_line(&off).starts_with("Saved: SearXNG off, this desktop's own browser searches · "));
        let mut bad = saved(Service::Searxng, "http://search.example.com");
        bad.saved_at = ws.saved_at.clone();
        assert!(saved_line(&bad).contains("searches fail until it is fixed"));
    }

    /// A passing Test is spent by a save and by a switch, so it cannot let Save through later for
    /// an address nobody tested since.
    #[test]
    fn a_test_result_is_spent_by_a_save_a_switch_and_an_edit() {
        let src = include_str!("mod.rs");
        let src = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        let apply = &src[src.find("pub fn apply(").unwrap()..];
        assert!(apply[..apply.find("yantrik_web_search::save").unwrap()].contains("forget_test();"), "apply spends the test first");
        let choose = &src[src.find("g.on_choose(").unwrap()..src.find("g.on_url_edited(").unwrap()];
        assert!(choose.contains("forget_test();"), "a switch spends it:\n{choose}");
        let edited = &src[src.find("g.on_url_edited(").unwrap()..src.find("g.on_test(").unwrap()];
        assert!(edited.contains("forget_test();"));
        let field = "http://192.168.4.42:8888";
        let good = (field.to_string(), true);
        let off = saved(Service::Off, field);
        assert!(can_save(Service::Searxng, field, &off, Some(&good)));
        *TESTED.lock().unwrap() = Some(good);
        forget_test();
        assert!(!can_save(Service::Searxng, field, &off, TESTED.lock().unwrap().as_ref()));
    }

    /// Save answers a pointer and nothing else, so no process in the session can press it through
    /// the accessibility default action. Test changes nothing and stays reachable from the
    /// keyboard. YButton's own gate is tested in yantrik-ui-kit.
    #[test]
    fn save_is_pointer_only_and_test_is_not() {
        let src = include_str!("../../../../yantrik-ui-slint/ui/components/web_search_card.slint");
        let button_with = |needle: &str| {
            let at = src.find(needle).unwrap_or_else(|| panic!("`{needle}` is no longer on the card"));
            let open = src[..at].rfind("YButton {").expect("a YButton opens before it");
            src[open..open + src[open..].find('}').unwrap()].to_string()
        };
        assert!(button_with("label: \"Save\";").contains("pointer-only: true;"));
        let test = button_with("label: WebSearchState.testing ? \"Testing…\" : \"Test\";");
        assert!(!test.contains("pointer-only"), "Test is reachable from the keyboard:\n{test}");
        assert_eq!(src.matches("YButton {").count(), 2, "a third button needs a decision and a line here");
        assert_eq!(src.matches("pointer-only: true;").count(), 1);
        for banned in ["accessible-action", "forward-focus", ".focus()"] {
            assert!(!src.contains(banned), "the card must not use `{banned}`");
        }
    }
}
