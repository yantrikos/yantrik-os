use std::path::PathBuf;
use std::sync::Mutex;

use super::*;
use crate::wire::onboarding::{CheckKind, Proof, Proofs};

const MARKER: &str = "MARKER-window-title-secret-9f3a";

/// Proofs that say what they were told and remember the order they were asked in.
struct Fake {
    log: Mutex<Vec<&'static str>>,
    remember: Proof,
    desktop: Proof,
}

impl Fake {
    fn passing() -> Self {
        Fake { log: Mutex::default(), remember: Proof::Passed { details: "wrote a note".into() }, desktop: Proof::Passed { details: "2 actions".into() } }
    }
}

impl Proofs for Fake {
    fn minds(&self) -> Vec<(String, String)> {
        vec![("companion".into(), "Yantrik Companion".into())]
    }
    fn checks(&self, _mind: &str) -> Vec<CheckKind> {
        CheckKind::ALL.to_vec()
    }
    fn remember(&self, _mind: &str) -> Proof {
        self.log.lock().unwrap().push("remember");
        self.remember.clone()
    }
    fn desktop(&self, _mind: &str) -> Proof {
        self.log.lock().unwrap().push("desktop");
        self.desktop.clone()
    }
}

/// Proofs that fail the test if anything asks them.
struct Forbidden;

impl Proofs for Forbidden {
    fn minds(&self) -> Vec<(String, String)> {
        panic!("the companion was asked");
    }
    fn checks(&self, _: &str) -> Vec<CheckKind> {
        panic!("the companion was asked");
    }
    fn remember(&self, _: &str) -> Proof {
        panic!("the companion was asked");
    }
    fn desktop(&self, _: &str) -> Proof {
        panic!("the companion was asked");
    }
}

fn passes(_: &str) -> Proof {
    Proof::Passed { details: "answered".into() }
}

fn forbidden_ask(_: &str) -> Proof {
    panic!("the companion was asked");
}

