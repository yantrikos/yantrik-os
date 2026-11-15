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

/// Answers every request with its own process id, so a test can tell one reader from the next.
const ANSWERS: &str = r#"while read line; do echo "{\"lines\": [], \"pid\": $$}"; done"#;

fn look() -> serde_json::Value {
    serde_json::json!({ "image": "/nowhere.ppm", "space": "desktop" })
}

const SOON: Duration = Duration::from_secs(5);

#[test]
fn the_reader_is_started_once_and_asked_again() {
    let keeper = Keeper::default();
    let first = keeper.ask(stand_in(ANSWERS), &look(), SOON).expect("an answer");
    let second = keeper.ask(|| unreachable!("a reader is already running"), &look(), SOON).expect("an answer");
    assert!(first["pid"].is_u64());
    assert_eq!(first["pid"], second["pid"], "the same reader answered both");
    keeper.forget();
}

#[test]
fn a_reader_that_does_not_answer_in_time_is_ended_and_the_next_read_starts_another() {
    let keeper = Keeper::default();
    let err = keeper.ask(stand_in("exec sleep 60"), &look(), Duration::from_millis(200)).unwrap_err();
    assert!(err.contains("did not finish"), "{err}");
    assert!(held(&keeper.kept).is_none(), "the late reader is not kept");
    let answer = keeper.ask(stand_in(ANSWERS), &look(), SOON).expect("a new reader answers");
    assert!(answer["pid"].is_u64());
    keeper.forget();
}

#[test]
fn a_reader_that_stops_is_replaced_by_the_next_read() {
    let keeper = Keeper::default();
    let err = keeper.ask(stand_in("read line; exit 0"), &look(), SOON).unwrap_err();
    assert!(err.contains("stopped before it answered"), "{err}");
    assert!(keeper.ask(stand_in(ANSWERS), &look(), SOON).is_ok());
    keeper.forget();
}

/// The reader left by itself (it does after two minutes unasked): the next read starts one.
#[test]
fn a_reader_that_left_by_itself_is_started_again() {
    let keeper = Keeper::default();
    let once = r#"read line; echo "{\"lines\": [], \"pid\": $$}""#;
    let first = keeper.ask(stand_in(once), &look(), SOON).expect("an answer");
    // It has answered once and exited. Give it a moment to be gone.
    std::thread::sleep(Duration::from_millis(200));
    let second = keeper.ask(stand_in(ANSWERS), &look(), SOON).expect("an answer from a new reader");
    assert_ne!(first["pid"], second["pid"]);
    keeper.forget();
}

#[test]
fn forgetting_ends_the_reader_and_the_next_read_starts_a_new_one() {
    let keeper = Keeper::default();
    let first = keeper.ask(stand_in(ANSWERS), &look(), SOON).expect("an answer");
    keeper.forget();
    let second = keeper.ask(stand_in(ANSWERS), &look(), SOON).expect("an answer");
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
        std::thread::spawn(move || keeper.ask(stand_in("exec sleep 60"), &look(), Duration::from_secs(30)))
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
    let refuses = r#"while read line; do echo "{\"error\": \"cannot read /nowhere.ppm\"}"; done"#;
    let err = keeper.ask(stand_in(refuses), &look(), SOON).unwrap_err();
    assert!(err.contains("cannot read /nowhere.ppm"), "{err}");
    assert!(held(&keeper.kept).is_some(), "a reader that refused one image is still a reader");
    keeper.forget();
}

#[test]
fn a_reader_that_cannot_be_started_says_so() {
    let keeper = Keeper::default();
    let err = keeper.ask(|| Command::new("/nonexistent/yantrik-ocr"), &look(), SOON).unwrap_err();
    assert!(err.contains("could not start the text reader"), "{err}");
}

#[test]
fn the_desktops_reader_is_told_to_serve_and_when_to_leave() {
    let cmd = serving(Path::new("/opt/yantrik/bin/yantrik-ocr"));
    let args: Vec<_> = cmd.get_args().map(|arg| arg.to_string_lossy().to_string()).collect();
    assert_eq!(args, ["--idle-secs", "120", "--serve"]);
}
