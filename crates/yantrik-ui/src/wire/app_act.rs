//! One press carried out by an app, and what the app said about it.
//!
//! `forward_to_app` starts the app and calls its action, and logs the answer: right for a
//! notification button whose card is gone. A row on a card that was pressed has to say when the
//! app could not do it ("Not opened: ..."), so this one brings the app's answer back to the
//! line the press came from.

use std::time::{Duration, Instant};

use yantrik_ipc_transport::SyncRpcClient;

use super::for_you::Say;
use super::notifications::button_route;
use crate::App;

/// How long a cold app has to open its socket.
const COLD_START: Duration = Duration::from_secs(5);
/// How long the app has to answer: a message is read from the mail server first.
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// What a press says on its line: nothing when the app did it, else why it did not.
pub fn said(reply: &Result<serde_json::Value, String>) -> String {
    match reply {
        Ok(v) if v["accepted"].as_bool().unwrap_or(true) => String::new(),
        Ok(_) => "Not opened: the app did not accept it.".to_string(),
        Err(why) => format!("Not opened: {}.", why.trim().trim_end_matches('.')),
    }
}

/// Ask `app` to do `action` with `args`, starting it if it is closed, and put its answer on the
/// line `key` names through `say`. Asked on a thread of its own, never the UI thread.
pub fn act(ui: &App, app: &str, action: &str, args: serde_json::Value, key: &str, say: Say) {
    let installed = crate::apps::Catalogue::shared().get();
    let Some((surface, opens_as)) = button_route(app, &installed) else {
        say(ui, key, "Not opened: this desktop does not open that app.");
        return;
    };
    let address = format!("app-{surface}");
    if !yantrik_app_runtime::service::is_up(&address) {
        match opens_as {
            Some(name) => ui.invoke_launch_app(name.into()),
            None => {
                say(ui, key, "Not opened: that app is closed and cannot be started.");
                return;
            }
        }
    }
    let (weak, key, action) = (ui.as_weak(), key.to_string(), action.to_string());
    let _ = std::thread::Builder::new().name("yos-app-act".into()).spawn(move || {
        let deadline = Instant::now() + COLD_START;
        while Instant::now() < deadline && !yantrik_app_runtime::service::is_up(&address) {
            std::thread::sleep(Duration::from_millis(150));
        }
        let reply = if yantrik_app_runtime::service::is_up(&address) {
            SyncRpcClient::clear_breaker(&yantrik_ipc_transport::RpcServer::default_address(&address));
            SyncRpcClient::for_service(&address)
                .with_timeout(ANSWER_WITHIN)
                .call("app.act", serde_json::json!({ "action": action, "args": args }))
                .map_err(|e| e.message)
        } else {
            Err("the app did not open".to_string())
        };
        let words = said(&reply);
        let _ = weak.upgrade_in_event_loop(move |ui| say(&ui, &key, &words));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_accepted_press_says_nothing_and_a_refused_one_says_why() {
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": true, "settled": true }))), "");
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": false }))), "Not opened: the app did not accept it.");
        assert_eq!(said(&Err("no message with that id.".into())), "Not opened: no message with that id.");
    }
}
