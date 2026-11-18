//! The document rules, each as a test (design/notes-gate-2026-10-21.md §2 and §3).

use std::path::PathBuf;

use super::close::{Next, Stage};
use super::disk::Step;
use super::draft::recover;
use super::*;

fn fp(text: &str) -> Fingerprint {
    Fingerprint::of_bytes(text.as_bytes())
}

fn file(name: &str) -> Source {
    Source::File(PathBuf::from(format!("/notes/{name}.md")))
}

/// A table with one document read from `name` with `text`.
fn opened(name: &str, text: &str) -> (Documents<String>, DocumentId) {
    let mut docs = Documents::new();
    let id = docs.open(file(name), text.to_string(), Some(fp(text)));
    (docs, id)
}

fn ticket(begin: BeginSave<String>) -> SaveTicket<String> {
    match begin {
        BeginSave::Ticket(t) => t,
        other => panic!("expected a ticket, got {other:?}"),
    }
}

/// A save that landed with a follow-up owed: the follow-up's ticket.
fn follow_up(outcome: SaveOutcome<String>) -> SaveTicket<String> {
    match outcome {
        SaveOutcome::Saved { follow_up: Some(t), .. } => t,
        other => panic!("expected a follow-up, got {other:?}"),
    }
}

/// A save that landed with nothing owed and nothing replaced.
fn plainly_saved(outcome: &SaveOutcome<String>) -> bool {
    matches!(outcome, SaveOutcome::Saved { follow_up: None, replaced: None })
}

// ---- the table: one document per file, views share it ----

#[test]
fn one_file_opened_twice_is_one_document() {
    let (mut docs, id) = opened("a", "hello");
    let again = docs.open(file("a"), "stale copy".into(), Some(fp("stale copy")));
    assert_eq!(again, id);
    assert_eq!(docs.get(id).unwrap().content, "hello", "the open document is not replaced");
    let one = docs.show(id, WindowId(1));
    let two = docs.show(id, WindowId(2));
    assert_eq!(docs.views_of(id).count(), 2);
    docs.get_mut(id).unwrap().edit(|c| c.push('!'));
    // Both views read the one document: content and dirty state cannot diverge.
    for v in [one, two] {
        let shown = docs.view(v).unwrap().document;
        assert!(docs.get(shown).unwrap().dirty());
    }
}

#[test]
fn forgetting_the_last_view_hands_back_the_document() {
    let (mut docs, id) = opened("a", "x");
    let one = docs.show(id, WindowId(1));
    let two = docs.show(id, WindowId(2));
    assert_eq!(docs.forget_view(one), None, "another view still shows it");
    assert_eq!(docs.forget_view(two), Some(id));
    assert!(docs.close_document(id).is_some());
    assert!(docs.get(id).is_none());
}

#[test]
fn a_key_names_the_file_without_carrying_its_path() {
    let key = file("private-diary").key();
    assert!(key.starts_with("doc-"));
    assert!(!key.contains("diary") && !key.contains('/'));
    assert_eq!(key, file("private-diary").key(), "stable across runs");
    assert_ne!(key, file("other").key());
    assert_eq!(Source::New("7".into()).key(), "new-7");
}

// ---- saves: one in flight, one follow-up, no retry spam ----

#[test]
fn edits_during_a_save_owe_exactly_one_follow_up() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let first = ticket(d.begin_save());
    assert_eq!(first.content, "v1");
    d.edit(|c| *c = "v2".into());
    assert!(matches!(d.begin_save(), BeginSave::InFlight));
    d.edit(|c| *c = "v3".into());
    assert!(matches!(d.begin_save(), BeginSave::InFlight), "still one in flight, not a queue");
    let follow = follow_up(d.save_landed(first.seq, Ok(fp("v1"))));
    assert_eq!(follow.content, "v3", "the follow-up saves the newest edits, once");
    assert_eq!(follow.baseline, Some(fp("v1")), "and expects our own write on disk");
    assert!(plainly_saved(&d.save_landed(follow.seq, Ok(fp("v3")))));
    assert!(!d.dirty());
    assert!(matches!(d.begin_save(), BeginSave::Clean));
}

#[test]
fn an_outcome_for_a_save_no_longer_expected_changes_nothing() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    assert!(matches!(d.save_landed(t.seq + 5, Ok(fp("zzz"))), SaveOutcome::Stale));
    assert!(matches!(d.save, SaveState::Saving { .. }));
    assert_eq!(d.baseline, Some(fp("v0")));
}

