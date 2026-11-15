//! One press carried out by an app, and what the app said about it.
//!
//! `forward_to_app` starts the app and calls its action, and logs the answer: right for a
//! notification button whose card is gone. A row on a card that was pressed has to say when the
//! app could not do it ("Not opened: ..."), so this one brings the app's answer back to the
//! line the press came from.
//!
//! Who may press is the compiler's to hold: see [`press_message`].

use std::time::{Duration, Instant};

use slint::ComponentHandle;

use yantrik_ipc_transport::SyncRpcClient;

use super::for_you::Say;
use super::meeting_brief::BriefMessage;
use super::notifications::button_route;
use crate::App;

/// How long a cold app has to open its socket.
const COLD_START: Duration = Duration::from_secs(5);
/// How long the app has to answer: a message is read from the mail server first.
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// The method every ordinary press goes through: `app.act`, which an app refuses for an action it
/// declares `shell_only`, whoever asks.
const ACT: &str = "app.act";
/// The method for the one press only the shell makes (`app.press`, which an app runs for the shell
/// alone). It is spelled out here and nowhere else in the shell: only [`press_message`] sends it,
/// and that takes a `BriefMessage`, which only the meeting brief can make. So the companion's
/// `app_action`, a recipe, a notification button's relay and every other path that sends `app.act`
/// for someone else cannot reach a press (the scans in `press_scan` back the compiler).
const PRESS: &str = "app.press";

/// The key of an app's `describe` state that says its first read has landed, for an app that
/// answers a call before it (Notes reads its library after its socket is up, and refuses a call
/// while it does). Email handles it inside the app; an app not listed is asked at once.
fn ready_key(app: &str) -> Option<&'static str> {
    (app == "notes").then_some("library_ready")
}

/// Whether `describe` (an app's answer) says its first read has landed, by `key`.
fn is_ready(describe: &serde_json::Value, key: &str) -> bool {
    describe["state"][key].as_bool().unwrap_or(false)
}

/// What a press says on its line: nothing when the app did it, else why it did not.
pub fn said(reply: &Result<serde_json::Value, String>) -> String {
    match reply {
        Ok(v) if v["accepted"].as_bool().unwrap_or(true) => String::new(),
        Ok(_) => "Not opened: the app did not accept it.".to_string(),
        Err(why) => format!("Not opened: {}.", why.trim().trim_end_matches('.')),
    }
}

/// Ask `app` to do `action` with `args`, starting it if it is closed, and put its answer on the
/// line `key` names through `say`. Asked on a thread of its own, never the UI thread. Always
/// `app.act`: no caller can ask for a press through here.
pub fn act(ui: &App, app: &str, action: &str, args: serde_json::Value, key: &str, say: Say) {
    send(ui, app, ACT, action, args, key, say);
}

/// Press "open this message" in Email, as the shell, for a row of the meeting brief: the one
/// `app.press`. Email opens exactly the message `message` names and answers on the row's line.
pub fn press_message(ui: &App, message: &BriefMessage, key: &str, say: Say) {
    let args = serde_json::json!({ "account": message.account(), "folder": message.folder(), "message_id": message.id() });
    send(ui, "email", PRESS, "open_message_id", args, key, say);
}

fn send(ui: &App, app: &str, method: &'static str, action: &str, args: serde_json::Value, key: &str, say: Say) {
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
            let client = SyncRpcClient::for_service(&address).with_timeout(ANSWER_WITHIN);
            // A cold app is up before it has read what the press names; ask once it has, within
            // the same start-up allowance.
            if let Some(key) = ready_key(&surface) {
                while Instant::now() < deadline
                    && !client.call("app.describe", serde_json::json!({})).is_ok_and(|d| is_ready(&d, key))
                {
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
            client.call(method, serde_json::json!({ "action": action, "args": args })).map_err(|e| e.message)
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

    /// The compiler holds who can press; this holds what the press is: `act` sends `app.act`, the
    /// message press sends `app.press`, and each method is spelled out once.
    #[test]
    fn act_is_never_a_press_and_the_message_press_is_the_only_one() {
        assert_eq!((ACT, PRESS), ("app.act", "app.press"));
        let code = include_str!("app_act.rs").split("#[cfg(test)]").next().unwrap();
        let sends = |m: &str| code.lines().filter(|l| l.contains(&format!("\"{m}\""))).count();
        assert_eq!((sends("app.press"), sends("app.act")), (1, 1), "each method has one place");
        assert!(code.contains("send(ui, app, ACT,"), "act sends app.act");
        assert_eq!(code.matches("PRESS,").count(), 1, "one caller sends the press");
    }

    #[test]
    fn a_cold_notes_is_waited_for_until_its_library_has_loaded() {
        assert_eq!(ready_key("notes"), Some("library_ready"));
        assert_eq!(ready_key("email"), None, "Email waits inside the app");
        assert!(!is_ready(&serde_json::json!({ "state": { "library_ready": false } }), "library_ready"));
        assert!(!is_ready(&serde_json::json!({ "state": {} }), "library_ready"), "no word is not ready");
        assert!(is_ready(&serde_json::json!({ "state": { "library_ready": true } }), "library_ready"));
    }

    #[test]
    fn an_accepted_press_says_nothing_and_a_refused_one_says_why() {
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": true, "settled": true }))), "");
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": false }))), "Not opened: the app did not accept it.");
        assert_eq!(said(&Err("no message with that id.".into())), "Not opened: no message with that id.");
    }
}
