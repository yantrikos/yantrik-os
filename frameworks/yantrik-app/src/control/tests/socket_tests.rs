//! The tests that drive the control dispatch over a real socket: the reach an agent is held to, the
//! caller reaching the handler across the UI hop, answers that take time, and how a grant and an
//! agent token travel with a call. Separate from the in-process dispatch tests in the parent only
//! to keep control.rs from growing; the helpers (the served test surface, the socket call) are the
//! parent's.

use super::*;

/// Agents catalog: an agent started from a role is held to the role's reach on the real
/// dispatch — its token names a reach (here through an installed reader, as the shell installs
/// its own registry), an act on its surfaces runs, one off them is refused in the reach's words
/// before the handler and before any grant is spent, and a token with no reach is not held.
#[cfg(unix)]
#[test]
fn an_agent_is_held_to_its_reach_on_the_socket_before_any_grant_is_spent() {
    // The stand-in shell's reader holds `tok-reach-reviewer` to the reviewer's reach.
    spend_through_a_stand_in_shell();
    let act = |action: &str, token: &str, grant: Option<&str>| {
        let mut params =
            serde_json::json!({ "action": action, "args": {}, "agent_token": token });
        if let Some(grant) = grant {
            params["grant"] = grant.into();
        }
        call(&serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "app.act", "params": params }).to_string())
    };

    let reply = act("echo", "tok-reach-reviewer", None);
    assert_eq!(
        reply["result"]["result"]["agent_token"], "tok-reach-reviewer",
        "on its surfaces it runs: {reply}"
    );

    let reply = act("who", "tok-reach-reviewer", None);
    let err = reply["error"]["message"].as_str().unwrap_or_default();
    assert!(
        err.starts_with("REACH: caller-test.who is outside the Reviewer's reach"),
        "{reply}"
    );
    assert!(err.contains("`deepseek:c-reach1` is the Reviewer"), "{err}");
    assert_eq!(
        reply["error"]["code"], -32602,
        "a policy answer, not a transport fault"
    );

    // A grade off the ladder is refused by the reach before the machine's ceiling is asked.
    let reply = act("nuke", "tok-reach-reviewer", None);
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .starts_with("REACH: caller-test.nuke is graded"),
        "{reply}"
    );

    // Refused by the reach, a person's Allow is not used up on it.
    let reply = act("who", "tok-reach-reviewer", Some("fresh-reach"));
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .starts_with("REACH:"),
        "{reply}"
    );
    spend_for_render(open(), "fresh-reach", &serde_json::json!({"out": "x.png"}))
        .expect("the reach's refusal spent nothing");

    // Another agent's token, with no reach, is not held; and the person's call has none.
    let reply = act("who", "tok-no-reach", None);
    assert!(reply["error"].is_null(), "{reply}");
    let reply = call(
        r#"{"jsonrpc":"2.0","id":9,"method":"app.act","params":{"action":"who","args":{}}}"#,
    );
    assert!(reply["error"].is_null(), "{reply}");
}

/// See `test_ui_thread` for why the hop is a channel.
#[cfg(unix)]
#[test]
fn the_caller_reaches_the_handler_across_the_ui_hop() {
    use std::os::unix::fs::MetadataExt;

    let reply = call(
        r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"who","args":{}}}"#,
    );
    let seen = &reply["result"]["result"];
    assert!(
        !seen.is_null(),
        "the handler saw no caller at all — the credentials did not cross the hop: {reply}"
    );
    assert_eq!(
        seen["pid"].as_u64(),
        Some(u64::from(std::process::id())),
        "the kernel's pid for this connection is this test process: {reply}"
    );
    // The uid the kernel reported has to be the uid that owns the socket — this test is both
    // ends of the connection, so anything else means the field is not the peer's.
    let owner = std::fs::metadata(served_test_surface())
        .expect("the socket exists")
        .uid();
    assert_eq!(seen["uid"].as_u64(), Some(u64::from(owner)), "{reply}");
    assert!(
        reply["result"]["action_id"]
            .as_str()
            .unwrap()
            .starts_with("app-caller-test#"),
        "{reply}"
    );
}