#[test]
fn a_failed_save_is_not_retried_by_autosave_until_something_changes() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    let failed = SaveError::new("The disk is read-only.");
    assert!(matches!(d.save_landed(t.seq, Err(failed.clone())), SaveOutcome::Failed));
    assert!(d.dirty(), "nothing was saved");
    assert!(matches!(d.begin_save(), BeginSave::Failed), "an autosave tick does not spam");
    assert!(matches!(d.begin_save(), BeginSave::Failed));
    // The person asks: retried.
    let retry = ticket(d.retry_save());
    assert_eq!(retry.seq, t.seq);
    assert!(matches!(d.save_landed(retry.seq, Err(failed)), SaveOutcome::Failed));
    // A new edit is new content: autosave tries it.
    d.edit(|c| *c = "v2".into());
    assert_eq!(ticket(d.begin_save()).content, "v2");
}

#[test]
fn a_failure_after_the_rename_is_not_called_saved() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    let unknown = SaveError::after_writing(Step::SyncDir.durability(), fp("v1"), "Saved, but the disk did not confirm it.");
    d.save_landed(t.seq, Err(unknown));
    assert!(d.dirty(), "Unknown durability is treated as not saved");
    match &d.save {
        SaveState::Failed { error, .. } => assert_eq!(error.durability, Durability::Unknown),
        other => panic!("{other:?}"),
    }
}

#[test]
fn after_an_unknown_failure_our_own_bytes_are_ours_and_the_retry_lands() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    let unknown = SaveError::after_writing(Durability::Unknown, fp("v1"), "The disk did not confirm the save.");
    d.save_landed(t.seq, Err(unknown));
    assert!(d.dirty());
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Clean, "not a conflict with ourselves");
    let retry = ticket(d.retry_save());
    assert!(retry.may_replace(Some(&fp("v1"))), "the store accepts our unsure bytes");
    assert!(retry.may_replace(Some(&fp("v0"))), "and the baseline");
    assert!(!retry.may_replace(Some(&fp("theirs"))));
    assert!(plainly_saved(&d.save_landed(retry.seq, Ok(fp("v1")))));
    assert!(!d.dirty());
}

#[test]
fn a_failure_that_wrote_nothing_leaves_nothing_unsure() {
    let e = SaveError::after_writing(Durability::NotWritten, fp("v1"), "Disk full.");
    assert_eq!(e.landed, None);
}

#[test]
fn a_new_file_may_be_created_only_where_there_is_none() {
    let mut docs: Documents<String> = Documents::new();
    let id = docs.open(Source::New("1".into()), String::new(), None);
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| c.push('x'));
    let t = ticket(d.begin_save());
    assert!(t.may_replace(None));
    assert!(!t.may_replace(Some(&fp("someone else's"))));
}

#[test]
fn a_save_that_finds_the_file_changed_becomes_a_conflict_and_blocks_saves() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "mine".into());
    let t = ticket(d.begin_save());
    d.save_landed(t.seq, Err(SaveError::conflict("The note changed on disk.")));
    assert_eq!(d.outside, OutsideState::Conflict { now: None });
    assert!(matches!(d.begin_save(), BeginSave::Blocked), "not Failed: the person has a choice");
    assert!(matches!(d.retry_save(), BeginSave::Blocked), "the person chooses first");
}

// ---- outside changes: classified, our own writes recognised ----

#[test]
fn the_watch_seeing_the_baseline_we_wrote_is_clean() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.save_landed(t.seq, Ok(fp("v1")));
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Clean);
    assert!(d.rewatch().is_none());
}

#[test]
fn our_own_write_seen_by_the_watch_is_not_an_outside_edit() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.save_landed(t.seq, Ok(fp("v1")));
    // A look taken before our newer write landed, by a watch that never reported v1:
    // suppressed once, and the watch re-armed.
    d.edit(|c| *c = "v2".into());
    let t = ticket(d.begin_save());
    d.save_landed(t.seq, Ok(fp("v2")));
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Clean);
    let (_, _, from) = d.rewatch().expect("the watch is re-armed");
    assert_eq!(from, Some(Observed::File(fp("v2"))), "from the baseline");
    assert!(d.rewatch().is_none(), "once");
}

#[test]
fn an_outside_revert_to_an_older_write_of_ours_is_an_outside_edit() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    for text in ["v1", "v2"] {
        d.edit(|c| *c = text.into());
        let t = ticket(d.begin_save());
        d.save_landed(t.seq, Ok(fp(text)));
    }
    // The watch caught up with v2; a sync client then puts v1 back.
    d.observe(Observed::File(fp("v2")));
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Changed { now: fp("v1") });
}

