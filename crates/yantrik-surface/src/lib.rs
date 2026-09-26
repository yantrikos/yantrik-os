//! Put something where a mind can find it: the `app.describe` / `app.act` surface, with no UI.
//!
//! A surface is a process's account of itself — one line and a small state object a mind reads
//! instead of a screenshot — and the actions it offers, each graded by how much damage it can do.
//! This crate is the whole of what answering one takes: the registry of actions, the JSON Schema
//! `describe` publishes, the argument checks (present, known, and of the declared type), the
//! revision guard (`expect_revision` → `STALE:`), the machine's ceiling and the person's mode
//! (`gate`, → `CEILING:` / `GRANT:`), spending a person's grant, who is calling
//! ([`caller`]), which agent a call is for ([`agent_token`]), and answers that take time
//! ([`answer_later`]). It has no UI dependency and runs on a plain thread or under tokio.
//!
//! Everything that speaks the protocol in this repository dispatches through it: every app
//! window (through `yantrik-app-runtime::control`, which adds only the hop to the UI thread), and
//! the services that answer `app.act` themselves (weather, system-monitor, notifications). A call
//! meets the same checks, in the same order, refused in the same words, whichever it reaches.
//!
//! # Hello, surface
//!
//! [`examples/hello-surface`](https://github.com/yantrikos/yantrik-os/blob/main/examples/hello-surface/src/main.rs)
//! is the smallest complete surface: a counter a mind can read, add to, and — with the person's
//! Allow — reset. Run it, and drive it the way a mind does:
//!
//! ```text
//! cargo run -p hello-surface
//! yos describe counter                 # the count, and three actions with their grades
//! yos act counter increment by=2       # standard: runs in every mode
//! yos act counter increment by=two     # refused: `by` must be an integer
//! yos act counter reset                # sensitive: in ask mode, a card for the person first
//! ```
//!
//! It is the Rust twin of `crates/yantrik-harness/examples/echo_harness.rs`: everything in it
//! that is not the counter is the entire cost of putting something where a mind can find it.
//! The guide for authors — quickstarts, grades, designing a `describe`, `.desktop` keys, `yos
//! check` — is `docs/sdk/`, and `templates/rust-surface` is a starting point to copy.
//!
//! # Found while closed
//!
//! A running surface is found by its socket. To be found while it is not running — listed in
//! `describe shell`, opened by `open_app`, reached by its other names and by a notification's
//! buttons — an app declares its surface in its own `.desktop` file, which every Debian app already
//! ships; there is no table in this repository to add it to:
//!
//! ```text
//! [Desktop Entry]
//! Name=Counter
//! Exec=/usr/bin/hello-surface
//! X-Yantrik-Surface=counter
//! X-Yantrik-Purpose=a number a mind can read and add to
//! X-Yantrik-Aliases=tally
//! ```
//!
//! `X-Yantrik-Adapter=<command>` names a separate process that serves the surface for an app that
//! cannot host one itself. The keys are in `docs/surface-protocol.md`, section 3.
//!
//! # The shape
//!
//! ```rust,no_run
//! use yantrik_surface::{Action, Param, Surface, View};
//!
//! Surface::new("counter")
//!     .describe(|| View::new("Counter — 3").with("count", 3))
//!     .action(
//!         Action::new("increment", "Add to the counter").arg(Param::integer("by").default(1)),
//!         |args| Ok(serde_json::json!({ "added": args["by"] })),
//!     )
//!     .serve()
//!     .expect("bound app-counter.sock");
//! ```
//!
//! A service that already serves its own methods keeps its `ServiceHandler` and hands the two
//! surface methods to [`Surface::answer`]. An app window uses `yantrik_app_runtime::control::App`,
//! which is this dispatch plus the hop to the thread that owns the window.
//!
//! # Parameters
//!
//! [`Param`] declares `text`, `number`, `integer`, `flag`, `one_of` (an enum), `array` (of a
//! type), `object`, each optionally with a `default`; `describe` publishes them as JSON Schema.
//! A handler always reads the type it declared: what a caller sends that converts to it without
//! loss is converted ([`coerced`]: `67` for text, `"12"` for an integer, `"true"` for a flag), and
//! anything else is refused before the handler runs — and before any grant is spent on the call.
//! The rules and the sentences are in [`check_arguments`]; `deploy/yantrik-os/dispatch-vectors.json`
//! writes them out, and the Python SDK replays it.
//!
//! # Codes
//!
//! Every refusal — unknown action, argument, `STALE:`, `CEILING:`, `GRANT:`, a handler's own
//! `Err` — is `-32602` ([`REFUSED`]), an application answer rather than a transport failure. An
//! unknown method is `-32601`; a window whose UI thread did not answer in time, `-32000`.

