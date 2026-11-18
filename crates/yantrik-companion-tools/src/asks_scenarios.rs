//! The situations every tool that asks must get right (#1311), run against the fake shell
//! ([`crate::fake_shell`]): one function, used by every tool's tests here and in
//! `yantrik-companion`, so that "a denial changes nothing" is checked the same way for each.
//!
//! A tool's test describes one call of its own: the card it must raise, exactly, and how to make
//! the call with `variant` 0 (the call), 1 (the same tool with other values) or 2 (values the card
//! cannot show whole). The call runs its effect through a counter; the checks read that counter
//! and the fake's log:
//!
//! - the card is asked with exactly the values given, at the question's grade;
//! - an allow runs the effect once and spends the grant once; a second call with other values
//!   raises a card of its own and spends its own grant;
//! - a denial, silence and an unreachable shell run nothing and say so, spending nothing;
//! - values too long to show whole are not asked about, and nothing runs;
//! - full bypass runs it and writes down that nobody was asked.

use std::cell::Cell;

use crate::fake_shell::{Answer, FakeShell, First};

/// One tool's call, to check.
pub struct Scenario<'a> {
    /// The shell question it must ask: `delete_for_good`, `send_message`, `plan_work`, `change_item`.
    pub action: &'a str,
    /// The grade that question is published at.
    pub grade: &'a str,
    /// The arguments the card must carry for variant 0, exactly.
    pub card: serde_json::Value,
    /// Make the call. Run the tool's effect through `effect` (it counts); return the tool's
    /// outcome as a result: `Err` is the sentence the model reads.
    pub call: &'a dyn Fn(u8, &dyn Fn() -> Result<(), String>) -> Result<(), String>,
}

fn counted<'a>(effects: &'a Cell<usize>) -> impl Fn() -> Result<(), String> + 'a {
    move || {
        effects.set(effects.get() + 1);
        Ok(())
    }
}

/// Run every situation; panics with the tool's own words where one is wrong.
pub fn check_all(s: &Scenario<'_>) {
    // The card speaks the published vocabulary and nothing else: the shell refuses an unknown field.
    let known: Vec<String> = crate::asks::fields().into_iter().map(|(k, _)| k).collect();
    for key in s.card.as_object().expect("a card is an object").keys() {
        assert!(known.contains(key), "`{key}` is not a field the shell publishes for the questions");
    }

    // An allow: asked with exactly these values, run once, grant spent once.
    let effects = Cell::new(0);
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    (s.call)(0, &counted(&effects)).expect("an allow runs it");
    assert_eq!(effects.get(), 1, "the effect ran once");
    let cards = shell.cards();
    assert_eq!(cards.len(), 1, "{:?}", shell.calls());
    assert_eq!(cards[0]["app"], "shell");
    assert_eq!(cards[0]["action"], s.action);
    assert_eq!(cards[0]["grade"], s.grade);
    assert_eq!(cards[0]["args_json"], s.card, "the card carries exactly the values of the change");
    assert_eq!(shell.spent(), 1, "the grant is spent");
    let spent: Vec<_> = shell.calls().into_iter().filter(|c| c["action"] == "consume_approval").collect();
    assert_eq!(spent[0]["args"]["args_json"], s.card, "the Allow is spent on what was shown");

    // Other values: a card of their own, a grant of their own.
    (s.call)(1, &counted(&effects)).expect("an allow runs the second call");
    let cards = shell.cards();
    assert_eq!(cards.len(), 2, "the second call needs its own card");
    assert_ne!(cards[0]["args_json"], cards[1]["args_json"], "and it shows its own values");
    assert_eq!((effects.get(), shell.spent()), (2, 2));
    drop(shell);

    // A denial and silence: nothing runs, nothing is spent, and the model is told.
    for (answer, word) in [(Answer::Denied, "said no"), (Answer::Silence, "Nobody answered")] {
        let effects = Cell::new(0);
        let shell = FakeShell::session(First::Pending, answer);
        let err = (s.call)(0, &counted(&effects)).expect_err("no allow, no change");
        assert!(err.contains(word), "{err}");
        assert!(err.to_lowercase().contains("nothing was"), "it says nothing was done: {err}");
        assert_eq!((effects.get(), shell.spent()), (0, 0), "{answer:?}");
    }

    // A grant that was already spent is not an allow.
    let effects = Cell::new(0);
    let shell = FakeShell::session(First::Pending, Answer::Consumed);
    let err = (s.call)(0, &counted(&effects)).expect_err("a spent Allow runs nothing");
    assert!(err.contains("already been spent"), "{err}");
    assert_eq!((effects.get(), shell.spent()), (0, 0));
    drop(shell);

    // No shell to ask is a no.
    let effects = Cell::new(0);
    let nobody = FakeShell::nobody();
    let err = (s.call)(0, &counted(&effects)).expect_err("an unreachable shell is a no");
    assert!(err.contains("could not be asked") && err.to_lowercase().contains("nothing was"), "{err}");
    assert_eq!(effects.get(), 0);
    drop(nobody);

    // Too long to show whole: not asked about, and nothing runs. Once from this side's own check...
    let effects = Cell::new(0);
    let shell = FakeShell::session(First::Pending, Answer::Granted);
    let err = (s.call)(2, &counted(&effects)).expect_err("what cannot be shown whole is not asked");
    assert!(err.contains("too long") && err.contains("Nothing was changed") && err.to_lowercase().contains("in the app"), "{err}");
    assert_eq!((effects.get(), shell.calls().len()), (0, 0), "not even a card was raised: {:?}", shell.calls());
    drop(shell);
    // ...and once from the shell's, which refuses a request its card cannot show.
    let effects = Cell::new(0);
    let shell = FakeShell::session(First::TooLong, Answer::Granted);
    let err = (s.call)(0, &counted(&effects)).expect_err("the shell would not put it up");
    assert!(err.contains("too long") && err.contains("Nothing was changed"), "{err}");
    assert_eq!((effects.get(), shell.spent()), (0, 0));
    drop(shell);

    // No mode answers it: a shell that says `not_needed` (full bypass, a rule) gets no effect, no
    // spent grant and no record of an unasked run (#1313, round 4).
    let effects = Cell::new(0);
    let shell = FakeShell::session(First::NotNeeded, Answer::Granted);
    let err = (s.call)(0, &counted(&effects)).expect_err("only a press runs it");
    assert!(err.contains("needs a card") && err.to_lowercase().contains("nothing was"), "{err}");
    assert_eq!((effects.get(), shell.spent()), (0, 0));
    assert_eq!(shell.actions(), ["request_approval"], "{:?}", shell.calls());
}
