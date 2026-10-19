//! "Can it remember?" and "Can it reach the desktop?" for the built-in companion, done for real.
//!
//! The first run showed both rows as "Needs attention: this build cannot check … yet" on a fresh
//! VM (13 Oct): on the person's first look at their AI, the system read as broken when nothing
//! was wrong. A row on that screen is now a check that ran, or it is not there.
//!
//! - **Remember**: a note written to the companion's own YantrikDB, recalled by its words, then
//!   deleted. Done on the companion's worker (`CompanionCommand::ProveMemory`), because the tool
//!   path an outside caller has is incognito and held to Safe: through it `remember` saves nothing.
//! - **Desktop**: the companion's own `describe_app` tool asked for the shell's surface
//!   (`app-shell`, control.rs). The same door its chat turns use to read the desktop, so a pass
//!   means the companion can see the desktop, not only that a socket exists.
//!
//! Both are pure over a small trait, so the rules are tested without a companion.

use super::onboarding::Proof;

/// Where the memory proof writes, recalls and deletes: the companion's YantrikDB on its worker.
pub trait NoteStore {
    /// Write `text`; the memory's id.
    fn write(&self, text: &str) -> Result<String, String>;
    /// Recall by `query`: `(id, text)` of what came back.
    fn recall(&self, query: &str) -> Result<Vec<(String, String)>, String>;
    /// Delete the memory `rid`.
    fn forget(&self, rid: &str) -> Result<(), String>;
}

/// The note the proof writes. Plain words, so a person who finds it in Memory before it is
/// deleted (or after a crash between the write and the delete) knows what it was.
pub fn note_text(nonce: &str) -> String {
    format!("Yantrik first-run memory check {nonce}. This note is deleted as soon as it is read back.")
}

/// Write a note, recall it by its words, delete it. `Ok` carries what Details shows; `Err` the
/// plain reason the row gives. The delete failing is said, not hidden: the check passed, and a
/// note was left behind.
pub fn prove_memory(store: &dyn NoteStore, nonce: &str) -> Result<String, String> {
    let note = note_text(nonce);
    let rid = store.write(&note).map_err(|e| format!("it could not write a note ({e})"))?;
    // A failed check does not leave its note behind either.
    let back = store.recall(&note).map_err(|e| {
        let _ = store.forget(&rid);
        format!("it wrote a note but could not search its memory ({e})")
    })?;
    let Some((found, _)) = back.iter().find(|(_, text)| *text == note) else {
        let _ = store.forget(&rid);
        return Err("the note it wrote did not come back".into());
    };
    match store.forget(found) {
        Ok(()) => Ok(format!("wrote a note to its memory, recalled it by its words ({} found), deleted it", back.len())),
        Err(e) => Ok(format!("wrote a note, recalled it, but could not delete it ({e}); it is in Memory as \u{201c}{note}\u{201d}")),
    }
}

/// What the companion's answer about the shell's surface proves. `describe_app` hands back the
/// surface as JSON when the shell answered, and a sentence when it did not.
pub fn desktop_proof(said: Result<String, String>, ms: u128) -> Proof {
    let said = match said {
        Ok(s) => s,
        Err(why) => {
            return Proof::Attention { reason: "it could not ask the desktop".into(), details: why };
        }
    };
    match serde_json::from_str::<serde_json::Value>(&said) {
        Ok(view) if view.is_object() => {
            let actions = view.get("actions").and_then(|a| a.as_array()).map_or(0, Vec::len);
            let summary: String = view.get("summary").and_then(|s| s.as_str()).unwrap_or("").chars().take(80).collect();
            Proof::Passed {
                details: format!("described the desktop in {ms} ms: {actions} actions it can take{}", if summary.is_empty() { String::new() } else { format!(", \u{201c}{summary}\u{201d}") }),
            }
        }
        _ => {
            let line: String = said.trim().chars().take(160).collect();
            Proof::Attention { reason: "the desktop did not answer it".into(), details: line }
        }
    }
}