mod args;
mod call;
mod context;
mod registry;
mod surface;
#[cfg(test)]
mod vectors;

pub use args::{
    as_declared, check_argument, check_arguments, check_value, coerced, declaration_problems,
    with_defaults,
};
pub use call::{
    finish_later, next_action_id, refusal, service_id_for, ActCall, NO_SUCH_METHOD, REFUSED,
    UNANSWERED,
};
pub use context::{
    agent_token, answer_later, caller, off_the_reactor, AgentTokenScope, Caller, CallerScope, Later,
    LaterScope,
};
pub use registry::{
    check_grade, Describer, Handler, LocalRegistry, Registry, SharedDescriber, SharedHandler,
    SharedRegistry,
};
pub use surface::Surface;

pub use serde_json;
/// The envelope: what an app reports ([`View`]), what it offers ([`Action`], [`Param`]), and the
/// two replies built from them. [`Explainer`] is the sentence an action says about one call of
/// itself (#137); most authors meet it only through [`Action::explain`].
pub use yantrik_ipc_contracts::control_surface::{
    act_json, describe_json, Action, Explainer, Param, View, PARAM_TYPES,
};
pub use yantrik_ipc_contracts::email::ServiceError;
/// The ceiling, mode and grant rule every `app.act` meets. The dispatch calls it; it is exported
/// for a caller that needs to read the same files or spend a grant the same way.
pub use yantrik_ipc_transport::gate;
pub use yantrik_ipc_transport::gate::Authority;
pub use yantrik_ipc_transport::server::{PeerCred, RpcServer, ServiceHandler};

/// A stand-in for the shell's grant store, for every test in this crate: the spender is
/// process-wide, as the shell's is. A grant holds once, for exactly the call a person was shown
/// ([`stand_in::allow`]); anything else is refused in words of its own.
#[cfg(test)]
pub(crate) mod stand_in {
    use std::sync::{Mutex, Once};

    use serde_json::Value;

    static ALLOWED: Mutex<Vec<(String, String, String, Value)>> = Mutex::new(Vec::new());
    static SPENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

    fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            crate::gate::spend_grants_with(|id, app, action, args| {
                let allowed = ALLOWED.lock().unwrap_or_else(|e| e.into_inner());
                let Some((_, a, x, bound)) = allowed.iter().find(|(g, ..)| g == id) else {
                    return Err(format!("no approval request `{id}`."));
                };
                if (a.as_str(), x.as_str(), bound) != (app, action, args) {
                    return Err(format!("`{id}` was approved for {a}.{x} with {bound}, and this call carries {args}."));
                }
                let mut spent = SPENT.lock().unwrap_or_else(|e| e.into_inner());
                if spent.iter().any(|g| g == id) {
                    return Err(format!("`{id}` was already used."));
                }
                spent.push(id.to_string());
                Ok(())
            });
        });
    }

    /// A person pressed Allow on a card for exactly `app.action(args)`.
    pub(crate) fn allow(id: &str, app: &str, action: &str, args: Value) {
        install();
        ALLOWED.lock().unwrap_or_else(|e| e.into_inner()).push((id.into(), app.into(), action.into(), args));
    }

    /// Whether the grant has been spent.
    pub(crate) fn spent(id: &str) -> bool {
        SPENT.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|g| g == id)
    }
}