/// The shell's `agent_run` owes its caller an exit code that may be minutes away. The handler
/// hands the wait to `answer_later`; the caller gets the work's own result, and in the
/// meantime the socket and the UI thread both go on answering everybody else.
#[cfg(unix)]
#[test]
fn an_answer_that_takes_time_is_finished_off_the_ui_thread_and_holds_up_nobody() {
    use std::time::Instant;

    served_test_surface();
    let asked = Instant::now();
    let slow = std::thread::spawn(|| {
        call(
            r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"slow","args":{"ms":1500}}}"#,
        )
    });

    // While that one waits: another caller, another connection, served at once. `describe`
    // runs on the UI stand-in, so this also shows the UI thread is not the one waiting.
    std::thread::sleep(Duration::from_millis(200));
    let glance = Instant::now();
    let described = call(r#"{"jsonrpc":"2.0","id":2,"method":"app.describe","params":{}}"#);
    assert_eq!(described["result"]["app"], "caller-test", "{described}");
    assert!(
        glance.elapsed() < Duration::from_millis(700),
        "a describe waited {:?} behind a slow act",
        glance.elapsed()
    );

    let reply = slow.join().expect("the slow call");
    assert!(
        asked.elapsed() >= Duration::from_millis(1500),
        "the reply is the finished work"
    );
    assert_eq!(reply["result"]["accepted"], true, "{reply}");
    assert_eq!(
        reply["result"]["result"]["slept_ms"], 1500,
        "the work's result, not the handler's: {reply}"
    );
    assert_eq!(
        reply["result"]["result"]["pid"].as_u64(),
        Some(u64::from(std::process::id())),
        "the caller read in the handler reached the work: {reply}"
    );
    assert!(
        reply["result"]["revision"].as_str().is_some(),
        "the envelope keeps its view: {reply}"
    );

    // Work that refuses is refused to the caller, as a handler's refusal would be.
    let refused = call(
        r#"{"jsonrpc":"2.0","id":3,"method":"app.act","params":{"action":"slow","args":{"ms":10,"refuse":true}}}"#,
    );
    assert_eq!(
        refused["error"]["message"], "refused after 10 ms",
        "{refused}"
    );
    assert_eq!(
        refused["error"]["code"], -32602,
        "an application refusal, not a transport fault"
    );

    // And a wrong type is refused on the socket as it is in the dispatch, before the handler.
    let mistyped = call(
        r#"{"jsonrpc":"2.0","id":4,"method":"app.act","params":{"action":"slow","args":{"ms":"soon"}}}"#,
    );
    assert_eq!(
        mistyped["error"]["message"],
        "`slow` argument `ms` must be an integer, and a string arrived",
        "{mistyped}"
    );
    assert_eq!(mistyped["error"]["code"], -32602);
}

