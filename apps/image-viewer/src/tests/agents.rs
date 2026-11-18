//! What an agent may have this window do (#443): open only pictures in the person's home, and be
//! shown nothing of a picture the person opened anywhere else — not its name, not the names
//! beside it, not one more of them by stepping. Through the real dispatch, as an agent's call.

use serde_json::json;
use yantrik_testing::Call;

use super::fixture::{open, showing, Folder};
use crate::agent_rule::{NOT_STEPPED, STILL_OPENING};
use crate::Command;

const AGENT: &str = "tok-viewer-test";

#[test]
fn an_agent_is_shown_nothing_of_a_picture_opened_outside_the_home() {
    // The person opened a picture in the temporary directory, outside the home.
    let folder = Folder::new("agent");
    let app = showing(&folder, "a.png");
    assert_eq!(app.describe()["state"]["file"], json!("a.png"), "the person sees it");

    // `show` as an agent: the answer and the view beside it name nothing.
    let shown = app.act_as(Call::new("show", json!({})).as_agent(AGENT)).unwrap();
    let envelope = shown.envelope().clone();
    assert_eq!(envelope["result"]["showing"], json!("(hidden)"), "{envelope}");
    assert!(envelope["result"].get("position").is_none(), "{envelope}");
    assert_eq!(envelope["summary"], json!("Images — showing a picture an agent is not shown"));
    assert!(envelope["state"]["hidden"].as_str().unwrap().ends_with(" is outside"), "{envelope}");
    assert!(envelope["state"].get("folder").is_none(), "{envelope}");

    // Stepping would name the folder's pictures one by one: refused, and nothing moves.
    let e = app.act_as(Call::new("next", json!({})).as_agent(AGENT)).unwrap_err();
    assert_eq!(e, NOT_STEPPED);
    let e = app.act_as(Call::new("previous", json!({})).as_agent(AGENT).in_mode("ask")).unwrap_err();
    assert_eq!(e, NOT_STEPPED);
    assert_eq!(app.model(|m| m.gallery.index()), 0);
    assert_eq!(app.jobs_waiting(), 0);

    // An agent's `open` outside the home is refused before the path is looked at at all: a
    // file that is not there is refused the same way as one that is.
    for path in ["/etc/hostname".to_string(), folder.text("gone.png")] {
        let pending = app.act_as(Call::new("open", json!({ "path": path })).as_agent(AGENT)).unwrap();
        app.run_jobs();
        let e = pending.settle().unwrap_err();
        assert!(e.ends_with(" is outside"), "{path}: {e}");
    }
    assert_eq!(app.describe()["state"]["file"], json!("a.png"), "the window is left as it was");

    // The view controls change only how the picture is drawn, and an agent may use them.
    let turned = app.act_as(Call::new("rotate", json!({ "direction": "left" })).as_agent(AGENT).in_mode("ask"));
    assert_eq!(turned.unwrap().settle().unwrap()["rotation"], json!(270));
}

#[test]
fn while_a_picture_loads_an_agent_is_told_to_wait_and_a_picture_in_the_home_is_shown() {
    let folder = Folder::new("agent-wait");
    let app = showing(&folder, "a.png");
    app.run(Command::Next);
    let e = app.act_as(Call::new("next", json!({})).as_agent(AGENT)).unwrap_err();
    assert_eq!(e, STILL_OPENING, "no verdict yet, so nothing is named");
    let shown = app.act_as(Call::new("show", json!({})).as_agent(AGENT)).unwrap();
    assert_eq!(shown.envelope()["state"]["hidden"], json!(STILL_OPENING));
    app.run_jobs();

    // A picture whose verdict says an agent may see it (the load reads it off the disk; here it
    // is given): the agent is told its name and place, and sees the folder.
    let path = folder.path("b.png");
    let mut loaded = crate::open::load(&path);
    loaded.shown_to_agents = Ok(());
    let load = app.model(|m| m.load);
    app.run(Command::Loaded { load, loaded });
    let shown = app.act_as(Call::new("show", json!({})).as_agent(AGENT).in_mode("ask")).unwrap();
    assert_eq!(shown.envelope()["result"], json!({ "showing": "b.png" }));
    assert_eq!(shown.envelope()["state"]["file"], json!("b.png"));
    assert_eq!(shown.envelope()["state"]["folder"], json!(["a.png", "b.png", "notes.png"]));

    // The person is never held to any of it.
    assert_eq!(open(&app, std::path::Path::new("/etc/hostname")).unwrap()["showing"], json!("hostname"));
}
