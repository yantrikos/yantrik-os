//! Every Rust road to a browser (#1085): none names the person's port, none fetches a browser's
//! targets itself, none finds a browser by its program name, and each goes through the one checked
//! reader (`browser_side`) or, for Arcade, through the listener check. Source text, like the
//! tests that hold dock.rs and the JavaScript scan; the behaviour is in the unit tests of
//! `browser_side`, `browser_lifecycle`, `yantrik-browser-ports` and `play`.

fn code(src: &str) -> String {
    let end = ["#[cfg(test)]", "#[cfg(all(test"].iter().filter_map(|m| src.find(m)).min().unwrap_or(src.len());
    src[..end].lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n")
}

const ROADS: &[(&str, &str)] = &[
    ("browser.rs (browser_*, the CDP helpers, launch)", include_str!("../src/browser.rs")),
    ("browser_lifecycle.rs (the watchdog, browser_cleanup, browser_status)", include_str!("../src/browser_lifecycle.rs")),
    ("browser_side.rs (the checked reader)", include_str!("../src/browser_side.rs")),
    ("life_assistant/orchestrator.rs (cdp_first_tab_ws)", include_str!("../../yantrik-companion/src/life_assistant/orchestrator.rs")),
    ("arcade play.rs", include_str!("../../../apps/arcade/src/play.rs")),
];

#[test]
fn no_road_names_a_port_or_finds_a_browser_by_its_program() {
    for (road, src) in ROADS {
        let code = code(src);
        for banned in ["9222", "9223", "pgrep", "pkill", "mind-view-browser"] {
            assert!(!code.contains(banned), "{road} spells `{banned}`");
        }
    }
}

#[test]
fn the_companions_roads_fetch_targets_only_through_the_checked_reader() {
    for (road, src) in &ROADS[..4] {
        let code = code(src);
        if road.starts_with("browser_side") {
            continue;
        }
        for fetch in ["CDP_PORT}/json", "/json/list", "/json/new", "/json/close", "/json/version"] {
            assert!(!code.contains(fetch), "{road} fetches a browser's targets itself (`{fetch}`)");
        }
    }
    let orchestrator = code(ROADS[3].1);
    assert!(orchestrator.contains("browser_side::targets()") && orchestrator.contains("browser_side::is_running()"));
    let lifecycle = code(ROADS[1].1);
    assert!(lifecycle.contains("browser_side::targets()") && lifecycle.contains("browser_side::close_target("));
    assert!(lifecycle.contains("procs::mind_side_family()"), "the watchdog counts and kills a mind's browsers only");
}

#[test]
fn arcade_checks_what_holds_the_port_before_and_after_it_waits() {
    let play = code(ROADS[4].1);
    assert!(play.matches("vet_now(side)?").count() >= 2, "before the wait and again just before the tab is opened");
    assert!(play.contains("yantrik_app::for_mind::from_env()"), "whose Arcade it is picks the browser");
}
