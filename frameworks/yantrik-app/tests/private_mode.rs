//! Private mode reaches every way an app's state leaves it (#924), read from the file the shell
//! publishes, as a shipped app reads it: through `TestApp` (the registry the runner serves) and
//! over a real socket (the door every caller comes in by, `yos` included).
//!
//! Its own test binary, so it can own `HOME` (where `privacy.json` is looked for) and
//! `XDG_RUNTIME_DIR` without moving them under any other test; every test here holds [`machine`]
//! for its whole run, so no two of them disagree about whether the person is private.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use yantrik_app::app::{App, Boot, Changed, Startup, WindowId};
use yantrik_app::control::private_mode::PRIVATE_SUMMARY;
use yantrik_surface::{WITHHELD, WITHHELD_REFUSAL};
use yantrik_app::control::{self, answer_later, Action, Param, View};
use yantrik_app::cx::Cx;
use yantrik_app::effects::Effects;
use yantrik_app::intents::{Intent, Intents};
use yantrik_app::refused::Refused;
use yantrik_ipc_transport::privacy;
use yantrik_testing::TestApp;

/// The person's own words, in the model, the summary, a key, a nested value and an act's reply.
const SENTINEL: &str = "Lunch with Priya SENTINEL-924";

fn leaks(answer: &Value) -> bool {
    let text = answer.to_string();
    text.contains("SENTINEL-924") || text.contains("Priya")
}

// ── The machine: one home, one runtime dir, one Private switch ──────

struct Machine {
    _held: MutexGuard<'static, ()>,
}

impl Machine {
    /// The person turns Private mode on or off, as the shell does: `privacy.json`, written whole.
    fn private(&self, on: bool) {
        privacy::publish(on, 1).expect("privacy.json written");
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = privacy::publish(false, 1);
    }
}

/// This binary's home and runtime dir, set once before anything reads them, and the lock that
/// keeps one test at a time in charge of the switch. Starts with Private mode off.
fn machine() -> Machine {
    static LOCK: Mutex<()> = Mutex::new(());
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    let held = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("yantrik-private-mode-{}", std::process::id()));
        std::fs::create_dir_all(root.join("home")).expect("a home of our own");
        std::fs::create_dir_all(root.join("run")).expect("a runtime dir of our own");
        std::env::set_var("HOME", root.join("home"));
        std::env::set_var("XDG_RUNTIME_DIR", root.join("run"));
        root
    });
    let machine = Machine { _held: held };
    machine.private(false);
    machine
}

// ── A toy app whose every word is the person's ──────────────────────

slint::slint! {
    export component DiaryWin inherits Window {
        in-out property <string> line;
    }
}

impl yantrik_app::Window for DiaryWin {
    fn make() -> Result<Self, slint::PlatformError> {
        DiaryWin::new()
    }
    fn apply_theme(&self, _: &yantrik_app::theme::ThemeSettings) {}
    fn connect_chrome(&self) {}
}

#[derive(Debug, Clone)]
enum Command {
    Write(String),
}

struct Diary;

impl App for Diary {
    type Ui = DiaryWin;
    type Model = Vec<String>;
    type Command = Command;
    type Settings = ();
    const ID: &'static str = "diary-test";

    fn model(_: &Boot) -> Result<Vec<String>, Startup> {
        Ok(vec![SENTINEL.to_string()])
    }

    fn wire(_: &Cx<Self>) {}

    fn update(m: &mut Vec<String>, command: Command, _: &mut Effects<Self>) -> Result<(), Refused> {
        let Command::Write(line) = command;
        m.push(line);
        Ok(())
    }

    fn render(ui: &DiaryWin, _: WindowId, m: &Vec<String>, _: &Changed) {
        ui.set_line(m.last().cloned().unwrap_or_default().into());
    }

    fn describe(m: &Vec<String>) -> View {
        View::new(format!("Diary — last: {}", m.last().cloned().unwrap_or_default()))
            .with("last", m.last().cloned())
            .with("entries", json!(m))
            .with("count", m.len())
            .with("unit", "lines")
    }

    fn intents(table: &mut Intents<Self>) {
        // One key that says nothing about the person; the rest stay the person's.
        table.shown_in_private(&["unit"]);
        table.add(
            Intent::new(Action::new("write", "Add a line").arg(Param::text("line")), |a| {
                Ok(Command::Write(a.text("line")?.to_string()))
            })
            .reply(|m: &Vec<String>| json!({ "count": m.len() })),
        );
    }
}

// ── Through the registry the runner serves ──────────────────────────

#[test]
fn with_private_mode_off_describe_is_unchanged() {
    let _machine = machine();
    let app = TestApp::<Diary>::start();
    let d = app.describe();
    assert_eq!(d["summary"], format!("Diary — last: {SENTINEL}"));
    assert_eq!(d["state"]["last"], SENTINEL);
    assert!(d.get("private").is_none() && d["state"].get("windows_open").is_none());
}

