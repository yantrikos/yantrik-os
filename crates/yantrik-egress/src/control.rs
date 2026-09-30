//! The control socket: how the person's desktop reads and changes the policy.
//!
//! `/run/yantrik-egress/control`, a unix socket. The kernel says who connected (`SO_PEERCRED`),
//! and only root and the desktop's owner are answered — so a mind cannot widen its own rules, turn
//! off Private mode or read where it has been. One JSON request per line, one JSON answer per line:
//!
//! | `op` | with | does |
//! |---|---|---|
//! | `status` | — | the mode, Private mode, the rules |
//! | `seen` | — | every destination, most recent first |
//! | `proposals` | — | destinations refused with no rule for them |
//! | `mode` | `mode`: `audit` / `enforce` | switches the whole policy |
//! | `private` | `on` | the person's Private mode |
//! | `allow` | `rule` | adds or replaces a rule |
//! | `remove` | `host` | removes that host's rules |
//! | `forget` | `host`, `port` | drops a destination from the ledger (the person said No) |

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::policy::{Mode, Rule};
use crate::state::State;

/// The most a request line may be.
const MOST_LINE: usize = 16 * 1024;

/// Whether `uid` may use the control socket.
pub fn admitted(uid: u32, owner: Option<u32>) -> bool {
    uid == 0 || owner == Some(uid)
}

/// Answer one request.
pub fn handle(state: &Mutex<State>, request: &Value) -> Value {
    let Ok(mut s) = state.lock() else {
        return json!({ "ok": false, "error": "the proxy's state is poisoned" });
    };
    let op = request["op"].as_str().unwrap_or_default();
    let saved = |r: std::io::Result<()>| match r {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": format!("could not be saved: {e}") }),
    };
    match op {
        "status" => json!({
            "ok": true,
            "mode": s.policy.mode,
            "private": s.private,
            "rules": s.policy.rules,
        }),
        "seen" => json!({ "ok": true, "seen": s.ledger.list() }),
        "proposals" => {
            let p = s.ledger.proposals(&s.policy);
            json!({ "ok": true, "proposals": p })
        }
        "mode" => match serde_json::from_value::<Mode>(request["mode"].clone()) {
            Ok(m) => {
                s.policy.mode = m;
                saved(s.save_policy())
            }
            Err(_) => json!({ "ok": false, "error": "mode is `audit` or `enforce`" }),
        },
        "private" => match request["on"].as_bool() {
            Some(on) => saved(s.set_private(on)),
            None => json!({ "ok": false, "error": "`on` is true or false" }),
        },
        "allow" => match serde_json::from_value::<Rule>(request["rule"].clone()) {
            Ok(rule) => match s.policy.allow(rule) {
                Ok(()) => saved(s.save_policy()),
                Err(e) => json!({ "ok": false, "error": e }),
            },
            Err(e) => json!({ "ok": false, "error": format!("not a rule: {e}") }),
        },
        "remove" => {
            let host = request["host"].as_str().unwrap_or_default();
            let n = s.policy.remove(host);
            match s.save_policy() {
                Ok(()) => json!({ "ok": true, "removed": n }),
                Err(e) => json!({ "ok": false, "error": format!("could not be saved: {e}") }),
            }
        }
        "forget" => {
            let host = request["host"].as_str().unwrap_or_default();
            let port = request["port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(0);
            s.ledger.forget(host, port);
            json!({ "ok": true })
        }
        _ => json!({ "ok": false, "error": format!("`{op}` is not an op here: status, seen, proposals, mode, private, allow, remove, forget") }),
    }
}

/// Serve the control socket until the process ends.
#[cfg(unix)]
pub async fn serve(listener: tokio::net::UnixListener, state: Arc<Mutex<State>>, owner: Option<u32>) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let uid = stream.peer_cred().ok().map(|c| c.uid());
        let state = state.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            if !uid.is_some_and(|u| admitted(u, owner)) {
                let _ = write.write_all(b"{\"ok\":false,\"error\":\"this socket answers the desktop's owner and root only\"}\n").await;
                return;
            }
            let mut lines = BufReader::new(read.take(MOST_LINE as u64 * 64)).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let answer = if line.len() > MOST_LINE {
                    json!({ "ok": false, "error": "the request is too long" })
                } else {
                    match serde_json::from_str::<Value>(&line) {
                        Ok(req) => handle(&state, &req),
                        Err(_) => json!({ "ok": false, "error": "not JSON" }),
                    }
                };
                let mut out = answer.to_string();
                out.push('\n');
                if write.write_all(out.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(name: &str) -> Mutex<State> {
        let d = std::env::temp_dir().join(format!("yantrik-egress-control-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Mutex::new(State::load(&d))
    }

    #[test]
    fn only_root_and_the_desktops_owner_are_answered() {
        assert!(admitted(0, None));
        assert!(admitted(1000, Some(1000)));
        assert!(!admitted(990, Some(1000)), "the mind account");
        assert!(!admitted(1000, None), "no owner known: root only");
    }

    #[test]
    fn the_person_switches_the_mode_adds_rules_and_turns_private_mode() {
        let s = state("ops");
        assert_eq!(handle(&s, &json!({"op":"status"}))["mode"], "audit");
        assert_eq!(handle(&s, &json!({"op":"mode","mode":"enforce"}))["ok"], true);
        assert_eq!(handle(&s, &json!({"op":"mode","mode":"wide-open"}))["ok"], false);
        let r = handle(&s, &json!({"op":"allow","rule":{"host":"api.x.ai","ports":[443],"why":"the model"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(handle(&s, &json!({"op":"allow","rule":{"host":"*","ports":[443],"why":"all"}}))["ok"], false);
        assert_eq!(handle(&s, &json!({"op":"allow","rule":{"host":"a.com","ports":[443],"why":"x","extra":1}}))["ok"], false, "no unknown keys");
        let st = handle(&s, &json!({"op":"status"}));
        assert_eq!(st["mode"], "enforce");
        assert_eq!(st["rules"].as_array().unwrap().len(), 1);
        assert_eq!(handle(&s, &json!({"op":"private","on":true}))["ok"], true);
        assert_eq!(handle(&s, &json!({"op":"status"}))["private"], true);
        assert_eq!(handle(&s, &json!({"op":"remove","host":"api.x.ai"}))["removed"], 1);
        assert_eq!(handle(&s, &json!({"op":"nonsense"}))["ok"], false);
    }
}
