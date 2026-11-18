//! Tests of `control_agent_terminal`, a child module of it so they reach its private items.

use super::*;
use yantrik_agent_terminal::{TokenTable, NO_AGENT};

fn call(token: &str) -> Call {
    Call { pid: Some(std::process::id()), token: Some(token.to_string()) }
}

/// A call on a session from this process, which stands in for the harness that attached it:
/// a session answers only the process that attached it and the ones it started.
fn as_harness(
    host: &yantrik_harness::Host,
    method: &str,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    host.handle_from(method, params, Some(std::process::id()), None)
}

/// One job's card in its agent's pane, once the store has its end: `(args, provenance, exit
/// code, output)`. Asked again until the end is there — the terminal answers a waiting call
/// before its finish listeners have run.
fn settled_card(agent: &AgentId, job: &str) -> (Value, crate::agents::Provenance, Option<i32>, String) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let found = crate::agents::store().read(|s| {
            s.agent(agent)?
                .cards()
                .find(|c| c.call == job && !c.running())
                .map(|c| (c.args.clone(), c.provenance, c.exit_code, c.output.all()))
        });
        if let Some(found) = found {
            return found;
        }
        assert!(std::time::Instant::now() < deadline, "no settled card for {job} in {agent}'s pane");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Each command's terminal lands in the card of the agent it runs for, and in no other — two
/// agents' commands running side by side, one output each.
#[test]
fn a_commands_output_reaches_its_own_agents_card_and_no_other() {
    let a = AgentId::new("pi", "c-glue-out");
    let b = AgentId::new("deepseek", "c-glue-out");
    let ja = jobs().start(&a, "printf 'from-a-1\\n'; sleep 0.3; printf 'from-a-2\\n'", None).unwrap();
    let jb = jobs().start(&b, "printf 'from-b\\n'; sleep 0.1; exit 3", None).unwrap();
    jobs().job(&a, &ja, Duration::from_secs(10)).unwrap();
    jobs().job(&b, &jb, Duration::from_secs(10)).unwrap();

    let (_, provenance, exit, output) = settled_card(&a, &ja.0);
    assert_eq!((provenance, exit), (crate::agents::Provenance::Verified, Some(0)));
    assert!(output.contains("from-a-1") && output.contains("from-a-2"), "{output:?}");
    assert!(!output.contains("from-b"), "another agent's output is not in this card: {output:?}");

    let (_, _, exit, output) = settled_card(&b, &jb.0);
    assert_eq!(exit, Some(3), "the process's own exit code");
    assert!(output.contains("from-b") && !output.contains("from-a"), "{output:?}");

    // Neither job's card is in the other agent's pane at all.
    crate::agents::store().read(|s| {
        assert!(s.agent(&a).unwrap().cards().all(|c| c.call != jb.0));
        assert!(s.agent(&b).unwrap().cards().all(|c| c.call != ja.0));
    });
}

/// The token is not an argument of any of the four, so nothing that shows or keeps
/// arguments — `describe shell`'s action list, an approval card, an audit line — is ever
/// handed one by these actions.
#[test]
fn no_agent_action_takes_the_token_as_an_argument() {
    for spec in specs() {
        let schema = spec.schema();
        let params = &schema["parameters"]["properties"];
        assert!(params.get("agent_token").is_none(), "{} takes the token as an argument: {schema}", spec.name);
        assert!(params.get("agent").is_none(), "{} takes the agent as an argument: {schema}", spec.name);
        assert!(
            schema["description"].as_str().is_some_and(|d| d.contains("beside `args`")),
            "{} does not say where the token goes: {schema}",
            spec.name
        );
    }
}

/// A mind sent `wait: true` again in the #980 gate, so the parameter says what it is, first.
#[test]
fn wait_says_it_is_a_number_of_seconds_in_both_actions() {
    for spec in specs().into_iter().filter(|a| a.name == "agent_run" || a.name == "agent_job") {
        let wait = spec.params.iter().find(|p| p.name == "wait").expect("wait");
        assert_eq!(wait.kind, "number", "{}", spec.name);
        assert!(
            wait.description.starts_with("A number of seconds, e.g. 30, not true or false."),
            "{}: {}",
            spec.name,
            wait.description
        );
    }
    let run = specs().into_iter().find(|a| a.name == "agent_run").unwrap();
    assert!(run.description.contains("still going after `wait` seconds it answers `running: true`"));
    let job = specs().into_iter().find(|a| a.name == "agent_job").unwrap();
    assert!(job.description.contains("`wait` is a number of seconds"), "{}", job.description);
}

