//! Asking the person before an app acts, when the app says to (see the note below), and the desk the
//! asking goes through. Split from `app_ui.rs`, which re-exports it.

use std::time::{Duration, Instant};

use yantrik_ipc_transport::gate;
use yantrik_ipc_transport::SyncRpcClient;

// ── Asking the person, when the app says to ──
//
// Since the dispatch enforces the person's mode on every door (#116, #152), an app answers a
// `sensitive` act in `ask` mode with `GRANT:` unless the call carries a grant. Every other mind
// has a way through that — the MCP bridge and `yos act` put the card up, wait for the person, and
// act again with their Allow — and the built-in companion had none: its `app_action` simply
// failed where any other mind would have asked (#154, item 3). So it takes the same three steps.
// Nothing here decides anything: `request_approval` puts a card up, `approval_status` reads an
// answer somebody else gave, and the app's own dispatch spends the grant when the action runs.

/// What the card says is asking.
const REQUESTER: &str = "companion";

/// How long a card waits for a person, as `yos act` waits. The tool runs on the bridge's worker,
/// never on the thread that paints, so a wait costs this tool call and nothing else.
const APPROVAL_WAIT: Duration = Duration::from_secs(120);
const APPROVAL_POLL: Duration = Duration::from_secs(1);

/// A request to the shell is one hop to its UI thread and back.
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// Where the desktop's sockets are and how long to wait for a person. The session's own in use;
/// a test hands in a scratch directory and a shorter wait. `pub(crate)` because `window.rs`
/// drives the shell's own window actions through the same desk and the same asking.
#[derive(Clone)]
pub(crate) struct Desk {
    pub(crate) dir: Option<std::path::PathBuf>,
    pub(crate) wait: Duration,
    pub(crate) poll: Duration,
}

impl Desk {
    pub(crate) fn session() -> Desk {
        Desk { dir: None, wait: APPROVAL_WAIT, poll: APPROVAL_POLL }
    }

    /// One call to the shell, which the approvals module lives in.
    pub(crate) fn shell_call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, yantrik_ipc_transport::protocol::RpcError> {
        self.client("app-shell", SHELL_TIMEOUT).call(method, params)
    }

    pub(crate) fn client(&self, service: &str, timeout: Duration) -> SyncRpcClient {
        let address = match &self.dir {
            Some(dir) => dir.join(format!("{service}.sock")).to_string_lossy().to_string(),
            None => yantrik_ipc_transport::server::RpcServer::default_address(service),
        };
        SyncRpcClient::new(&address).with_timeout(timeout)
    }
}

/// Put the card for `app.action` in front of the person: the shell's `request_approval`. The
/// answer is the shell's own (`request_id` and `pending`, or `not_needed` where the mode runs the
/// action unasked); an `Err` is a sentence for the model, and nothing was raised.
pub(crate) fn raise_card(
    desk: &Desk,
    app: &str,
    action: &str,
    grade: &str,
    args: &serde_json::Value,
    purpose: &str,
) -> Result<serde_json::Value, String> {
    desk.client("app-shell", SHELL_TIMEOUT)
        .call(
            "app.act",
            serde_json::json!({ "action": "request_approval", "args": {
                "app": app, "action": action, "grade": grade, "args_json": args,
                "purpose": purpose, "requester": REQUESTER,
            }}),
        )
        .map_err(|e| {
            format!("{app}.{action} needs the person's Allow and they could not be asked: {}", e.message)
        })
}

/// Wait up to the desk's patience for the person's answer to card `request_id`: `granted`,
/// `denied`, `consumed`, or `pending` when nobody answered in time. A poll that fails is not an
/// answer; the card is still up, so it asks again.
pub(crate) fn await_answer(desk: &Desk, request_id: &str) -> String {
    let deadline = Instant::now() + desk.wait;
    let mut status = "pending".to_string();
    while Instant::now() < deadline {
        std::thread::sleep(desk.poll.min(deadline.saturating_duration_since(Instant::now())));
        let Ok(poll) = desk.client("app-shell", SHELL_TIMEOUT).call(
            "app.act",
            serde_json::json!({ "action": "approval_status", "args": { "request_id": request_id } }),
        ) else {
            continue;
        };
        status = poll["result"]["status"].as_str().unwrap_or("pending").to_string();
        if status != "pending" {
            break;
        }
    }
    status
}