#[test]
fn a_revert_before_the_watch_caught_up_is_seen_on_the_second_look() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    for text in ["v1", "v2"] {
        d.edit(|c| *c = text.into());
        let t = ticket(d.begin_save());
        d.save_landed(t.seq, Ok(fp(text)));
    }
    // Late look or revert: not trusted yet.
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Clean);
    assert!(d.rewatch().is_some());
    // The re-armed watch looks again and still finds v1: it was a revert.
    assert_eq!(d.observe(Observed::File(fp("v1"))), &OutsideState::Changed { now: fp("v1") });
}

#[test]
fn an_outside_edit_to_a_clean_document_is_offered() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    assert_eq!(d.observe(Observed::File(fp("theirs"))), &OutsideState::Changed { now: fp("theirs") });
    d.take_theirs("theirs".into(), fp("theirs"));
    assert_eq!(d.outside, OutsideState::Clean);
    assert!(!d.dirty());
    assert_eq!(d.baseline, Some(fp("theirs")));
}

#[test]
fn an_outside_edit_to_a_dirty_document_is_a_conflict_never_last_writer_wins() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "mine".into());
    assert_eq!(d.observe(Observed::File(fp("theirs"))), &OutsideState::Conflict { now: Some(fp("theirs")) });
    assert!(matches!(d.begin_save(), BeginSave::Blocked));
    // Seeing the old file again does not clear a conflict on its own.
    assert!(matches!(d.observe(Observed::File(fp("v0"))), OutsideState::Conflict { .. }));
    let mine = d.keep_mine_as_copy("theirs".into(), fp("theirs"));
    assert_eq!(mine, "mine", "the person's text is handed back to be saved as a copy");
    assert_eq!(d.content, "theirs");
    assert_eq!(d.outside, OutsideState::Clean);
}

#[test]
fn a_deleted_file_is_said_and_a_save_becomes_save_as_new() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "keep me".into());
    assert_eq!(d.observe(Observed::Gone), &OutsideState::Deleted);
    assert!(matches!(d.begin_save(), BeginSave::Blocked));
    d.save_as_new("recovered-1");
    let t = ticket(d.begin_save());
    assert_eq!(t.baseline, None, "a new file is created, not one replaced");
    assert_eq!(t.content, "keep me");
}

#[test]
fn a_document_never_saved_is_not_deleted_by_an_absent_file() {
    let mut docs: Documents<String> = Documents::new();
    let id = docs.open(Source::New("1".into()), String::new(), None);
    let d = docs.get_mut(id).unwrap();
    assert_eq!(d.observe(Observed::Gone), &OutsideState::Clean);
}

// ---- a change seen during a save: three outcomes ----

#[test]
fn during_a_save_the_watch_seeing_our_write_is_ours() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::File(fp("v1")));
    assert_eq!(d.outside, OutsideState::Clean, "judged when the save lands, not now");
    assert!(plainly_saved(&d.save_landed(t.seq, Ok(fp("v1")))));
    assert_eq!(d.outside, OutsideState::Clean);
}

#[test]
fn during_a_save_an_outside_write_that_ours_replaced_is_said() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::File(fp("theirs")));
    assert!(matches!(
        d.save_landed(t.seq, Ok(fp("v1"))),
        SaveOutcome::Saved { replaced: Some(Replaced::OutsideEdit), follow_up: None }
    ));
}

#[test]
fn an_outside_write_ours_replaced_is_said_even_with_a_follow_up_owed() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::File(fp("theirs")));
    d.edit(|c| *c = "v2".into());
    assert!(matches!(d.begin_save(), BeginSave::InFlight));
    match d.save_landed(t.seq, Ok(fp("v1"))) {
        SaveOutcome::Saved { follow_up: Some(f), replaced: Some(Replaced::OutsideEdit) } => assert_eq!(f.content, "v2"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_deletion_seen_during_a_save_that_lands_is_said() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::Gone);
    assert!(matches!(
        d.save_landed(t.seq, Ok(fp("v1"))),
        SaveOutcome::Saved { replaced: Some(Replaced::Deletion), .. }
    ));
}

#[test]
fn taking_theirs_forgets_a_sighting_made_during_a_save() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let _ = ticket(d.begin_save());
    d.observe(Observed::File(fp("theirs")));
    d.take_theirs("theirs".into(), fp("theirs"));
    // Nothing stale is judged at the next landing.
    d.edit(|c| *c = "v2".into());
    let t = ticket(d.begin_save());
    assert!(plainly_saved(&d.save_landed(t.seq, Ok(fp("v2")))));
}

#[test]
fn during_a_save_that_fails_an_outside_write_is_judged_against_the_old_file() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::File(fp("theirs")));
    d.save_landed(t.seq, Err(SaveError::new("Disk full.")));
    assert_eq!(d.outside, OutsideState::Conflict { now: Some(fp("theirs")) });
}

