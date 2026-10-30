//! The rules every program that keeps a secret in the vault shares. Every value is obviously fake.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::memory::MemoryVault;
use super::opening::{self, Pass};
use super::*;

/// The queue and the listeners are process-wide, so the tests that use them take turns.
fn turn() -> std::sync::MutexGuard<'static, ()> {
    static TURN: Mutex<()> = Mutex::new(());
    let guard = TURN.lock().unwrap_or_else(|e| e.into_inner());
    opening::forget_waiting();
    guard
}

#[test]
fn a_value_is_kept_only_when_it_reads_back_the_same() {
    let vault = MemoryVault::new();
    assert_eq!(keep(&vault, "a", "fake-value-1"), Ok(()));
    assert_eq!(vault.peek("a").as_deref(), Some("fake-value-1"));

    vault.garble(true);
    assert_eq!(keep(&vault, "b", "fake-value-2"), Err(VaultError::Failed), "read back as something else");
}

#[test]
fn a_shut_vault_and_a_silent_one_are_different_errors() {
    let vault = MemoryVault::new();
    vault.shut(true);
    assert_eq!(keep(&vault, "a", "fake-value-1"), Err(VaultError::Locked));
    assert!(!vault.has("a"), "nothing was written to a shut vault");

    vault.shut(false);
    vault.go_silent(true);
    assert_eq!(keep(&vault, "a", "fake-value-1"), Err(VaultError::Failed), "stored, but could not be read back");
    vault.refuse_writes(true);
    vault.go_silent(false);
    assert_eq!(keep(&vault, "c", "fake-value-3"), Err(VaultError::Failed));
}

#[test]
fn what_was_refused_for_a_shut_vault_runs_once_when_it_opens() {
    let _turn = turn();
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    opening::after_open(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(opening::waiting(), 1);
    opening::opened();
    opening::opened();
    assert_eq!(runs.load(Ordering::SeqCst), 1, "a second opening replays nothing");
}

#[test]
fn a_job_that_still_has_work_asks_again_at_the_next_opening_and_then_stops() {
    let _turn = turn();
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    // Work is left on the first two passes only.
    let pass: Pass = Arc::new(move |_| counted.fetch_add(1, Ordering::SeqCst) < 2);
    assert!(opening::begin(&pass), "the first pass left work");
    assert_eq!(opening::waiting(), 1);
    opening::opened();
    assert_eq!(runs.load(Ordering::SeqCst), 2, "the opening ran the pass, which left work again");
    assert_eq!(opening::waiting(), 1);
    opening::opened();
    assert_eq!(runs.load(Ordering::SeqCst), 3);
    assert_eq!(opening::waiting(), 0, "the third pass finished, so nothing waits for a fourth");
}

#[test]
fn a_pass_that_finishes_at_the_start_waits_for_nothing() {
    let _turn = turn();
    let pass: Pass = Arc::new(|_| false);
    assert!(!opening::begin(&pass));
    assert_eq!(opening::waiting(), 0);
}

#[test]
fn a_listener_hears_every_opening() {
    let _turn = turn();
    let heard = Arc::new(AtomicUsize::new(0));
    let counted = heard.clone();
    opening::on_open(move || {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    opening::opened();
    opening::opened();
    assert_eq!(heard.load(Ordering::SeqCst), 2);
}