/// `app.act`, and when the app refuses for want of a grant, the person's Allow and `app.act` again.
///
/// `Err` is always a sentence for the model to read and relay: the app's own refusal passed
/// through, or what became of the card — denied, unanswered, not needed after all.
pub(crate) fn act_asking(
    desk: &Desk,
    app: &str,
    action: &str,
    args: &serde_json::Value,
    expect: &str,
    purpose: &str,
) -> Result<serde_json::Value, String> {
    let mut call = serde_json::json!({ "action": action, "args": args });
    if !expect.is_empty() {
        call["expect_revision"] = serde_json::Value::String(expect.to_string());
    }
    let refusal = match desk.client(&format!("app-{app}"), crate::app_ui::ACT_TIMEOUT).call("app.act", call.clone()) {
        Ok(reply) => return Ok(reply),
        Err(e) if !e.message.starts_with("GRANT:") => return Err(format!("{app}.{action} failed: {}", e.message)),
        Err(e) => e.message,
    };

    // Every GRANT sentence names the grade, and the shell re-reads the published one anyway.
    let grade = refusal
        .split("graded `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .filter(|g| gate::grade(g).is_some())
        .unwrap_or("sensitive");
    let asked = raise_card(desk, app, action, grade, args, purpose)?;
    let answer = asked.get("result").cloned().unwrap_or_default();
    if answer["status"] == "not_needed" {
        return Err(format!(
            "{app} refused {app}.{action} without a grant, but the desktop says its {} mode runs it \
             unasked. Nothing was run; the two should agree, so try once more in a moment. The app \
             said: {refusal}",
            answer["mode"].as_str().unwrap_or("current")
        ));
    }
    let Some(request_id) = answer["request_id"].as_str().map(str::to_string) else {
        return Err(format!(
            "the desktop was asked to put a card up for {app}.{action} and did not say which \
             request it was, so there is nothing to wait on. Nothing was run."
        ));
    };

    let status = await_answer(desk, &request_id);

    match status.as_str() {
        "granted" => {
            call["grant"] = serde_json::Value::String(request_id);
            desk.client(&format!("app-{app}"), crate::app_ui::ACT_TIMEOUT).call("app.act", call).map_err(|e| {
                format!("The person allowed {app}.{action}, but it did not go through: {}", e.message)
            })
        }
        "denied" => Err(format!(
            "The person at the machine said no to {app}.{action}. Nothing was run. That is an \
             answer, not an error: do not ask again unless they bring it up."
        )),
        "consumed" => Err(format!(
            "The grant for {app}.{action} had already been spent, so nothing was run. Ask again if \
             it still needs doing."
        )),
        _ => Err(format!(
            "Nobody answered the card for {app}.{action} within {} s, so nothing was run. They were \
             probably away from the keyboard; say what you wanted to do and try again when they \
             are back.",
            desk.wait.as_secs()
        )),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    // ── Asking the person (#154, item 3) ──

    /// One fake socket: answers each JSON-RPC line with whatever `reply` makes of the request,
    /// and records every request. `reply` returns the whole response object minus `jsonrpc`/`id`.
    /// `pub(crate)` because `window.rs` tests the same asking through the same kind of fake.
    #[cfg(unix)]
    pub(crate) fn fake(
        path: std::path::PathBuf,
        reply: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
    ) -> std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>> {
        use std::io::{BufRead, BufReader, Write};
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = heard.clone();
        let reply = std::sync::Arc::new(reply);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind a fake socket");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let (log, reply) = (log.clone(), reply.clone());
                std::thread::spawn(move || {
                    let mut writer = stream.try_clone().unwrap();
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { return };
                        let asked: serde_json::Value = serde_json::from_str(&line).unwrap();
                        log.lock().unwrap().push(asked["params"].clone());
                        let mut answer = reply(&asked["params"]);
                        answer["jsonrpc"] = "2.0".into();
                        answer["id"] = asked["id"].clone();
                        if writer.write_all(format!("{answer}\n").as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        heard
    }

    const GRANT_REFUSAL: &str = "GRANT: notes.purge is graded `sensitive` and this machine is in \
        ask mode, which runs nothing above `standard` without asking — so it was not run.";

    /// A desk in a scratch directory: an app that refuses without a grant and runs with
    /// `appr-1`, and a shell whose card is answered `answer` on the second poll.
    #[cfg(unix)]
    fn desk_with(
        tag: &str,
        answer: &'static str,
    ) -> (Desk, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>, std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>) {
        let dir = std::env::temp_dir().join(format!("yantrik-companion-ask-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let app = fake(dir.join("app-notes.sock"), |params| {
            if params["action"] != "purge" {
                return serde_json::json!({"error": {"code": -32602, "message": "`nope` needs argument `id`"}});
            }
            match params["grant"].as_str() {
                None => serde_json::json!({"error": {"code": -32602, "message": GRANT_REFUSAL}}),
                Some("appr-1") => serde_json::json!({"result": {
                    "app": "notes", "action_id": "app-notes#2", "accepted": true, "settled": true,
                    "result": {"purged": 3}, "revision": "0123456789abcdef", "summary": "Notes — empty", "state": {}
                }}),
                Some(other) => serde_json::json!({"error": {"code": -32602, "message": format!("GRANT: `{other}` does not authorise notes.purge")}}),
            }
        });
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let shell = fake(dir.join("app-shell.sock"), move |params| match params["action"].as_str() {
            Some("request_approval") => serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": "pending"}}}),
            Some("approval_status") => {
                let n = polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let status = if n == 0 { "pending" } else { answer };
                serde_json::json!({"result": {"result": {"request_id": "appr-1", "status": status}}})
            }
            _ => serde_json::json!({"error": {"code": -32602, "message": "unknown action"}}),
        });
        let desk = Desk { dir: Some(dir), wait: Duration::from_millis(600), poll: Duration::from_millis(20) };
        (desk, app, shell)
    }

    #[cfg(unix)]
    #[test]
    fn a_grant_refusal_asks_the_person_and_acts_again_with_their_allow() {
        let (desk, app, shell) = desk_with("granted", "granted");
        let args = serde_json::json!({"folder": "Old"});
        let reply = act_asking(&desk, "notes", "purge", &args, "", "Purge a folder").expect("allowed, then ran");
        assert_eq!(reply["result"]["purged"], 3);

        let asked = shell.lock().unwrap().clone();
        assert_eq!(asked[0]["action"], "request_approval");
        assert_eq!(
            asked[0]["args"],
            serde_json::json!({"app": "notes", "action": "purge", "grade": "sensitive",
                               "args_json": {"folder": "Old"}, "purpose": "Purge a folder",
                               "requester": "companion"}),
            "the card is bound to the exact arguments the act carries"
        );
        assert!(asked[1..].iter().all(|p| p["action"] == "approval_status"));

        let acts = app.lock().unwrap().clone();
        assert_eq!(acts.len(), 2, "once refused, once with the grant: {acts:?}");
        assert!(acts[0].get("grant").is_none());
        assert_eq!(acts[1]["grant"], "appr-1");
        assert_eq!(acts[0]["args"], acts[1]["args"], "the same arguments both times");
    }

    #[cfg(unix)]
    #[test]
    fn a_denied_or_unanswered_card_runs_nothing_and_says_so() {
        let (desk, app, _) = desk_with("denied", "denied");
        let err = act_asking(&desk, "notes", "purge", &serde_json::json!({}), "", "").unwrap_err();
        assert!(err.contains("said no") && err.contains("Nothing was run"), "{err}");
        assert_eq!(app.lock().unwrap().len(), 1, "only the refused call reached the app");

        let (desk, app, _) = desk_with("silent", "pending");
        let err = act_asking(&desk, "notes", "purge", &serde_json::json!({}), "", "").unwrap_err();
        assert!(err.contains("Nobody answered"), "{err}");
        assert_eq!(app.lock().unwrap().len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn any_other_refusal_passes_through_and_nobody_is_asked() {
        let (desk, _, shell) = desk_with("other", "granted");
        let err = act_asking(&desk, "notes", "nope", &serde_json::json!({}), "", "").unwrap_err();
        assert_eq!(err, "notes.nope failed: `nope` needs argument `id`");
        assert!(shell.lock().unwrap().is_empty(), "no card for a refusal a card cannot answer");
    }

}
