//! A step the worker runs is a turn of its own (`recipe_executor::step` holds `unattended::own_turn()`).

use super::*;

/// The person's last turn read a page, and the step's tool does not meet that (it would be refused for what the
/// person's message read); the person's turn is as it was.
#[test]
fn a_recipe_step_does_not_inherit_the_persons_last_turn() {
    use yantrik_companion_core::taint;
    let mut desk = Desk::new();
    let id = desk.start(&[tool("create_schedule", "made")]);
    taint::begin_turn();
    taint::note("browse", "browser");
    step(&mut desk, &id);
    assert_eq!(desk.called, vec!["create_schedule"], "the step did not run its tool");
    assert_eq!(desk.read_then, vec![None], "the step inherited what the person's turn read");
    assert_eq!(taint::state().0.as_deref(), Some("browse"), "the person's turn is as it was");
    taint::end_turn();
    // What a step reads stays with the step.
    let id = desk.start(&[tool("a", "a")]);
    taint::begin_turn();
    step(&mut desk, &id);
    assert_eq!(taint::state(), (None, None));
    taint::end_turn();
}
