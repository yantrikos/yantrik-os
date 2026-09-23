//! #189, item 2: a door that cannot learn what an agent token is refuses the call.
//!
//! The reach used to be a file every door read, and a missing file was no reach for anyone —
//! anything running as the person could delete it, and the next act carrying the token went
//! through unheld. The shell keeps it now, and a door asks the shell. This is that, end to end,
//! from a door's side: a process that keeps no reach of its own (the shell's store is never
//! installed here, which is why this is a test binary of its own), on a machine of its own —
//! `HOME` and `XDG_RUNTIME_DIR` in a temporary directory — whose shell is one this test starts, as
//! a copy of this very binary named `yantrik-ui`, or none at all.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use yantrik_ipc_transport::reach::{self, token_digest, Reach, Standing};
use yantrik_ipc_transport::server::{RpcServer, ServiceHandler};
use yantrik_ipc_transport::SyncRpcClient;
use yantrik_surface::gate::{Authority, Mode};
use yantrik_surface::{Action, ServiceError, Surface, View};

/// Set in the copy this test starts as the shell: run [`stand_in_shell`] and nothing else.
const STAND_IN: &str = "YANTRIK_TEST_STAND_IN_SHELL";

/// The machine every test here runs on, made once before anything reads the environment.
fn machine() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Ok(root) = std::env::var(STAND_IN) {
            return PathBuf::from(root);
        }
        let root = std::env::temp_dir().join(format!("yantrik-fail-closed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("home/.config/yantrik")).unwrap();
        std::fs::create_dir_all(root.join("run")).unwrap();
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("XDG_RUNTIME_DIR", root.join("run"));
        root
    })
}

/// The Reviewer the stand-in shell holds one token to.
fn reviewer() -> Reach {
    Reach {
        agent: "deepseek:c-rev1".into(),
        role: "reviewer".into(),
        name: "Reviewer".into(),
        surfaces: vec!["counter.who".into()],
        ceiling: "safe".into(),
    }
}

/// What the stand-in shell knows: a Reviewer, a plain agent, and no other live agent.
fn standing(digest: &str) -> Standing {
    if digest == token_digest("tok-reviewer") {
        Standing::Held(reviewer())
    } else if digest == token_digest("tok-plain") {
        Standing::Plain
    } else {
        Standing::Unknown
    }
}

/// A door: a counter a mind can add to (`standard`) and ask who it is (`safe`).
fn counter() -> Surface {
    Surface::new("counter")
        .describe(|| View::new("Counter"))
        .action(Action::new("increment", "Add one"), |_| Ok(json!({ "count": 1 })))
        .action(Action::new("who", "Say who is asking").risk("safe"), |_| {
            Ok(json!({ "agent_token": yantrik_surface::agent_token() }))
        })
}

/// One call to the door, on a machine at the shipped ceiling in ask mode.
fn act(door: &Surface, action: &str, token: Option<&str>) -> Result<Value, ServiceError> {
    let mut params = json!({ "action": action, "args": {} });
    if let Some(token) = token {
        params["agent_token"] = token.into();
    }
    door.act(&params, None, Authority { ceiling: "sensitive".into(), mode: Mode::named("ask"), granted: false })
}

fn refusal(answer: Result<Value, ServiceError>) -> String {
    let err = answer.expect_err("refused");
    assert_eq!(err.code, -32602, "a policy answer, not a transport fault: {}", err.message);
    err.message
}

/// The shell, played by a copy of this binary under the shell's name, so the door's check of
/// who is answering (`reach::answered_by_the_shell`) is met as the real shell meets it.
struct StandInShell(Child);

impl StandInShell {
    fn start() -> StandInShell {
        let root = machine();
        let exe = root.join("bin/yantrik-ui");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &exe).expect("a copy of this binary named yantrik-ui");
        let child = Command::new(&exe)
            .args(["stand_in_shell", "--exact", "--nocapture", "--test-threads=1"])
            .env(STAND_IN, root)
            .env("HOME", root.join("home"))
            .env("XDG_RUNTIME_DIR", root.join("run"))
            .spawn()
            .expect("start the stand-in shell");
        let address = RpcServer::default_address("app-shell");
        let deadline = Instant::now() + Duration::from_secs(20);
        while std::os::unix::net::UnixStream::connect(&address).is_err() {
            assert!(Instant::now() < deadline, "the stand-in shell never bound {address}");
            std::thread::sleep(Duration::from_millis(30));
        }
        // A door that failed to reach the shell a moment ago would wait out its breaker first.
        SyncRpcClient::clear_breaker(&address);
        StandInShell(child)
    }
}