/// The four actions end to end, minus only the socket: with no dispatch in progress the work
/// runs inline, so this drives the real token check, the real terminal and `describe`.
///
/// One test, because the resolver is the shell's one global: "no tokens yet" has to be seen
/// before any are installed.
#[test]
fn the_actions_answer_for_the_agent_the_token_names_and_nobody_else() {
    // The resolver is the shell's one global; hold it for the length of the test (see
    // `RESOLVER_TESTS`).
    let _held = RESOLVER_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    let me = Some(std::process::id());

    // Before the host issues tokens, every call is inert — and says so.
    let err = agent_run(&json!({"command": "echo hi"}), call("t-pi")).unwrap_err();
    assert!(err.starts_with(NO_AGENT), "{err}");
    let err = agent_run(&json!({"command": "echo hi"}), Call { pid: me, token: None }).unwrap_err();
    assert!(err.contains("no agent token came with this call"), "{err}");

    // This test process stands in for the harness that holds both tokens.
    let table = Arc::new(TokenTable::new());
    table.issue("t-pi", AgentId::new("pi", "c-shell"), me);
    table.issue("t-ds", AgentId::new("deepseek", "c-shell"), me);
    install_resolver(table);

    let done = agent_run(&json!({"command": "cd /tmp && echo hi", "wait": 10}), call("t-pi")).unwrap();
    assert_eq!(done["exit_code"], 0, "{done}");
    assert_eq!(done["cwd_after"], "/tmp", "{done}");
    assert_eq!(done["tail"], "hi", "{done}");
    assert_eq!(done["agent"], "pi:c-shell", "the agent is the token's");
    assert!(!done.to_string().contains("t-pi"), "the answer does not repeat the token: {done}");
    // And it is a verified card in pi's pane: the command and where it started, its output,
    // the process's own exit code — and nothing of the token.
    let pi_agent = AgentId::new("pi", "c-shell");
    let job = done["job"].as_str().unwrap().to_string();
    let card = settled_card(&pi_agent, &job);
    assert_eq!(card.0["command"], "cd /tmp && echo hi", "{card:?}");
    assert!(card.0["cwd"].as_str().is_some_and(|c| !c.is_empty()), "{card:?}");
    assert_eq!((card.1, card.2), (crate::agents::Provenance::Verified, Some(0)));
    assert!(card.3.contains("hi") && !card.3.contains("t-pi"), "{card:?}");

    let slow = agent_run(&json!({"command": "sleep 30", "wait": 0.5}), call("t-pi")).unwrap();
    assert_eq!(slow["running"], true, "{slow}");
    assert!(slow["next"].as_str().is_some_and(|n| n.contains("agent_job")), "{slow}");
    let job = slow["job"].clone();

    // `describe shell` lists it under its agent, and nowhere names the token.
    let described = for_describe();
    let pi = described
        .as_array()
        .and_then(|agents| agents.iter().find(|a| a["agent"] == "pi:c-shell"))
        .unwrap_or_else(|| panic!("pi is not listed: {described}"));
    assert!(pi["running"].as_array().unwrap().iter().any(|j| j["job"] == job && j["command"] == "sleep 30"));
    assert!(!described.to_string().contains("t-pi"), "{described}");

    // Another agent's token cannot touch it; nor can pi's token from outside pi's harness.
    let err = agent_kill(&json!({"job": job}), call("t-ds")).unwrap_err();
    assert!(err.contains("belongs to another agent"), "{err}");
    let outsider = Call { pid: Some(1), token: Some("t-pi".into()) };
    let err = agent_job(&json!({"job": job, "wait": 0}), outsider).unwrap_err();
    assert!(err.contains("not issued to the process"), "{err}");

    let looked = agent_job(&json!({"job": job, "wait": 0}), call("t-pi")).unwrap();
    assert_eq!(looked["running"], true);

    let killed = agent_kill(&json!({"job": job}), call("t-pi")).unwrap();
    assert_eq!((killed["stopped"].clone(), killed["signal_name"].clone()), (json!(true), json!("SIGTERM")), "{killed}");

    let answered = agent_run(&json!({"command": "read -r x; echo \"[$x]\"", "wait": 0.3}), call("t-pi")).unwrap();
    let typed = agent_input(&json!({"job": answered["job"], "text": "yes\n"}), call("t-pi")).unwrap();
    assert_eq!(typed["sent_bytes"], 4);
    let finished = agent_job(&json!({"job": answered["job"], "wait": 10}), call("t-pi")).unwrap();
    assert_eq!(finished["tail"], "yes\n[yes]", "the echo of what was typed, then the answer: {finished}");

    // The bounds on `wait`, refused before anything is started.
    let err = agent_run(&json!({"command": "true", "wait": 601}), call("t-pi")).unwrap_err();
    assert!(err.contains("between 0 and 600"), "{err}");
    // A yes or no is what it plainly means, not a refusal: on 561 the Mind sent
    // `wait: true` twice and never saw its output.
    assert_eq!(wait_arg(&json!({"wait": true})).unwrap(), DEFAULT_WAIT);
    assert_eq!(wait_arg(&json!({"wait": false})).unwrap(), Duration::ZERO);
    let yes = agent_run(&json!({"command": "echo yes-wait", "wait": true}), call("t-pi")).unwrap();
    assert_eq!(yes["exit_code"], 0, "`wait: true` waits for the command: {yes}");
    assert!(wait_arg(&json!({"wait": "soon"})).unwrap_err().contains("number of seconds"));

    // ── Wired to the harness host, as the shell does when it makes one ──
    //
    // This test process stands in for the harness again: it attaches with its own pid, the
    // way the socket's SO_PEERCRED would report it, and its agent is handed a turn and so a
    // token.
    use yantrik_harness::{protocol, Host, Turn};
    let host = Host::new(vec![]);
    let attach = json!({ "id": "pi", "name": "Pi", "conversations": true });
    let session = host.handle_from(protocol::ATTACH, &attach, me, None).unwrap()["session"].as_str().unwrap().to_string();
    let agent = host.start_agent("pi").unwrap();
    let turn = |text: &str| {
        let _answer = host.send_to(&agent, Turn::new(text)).unwrap();
        let handed = as_harness(&host, protocol::POLL, &json!({ "session": session })).unwrap();
        as_harness(&host, protocol::COMPLETE, &json!({ "session": session, "turn_id": handed["turn_id"] })).unwrap();
        handed
    };
    let token = turn("build it")["agent_token"].as_str().unwrap().to_string();
    serve_host(&host);

    // The table's tokens name nothing now; the host's do, for its agent.
    assert!(agent_run(&json!({"command": "true"}), call("t-pi")).unwrap_err().starts_with(NO_AGENT));
    let slow = agent_run(&json!({"command": "sleep 1; echo built", "wait": 0.2}), call(&token)).unwrap();
    assert_eq!((slow["running"].clone(), slow["agent"].clone()), (json!(true), json!(agent.to_string())), "{slow}");
    // A command whose end is carried by the agent's own later call is not owed a note.
    let waited = agent_run(&json!({"command": "sleep 1.5; echo waited", "wait": 0.1}), call(&token)).unwrap();
    let ended = agent_job(&json!({"job": waited["job"], "wait": 10}), call(&token)).unwrap();
    assert_eq!(ended["exit_code"], 0, "{ended}");

    // The first one finished after its call returned: its end rides into the agent's next
    // turn, once, as a note in the context. Asked again until it is there, not after a sleep
    // that a loaded machine could outlast.
    let notes_in = |handed: &Value| -> Vec<String> {
        let context: Value = handed["context"].as_str().map(|c| serde_json::from_str(c).unwrap()).unwrap_or_default();
        context["notes"].as_array().into_iter().flatten().map(|n| n.as_str().unwrap().to_string()).collect()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let notes = loop {
        let notes = notes_in(&turn("what happened?"));
        if !notes.is_empty() || std::time::Instant::now() > deadline {
            break notes;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(notes.len(), 1, "one late end, one note: {notes:?}");
    let note = &notes[0];
    for said in ["`sleep 1; echo built`", slow["job"].as_str().unwrap(), "exit code 0", "built"] {
        assert!(note.contains(said), "the note does not say {said:?}: {note}");
    }
    assert!(!note.contains(&token), "the note does not carry the token");
    assert!(notes_in(&turn("and now?")).is_empty(), "a note is delivered once");
}