#[test]
fn in_private_mode_describe_says_the_app_is_there_and_nothing_it_holds() {
    let machine = machine();
    let app = TestApp::<Diary>::start();
    machine.private(true);
    let d = app.describe();
    assert_eq!(d["app"], "diary-test");
    assert_eq!(d["private"], true);
    assert_eq!(d["summary"], PRIVATE_SUMMARY);
    assert_eq!(
        d["state"],
        json!({ "private": true, "open": true, "windows_open": 1, "unit": "lines" }),
        "the window, the marker, the one key the app listed, and nothing else of it"
    );
    assert!(!leaks(&d), "{d}");
    assert_eq!(d["actions"].as_array().map(Vec::len), Some(2), "write, and the framework's launch");
}

#[test]
fn turning_private_mode_on_and_off_holds_from_the_next_call() {
    let machine = machine();
    let app = TestApp::<Diary>::start();
    let before = app.revision();
    machine.private(true);
    assert_eq!(app.describe()["private"], true, "no restart, no refresh, no cache");
    // What the person writes meanwhile does not move the revision a private caller reads.
    let private_revision = app.revision();
    app.run(Command::Write(format!("{SENTINEL} again")));
    assert_eq!(app.revision(), private_revision);
    let reply = app.act("write", json!({ "line": "x" })).unwrap();
    assert!(!leaks(reply.envelope()) && reply.envelope()["private"] == true, "{:?}", reply.envelope());
    machine.private(false);
    let d = app.describe();
    assert_eq!(d["state"]["count"], 3);
    assert!(d.get("private").is_none());
    assert_ne!(app.revision(), before);
}

#[test]
fn a_privacy_file_that_cannot_be_read_hides_the_app_too() {
    let _machine = machine();
    let app = TestApp::<Diary>::start();
    let file = privacy::privacy_path().expect("a home");
    std::fs::write(&file, "{ not json").unwrap();
    assert_eq!(app.describe()["private"], true, "unclear is private, never the other way");
}

// ── Through the socket: `yos describe`, a harness, anything that connects ──

const SURFACE: &str = "private-door-test";

/// Set by a test: the next describe turns Private mode on while it runs on the UI thread.
static FLIP_IN_DESCRIBE: AtomicBool = AtomicBool::new(false);