/// A surface served on a real socket and spoken to the way `yos` speaks to one: one JSON-RPC line
/// out, one back, on a connection of its own.
#[cfg(all(test, unix))]
mod over_a_socket {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    use serde_json::{json, Value};

    use super::*;

    /// One surface per test binary, on a socket in a directory of its own, under a home whose
    /// settings say what the shell ships: a `sensitive` ceiling, and `ask` mode.
    fn served() -> &'static str {
        static ADDRESS: OnceLock<String> = OnceLock::new();
        ADDRESS.get_or_init(|| {
            let root = std::env::temp_dir().join(format!("yantrik-surface-test-{}", std::process::id()));
            let config = root.join("home/.config/yantrik");
            std::fs::create_dir_all(&config).expect("a home of our own");
            std::fs::write(config.join("settings.yaml"), "tool_permission: sensitive\n").unwrap();
            std::fs::write(config.join("mind-mode.json"), r#"{"mode":"ask","session_rules":[]}"#).unwrap();
            // Nothing else in this crate's tests reads the files; the socket is the only door that
            // meets `Authority::now()`.
            std::env::set_var("HOME", root.join("home"));
            // One agent with a role: it may count, and read who it is, and nothing above
            // `standard`. Installed as the shell installs its own registry.
            yantrik_ipc_transport::reach::read_reach_with(|token| {
                (token == "tok-counter-role").then(|| yantrik_ipc_transport::reach::Reach {
                    agent: "pi:c-count1".into(),
                    role: "counter".into(),
                    name: "Counter".into(),
                    surfaces: vec!["counter.increment".into(), "counter.who".into()],
                    ceiling: "standard".into(),
                })
            });

            let count = Arc::new(AtomicI64::new(0));
            let surface = Surface::new("counter")
                .socket_name("counter-test")
                .describe({
                    let count = count.clone();
                    move || {
                        let n = count.load(Ordering::SeqCst);
                        View::new(format!("Counter — {n}")).with("count", n)
                    }
                })
                .action(
                    Action::new("increment", "Add to the counter")
                        .arg(Param::integer("by").default(1).describe("How much to add")),
                    {
                        let count = count.clone();
                        move |args| {
                            let by = args["by"].as_i64().expect("checked and defaulted by the dispatch");
                            Ok(json!({ "count": count.fetch_add(by, Ordering::SeqCst) + by }))
                        }
                    },
                )
                .action(Action::new("reset", "Set the counter to zero").risk("sensitive"), {
                    let count = count.clone();
                    move |_| {
                        count.store(0, Ordering::SeqCst);
                        Ok(json!({ "count": 0 }))
                    }
                })
                .action(
                    Action::new("set", "Set the counter to a number").risk("sensitive").arg(Param::integer("to")),
                    {
                        let count = count.clone();
                        move |args| {
                            let to = args["to"].as_i64().expect("an integer, converted by the dispatch if need be");
                            count.store(to, Ordering::SeqCst);
                            Ok(json!({ "count": to }))
                        }
                    },
                )
                .action(Action::new("who", "Report who is calling and for which agent").risk("safe"), |_| {
                    Ok(json!({ "pid": caller().map(|c| c.pid), "agent_token": agent_token() }))
                })
                .action(
                    Action::new("slow", "Answer after `ms` milliseconds, off the worker")
                        .risk("safe")
                        .expected_seconds(2)
                        .arg(Param::integer("ms")),
                    |args| {
                        let ms = args["ms"].as_u64().unwrap_or(0);
                        answer_later(move || {
                            std::thread::sleep(Duration::from_millis(ms));
                            Ok(json!({ "slept_ms": ms }))
                        })
                        .map(|()| json!("replaced"))
                        .or_else(|work| work())
                    },
                );
            assert!(surface.registry().problems().is_empty(), "{:?}", surface.registry().problems());

            let address = root.join("counter-test.sock").display().to_string();
            {
                let address = address.clone();
                std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .worker_threads(2)
                        .enable_all()
                        .build()
                        .expect("a runtime");
                    let _ = runtime.block_on(RpcServer::new(&address).serve(Arc::new(surface)));
                });
            }
            for _ in 0..1000 {
                if UnixStream::connect(&address).is_ok() {
                    return address;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("nothing ever bound {address}");
        })
    }