/// The companion's worker, as the memory proof needs it: its own database.
pub struct Db<'a>(pub &'a yantrikdb_core::YantrikDB);

impl NoteStore for Db<'_> {
    fn write(&self, text: &str) -> Result<String, String> {
        // Low importance and a short half-life: were the delete to fail, it fades on its own.
        self.0
            .record_text(text, "episodic", 0.05, 0.0, 3600.0, &serde_json::json!({ "first_run_check": true }), "default", 1.0, "general", "first-run", None)
            .map_err(|e| e.to_string())
    }
    fn recall(&self, query: &str) -> Result<Vec<(String, String)>, String> {
        self.0.recall_text(query, 5).map(|r| r.into_iter().map(|m| (m.rid, m.text)).collect()).map_err(|e| e.to_string())
    }
    fn forget(&self, rid: &str) -> Result<(), String> {
        self.0.forget(rid).map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fake {
        notes: RefCell<Vec<(String, String)>>,
        write_fails: bool,
        recall_loses_it: bool,
        forget_fails: bool,
    }

    impl NoteStore for Fake {
        fn write(&self, text: &str) -> Result<String, String> {
            if self.write_fails {
                return Err("the store is read-only".into());
            }
            let rid = format!("rid-{}", self.notes.borrow().len());
            self.notes.borrow_mut().push((rid.clone(), text.to_string()));
            Ok(rid)
        }
        fn recall(&self, _query: &str) -> Result<Vec<(String, String)>, String> {
            if self.recall_loses_it {
                return Ok(vec![("other".into(), "something else".into())]);
            }
            Ok(self.notes.borrow().clone())
        }
        fn forget(&self, rid: &str) -> Result<(), String> {
            if self.forget_fails {
                return Err("locked".into());
            }
            self.notes.borrow_mut().retain(|(r, _)| r != rid);
            Ok(())
        }
    }

    #[test]
    fn the_memory_check_passes_only_when_the_note_it_wrote_comes_back_and_is_deleted() {
        let store = Fake::default();
        let details = prove_memory(&store, "n1").unwrap();
        assert!(details.contains("recalled") && details.contains("deleted"), "{details}");
        assert!(store.notes.borrow().is_empty(), "the check leaves nothing in the person's memory");
    }

    #[test]
    fn a_note_that_was_not_written_or_did_not_come_back_is_the_plain_reason() {
        let cannot = Fake { write_fails: true, ..Fake::default() };
        assert!(prove_memory(&cannot, "n").unwrap_err().contains("could not write"));
        let lost = Fake { recall_loses_it: true, ..Fake::default() };
        assert_eq!(prove_memory(&lost, "n").unwrap_err(), "the note it wrote did not come back");
        assert!(lost.notes.borrow().is_empty(), "a failed check does not leave its note behind");
    }

    #[test]
    fn a_note_that_could_not_be_deleted_is_said_with_its_words() {
        let stuck = Fake { forget_fails: true, ..Fake::default() };
        let details = prove_memory(&stuck, "n2").unwrap();
        assert!(details.contains("could not delete") && details.contains(&note_text("n2")), "{details}");
    }

    #[test]
    fn the_desktop_check_passes_only_on_the_shells_own_description() {
        let view = r#"{"summary":"Desktop, 0 windows open","actions":[{"name":"open_app"},{"name":"notify"}]}"#;
        match desktop_proof(Ok(view.into()), 12) {
            Proof::Passed { details } => assert!(details.contains("2 actions") && details.contains("Desktop, 0 windows"), "{details}"),
            p => panic!("{p:?}"),
        }
        // describe_app's sentence when the shell did not answer is not a pass.
        let said = "'shell' is not open, and no Yantrik app is. (connection refused)";
        assert!(matches!(desktop_proof(Ok(said.into()), 3), Proof::Attention { .. }));
        assert!(matches!(desktop_proof(Err("companion worker is not running".into()), 0), Proof::Attention { .. }));
    }
}
