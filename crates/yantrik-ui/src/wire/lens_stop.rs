//! The Lens's Stop (#1058): the running turn of the mind answering, dropped on the person's word.
//!
//! The harness does not poll for new turns while one runs. It sends an empty chunk every
//! heartbeat (30 s) and ends the turn when the desktop answers one "dropped". So Stop only has to
//! mark the turn dropped (`launch::stop_turn`, the host's one drop path, which the Mind desk's stop
//! also ends in), and the harness stops by itself within a heartbeat. Until it does, the Lens says
//! "Stopping…".
//!
//! Two doors, one act: the Stop button, and a bare "stop" typed while a turn is in flight
//! (`lens_queue::is_bare_stop`). A typed stop is consumed here. It used to be held as a new turn
//! and delivered after the turn it was meant to end, and the mind answered it as a greeting.

use crate::agents::launch;
use crate::{streaming, App};

/// What the desktop says under a typed stop.
const TYPED_LINE: &str = "Stopping. It can take up to half a minute for the mind to notice.";

/// Drop the running turn of the mind answering in the Lens. `typed` is the person's own word when
/// they wrote it instead of pressing the button: it is shown in the conversation, once, and never
/// sent. The composer is republished at once so "Stopping…" is there before the next tick.
pub fn press(ui_weak: &slint::Weak<App>, typed: Option<&str>) {
    if let Some(host) = super::harness::host() {
        let agent = crate::agents::feed::main_agent(&host.active_id());
        launch::stop_turn(host, &agent);
    }
    if let Some(text) = typed {
        streaming::say(ui_weak, Some(text), "desktop", TYPED_LINE);
    }
    if let Some(ui) = ui_weak.upgrade() {
        super::agents::publish_lens_composer(&ui);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yantrik_harness::{protocol, Chunk, Host, Turn};

    use crate::agents::{launch, model::AgentId};

    /// The Lens's Stop button: the turn is dropped, the harness's next heartbeat is answered
    /// "dropped", the agent stays, and the host says "stopping" until the harness closes the turn.
    #[test]
    fn the_stop_button_drops_the_turn_and_the_next_heartbeat_is_told() {
        let host = Host::new(vec![]);
        let attach = json!({ "id": "stop-probe", "name": "Probe", "conversations": false });
        let session = host.handle_from(protocol::ATTACH, &attach, Some(std::process::id()), None).unwrap()["session"]
            .as_str()
            .unwrap()
            .to_string();
        let agent = AgentId::new("stop-probe", AgentId::MAIN);
        let answer = host.send_to(&agent, Turn::new("a long task")).unwrap();
        let turn = host.handle_from(protocol::POLL, &json!({ "session": session }), Some(std::process::id()), None).unwrap();
        let turn_id = turn["turn_id"].as_u64().expect("handed the turn");
        let beat = || {
            host.handle_from(protocol::CHUNK, &json!({ "session": session, "turn_id": turn_id, "delta": "" }), Some(std::process::id()), None)
                .unwrap()
        };
        assert_eq!(beat(), json!({}));
        assert!(!host.stopping(&agent));

        launch::stop_turn(&host, &agent);

        assert_eq!(beat(), json!({ "dropped": true }));
        assert!(host.stopping(&agent), "Stopping… until the harness closes it");
        assert!(matches!(answer.iter().last(), Some(Chunk::Failed(_))), "the reader is told it was stopped");
        assert!(host.send_to(&agent, Turn::new("next")).is_ok(), "the agent stays");
    }
}
