//! A snippet may be a password, a key or a token. Its code goes to the store's file, to the
//! clipboard when the person or a mind copies it, and to the draft lane while it is typed; never
//! to a log line, an answer, a notice, `describe` past the list's one-line preview, or a debug
//! print. A sentinel on the code's second line (the preview is the first) is followed through
//! everything the app does.

use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};
use yantrik_app::app::WindowId;

use super::open;

const SENTINEL: &str = "SENTINEL-c0de-5ecret";

/// Every log line any test in this process writes, at every level.
fn captured() -> Arc<Mutex<Vec<u8>>> {
    static LOG: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let log = Arc::new(Mutex::new(Vec::new()));
        let sink = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || Sink(sink.clone()))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        log
    })
    .clone()
}

struct Sink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn leaks(value: &Value) -> bool {
    value.to_string().contains(SENTINEL)
}

#[test]
fn a_snippets_code_reaches_no_log_answer_notice_or_describe() {
    let log = captured();
    tracing::info!("the capture is listening");
    let code = format!("# deploy\nexport TOKEN={SENTINEL}");
    let f = open();
    let mut answers = Vec::new();

    answers.push(f.act("new", json!({ "title": "Deploy token", "language": "Shell", "code": code })).unwrap());
    let id = answers[0]["id"].as_i64().unwrap() as i32;
    answers.push(f.act("open", json!({ "id": id.to_string() })).unwrap());
    answers.push(f.act("search", json!({ "query": "deploy" })).unwrap());
    answers.push(f.act("copy", json!({ "id": id.to_string() })).unwrap());
    answers.push(f.act("toggle_favorite", json!({ "id": id.to_string() })).unwrap());
    answers.push(f.act("save", json!({ "id": id.to_string(), "code": format!("{code}\n# {SENTINEL} again") })).unwrap());
    answers.push(f.act("show", json!({})).unwrap());
    assert!(!leaks(&f.describe()), "{}", f.describe());

    // The person's own path: typing, the buttons, a write the disk refuses, an export.
    f.type_into("Deploy token", &format!("{code}\n# typed {SENTINEL}"));
    f.app.ui(move |ui| ui.invoke_snip_copy(id));
    f.quiet();
    f.disk.refuse(true);
    f.type_into("Deploy token", &format!("{code}\n# refused {SENTINEL}"));
    f.app.ui(move |ui| ui.invoke_snip_toggle_favorite(id));
    f.quiet();
    let refused = f.act("new", json!({ "title": "Another", "language": "Shell", "code": code })).unwrap_err();
    f.disk.refuse(false);
    f.app.ui(|ui| ui.invoke_snip_export_all());
    f.quiet();
    assert!(!refused.contains(SENTINEL), "{refused}");
    assert!(!f.notice().contains(SENTINEL), "{}", f.notice());
    assert!(!f.app.ui(|ui| ui.get_notice().to_string()).contains(SENTINEL));
    assert!(!leaks(&f.describe()), "{}", f.describe());
    for answer in &answers {
        assert!(!leaks(answer), "{answer}");
    }

    // The editor and the snippet print without their code.
    let printed = f.app.model(|m| format!("{:?} {:?} {:?}", m.editor, m.open(), m.library));
    assert!(!printed.contains(SENTINEL), "{printed}");

    // The close keeps the typing in the draft lane: a file of the person's own.
    f.app.close_requested(WindowId::PRIMARY);
    let _ = f.app.shutdown();
    let draft = yantrik_app::drafts::path_in(f.paths.state_dir(), &crate::model::draft_name(id));
    assert!(std::fs::read_to_string(&draft).unwrap().contains(SENTINEL), "the typing is kept");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for file in [draft.clone(), f.path()] {
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} is the person's alone", file.display());
        }
    }

    let text = String::from_utf8_lossy(&log.lock().unwrap_or_else(|p| p.into_inner())).to_string();
    assert!(text.contains("the capture is listening"), "the capture works");
    assert!(!text.contains(SENTINEL), "a log line carried the code:\n{text}");
}
