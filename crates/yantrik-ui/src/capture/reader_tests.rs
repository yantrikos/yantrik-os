//! The kept reader, against stand-ins that behave the ways a reader can.

use super::*;

/// A stand-in reader: a shell that does `script` for each request line it is sent.
fn stand_in(script: &'static str) -> impl FnOnce() -> Command {
    move || {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd
    }
}

/// Takes the request's `id` out of the line in `$line`, into `$id`.
macro_rules! take_id {
    () => {
        r#"id=${line#*\"id\":}; id=${id%%[,\}]*};"#
    };
}

/// Answers every request, with its id and its own process id, so a test can tell one reader
/// from the next.
const ANSWERS: &str = concat!("while read line; do ", take_id!(), r#" echo "{\"id\": $id, \"lines\": [], \"pid\": $$}"; done"#);

/// Ask as a read that began now.
fn ask(keeper: &Keeper, command: impl FnOnce() -> Command, limit: Duration) -> Result<serde_json::Value, String> {
    keeper.ask(command, &look(), limit, keeper.epoch())
}

fn look() -> serde_json::Value {
    serde_json::json!({ "image": "/nowhere.ppm", "space": "desktop" })
}

const SOON: Duration = Duration::from_secs(5);

#[test]
fn the_reader_is_started_once_and_asked_again() {
    let keeper = Keeper::default();
    let first = ask(&keeper, stand_in(ANSWERS), SOON).expect("an answer");
    let second = ask(&keeper, || unreachable!("a reader is already running"), SOON).expect("an answer");
    assert!(first["pid"].is_u64());
    assert_eq!(first["pid"], second["pid"], "the same reader answered both");
    keeper.forget();
}

#[test]
fn a_reader_that_does_not_answer_in_time_is_ended_and_the_next_read_starts_another() {
    let keeper = Keeper::default();
    let err = ask(&keeper, stand_in("exec sleep 60"), Duration::from_millis(200)).unwrap_err();
    assert!(err.contains("did not finish"), "{err}");
    assert!(held(&keeper.kept).is_none(), "the late reader is not kept");
    let answer = ask(&keeper, stand_in(ANSWERS), SOON).expect("a new reader answers");
    assert!(answer["pid"].is_u64());
    keeper.forget();
}

#[test]
fn a_reader_that_stops_is_replaced_by_the_next_read() {
    let keeper = Keeper::default();
    let err = ask(&keeper, stand_in("read line; exit 0"), SOON).unwrap_err();
    assert!(err.contains("stopped before it answered"), "{err}");
    assert!(ask(&keeper, stand_in(ANSWERS), SOON).is_ok());
    keeper.forget();
}

/// The reader left by itself (it does after two minutes unasked): the next read starts one.
#[test]
fn a_reader_that_left_by_itself_is_started_again() {
    let keeper = Keeper::default();
    let once = concat!("read line; ", take_id!(), r#" echo "{\"id\": $id, \"lines\": [], \"pid\": $$}""#);
    let first = ask(&keeper, stand_in(once), SOON).expect("an answer");
    // It has answered once and exited. Give it a moment to be gone.
    std::thread::sleep(Duration::from_millis(200));
    let second = ask(&keeper, stand_in(ANSWERS), SOON).expect("an answer from a new reader");
    assert_ne!(first["pid"], second["pid"]);
    keeper.forget();
}

#[test]
fn forgetting_ends_the_reader_and_the_next_read_starts_a_new_one() {
    let keeper = Keeper::default();
    let first = ask(&keeper, stand_in(ANSWERS), SOON).expect("an answer");
    keeper.forget();
    let second = ask(&keeper, stand_in(ANSWERS), SOON).expect("an answer");
    assert_ne!(first["pid"], second["pid"], "what the first one remembered went with it");
    keeper.forget();
}

/// The screen locks, or the person goes private, while a read is running: forgetting does not
/// wait for the read, and the read does not come back with an answer.
#[test]
fn forgetting_does_not_wait_for_a_read_in_flight_and_that_read_fails() {
    let keeper = std::sync::Arc::new(Keeper::default());
    let reading = {
        let keeper = std::sync::Arc::clone(&keeper);
        std::thread::spawn(move || ask(&keeper, stand_in("exec sleep 60"), Duration::from_secs(30)))
    };
    // Until the read has started its reader there is nothing to forget.
    let started = std::time::Instant::now();
    while held(&keeper.child).is_none() && started.elapsed() < SOON {
        std::thread::sleep(Duration::from_millis(10));
    }
    let asked = std::time::Instant::now();
    keeper.forget();
    assert!(asked.elapsed() < Duration::from_secs(1), "forgetting returned at once");
    let err = reading.join().expect("the reading thread").unwrap_err();
    assert!(err.contains("stopped before it answered"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(10), "the read ended when its reader did, not at its limit");
}

#[test]
fn an_answer_that_is_an_error_is_an_error_and_the_reader_stays() {
    let keeper = Keeper::default();
    let refuses = concat!("while read line; do ", take_id!(), r#" echo "{\"id\": $id, \"error\": \"cannot read /nowhere.ppm\"}"; done"#);
    let err = ask(&keeper, stand_in(refuses), SOON).unwrap_err();
    assert!(err.contains("cannot read /nowhere.ppm"), "{err}");
    assert!(held(&keeper.kept).is_some(), "a reader that refused one image is still a reader");
    keeper.forget();
}

#[test]
fn a_reader_that_cannot_be_started_says_so() {
    let keeper = Keeper::default();
    let err = ask(&keeper, || Command::new("/nonexistent/yantrik-ocr"), SOON).unwrap_err();
    assert!(err.contains("could not start the text reader"), "{err}");
}

#[test]
fn the_desktops_reader_is_told_to_serve_and_when_to_leave() {
    let cmd = serving(Path::new("/opt/yantrik/bin/yantrik-ocr"));
    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_string_lossy().to_string()).collect();
    assert_eq!(args, ["--idle-secs", "120", "--serve"]);
}

/// Anything else on the reader's output, however it is shaped, is not the answer.
#[test]
fn a_stray_line_on_the_readers_output_is_never_taken_for_an_answer() {
    let keeper = Keeper::default();
    let strays = concat!(
        "while read line; do ",
        take_id!(),
        r#" echo "Processing group 1"; echo '{"id": 0, "lines": ["somebody else"], "pid": 0}'; echo "{\"id\": $id, \"lines\": [], \"pid\": $$, \"n\": $id}"; done"#
    );
    let first = ask(&keeper, stand_in(strays), SOON).expect("an answer");
    let second = ask(&keeper, || unreachable!("running"), SOON).expect("an answer");
    assert_eq!(first["lines"], serde_json::json!([]), "not the other asker's lines");
    assert_ne!(first["n"], second["n"], "each answer is its own request's");
    assert_eq!(second["n"].as_u64(), first["n"].as_u64().map(|n| n + 1));
    keeper.forget();
}

/// A reader that exits by itself is collected at once, not at the next read.
#[test]
fn a_reader_that_left_by_itself_does_not_stay_a_zombie() {
    let keeper = Keeper::default();
    let once = concat!("read line; ", take_id!(), r#" echo "{\"id\": $id, \"lines\": [], \"pid\": $$}""#);
    let answer = ask(&keeper, stand_in(once), SOON).expect("an answer");
    let proc = format!("/proc/{}", answer["pid"]);
    let started = std::time::Instant::now();
    while Path::new(&proc).exists() && started.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!Path::new(&proc).exists(), "{proc} is still there (a zombie keeps its entry)");
    keeper.forget();
}

/// The reader left in the instant before the request reached it: the read is tried once more.
#[test]
fn a_reader_that_left_as_it_was_asked_is_replaced_and_the_read_goes_through() {
    let keeper = Keeper::default();
    // Answers the first request, and ends on the second without answering it.
    let one_then_gone = concat!("read line; ", take_id!(), r#" echo "{\"id\": $id, \"lines\": [], \"pid\": $$}"; read line; exit 0"#);
    let first = ask(&keeper, stand_in(one_then_gone), SOON).expect("an answer");
    let second = ask(&keeper, stand_in(ANSWERS), SOON).expect("the read goes to a new reader");
    assert_ne!(first["pid"], second["pid"]);
    keeper.forget();
}

#[test]
fn a_reader_that_cannot_answer_twice_in_a_row_fails_the_read() {
    let keeper = Keeper::default();
    let err = ask(&keeper, stand_in("read line; exit 0"), SOON).unwrap_err();
    assert!(err.contains("stopped before it answered"), "{err}");
}

/// The lock, or Private mode, comes after the read was asked for and before its reader starts.
#[test]
fn a_forget_before_the_reader_starts_fails_the_read_and_leaves_no_reader() {
    let keeper = Keeper::default();
    let epoch = keeper.epoch();
    keeper.forget();
    let err = keeper.ask(|| unreachable!("no reader is started for a read that was forgotten"), &look(), SOON, epoch).unwrap_err();
    assert!(err.contains("nothing was read"), "{err}");
    assert!(held(&keeper.kept).is_none() && held(&keeper.child).is_none(), "no reader was left");
    // A read asked for after it is a new read.
    assert!(ask(&keeper, stand_in(ANSWERS), SOON).is_ok());
    keeper.forget();
}

/// The reader answered, and then the screen locked: the answer is of what was on screen before.
#[test]
fn a_forget_after_the_answer_arrived_still_fails_the_read() {
    let keeper = Keeper::default();
    let epoch = keeper.epoch();
    let err = keeper.ask_then(stand_in(ANSWERS), &look(), SOON, epoch, || keeper.forget()).unwrap_err();
    assert!(err.contains("nothing was read"), "{err}");
    assert!(held(&keeper.kept).is_none(), "the reader that held what it saw was ended");
    assert!(held(&keeper.child).is_none());
}

#[test]
fn what_the_reader_said_before_it_stopped_is_in_the_error() {
    let keeper = Keeper::default();
    let err = ask(&keeper, stand_in("echo 'yantrik-ocr: cannot load the OCR model /opt/yantrik/share/ocr/x.rten: boom' >&2; exit 1"), SOON).unwrap_err();
    assert!(err.contains("boom"), "{err}");
    assert!(err.contains("/opt/yantrik/share/ocr/x.rten"), "{err}");
}

#[test]
fn a_line_it_said_is_kept_to_a_bound_in_the_error() {
    let keeper = Keeper::default();
    let err = ask(&keeper, stand_in("head -c 200000 /dev/zero | tr '\\0' x >&2; echo >&2; exit 1"), SOON).unwrap_err();
    assert!(err.contains("xxx") && err.len() < 600, "{} bytes", err.len());
}

#[test]
fn the_address_space_ceiling_is_set_in_the_reader() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("yantrik-reader-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    let bin = dir.join("yantrik-ocr");
    std::fs::write(&bin, "#!/bin/sh\nulimit -v\n").expect("a stand-in");
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("executable");
    let out = serving(&bin).output().expect("it runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), (READER_MEMORY / 1024).to_string());
}

/// The one-line hooks have no way to be run in a test; they are checked where they stand.
#[test]
fn the_lock_and_private_mode_end_the_reader_when_they_go_on_and_off() {
    let private = include_str!("../private_mode.rs");
    let enforce = private.split("pub fn enforce").nth(1).and_then(|rest| rest.split("\n}\n").next()).expect("enforce");
    let (hook, branch) = (enforce.find("crate::capture::reader::forget();"), enforce.find("if on {"));
    assert!(hook.is_some() && hook < branch, "forget runs whether Private mode went on or off, not inside `if on`");
    let lock = include_str!("../session_lock.rs");
    let engage = lock.split("pub fn engage").nth(1).and_then(|rest| rest.split("\n}\n").next()).expect("engage");
    assert!(engage.contains("crate::capture::reader::forget();"), "locking ends the reader");
}
