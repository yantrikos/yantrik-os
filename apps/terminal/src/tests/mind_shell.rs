//! A Mind's input reaches only a shell in the outside view (#1124, H1).

use serde_json::json;

use super::open;
use crate::command::Command;

#[test]
fn a_mind_never_types_into_the_persons_shell_and_its_own_tab_is_an_outside_one() {
    let f = open();
    assert!(!f.app.model(|m| m.tab().unwrap().outside), "the person's first shell is not wrapped");
    for (name, args) in [("run", json!({ "command": "echo INTO_PERSONS_SHELL" })), ("send_input", json!({ "text": "echo INTO_PERSONS_SHELL\n" }))] {
        assert_eq!(f.agent(name, args).unwrap_err(), crate::update::NOT_YOURS, "{name}");
    }
    f.quiet();
    assert!(!f.screen().contains("INTO_PERSONS_SHELL"), "nothing reached the person's shell");
    // Its own tab is one it opened, and that one takes input.
    f.agent("new_tab", json!({})).unwrap();
    assert!(f.app.model(|m| m.tab().unwrap().outside));
    assert!(f.agent("run", json!({ "command": "echo IN_ITS_OWN" })).unwrap()["output"].as_str().unwrap().contains("IN_ITS_OWN"));
    // Back on the person's tab by the person's click, the Mind is refused again.
    f.app.run(Command::SelectTab(0));
    f.quiet();
    assert!(f.agent("run", json!({ "command": "echo AGAIN" })).is_err());
    // The person's own call (yos, no agent token) is the person's act, as it was.
    assert!(f.act("run", json!({ "command": "echo PERSON_OK" })).unwrap()["output"].as_str().unwrap().contains("PERSON_OK"));
}

#[test]
fn no_shell_is_opened_in_a_workshop_for_anyone() {
    let f = open();
    let home = std::env::var("HOME").unwrap_or_default();
    let shop = format!("{home}/.local/share/yantrik/workshops/hermes/shop");
    let before = f.tabs();
    for who in [true, false] {
        let args = json!({ "directory": shop });
        let why = if who { f.agent("open_directory", args) } else { f.act("open_directory", args) }.unwrap_err();
        assert!(why.contains("workshop"), "{why}");
    }
    assert_eq!(f.tabs(), before);
}