/// Every app's agent rail asks whether the companion can be reached on a timer, on the
/// window's thread — the thread `describe` runs on. It used to be a round trip to the shell
/// right there, so a describe that arrived during one waited for the shell; `yos check
/// weather` measured ~600 ms. Here the shell takes two seconds to answer, the rail ticks on the
/// UI stand-in just before each describe, and each describe still answers within the bound.
#[cfg(unix)]
#[test]
fn a_rail_asking_a_slow_shell_on_the_ui_thread_holds_up_no_describe() {
    use crate::companion::{Reach, ReachWatch, REACH_FIRST_WAIT};
    use std::time::Instant;

    served_test_surface();
    let watch = ReachWatch::new(
        Box::new(|| {
            std::thread::sleep(Duration::from_secs(2));
            Reach::Ready
        }),
        Duration::from_millis(50),
        REACH_FIRST_WAIT,
        Duration::from_secs(30),
    );
    // What `refresh_agent_rail` does on its timer, on the thread that owns the window.
    let rail_tick = |watch: &Arc<ReachWatch>| {
        let watch = Arc::clone(watch);
        post_to_ui(Box::new(move || {
            let _ = watch.get();
        }))
        .expect("the UI stand-in takes the tick");
    };

    for read in 0..3 {
        rail_tick(&watch);
        let glance = Instant::now();
        let described = call(r#"{"jsonrpc":"2.0","id":1,"method":"app.describe","params":{}}"#);
        assert_eq!(described["result"]["app"], "caller-test", "{described}");
        assert!(
            glance.elapsed() < Duration::from_millis(700),
            "describe {read} waited {:?} behind the rail asking a two-second shell",
            glance.elapsed()
        );
    }
}

/// The token rides beside `args` and reaches the handler through `agent_token()`; `args` —
/// what an approval card shows and an audit line keeps — never holds it, even when a caller
/// puts it there.
#[cfg(unix)]
#[test]
fn an_agent_token_reaches_the_handler_beside_the_arguments_and_never_among_them() {
    let reply = call(
        r#"{"jsonrpc":"2.0","id":1,"method":"app.act","params":{"action":"echo","args":{"command":"ls"},"agent_token":"tok-7f3a"}}"#,
    );
    let seen = &reply["result"]["result"];
    assert_eq!(
        seen["agent_token"], "tok-7f3a",
        "the handler reads the token: {reply}"
    );
    assert_eq!(
        seen["args"],
        serde_json::json!({"command": "ls"}),
        "and its args are only args: {reply}"
    );

    // Smuggled inside `args` as well: taken out, not used, and not refused as an undeclared
    // argument either — the call goes on as if it had never been there.
    let reply = call(
        r#"{"jsonrpc":"2.0","id":2,"method":"app.act","params":{"action":"echo","args":{"command":"ls","agent_token":"smuggled"},"agent_token":"tok-7f3a"}}"#,
    );
    assert_eq!(
        reply["result"]["result"]["args"],
        serde_json::json!({"command": "ls"}),
        "{reply}"
    );
    assert_eq!(
        reply["result"]["result"]["agent_token"], "tok-7f3a",
        "the one beside args wins: {reply}"
    );
    assert!(
        !reply.to_string().contains("smuggled"),
        "nothing in the reply carries it: {reply}"
    );

    // Only inside `args`: stripped, and the handler sees no token at all.
    let reply = call(
        r#"{"jsonrpc":"2.0","id":3,"method":"app.act","params":{"action":"echo","args":{"agent_token":"smuggled"}}}"#,
    );
    assert_eq!(
        reply["result"]["result"]["args"],
        serde_json::json!({}),
        "{reply}"
    );
    assert!(
        reply["result"]["result"]["agent_token"].is_null(),
        "{reply}"
    );

    // No token, no token.
    let reply = call(
        r#"{"jsonrpc":"2.0","id":4,"method":"app.act","params":{"action":"echo","args":{}}}"#,
    );
    assert!(
        reply["result"]["result"]["agent_token"].is_null(),
        "{reply}"
    );
}

/// #154 through the real dispatch: the RPC thread asks the UI thread for the grade before it
/// offers a grant to the shell, so an act the ceiling refuses, and an action the app does not
/// have, spend nothing. Before, both came back `GRANT: … does not authorise …` — the grant had
/// already gone to the shell by the time anything looked at the action.
#[cfg(unix)]
#[test]
fn a_grant_on_the_socket_is_offered_to_the_shell_only_past_the_ceiling() {
    spend_through_a_stand_in_shell();
    let act = |action: &str, args: serde_json::Value, grant: &str| {
        let request = serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "app.act",
            "params": { "action": action, "args": args, "grant": grant },
        });
        let reply = call(&request.to_string());
        reply["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };

    let err = act("nuke", serde_json::json!({}), "fresh-socket");
    assert!(
        err.starts_with("CEILING:"),
        "the ceiling's answer, not the shell's: {err}"
    );

    let err = act("nope", serde_json::json!({}), "fresh-socket");
    assert!(
        err.starts_with("unknown action `nope`"),
        "the app's answer, not the shell's: {err}"
    );

    // Past the ceiling the grant does go to the shell, and one that does not hold ends the
    // call in the shell's words.
    let err = act("who", serde_json::json!({}), "made-up");
    assert!(
        err.starts_with("GRANT:") && err.contains("no approval request"),
        "{err}"
    );

    // What a grant is spent against is the arguments with any agent token a caller put among
    // them already lifted off (see `agent_token`): the shell is shown `{"command":"ls"}` and
    // the grant is bound to that, never to a token. The stand-in shell names what it was
    // handed, which is how this can be seen from here.
    let err = act(
        "echo",
        serde_json::json!({"command": "ls", "agent_token": "smuggled"}),
        "fresh-echo",
    );
    assert!(
        err.starts_with("GRANT:") && err.contains(r#"this call carries {"command":"ls"}"#),
        "{err}"
    );
    assert!(
        !err.contains("smuggled"),
        "the token reached the shell as an argument: {err}"
    );

    // And the grant the two refusals carried was never spent.
    spend_for_render(open(), "fresh-socket", &serde_json::json!({"out": "x.png"}))
        .expect("nothing spent `fresh-socket` on the way to either refusal");
}

/// #182: a grant is for the agent it was asked for, and the shell can only hold that line if
/// the spend says who is spending. So when a call that carries an agent token spends a grant
/// through this dispatch, the shell is told the token — beside `args`, as it arrived — and
/// the pid the kernel stamped on the call: the process the token rode in from, checked the
/// same way the shell would have checked it at its own door. A call with no token spends as
/// no agent, exactly as the person's own `yos act` always has.
#[cfg(unix)]
#[test]
fn a_spend_names_the_agent_whose_call_it_rides_on() {
    spend_through_a_stand_in_shell();
    let args = serde_json::json!({"command": "ls"});
    allow("spend-182-agent", "caller-test", "echo", args.clone());
    allow("spend-182-person", "caller-test", "echo", args.clone());

    let act = |grant: &str, token: Option<&str>| {
        let mut params = serde_json::json!({ "action": "echo", "args": args, "grant": grant });
        if let Some(token) = token {
            params["agent_token"] = serde_json::json!(token);
        }
        call(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "app.act", "params": params}).to_string())
    };

    let reply = act("spend-182-agent", Some("tok-182"));
    assert_eq!(
        reply["result"]["result"]["agent_token"], "tok-182",
        "the act itself ran: {reply}"
    );
    assert_eq!(
        spend_caller("spend-182-agent"),
        Some(Some(("tok-182".to_string(), Some(std::process::id())))),
        "the spend carries the token and the kernel's account of the process it arrived in"
    );

    let reply = act("spend-182-person", None);
    assert!(
        reply["result"].is_object(),
        "the same act with no token still runs: {reply}"
    );
    assert_eq!(
        spend_caller("spend-182-person"),
        Some(None),
        "no token, no agent claimed"
    );
}

