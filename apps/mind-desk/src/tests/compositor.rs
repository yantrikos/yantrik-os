//! The nested compositor's side of the desk: `config/labwc-mind/rc.xml` reserves the margin the bar
//! sits in and places it, and the numbers and the title it uses are this app's.
//!
//! These read the file. Where labwc really puts the bar is not something a file can say, and is
//! measured on pixels by `tests/mind-desk-placement/run.sh` against the labwc the OS runs.

use crate::metrics::{BAR_HEIGHT, MARGIN_TOP, OVERLAP, TITLE};

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

fn the_rule() -> String {
    let all = rules(&config());
    let mine: Vec<&String> = all.iter().filter(|r| attribute(r, "title").as_deref() == Some(TITLE)).collect();
    assert_eq!(mine.len(), 1, "one rule for the bar");
    mine[0].clone()
}

/// The actions of a rule, in order, as `Name` or `Name attr=value ...`.
fn actions(rule: &str) -> Vec<String> {
    rule.split("<action ")
        .skip(1)
        .map(|a| a.split("/>").next().unwrap().split_whitespace().collect::<Vec<_>>().join(" "))
        .collect()
}

#[test]
fn the_compositor_reserves_exactly_the_bar_and_nothing_else() {
    let config = config();
    let margin = config.lines().find(|l| l.trim_start().starts_with("<margin ")).expect("a margin");
    assert_eq!(attribute(margin, "top"), Some(MARGIN_TOP.to_string()));
    assert_eq!(attribute(margin, "bottom"), Some("0".into()), "nothing below: the apps take the rest");
    assert_eq!(attribute(margin, "left"), Some("0".into()));
    assert_eq!(attribute(margin, "right"), Some("0".into()), "the apps take the whole width");
    assert_eq!(config.matches("<margin ").count(), 1);
    // The bar is the margin and a pixel over the apps (see `metrics`).
    assert_eq!(BAR_HEIGHT, MARGIN_TOP + OVERLAP);
    assert_eq!(OVERLAP, 1);
}

#[test]
fn the_bar_is_always_on_top_undecorated_out_of_alt_tab_and_never_moved() {
    let rule = the_rule();
    assert_eq!(attribute(&rule, "serverDecoration").as_deref(), Some("no"), "no title bar");
    assert_eq!(attribute(&rule, "skipWindowSwitcher").as_deref(), Some("yes"), "not in Alt+Tab");
    assert_eq!(attribute(&rule, "skipTaskbar").as_deref(), Some("yes"));
    assert_eq!(attribute(&rule, "fixedPosition").as_deref(), Some("yes"), "nothing moves it afterwards");
    assert!(rule.contains(r#"<action name="ToggleAlwaysOnTop""#), "always on top");
}

/// Every other window stays force-maximised, in what the margin leaves; the bar is never
/// force-maximised by the rule that does it, because it undoes it before anything else.
#[test]
fn the_bar_is_never_force_maximised_and_every_other_window_still_is() {
    let config = config();
    let all = rules(&config);
    let wildcard = all
        .iter()
        .position(|r| attribute(r, "identifier").as_deref() == Some("*"))
        .expect("the rule that maximises every window");
    assert!(all[wildcard].contains(r#"<action name="Maximize""#), "every other window is still maximised");
    let at = all.iter().position(|r| attribute(r, "title").as_deref() == Some(TITLE)).expect("a rule");
    assert!(at > wildcard, "it runs after the rule that maximises, so it can undo it");
    assert_eq!(actions(&all[at])[0], r#"name="UnMaximize""#, "undone first");
}

/// What labwc 0.8.3 was seen to do (tests/mind-desk-placement), as the actions that make the bar
/// land at the top and as wide as the display: placed at 0,0 while floating, made as wide as the
/// display, put at 0,0 again. `MoveToEdge` and `MoveRelative` are not in it: the first does nothing
/// when a rule runs, and the second moved the bar from where labwc centred it.
#[test]
fn the_bar_is_placed_by_the_actions_that_were_measured_to_place_it() {
    let rule = the_rule();
    let got = actions(&rule);
    let want = [
        r#"name="UnMaximize""#,
        r#"name="MoveTo" x="0" y="0""#,
        r#"name="ToggleMaximize" direction="horizontal""#,
        r#"name="MoveTo" x="0" y="0""#,
        r#"name="ToggleAlwaysOnTop""#,
    ];
    assert_eq!(got, want);
    for banned in ["MoveToEdge", "MoveRelative", "AutoPlace"] {
        assert!(!rule.contains(banned), "`{banned}` did not place the bar");
    }
}

/// The two files name the bar the same: the rule's title is the one the window sets.
#[test]
fn the_title_the_rule_matches_is_the_one_the_window_sets() {
    let ui = include_str!("../../ui/app.slint");
    assert!(ui.contains(&format!(r#"title: "{TITLE}";"#)), "ui/app.slint");
    assert_eq!(TITLE, "Yantrik Mind Desk");
    // And the rule is not the title of a second window that no longer exists.
    assert!(!RC.contains("Yantrik Mind Desk Top") && !RC.contains("Yantrik Mind Desk Bottom"));
}

#[test]
fn the_nested_compositor_keeps_its_two_keys_and_its_theme() {
    let config = config();
    assert!(config.contains(r#"<keybind key="A-Tab">"#) && config.contains(r#"<keybind key="A-F4">"#));
    assert!(config.contains("<name>Yantrik</name>"));
}

/// The placement check exists, runs the bar's own rc.xml, and says what it wants: one bar at the
/// top, as wide as the display, at the sizes Mind View has. It is what stops this regressing.
#[test]
fn the_placement_check_is_in_the_tree_and_asks_for_the_bar_where_it_belongs() {
    let run = include_str!("../../../../tests/mind-desk-placement/run.sh");
    assert!(run.contains("config/labwc-mind/rc.xml"), "it reads the real rc.xml");
    for size in ["896x560", "1280x690"] {
        assert!(run.contains(size), "{size}");
    }
    assert!(run.contains("after-restart"), "and after the desk is started again");
    let measure = include_str!("../../../../tests/mind-desk-placement/measure.py");
    assert!(measure.contains("rows 0.."), "the bar is measured from row 0");
}