impl Drop for StandInShell {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = std::fs::remove_file(RpcServer::default_address("app-shell"));
    }
}

/// The stand-in shell's whole life: keep the agents' standing, and answer `agent.reach` on
/// `app-shell.sock` with it, as the shell's dispatch does. A no-op in an ordinary run.
#[test]
fn stand_in_shell() {
    if std::env::var(STAND_IN).is_err() {
        return;
    }
    machine();
    reach::keep_reach_with(standing);
    struct Shell;
    impl ServiceHandler for Shell {
        fn service_id(&self) -> &str {
            "app-shell"
        }
        fn handle(&self, method: &str, params: Value) -> Result<Value, ServiceError> {
            match method {
                reach::ASK => reach::answer(&params).map_err(|message| ServiceError { code: -32602, message }),
                other => Err(ServiceError { code: -32601, message: format!("unknown method `{other}`") }),
            }
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let _ = runtime.block_on(RpcServer::new(&RpcServer::default_address("app-shell")).serve(Arc::new(Shell)));
}

/// With no shell to ask — and no reach file either, or a forged one — a call carrying a token is
/// refused in a sentence that says why, whatever it asks for. The person's own call, with no
/// token, asks nobody and runs. Then a shell answers: its Reviewer is held to its reach, a plain
/// agent is not held, and a token no live agent carries is refused. The shell goes, and the door
/// is closed again.
#[test]
fn a_door_that_cannot_ask_the_shell_refuses_every_token_and_nothing_else() {
    if std::env::var(STAND_IN).is_ok() {
        return;
    }
    let root = machine();
    let door = counter();

    // No shell, and the file the reach used to live in is gone.
    let reach_file = root.join("home/.config/yantrik/agent-reach.json");
    assert!(!reach_file.exists());
    for token in ["tok-reviewer", "tok-plain", "tok-anyone"] {
        for action in ["increment", "who", "no_such_action"] {
            let why = refusal(act(&door, action, Some(token)));
            assert!(
                why.starts_with("REACH: the shell, which keeps every agent's reach, did not answer ("),
                "{token} {action}: {why}"
            );
            assert!(why.contains("app-shell.sock"), "it names where it asked: {why}");
            assert!(why.ends_with("so no act carrying an agent token runs until it does. Nothing was run."), "{why}");
        }
    }
    assert_eq!(act(&door, "increment", None).expect("the person's own call")["accepted"], true);

    // A file claiming anything is not read: it is not where the reach is kept.
    let forged = json!({ "agents": [{ "token_sha256": token_digest("tok-anyone"), "agent": "x", "role": "x",
        "name": "Anyone", "surfaces": ["counter"], "ceiling": "dangerous" }] });
    std::fs::write(&reach_file, forged.to_string()).unwrap();
    assert!(refusal(act(&door, "increment", Some("tok-anyone"))).starts_with("REACH: the shell"));

    // The shell answers.
    let shell = StandInShell::start();
    let answer = act(&door, "who", Some("tok-reviewer")).expect("on its surfaces it runs");
    assert_eq!(answer["result"]["agent_token"], "tok-reviewer");
    let why = refusal(act(&door, "increment", Some("tok-reviewer")));
    assert!(why.starts_with("REACH: counter.increment is outside the Reviewer's reach"), "{why}");
    assert_eq!(act(&door, "increment", Some("tok-plain")).expect("a plain agent")["accepted"], true);
    let why = refusal(act(&door, "increment", Some("tok-anyone")));
    assert!(why.starts_with("REACH: the agent token this call carries names no live agent"), "{why}");
    assert_eq!(act(&door, "increment", None).expect("the person's own call")["accepted"], true);

    // What it costs a token-carrying call: one question on the shell's socket.
    let asked = Instant::now();
    for _ in 0..200 {
        act(&door, "increment", Some("tok-plain")).expect("a plain agent");
    }
    eprintln!("200 token-carrying acts, each asking the shell: {:?} in all", asked.elapsed());

    // The shell goes — restarting, or killed — and the door is closed again at once.
    drop(shell);
    let why = refusal(act(&door, "who", Some("tok-reviewer")));
    assert!(why.starts_with("REACH: the shell, which keeps every agent's reach, did not answer ("), "{why}");
    assert_eq!(act(&door, "increment", None).expect("the person's own call")["accepted"], true);
    let _ = std::fs::remove_dir_all(root);
}