struct Says(Option<&'static str>);

impl Gate for Says {
    fn refusal(&self) -> Option<String> {
        self.0.map(String::from)
    }
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("prove-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_proofs_run_in_the_first_runs_order_and_each_state_and_the_count_are_said() {
    let fake = Fake { remember: Proof::Attention { reason: "the note it wrote did not come back".into(), details: String::new() }, ..Fake::passing() };
    let asked = Mutex::new(Vec::new());
    let ask = |mind: &str| {
        asked.lock().unwrap().push(mind.to_string());
        Proof::Passed { details: String::new() }
    };
    let report = run(&fake, &ask, "v0.1.0-2768-g39f6e8af", 1_700_000_000);
    assert_eq!(*asked.lock().unwrap(), vec!["companion"], "the answer proof is asked of the mind");
    assert_eq!(*fake.log.lock().unwrap(), vec!["remember", "desktop"], "remember, then desktop, after the answer");
    let ids: Vec<_> = report.lines.iter().map(|l| (l.id.as_str(), l.label.as_str(), l.state.as_str())).collect();
    assert_eq!(
        ids,
        vec![("answer", "Can it answer?", "passed"), ("remember", "Can it remember?", "attention"), ("desktop", "Can it reach the desktop?", "passed")]
    );
    assert_eq!(report.lines[1].words, "Needs attention: the note it wrote did not come back");
    assert_eq!(report.lines[0].words, "asked to use a tool; the answer was right");
    assert_eq!((report.passed(), report.mind.as_str(), report.build.as_str()), (2, "Yantrik Companion", "v0.1.0-2768-g39f6e8af"));
    let text = report.text();
    assert!(text.starts_with("What works on this computer right now"), "{text}");
    assert!(text.contains("build v0.1.0-2768-g39f6e8af") && text.contains("Yantrik Companion\n"), "{text}");
    assert!(text.contains("Can it remember?") && text.contains("Needs attention: the note it wrote did not come back"), "{text}");
    assert!(text.ends_with("2 of 3 proven."), "{text}");
    let all = run(&Fake::passing(), &passes, "b", 0);
    assert!(all.text().ends_with("3 of 3 proven."));
}

#[test]
fn nothing_a_proof_saw_is_in_the_report_the_reply_the_file_or_describe() {
    let planted = || Proof::Passed { details: format!("described {MARKER}") };
    let fake = Fake { remember: Proof::Passed { details: format!("note {MARKER}") }, desktop: planted(), ..Fake::passing() };
    let ask = |_: &str| Proof::Passed { details: format!("the model said {MARKER}") };
    let path = dir("marker").join("last.json");
    let prover = Prover::new(Some(path.clone()));
    let reply = prover.prove(&Says(None), &fake, &ask, "b", 100);
    assert_eq!(reply.state, "proven");
    for (what, held) in [
        ("the reply's text", reply.text.clone()),
        ("the reply's json", reply.json().to_string()),
        ("describe", prover.describe(160).to_string()),
        ("the kept file", std::fs::read_to_string(&path).unwrap()),
    ] {
        assert!(!held.contains(MARKER), "{what} holds what a proof saw: {held}");
    }
    // A failure's details are left out too, and its reason is cut before a quoted model reply.
    let failing = |_: &str| Proof::Attention {
        reason: format!("it answered, but not the question (\u{201c}{MARKER}\u{201d})"),
        details: MARKER.into(),
    };
    let reply = prover.prove(&Says(None), &Fake::passing(), &failing, "b", 200);
    assert_eq!(reply.report.as_ref().unwrap().lines[0].words, "Needs attention: it answered, but not the question");
    assert!(!reply.text.contains(MARKER) && !reply.json().to_string().contains(MARKER) && !prover.describe(300).to_string().contains(MARKER));
}

#[test]
fn a_second_ask_while_one_runs_starts_nothing_and_is_told_so_with_the_last_report() {
    let prover = Prover::new(None);
    prover.prove(&Says(None), &Fake::passing(), &passes, "b", 10);
    let during = Mutex::new(None);
    let ask = |_: &str| {
        // Asked from inside a running proof: the second ask.
        *during.lock().unwrap() = Some(prover.prove(&Says(None), &Forbidden, &forbidden_ask, "b", 20));
        Proof::Passed { details: String::new() }
    };
    let outer = prover.prove(&Says(None), &Fake::passing(), &ask, "b", 30);
    assert_eq!(outer.state, "proven");
    let second = during.lock().unwrap().take().unwrap();
    assert_eq!(second.state, "already-running");
    assert!(second.text.starts_with("Already running"), "{}", second.text);
    assert_eq!(second.report.as_ref().map(|r| r.at), Some(10), "the last report, not a new one");
    assert!(second.text.contains("3 of 3 proven."), "{}", second.text);
    // The turn was handed back.
    assert_eq!(prover.prove(&Says(None), &Fake::passing(), &passes, "b", 40).state, "proven");
}

#[test]
fn private_mode_and_no_provider_say_so_and_ask_and_write_nothing() {
    let path = dir("refused").join("last.json");
    let prover = Prover::new(Some(path.clone()));
    for words in ["Private mode is on, so nothing was asked.", "Yantrik Companion has no provider yet."] {
        let reply = prover.prove(&Says(Some(words)), &Forbidden, &forbidden_ask, "b", 1);
        assert_eq!(reply.state, "refused");
        assert!(reply.text.starts_with(words) && reply.text.contains("never been proven"), "{}", reply.text);
        assert!(!path.exists(), "a refused proof wrote a file");
        assert_eq!(prover.describe(2), serde_json::json!("never"));
    }
    // The shell's gate asks the two switches the shell already has, and says each in its own words.
    let gate = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/prove/gate.rs")).unwrap();
    assert!(gate.contains("crate::private_mode::is_on()") && gate.contains("crate::wire::ai_status::companion_has_provider()"));
}

#[test]
fn the_last_report_is_kept_private_and_read_back_and_a_broken_file_is_never() {
    use std::os::unix::fs::PermissionsExt;
    let path = dir("kept").join("state").join("last.json");
    let report = Prover::new(Some(path.clone())).prove(&Says(None), &Fake::passing(), &passes, "v1", 500).report.unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the file is the person's alone");
    let after_restart = Prover::new(Some(path.clone()));
    assert_eq!(after_restart.last(), Some(report));
    assert_eq!(after_restart.describe(560)["age_secs"], 60);
    for broken in [&b"{ not json"[..], b"", b"{\"at\": 1}"] {
        std::fs::write(&path, broken).unwrap();
        assert_eq!(Prover::new(Some(path.clone())).describe(0), serde_json::json!("never"), "{}", String::from_utf8_lossy(broken));
    }
    std::fs::write(&path, vec![b' '; 70_000]).unwrap();
    assert!(Prover::new(Some(path.clone())).last().is_none(), "a file too big to be a report is never");
    assert!(Prover::new(Some(path.with_file_name("absent.json"))).last().is_none());
}

#[test]
fn describe_says_when_how_old_how_many_and_each_line() {
    let prover = Prover::new(None);
    assert_eq!(prover.describe(0), serde_json::json!("never"));
    prover.prove(&Says(None), &Fake { desktop: Proof::Attention { reason: "the desktop did not answer it".into(), details: String::new() }, ..Fake::passing() }, &passes, "v1", 1_000);
    let d = prover.describe(1_090);
    assert_eq!((d["age_secs"].as_i64(), d["passed"].as_u64(), d["of"].as_u64()), (Some(90), Some(2), Some(3)));
    let lines = d["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2]["id"], "desktop");
    assert_eq!(lines[2]["state"], "attention");
    assert_eq!(lines[2]["words"], "Needs attention: the desktop did not answer it");
    assert!(d["at"].is_string());
}