/// Final review of the card fix: the shell refused an act a person had just allowed, because
/// another card was waiting, and the Allow was spent for nothing. A handler can now see that
/// its call is the allowed one, over the real socket, and only that call: the next one on the
/// same thread, with no grant, is not.
#[cfg(unix)]
#[test]
fn a_handler_sees_the_call_a_person_allowed_and_no_other() {
    spend_through_a_stand_in_shell();
    let args = serde_json::json!({"command": "open"});
    allow("granted-seen", "caller-test", "echo", args.clone());
    let act = |grant: Option<&str>| {
        let mut params = serde_json::json!({ "action": "echo", "args": args });
        if let Some(grant) = grant {
            params["grant"] = serde_json::json!(grant);
        }
        call(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "app.act", "params": params}).to_string())
    };

    let reply = act(Some("granted-seen"));
    assert_eq!(
        reply["result"]["result"]["granted"], true,
        "the allowed act knows it: {reply}"
    );
    let reply = act(None);
    assert_eq!(
        reply["result"]["result"]["granted"], false,
        "nothing left over for the next call: {reply}"
    );
}

/// The guard puts back what was there, so a nested or panicking dispatch cannot leave the
/// next call on this thread reading as allowed.
#[test]
fn a_grant_is_seen_for_its_own_call_only() {
    assert!(
        !call_was_granted(),
        "outside a dispatch nothing was allowed"
    );
    {
        let _allowed = GrantedScope::enter(true);
        assert!(call_was_granted());
        {
            let _inner = GrantedScope::enter(false);
            assert!(!call_was_granted());
        }
        assert!(
            call_was_granted(),
            "the inner call's end restores the outer one's"
        );
    }
    assert!(!call_was_granted(), "and the grant goes with its call");
}