    use crate::stand_in;

    fn call(method: &str, params: Value) -> Value {
        let mut socket = UnixStream::connect(served()).expect("connect");
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        socket.write_all(format!("{request}\n").as_bytes()).expect("write the request");
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).expect("read the reply");
        serde_json::from_str(&line).expect(&line)
    }

    fn act(action: &str, args: Value) -> Value {
        call("app.act", json!({ "action": action, "args": args }))
    }

    /// `(code, message)` of a refusal, or a panic naming the reply that was not one.
    fn refused(reply: &Value) -> (i64, String) {
        let error = reply.get("error").unwrap_or_else(|| panic!("not refused: {reply}"));
        (error["code"].as_i64().unwrap(), error["message"].as_str().unwrap().to_string())
    }

    #[test]
    fn describe_publishes_the_actions_their_grades_and_their_types() {
        let described = call("app.describe", json!({}));
        let view = &described["result"];
        assert_eq!(view["app"], "counter");
        assert!(view["summary"].as_str().unwrap().starts_with("Counter — "), "{described}");
        assert_eq!(view["revision"].as_str().map(str::len), Some(16));
        let actions = view["actions"].as_array().unwrap();
        let increment = actions.iter().find(|a| a["name"] == "increment").unwrap();
        assert_eq!(increment["permission"], "standard");
        assert_eq!(
            increment["parameters"]["properties"]["by"],
            json!({"type": "integer", "description": "How much to add", "default": 1})
        );
        assert_eq!(increment["parameters"]["required"], json!([]));
        let reset = actions.iter().find(|a| a["name"] == "reset").unwrap();
        assert_eq!(reset["permission"], "sensitive");
        let slow = actions.iter().find(|a| a["name"] == "slow").unwrap();
        assert_eq!(slow["expected_seconds"], 2);
        assert!(reset.get("expected_seconds").is_none());
    }

    #[test]
    fn a_standard_act_runs_and_answers_with_the_view_after_it() {
        let reply = act("increment", json!({ "by": 2 }));
        let answer = &reply["result"];
        assert_eq!(answer["accepted"], true, "{reply}");
        assert_eq!(answer["settled"], true);
        assert!(answer["action_id"].as_str().unwrap().starts_with("counter-test#"), "{reply}");
        assert!(answer["result"]["count"].as_i64().unwrap() >= 2, "{reply}");
        assert!(answer["summary"].as_str().unwrap().starts_with("Counter — "));

        // Left out, the declared default is what the handler reads.
        let reply = act("increment", json!({}));
        assert_eq!(reply["result"]["accepted"], true, "{reply}");

        // Two calls, two names.
        let again = act("increment", json!({ "by": 1 }));
        assert_ne!(again["result"]["action_id"], reply["result"]["action_id"]);
    }

    #[test]
    fn every_refusal_is_the_apps_refusal_in_the_apps_words() {
        assert_eq!(
            refused(&act("decrement", json!({}))),
            (-32602, "unknown action `decrement`; this app offers: increment, reset, set, who, slow".into())
        );
        assert_eq!(
            refused(&act("increment", json!({ "by": "two" }))),
            (-32602, "`increment` argument `by` must be an integer, and a string arrived".into())
        );
        assert_eq!(
            refused(&act("increment", json!({ "by": 1, "times": 3 }))),
            (-32602, "`increment` has no argument `times`; it takes: by".into())
        );
        assert_eq!(
            refused(&act("slow", json!({}))),
            (-32602, "`slow` needs argument `ms`".into())
        );
        assert_eq!(
            refused(&call("app.act", json!({ "args": {} }))),
            (-32602, "act needs a non-empty `action`".into())
        );
        let (code, message) = refused(&call("counter.frobnicate", json!({})));
        assert_eq!((code, message.as_str()), (-32601, "unknown method `counter.frobnicate`; this app serves app.describe, app.act"));
    }

    #[test]
    fn an_act_decided_on_an_old_revision_is_refused_as_stale() {
        let reply = call(
            "app.act",
            json!({ "action": "increment", "args": {}, "expect_revision": "0000000000000000" }),
        );
        let (code, message) = refused(&reply);
        assert_eq!(code, -32602);
        assert!(message.starts_with("STALE: this app is at revision "), "{message}");
        assert!(message.contains("you acted on 0000000000000000") && message.contains("Counter — "), "{message}");
    }

    /// The machine's settings say `ask`: a `sensitive` act needs a person's Allow, the refusal
    /// says how to get one, and the grant — spent through the shell — lets it run once.
    #[test]
    fn a_sensitive_act_needs_a_grant_and_runs_on_one_once() {
        let (code, message) = refused(&act("reset", json!({})));
        assert_eq!(code, -32602);
        assert!(message.starts_with("GRANT: counter.reset is graded `sensitive`"), "{message}");
        assert!(message.contains("ask mode") && message.contains("request_approval"), "{message}");

        stand_in::allow("fresh-reset", "counter", "reset", json!({}));
        let granted = call("app.act", json!({ "action": "reset", "args": {}, "grant": "fresh-reset" }));
        assert_eq!(granted["result"]["result"]["count"], 0, "{granted}");

        let (_, message) = refused(&call("app.act", json!({ "action": "reset", "args": {}, "grant": "fresh-reset" })));
        assert!(message.starts_with("GRANT: `fresh-reset` does not authorise counter.reset") && message.contains("already used"), "{message}");

        // A grant on a call to an action this surface does not have is never offered to the
        // shell: the answer is the surface's, and the grant is still whole afterwards.
        stand_in::allow("fresh-kept", "counter", "reset", json!({}));
        let (_, message) = refused(&call("app.act", json!({ "action": "wipe", "args": {}, "grant": "fresh-kept" })));
        assert!(message.starts_with("unknown action `wipe`"), "{message}");
        let kept = call("app.act", json!({ "action": "reset", "args": {}, "grant": "fresh-kept" }));
        assert_eq!(kept["result"]["accepted"], true, "{kept}");
    }

    /// The kernel's account of the caller reaches the handler, and the agent token rides beside
    /// the arguments — never among them.
    #[test]
    fn the_caller_and_the_agent_token_reach_the_handler() {
        let reply = call(
            "app.act",
            json!({ "action": "who", "args": { "agent_token": "smuggled" }, "agent_token": "tok-7f3a" }),
        );
        assert_eq!(reply["result"]["result"]["pid"].as_i64(), Some(i64::from(std::process::id() as i32)), "{reply}");
        assert_eq!(reply["result"]["result"]["agent_token"], "tok-7f3a", "{reply}");
        assert!(!reply.to_string().contains("smuggled"), "{reply}");

        let reply = act("who", json!({}));
        assert!(reply["result"]["result"]["agent_token"].is_null(), "{reply}");
    }

    /// An agent started from a role is held to the role's reach on this door as on a window's:
    /// on its surfaces it runs, off them it is refused with `REACH:` before the ceiling, the mode
    /// or a grant is looked at — and a grant it carried is still whole afterwards.
    #[test]
    fn an_agent_is_held_to_its_reach_before_any_grant_is_spent() {
        let with = |action: &str, token: &str, grant: Option<&str>| {
            let mut params = json!({ "action": action, "args": {}, "agent_token": token });
            if let Some(grant) = grant {
                params["grant"] = grant.into();
            }
            call("app.act", params)
        };

        let reply = with("increment", "tok-counter-role", None);
        assert_eq!(reply["result"]["accepted"], true, "on its surfaces it runs: {reply}");
        let reply = with("who", "tok-counter-role", None);
        assert_eq!(reply["result"]["result"]["agent_token"], "tok-counter-role", "{reply}");

        stand_in::allow("fresh-held", "counter", "reset", json!({}));
        let (code, message) = refused(&with("reset", "tok-counter-role", Some("fresh-held")));
        assert_eq!(code, -32602);
        assert!(message.starts_with("REACH: counter.reset is outside the Counter's reach"), "{message}");
        assert!(message.contains("`pi:c-count1` is the Counter"), "{message}");

        // The Allow it carried was not used up: the person's own call spends it.
        let kept = call("app.act", json!({ "action": "reset", "args": {}, "grant": "fresh-held" }));
        assert_eq!(kept["result"]["accepted"], true, "{kept}");

        // A token with no reach is not held; the mode still is (reset is `sensitive`, and this
        // machine is in ask mode).
        let (_, message) = refused(&with("reset", "tok-no-role", None));
        assert!(message.starts_with("GRANT:"), "{message}");
    }

    /// A person's Allow is not used up on a call its own arguments refuse, on a service's door as
    /// on a window's: a grant for `set {"to": 5}` carried by a call whose `to` is not an integer,
    /// or that leaves it out, is refused for the argument and is still whole afterwards; the same
    /// grant with the arguments right runs, once. A grant is bound to the arguments as sent, so
    /// `"7"` on the card is `"7"` on the call — and the handler reads 7.
    #[test]
    fn a_malformed_call_is_refused_before_its_grant_is_spent_on_the_service_door() {
        served();
        stand_in::allow("service-set", "counter", "set", json!({"to": 5}));
        let with = |args: Value, grant: &str| call("app.act", json!({ "action": "set", "args": args, "grant": grant }));

        assert_eq!(
            refused(&with(json!({"to": "five"}), "service-set")),
            (-32602, "`set` argument `to` must be an integer, and a string arrived".into())
        );
        assert_eq!(refused(&with(json!({}), "service-set")), (-32602, "`set` needs argument `to`".into()));
        assert!(!stand_in::spent("service-set"), "refused for its arguments, and the Allow was used up anyway");

        let reply = with(json!({"to": 5}), "service-set");
        assert_eq!(reply["result"]["result"]["count"], 5, "the same grant, the arguments right: {reply}");
        assert!(stand_in::spent("service-set"));

        stand_in::allow("service-set-text", "counter", "set", json!({"to": "7"}));
        let reply = with(json!({"to": "7"}), "service-set-text");
        assert_eq!(reply["result"]["result"]["count"], 7, "{reply}");
        assert!(stand_in::spent("service-set-text"));

        // Without a grant, the well-formed call meets the mode (this machine is in ask mode).
        let (_, message) = refused(&call("app.act", json!({ "action": "set", "args": {"to": 1} })));
        assert!(message.starts_with("GRANT: counter.set is graded `sensitive`"), "{message}");
    }

    /// An answer finished later is the work's own result, and holds up no other caller.
    #[test]
    fn an_answer_that_takes_time_holds_up_nobody() {
        served();
        let asked = Instant::now();
        let slow = std::thread::spawn(|| act("slow", json!({ "ms": 1200 })));
        std::thread::sleep(Duration::from_millis(150));
        let glance = Instant::now();
        let described = call("app.describe", json!({}));
        assert_eq!(described["result"]["app"], "counter");
        assert!(glance.elapsed() < Duration::from_millis(700), "a describe waited {:?}", glance.elapsed());

        let reply = slow.join().expect("the slow call");
        assert!(asked.elapsed() >= Duration::from_millis(1200));
        assert_eq!(reply["result"]["result"]["slept_ms"], 1200, "{reply}");
        assert!(reply["result"]["revision"].as_str().is_some(), "{reply}");
    }
}