// ---- drafts ----

#[test]
fn a_draft_is_owed_only_for_edits_newer_than_the_last_one_kept() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    assert!(d.draft_owed(100).is_none(), "a clean document keeps no draft");
    d.edit(|c| *c = "v1".into());
    let (name, bytes) = d.draft_owed(100).expect("owed");
    assert_eq!(name, d.source.key());
    let envelope: Envelope<String> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!((envelope.seq, envelope.content.as_str(), envelope.at), (d.seq, "v1", 100));
    assert_eq!(envelope.path.as_deref(), Some("/notes/a.md"));
    assert_eq!(envelope.baseline, Some(fp("v0")), "the file it was written over");
    let older: Envelope<String> = serde_json::from_str(r#"{"version":1,"key":"k","path":null,"seq":1,"at":1,"content":"x"}"#)
        .expect("a draft from before the baseline was kept still reads");
    assert_eq!(older.baseline, None);
    d.draft_asked();
    assert!(d.draft_owed(101).is_none());
    d.edit(|c| *c = "v2".into());
    assert!(d.draft_owed(102).is_some());
}

#[test]
fn a_draft_is_superseded_once_a_save_of_its_edits_commits() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    d.draft_asked();
    let t = ticket(d.begin_save());
    d.edit(|c| *c = "v2".into());
    d.draft_asked();
    assert!(matches!(d.begin_save(), BeginSave::InFlight), "the autosave tick asks again");
    let follow = follow_up(d.save_landed(t.seq, Ok(fp("v1"))));
    assert!(!d.draft_superseded(), "the draft holds v2, which is not saved yet");
    d.save_landed(follow.seq, Ok(fp("v2")));
    assert!(d.draft_superseded());
}

fn candidate(content: &str, at_ms: u64, seq: Option<u64>) -> Option<Candidate<String>> {
    Some(Candidate { content: content.into(), at_ms, seq })
}

#[test]
fn recovery_offers_only_what_differs_from_the_file_and_the_newest_wins() {
    let on_disk = "saved".to_string();
    let file = Some(&on_disk);
    assert_eq!(recover(file, None, None), Recovered::File);
    assert_eq!(recover(file, candidate("saved", 20, None), candidate("saved", 30, None)), Recovered::File);
    assert_eq!(recover(file, candidate("journal", 20, None), None), Recovered::Journal("journal".into()));
    assert_eq!(recover(file, None, candidate("draft", 20, None)), Recovered::Draft("draft".into()));
    assert_eq!(
        recover(file, candidate("journal", 40, None), candidate("draft", 30, None)),
        Recovered::Journal("journal".into())
    );
    assert_eq!(recover::<String>(None, None, candidate("new", 1, None)), Recovered::Draft("new".into()));
}

#[test]
fn at_the_same_millisecond_the_later_edit_wins_and_otherwise_the_draft() {
    let on_disk = "saved".to_string();
    let file = Some(&on_disk);
    assert_eq!(
        recover(file, candidate("journal", 30, Some(6)), candidate("draft", 30, Some(5))),
        Recovered::Journal("journal".into()),
        "the journal holds the later edit"
    );
    assert_eq!(
        recover(file, candidate("journal", 30, Some(5)), candidate("draft", 30, Some(6))),
        Recovered::Draft("draft".into())
    );
    assert_eq!(
        recover(file, candidate("journal", 30, None), candidate("draft", 30, Some(6))),
        Recovered::Draft("draft".into()),
        "without both seqs, the draft"
    );
}

// ---- close as a document flow ----

#[test]
fn closing_a_view_asks_only_for_the_last_view_on_a_dirty_document() {
    let (mut docs, id) = opened("a", "v0");
    let one = docs.show(id, WindowId(1));
    let two = docs.show(id, WindowId(2));
    assert_eq!(docs.closing_view(one), Closing::Now, "clean");
    docs.get_mut(id).unwrap().edit(|c| c.push('!'));
    assert_eq!(docs.closing_view(one), Closing::Now, "another view still holds it");
    docs.forget_view(one);
    assert_eq!(docs.closing_view(two), Closing::Ask(vec![id]));
}

