//! A draft goes only when a write holding its text has landed, or when the person discards it
//! (`update::drafts::release`, the #938 delta review at eca867b2): each probe of that review, and a
//! property run of random refusals, panics, failed reads and ends.

use std::collections::BTreeMap;

use yantrik_app::app::WindowId;
use yantrik_app::mind::fake::FakeMind;

use super::{open, start_at, Fixture};
use crate::model::draft_name;
use crate::Command;

fn draft_file(f: &Fixture, id: i32) -> Option<String> {
    std::fs::read_to_string(yantrik_app::drafts::path_in(f.paths.state_dir(), &draft_name(id))).ok()
}

/// Every draft file's text, whatever its name.
fn draft_files(f: &Fixture) -> Vec<String> {
    let dir = f.paths.state_dir().join(yantrik_app::drafts::DIR);
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().filter_map(|e| std::fs::read_to_string(e.path()).ok()).collect())
        .unwrap_or_default()
}

/// Typed into the first snippet, then a move that commits it, with the write and the re-read set
/// to fail by `fail`.
fn typed_then_a_write_that_does_not_finish(fail: impl Fn(&Fixture)) -> (Fixture, i32) {
    let f = open();
    let (journal, borrow) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("J", "typed-before-panic");
    f.app.run_writes();
    f.disk.panic_next();
    fail(&f);
    f.app.run(Command::Select(borrow));
    f.quiet();
    f.app.run_writes();
    (f, journal)
}

fn the_typing_survives(f: Fixture, journal: i32) {
    assert!(draft_file(&f, journal).unwrap().contains("typed-before-panic"), "the draft is on disk");
    assert!(f.notice().contains("Your text is kept as a draft"), "{}", f.notice());
    // Moves that write nothing let nothing go either.
    f.app.run(Command::Select(journal));
    f.app.run(Command::Select(-1));
    f.quiet();
    f.app.run_writes();
    assert!(draft_file(&f, journal).unwrap().contains("typed-before-panic"));
    f.disk.fail_reads(false);
    let f = f.restart();
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "typed-before-panic", "offered back");
}

/// M2b, through `Reloaded(Err)`: the re-read after an unfinished write fails too.
#[test]
fn an_unfinished_write_and_a_failed_reread_keep_the_draft() {
    let (f, journal) = typed_then_a_write_that_does_not_finish(|f| f.disk.fail_reads(true));
    the_typing_survives(f, journal);
}

/// M2b, through `Work::Reload`: the re-read's own job ends without a result.
#[test]
fn an_unfinished_write_and_an_unfinished_reread_keep_the_draft() {
    let (f, journal) = typed_then_a_write_that_does_not_finish(|f| f.disk.panic_next_read());
    the_typing_survives(f, journal);
}

/// N1: a refused *Restore as new* never removes the draft file that holds the text; it stays under
/// its own name until the new snippet's write has landed.
#[test]
fn a_refused_restore_as_new_keeps_the_draft_file() {
    let f = open();
    let (journal, _) = f.seeded();
    f.app.run(Command::Select(journal));
    f.type_into("Orphan", "orphan-text");
    f.act("delete", serde_json::json!({ "id": journal.to_string() })).unwrap();
    f.app.run_writes();
    assert!(draft_file(&f, journal).unwrap().contains("orphan-text"));
    f.disk.refuse(true);
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    f.app.run_writes();
    assert!(draft_file(&f, journal).unwrap().contains("orphan-text"), "the only copy on disk stays");
    assert_eq!(f.app.ui(|ui| ui.get_recovery()), 3, "and is offered again");
    f.app.close_requested(WindowId::PRIMARY);
    let f = f.restart();
    assert_eq!(f.app.ui(|ui| ui.get_recovery()), 3, "after a restart too");
    // With the disk back, Restore as new lands, and only then does the draft go.
    f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
    f.quiet();
    f.app.run_writes();
    assert!(f.on_disk().snippets().iter().any(|s| s.code == "orphan-text"));
    assert!(draft_file(&f, journal).is_none(), "kept, and the draft let go");
}