/// A window surface on this runtime, served on the test UI thread, as a shipped window serves it.
fn door() -> &'static str {
    static ADDRESS: OnceLock<String> = OnceLock::new();
    ADDRESS.get_or_init(|| {
        // The headless platform and its UI thread, which the hop to the window posts to.
        drop(TestApp::<Diary>::start());
        slint::invoke_from_event_loop(|| {
            control::App::new(SURFACE)
                .describe(|| {
                    if FLIP_IN_DESCRIBE.swap(false, Ordering::SeqCst) {
                        privacy::publish(true, 1).expect("privacy.json written");
                    }
                    View::new(format!("Inbox — {SENTINEL}")).with("subject", SENTINEL)
                })
                .action(
                    Action::new("mark_read", "Mark the open message read").risk("safe"),
                    |_| Ok(json!({ "marked": true })),
                )
                .action(
                    Action::new("read_open", "Answer with the open message").risk("safe"),
                    |_| Ok(json!({ "subject": SENTINEL, "body": format!("see you at 4, {SENTINEL}") })),
                )
                .action(
                    Action::new("open_message", "Open a message by id").risk("safe").arg(Param::text("id")),
                    |_| Err(format!("no such message; the open one is {SENTINEL}")),
                )
                .action(
                    Action::new("go_private", "Turn Private mode on, as the person might mid-call").risk("safe"),
                    |_| {
                        privacy::publish(true, 1).map_err(|e| e.to_string())?;
                        Ok(json!({ "ok": true }))
                    },
                )
                .action(
                    Action::new("sync", "Fetch mail, off the UI thread").risk("safe"),
                    |_| {
                        // The person turns Private mode on while the work runs.
                        let work = || {
                            privacy::publish(true, 1).map_err(|e| e.to_string())?;
                            Ok(json!({ "fetched": 1 }))
                        };
                        answer_later(work).map(|()| json!("replaced")).or_else(|work| work())
                    },
                )
                .serve();
        })
        .expect("the UI thread takes the surface");
        let address = yantrik_ipc_transport::server::RpcServer::default_address(&control::service_id_for(SURFACE));
        for _ in 0..500 {
            if UnixStream::connect(&address).is_ok() {
                return address;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("nothing bound {address}");
    })
}

fn call(method: &str, params: Value) -> Value {
    let mut socket = UnixStream::connect(door()).expect("connect");
    let request = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    socket.write_all(format!("{request}\n").as_bytes()).expect("write");
    let mut line = String::new();
    BufReader::new(socket).read_line(&mut line).expect("read");
    serde_json::from_str(&line).expect(&line)
}

#[test]
fn the_socket_door_reads_private_mode_on_every_call() {
    let machine = machine();
    let open = call("app.describe", json!({}))["result"].clone();
    assert_eq!(open["state"]["subject"], SENTINEL);
    machine.private(true);
    let hidden = call("app.describe", json!({}))["result"].clone();
    assert_eq!(hidden["private"], true);
    assert_eq!(hidden["summary"], PRIVATE_SUMMARY);
    assert_eq!(hidden["state"], json!({ "private": true }), "a window app off the framework says no more");
    assert!(!leaks(&hidden), "{hidden}");
    machine.private(false);
    assert_eq!(call("app.describe", json!({}))["result"]["state"]["subject"], SENTINEL);
}

#[test]
fn an_act_on_the_socket_in_private_mode_replies_and_refuses_with_nothing_the_app_holds() {
    let machine = machine();
    let real = call("app.describe", json!({}))["result"]["revision"].clone();
    machine.private(true);
    // The person's own call (no token) still runs; its reply carries the private view.
    let reply = call("app.act", json!({ "action": "mark_read", "args": {} }));
    assert_eq!(reply["result"]["accepted"], true, "{reply}");
    assert_eq!(reply["result"]["private"], true);
    assert!(!leaks(&reply), "{reply}");
    // A guard on a revision a private caller could not have read is refused without the summary.
    let stale = call("app.act", json!({ "action": "mark_read", "args": {}, "expect_revision": real }));
    assert!(stale["error"]["message"].as_str().unwrap_or_default().starts_with("STALE:"), "{stale}");
    assert!(!leaks(&stale), "{stale}");
    // An agent is refused outright, as before.
    let agent = call("app.act", json!({ "action": "mark_read", "args": {}, "agent_token": "t" }));
    assert_eq!(agent["error"]["message"], privacy::REFUSAL);
}

#[test]
fn work_finished_later_is_read_back_under_private_mode_as_it_is_then() {
    let _machine = machine();
    let reply = call("app.act", json!({ "action": "sync", "args": {} }));
    // The work ran; what it answered is not this unvouched caller's to read.
    assert_eq!(reply["result"]["result"], Value::Null, "{reply}");
    assert_eq!(reply["result"]["withheld"], WITHHELD);
    assert_eq!(reply["result"]["private"], true, "Private mode came on while the work ran: {reply}");
    assert_eq!(reply["result"]["summary"], PRIVATE_SUMMARY);
    assert!(!leaks(&reply), "{reply}");
}

/// #926 review, L1: the person turns Private mode on while the call is on the UI thread. The door
/// reads it again once the answer is back, so what was built before the flip is not what goes out.
#[test]
fn private_mode_turned_on_during_a_call_holds_for_that_calls_answer() {
    let _machine = machine();
    let acted = call("app.act", json!({ "action": "go_private", "args": {} }));
    assert_eq!(acted["result"]["private"], true, "{acted}");
    assert_eq!(acted["result"]["summary"], PRIVATE_SUMMARY);
    assert!(!leaks(&acted), "{acted}");

    privacy::publish(false, 1).unwrap();
    FLIP_IN_DESCRIBE.store(true, Ordering::SeqCst);
    let described = call("app.describe", json!({}));
    assert_eq!(described["result"]["private"], true, "{described}");
    assert_eq!(described["result"]["state"], json!({ "private": true }));
    assert!(!leaks(&described), "{described}");
}

/// #926 review, M1: in Private mode the action still runs for a caller the kernel does not vouch
/// for (this test binary is not installed), but its answer is withheld and a handler's refusal is
/// a neutral sentence; with Private mode off both are the app's own.
#[test]
fn an_unvouched_caller_on_the_socket_gets_neither_the_result_nor_the_refusal_text() {
    let machine = machine();
    let open = call("app.act", json!({ "action": "read_open", "args": {} }));
    assert_eq!(open["result"]["result"]["subject"], SENTINEL, "off, the answer is the app's");
    machine.private(true);
    let read = call("app.act", json!({ "action": "read_open", "args": {} }));
    assert_eq!(read["result"]["accepted"], true, "{read}");
    assert_eq!(read["result"]["result"], Value::Null);
    assert_eq!(read["result"]["withheld"], WITHHELD);
    assert!(!leaks(&read), "{read}");
    let refused = call("app.act", json!({ "action": "open_message", "args": { "id": "m1" } }));
    assert_eq!(refused["error"]["message"], WITHHELD_REFUSAL, "{refused}");
    assert!(!leaks(&refused), "{refused}");
    machine.private(false);
    let why = call("app.act", json!({ "action": "open_message", "args": { "id": "m1" } }));
    assert!(leaks(&why), "off, the app's own refusal: {why}");
}

/// #926 review, M1: a second launch while the person is private reads the `private` marker, not a
/// `result` it may not be shown, and says the launch was handed over. (This running copy has no
/// window, so with Private mode off it truthfully says so.)
#[test]
fn a_second_launch_while_private_is_handed_over_and_says_so() {
    use yantrik_app::present::{self, Presented};
    let machine = machine();
    door();
    assert_eq!(present::ask(SURFACE), Presented::NoWindow, "off, the running copy's own answer");
    machine.private(true);
    let answered = present::ask(SURFACE);
    assert_eq!(answered, Presented::Shown, "{answered:?}");
    assert_eq!(answered.sentence(SURFACE), None, "nothing false is printed for the person");
}
