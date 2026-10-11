//! The control socket: how the person's desktop reads and changes the policy.
//!
//! `/run/yantrik-egress/control`, a unix socket. The kernel says who connected (`SO_PEERCRED`),
//! and only root and the desktop's owner are answered — so a mind cannot widen its own rules, turn
//! off Private mode or read where it has been. One JSON request per line, one JSON answer per line:
//!
//! | `op` | with | does |
//! |---|---|---|
//! | `status` | — | the mode, Private mode, the rules, `kernel_current`, `wide_links`, `assumed_links`, and `modes`: each mode's word and the plain words the desktop shows for it, the least strict first, one marked recommended |
//! | `seen` | — | every destination, most recent first |
//! | `proposals` | — | destinations refused with no rule for them |
//! | `mode` | `mode`: `audit` / `guarded` / `enforce` | switches the whole policy; answers `kernel_current` |
//! | `private` | `on` | the person's Private mode |
//! | `allow` | `rule` | adds or replaces a rule |
//! | `seed` | `rules` | root only: replaces the seeded rules (`crate::seed`), never the person's |
//! | `remove` | `host` | removes that host's rules |
//! | `forget` | `host`, `port` | drops a destination from the ledger (the person said No) |
//!
//! `wide_links` lists the prefixes this machine's interfaces are on that are wider than /16 (IPv4)
//! or /32 (IPv6): all of each is the local network, so a rule for a host in one needs `lan: true`
//! (`null` when the network cannot be read). `assumed_links` lists the /56 around each global
//! IPv6 address of this machine on a shared network: the local network by a guess (the usual
//! delegated prefix), so a host in one is refused without a rule that says `lan: true`, and its
//! refusal says so (`null` likewise).
//!
//! `kernel_current` is whether the kernel's table has caught up with the mode and Private mode
//! (`State::kernel_current`): the proxy follows a switch at once, the kernel a moment later, when
//! `yantrik-mind-egress.path` has run `apply`. Until then the mind's DNS and the status file are
//! still the old mode's. `null`: no status file to read. A desktop can ask `status` again until it
//! is `true`.

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