/// The draft lane itself refusing (its file cannot be written) is said, and no other copy goes.
#[test]
fn a_draft_the_lane_cannot_write_is_said() {
    let f = open();
    let (journal, _) = f.seeded();
    let blocked = yantrik_app::drafts::path_in(f.paths.state_dir(), &draft_name(journal));
    std::fs::create_dir_all(blocked.join("in-the-way")).unwrap();
    f.app.run(Command::Select(journal));
    f.type_into("J", "typed into a blocked draft");
    f.app.run_writes();
    assert!(f.notice().starts_with("Your unsaved text could not be kept as a draft"), "{}", f.notice());
    assert_eq!(f.app.ui(|ui| ui.get_detail_code().to_string()), "typed into a blocked draft");
}

/// A small, seeded generator: the run is the same every time, and a failure names its seed.
struct Steps(u64);

impl Steps {
    fn next(&mut self, below: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % below
    }
}

/// Random typing, moves, saves and restores under random refused writes, unfinished writes, failed
/// and unfinished re-reads and single landed jobs, ending with jobs still in flight. After each
/// run, the last text typed into every snippet is in the store or in a draft file.
#[test]
fn no_sequence_of_failures_loses_the_last_typed_text() {
    for seed in 1..=48u64 {
        let mut steps = Steps(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
        let f = open();
        let (a, b) = f.seeded();
        let mut last: BTreeMap<i32, String> = BTreeMap::new();
        let mut refusing = false;
        let mut failing_reads = false;
        let mut trail = Vec::new();
        for step in 0..16 {
            let roll = steps.next(11);
            trail.push(roll);
            match roll {
                0..=3 => {
                    if f.app.model(|m| m.editor.id) < 0 {
                        let id = if steps.next(2) == 0 { a } else { b };
                        f.app.run(Command::Select(id));
                    }
                    let id = f.app.model(|m| m.editor.id);
                    if id >= 0 {
                        let text = format!("seed{seed}-step{step}");
                        f.type_into("Typed", &text);
                        last.insert(id, text);
                    }
                }
                4 => {
                    let id = if steps.next(2) == 0 { a } else { b };
                    f.app.run(Command::Select(id));
                }
                5 => {
                    refusing = !refusing;
                    f.disk.refuse(refusing);
                }
                6 => f.disk.panic_next(),
                7 => {
                    failing_reads = !failing_reads;
                    f.disk.fail_reads(failing_reads);
                }
                8 => {
                    f.app.run_one_job();
                }
                9 => f.disk.panic_next_read(),
                _ => {
                    if f.app.ui(|ui| ui.get_recovery()) != 0 {
                        f.app.ui(|ui| ui.invoke_recovery_action("restore".into()));
                    } else {
                        f.app.ui(|ui| {
                            ui.invoke_snip_save(ui.get_detail_id(), ui.get_detail_title(), ui.get_detail_language(), ui.get_detail_code(), ui.get_detail_tags())
                        });
                    }
                }
            }
        }
        // The end: what the draft lane was given is written, and the store's jobs still in flight
        // never run, as at a kill in the middle of a write.
        f.app.run_writes();
        let paths = f.paths.clone();
        let _ = f.app.shutdown();
        drop(f);
        let g = start_at(paths, FakeMind::new(), Vec::new());
        g.app.run_writes();
        let stored: Vec<String> = g.on_disk().snippets().iter().map(|s| s.code.clone()).collect();
        let drafted = draft_files(&g);
        for (id, text) in &last {
            let kept = stored.iter().any(|c| c == text) || drafted.iter().any(|d| d.contains(text.as_str()));
            assert!(kept, "seed {seed}, snippet {id}: `{text}` is in neither the store nor a draft; steps {trail:?}");
        }
    }
}