#[test]
fn closing_a_window_asks_about_what_no_other_window_shows_and_quit_asks_once() {
    let mut docs: Documents<String> = Documents::new();
    let a = docs.open(file("a"), "a".into(), Some(fp("a")));
    let b = docs.open(file("b"), "b".into(), Some(fp("b")));
    let c = docs.open(file("c"), "c".into(), Some(fp("c")));
    docs.show(a, WindowId(1));
    docs.show(b, WindowId(1));
    docs.show(b, WindowId(2));
    docs.show(c, WindowId(2));
    for id in [a, b, c] {
        docs.get_mut(id).unwrap().edit(|t| t.push('!'));
    }
    assert_eq!(docs.closing_window(WindowId(1)), Closing::Ask(vec![a]), "b is still in window 2");
    assert_eq!(docs.quitting(), Closing::Ask(vec![a, b, c]), "every dirty document, once");
}

#[test]
fn save_then_close_waits_for_edits_made_meanwhile() {
    let (mut docs, id) = opened("a", "v0");
    docs.show(id, WindowId(1));
    docs.get_mut(id).unwrap().edit(|c| *c = "v1".into());
    let mut flow = CloseFlow::new(Some(WindowId(1)), vec![id]);
    assert_eq!(flow.answer(Choice::Save), Next::Save(vec![id]));
    assert_eq!(flow.answer(Choice::Save), Next::Wait, "a second close while saving changes nothing");
    let t = ticket(docs.get_mut(id).unwrap().begin_save());
    docs.get_mut(id).unwrap().edit(|c| *c = "v2".into());
    assert!(plainly_saved(&docs.get_mut(id).unwrap().save_landed(t.seq, Ok(fp("v1")))));
    assert_eq!(flow.progress(&docs, None), Next::Save(vec![id]), "v2 is not saved yet");
    let again = ticket(docs.get_mut(id).unwrap().begin_save());
    assert_eq!(again.content, "v2");
    docs.get_mut(id).unwrap().save_landed(again.seq, Ok(fp("v2")));
    assert_eq!(flow.progress(&docs, None), Next::Close { discard: vec![] });
}

#[test]
fn a_failed_save_while_closing_keeps_the_window() {
    let (docs, id) = opened("a", "v0");
    let mut flow = CloseFlow::new(Some(WindowId(1)), vec![id]);
    flow.answer(Choice::Save);
    assert_eq!(flow.progress(&docs, Some("Disk full.".into())), Next::Stay);
    assert_eq!(flow.stage, Stage::Blocked("Disk full.".into()));
}

#[test]
fn dont_save_discards_and_cancel_stays() {
    let (_, id) = opened("a", "v0");
    let mut flow = CloseFlow::new(None, vec![id]);
    assert_eq!(flow.answer(Choice::Cancel), Next::Stay);
    assert_eq!(flow.answer(Choice::DontSave), Next::Close { discard: vec![id] });
}

// ---- what a replace is known to have displaced ----

#[test]
fn a_ticket_judges_what_its_replace_displaced() {
    use super::disk::Displaced;
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    assert_eq!(t.judge(&Displaced::Bytes(b"v0".to_vec())), Witnessed::Same);
    assert_eq!(t.judge(&Displaced::Bytes(b"theirs".to_vec())), Witnessed::Replaced(Replaced::OutsideEdit));
    assert_eq!(t.judge(&Displaced::Nothing), Witnessed::Replaced(Replaced::Deletion), "it had a file");
    assert_eq!(t.judge(&Displaced::Unknown), Witnessed::Unknown);
}

#[test]
fn a_new_document_creating_its_file_displaced_nothing_to_say() {
    use super::disk::Displaced;
    let mut docs: Documents<String> = Documents::new();
    let id = docs.open(Source::New("1".into()), String::new(), None);
    let d = docs.get_mut(id).unwrap();
    d.edit(|c| c.push('x'));
    let t = ticket(d.begin_save());
    assert_eq!(t.judge(&Displaced::Nothing), Witnessed::Same);
}

#[test]
fn a_known_answer_replaces_the_watchs_guess() {
    let (mut docs, id) = opened("a", "v0");
    let d = docs.get_mut(id).unwrap();
    // The watch saw theirs during the save, but the replace displaced our own baseline: theirs
    // was written after ours, and is on disk now. Nothing of theirs was replaced.
    d.edit(|c| *c = "v1".into());
    let t = ticket(d.begin_save());
    d.observe(Observed::File(fp("theirs")));
    assert!(plainly_saved(&d.save_landed_witnessed(t.seq, Ok(fp("v1")), Witnessed::Same)));
    // No sighting at all, but the replace displaced another program's write: said.
    d.edit(|c| *c = "v2".into());
    let t = ticket(d.begin_save());
    assert!(matches!(
        d.save_landed_witnessed(t.seq, Ok(fp("v2")), Witnessed::Replaced(Replaced::OutsideEdit)),
        SaveOutcome::Saved { replaced: Some(Replaced::OutsideEdit), .. }
    ));
}