/// Answer one request. `root`: the caller is root, the only one that may seed.
pub fn handle(state: &Mutex<State>, request: &Value, root: bool) -> Value {
    let Ok(mut s) = state.lock() else {
        return json!({ "ok": false, "error": "the proxy's state is poisoned" });
    };
    let op = request["op"].as_str().unwrap_or_default();
    let saved = |r: std::io::Result<()>| match r {
        Ok(()) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": format!("could not be saved: {e}") }),
    };
    match op {
        "status" => {
            let net = crate::local::addresses();
            json!({
            "ok": true,
            "mode": s.policy.mode,
            "private": s.private,
            "rules": s.policy.rules,
            "kernel_current": s.kernel_current(),
            "wide_links": net.as_ref().map(|n| n.wide()),
            "assumed_links": net.as_ref().map(|n| n.assumed_links()),
            "modes": Mode::ALL.map(|m| json!({ "mode": m, "label": m.label(), "recommended": m.recommended() })),
            })
        }
        "seen" => json!({ "ok": true, "seen": s.ledger.list() }),
        "proposals" => {
            let p = s.ledger.proposals(&s.policy);
            json!({ "ok": true, "proposals": p })
        }
        "mode" => match serde_json::from_value::<Mode>(request["mode"].clone()) {
            Ok(m) => {
                s.policy.mode = m;
                let mut answer = saved(s.save_policy());
                answer["kernel_current"] = json!(s.kernel_current());
                answer
            }
            Err(_) => json!({ "ok": false, "error": "mode is `audit`, `guarded` or `enforce`" }),
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
        "seed" if !root => json!({ "ok": false, "error": "only root seeds rules (yantrik-update)" }),
        "seed" => match serde_json::from_value::<Vec<Rule>>(request["rules"].clone()) {
            Ok(rules) => match s.policy.seed(rules) {
                Ok(added) => match s.save_policy() {
                    Ok(()) => json!({ "ok": true, "seeded": added }),
                    Err(e) => json!({ "ok": false, "error": format!("could not be saved: {e}") }),
                },
                Err(e) => json!({ "ok": false, "error": e }),
            },
            Err(e) => json!({ "ok": false, "error": format!("not rules: {e}") }),
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
        _ => json!({ "ok": false, "error": format!("`{op}` is not an op here: status, seen, proposals, mode, private, allow, seed, remove, forget") }),
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
                        Ok(req) => handle(&state, &req, uid == Some(0)),
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

    fn dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("yantrik-egress-control-{name}-{}", std::process::id()))
    }

    fn state(name: &str) -> Mutex<State> {
        let d = dir(name);
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
        assert_eq!(handle(&s, &json!({"op":"status"}), false)["mode"], "audit");
        assert_eq!(handle(&s, &json!({"op":"mode","mode":"enforce"}), false)["ok"], true);
        assert_eq!(handle(&s, &json!({"op":"mode","mode":"wide-open"}), false)["ok"], false);
        let r = handle(&s, &json!({"op":"allow","rule":{"host":"api.x.ai","ports":[443],"why":"the model"}}), false);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(handle(&s, &json!({"op":"allow","rule":{"host":"*","ports":[443],"why":"all"}}), false)["ok"], false);
        assert_eq!(handle(&s, &json!({"op":"allow","rule":{"host":"a.com","ports":[443],"why":"x","extra":1}}), false)["ok"], false, "no unknown keys");
        let st = handle(&s, &json!({"op":"status"}), false);
        assert_eq!(st["mode"], "enforce");
        assert_eq!(st["rules"].as_array().unwrap().len(), 1);
        assert!(st["wide_links"].is_array() && st["assumed_links"].is_array(), "{st}");
        assert_eq!(handle(&s, &json!({"op":"private","on":true}), false)["ok"], true);
        assert_eq!(handle(&s, &json!({"op":"status"}), false)["private"], true);
        assert_eq!(handle(&s, &json!({"op":"remove","host":"api.x.ai"}), false)["removed"], 1);
        assert_eq!(handle(&s, &json!({"op":"nonsense"}), false)["ok"], false);
    }

    #[test]
    fn guarded_is_a_mode_the_person_can_choose_and_it_is_saved() {
        let s = state("guarded");
        let r = handle(&s, &json!({"op":"mode","mode":"guarded"}), false);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(handle(&s, &json!({"op":"status"}), false)["mode"], "guarded");
        let dir = dir("guarded");
        assert_eq!(crate::policy::Policy::read(&dir.join("policy.yaml")).unwrap().mode, Mode::Guarded, "written to the policy file");
        assert_eq!(State::load(&dir).policy.mode, Mode::Guarded, "and read back after a restart");
    }

    #[test]
    fn the_modes_are_offered_in_plain_words_the_least_strict_first() {
        let s = state("modes");
        let st = handle(&s, &json!({"op":"status"}), false);
        let got: Vec<(String, String, bool)> = st["modes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["mode"].as_str().unwrap().into(), m["label"].as_str().unwrap().into(), m["recommended"].as_bool().unwrap()))
            .collect();
        let want = [
            ("audit", "Watch only: everything allowed and recorded", false),
            ("guarded", "Home network closed, internet open", true),
            ("enforce", "Only places I approve", false),
        ];
        assert_eq!(got, want.map(|(m, l, r)| (m.to_string(), l.to_string(), r)));
    }

    #[test]
    fn only_root_seeds_and_a_seed_keeps_the_persons_rules() {
        let s = state("seed");
        handle(&s, &json!({"op":"allow","rule":{"host":"api.x.ai","ports":[443],"why":"the model"}}), false);
        let seed = json!({"op":"seed","rules":[{"host":"192.168.4.35","ports":[11434],"http":true,"lan":true,"why":"seeded from config.yaml"}]});
        assert_eq!(handle(&s, &seed, false)["ok"], false, "the desktop's owner does not seed");
        let r = handle(&s, &seed, true);
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["seeded"][0]["seeded"], true);
        let rules = handle(&s, &json!({"op":"status"}), false)["rules"].clone();
        assert_eq!(rules.as_array().unwrap().len(), 2);
        assert_eq!(handle(&s, &json!({"op":"seed","rules":[]}), true)["ok"], true);
        let rules = handle(&s, &json!({"op":"status"}), false)["rules"].clone();
        assert_eq!(rules.as_array().unwrap().len(), 1, "the seed is gone, the person's rule stays: {rules}");
        assert_eq!(rules[0]["host"], "api.x.ai");
    }
}
