//! The nested compositor's side of the desk: `config/labwc-mind/rc.xml` reserves the margins the
//! two bars sit in and places them, and the numbers and titles it uses are this app's.

use crate::metrics::{BOTTOM_HEIGHT, BOTTOM_TITLE, TOP_HEIGHT, TOP_TITLE};

const RC: &str = include_str!("../../../../config/labwc-mind/rc.xml");

/// The file with its comments taken out, which is what labwc reads.
fn config() -> String {
    let mut out = String::new();
    let mut rest = RC;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find("-->").expect("every comment is closed") + start + 3;
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The text of every `<windowRule ...>` element, whole, in file order.
fn rules(config: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = config;
    while let Some(start) = rest.find("<windowRule ") {
        let end = rest[start..].find("</windowRule>").expect("every rule is closed") + start + "</windowRule>".len();
        found.push(rest[start..end].to_string());
        rest = &rest[end..];
    }
    found
}

fn attribute(element: &str, name: &str) -> Option<String> {
    let at = element.find(&format!("{name}=\""))? + name.len() + 2;
    element[at..].split('"').next().map(str::to_string)
}

fn rule_for(title: &str) -> String {
    rules(&config())
        .into_iter()
        .find(|r| attribute(r, "title").as_deref() == Some(title))
        .unwrap_or_else(|| panic!("no window rule for `{title}`"))
}

#[test]
fn the_compositor_reserves_exactly_the_two_bars_and_nothing_else() {
    let config = config();
    let margin = config.lines().find(|l| l.trim_start().starts_with("<margin ")).expect("a margin");
    assert_eq!(attribute(margin, "top"), Some(TOP_HEIGHT.to_string()));
    assert_eq!(attribute(margin, "bottom"), Some(BOTTOM_HEIGHT.to_string()));
    assert_eq!(attribute(margin, "left"), Some("0".into()));
    assert_eq!(attribute(margin, "right"), Some("0".into()), "the apps take the whole width");
    assert_eq!(config.matches("<margin ").count(), 1);
}

#[test]
fn each_bar_is_always_on_top_undecorated_and_out_of_alt_tab() {
    for title in [TOP_TITLE, BOTTOM_TITLE] {
        let rule = rule_for(title);
        assert_eq!(attribute(&rule, "serverDecoration").as_deref(), Some("no"), "{title}: no title bar");
        assert_eq!(attribute(&rule, "skipWindowSwitcher").as_deref(), Some("yes"), "{title}: not in Alt+Tab");
        assert_eq!(attribute(&rule, "skipTaskbar").as_deref(), Some("yes"), "{title}");
        assert!(rule.contains(r#"<action name="ToggleAlwaysOnTop""#), "{title}: always on top");
    }
}

/// Every other window stays force-maximised, in what the margins leave; the bars are never
/// maximised by the rule that does it, because they undo it before anything else.
#[test]
fn the_bars_are_never_force_maximised_and_every_other_window_still_is() {
    let config = config();
    let all = rules(&config);
    let wildcard = all
        .iter()
        .position(|r| attribute(r, "identifier").as_deref() == Some("*"))
        .expect("the rule that maximises every window");
    assert!(all[wildcard].contains(r#"<action name="Maximize""#), "every other window is still maximised");
    for title in [TOP_TITLE, BOTTOM_TITLE] {
        let at = all.iter().position(|r| attribute(r, "title").as_deref() == Some(title)).expect("a rule");
        assert!(at > wildcard, "{title}: runs after the rule that maximises, so it can undo it");
        let rule = &all[at];
        let undo = rule.find(r#"<action name="UnMaximize""#).expect("it un-maximises first");
        assert!(rule.find(r#"<action name="Maximize""#).is_none(), "{title}: it never maximises both ways");
        // Wide, as the output is, and not tall: only horizontally.
        let wide = rule.find(r#"<action name="ToggleMaximize" direction="horizontal""#).expect("as wide as the output");
        assert!(undo < wide, "{title}: undone first, then widened");
    }
}

#[test]
fn each_bar_is_dropped_onto_the_edge_it_reserves() {
    // The top bar: onto the usable area's top, which is the margin's inside edge, then up by the
    // bar's own height, which is the screen's. The bottom: the same, downward.
    let top = rule_for(TOP_TITLE);
    assert!(top.contains(r#"<action name="MoveToEdge" direction="up""#), "{top}");
    assert!(top.contains(&format!(r#"<action name="MoveRelative" x="0" y="-{TOP_HEIGHT}""#)), "{top}");
    let bottom = rule_for(BOTTOM_TITLE);
    assert!(bottom.contains(r#"<action name="MoveToEdge" direction="down""#), "{bottom}");
    assert!(bottom.contains(&format!(r#"<action name="MoveRelative" x="0" y="{BOTTOM_HEIGHT}""#)), "{bottom}");
}

/// The two files name the bars the same: the rules' titles are the ones the window sets.
#[test]
fn the_titles_the_rules_match_are_the_ones_the_windows_set() {
    let ui = include_str!("../../ui/app.slint");
    assert!(ui.contains(&format!(r#"title: root.role == 0 ? "{TOP_TITLE}" : "{BOTTOM_TITLE}";"#)), "ui/app.slint");
    assert_eq!(TOP_TITLE, "Yantrik Mind Desk Top");
    assert_eq!(BOTTOM_TITLE, "Yantrik Mind Desk Bottom");
}

#[test]
fn the_nested_compositor_keeps_its_two_keys_and_its_theme() {
    let config = config();
    assert!(config.contains(r#"<keybind key="A-Tab">"#) && config.contains(r#"<keybind key="A-F4">"#));
    assert!(config.contains("<name>Yantrik</name>"));
}
