//! Moved out of `recipe_executor.rs`'s `tests` unchanged, to keep that file no longer than it was.

use super::*;

/// A Branch runs the steps of the side it takes, and only those (#176). The shell's own
/// executor marked the Branch `then` or `else` and went on, running neither side.
#[test]
fn a_branch_runs_the_side_it_takes() {
    let steps = [
        tool("check", "urgent"),
        RecipeStep::Branch {
            condition: "urgent".into(),
            then_steps: vec![tool("page", "paged"), notify("Paged: {{paged}}")],
            else_steps: vec![tool("file", "filed")],
        },
        notify("Done."),
    ];
    let mut desk = Desk::new();
    desk.tool_says("check", &["yes"]);
    let id = desk.start(&steps);
    run(&mut desk, &id, 20);
    assert_eq!(desk.status(&id).0, RecipeStatus::Done);
    assert_eq!(desk.called, ["check", "page"], "the then side ran, and the else side did not");
    assert!(desk.said.iter().any(|s| s == "Paged: ok:page"), "{:?}", desk.said);
    assert_eq!(desk.var(&id, "filed"), None);
    let v = desk.view(&id);
    assert_eq!(v.steps[1].state, "done");
    assert_eq!(v.steps[1].path.as_deref(), Some("took then: page → Notify"));

    // The other way.
    let mut desk = Desk::new();
    desk.tool_says("check", &[""]);
    let id = desk.start(&steps);
    run(&mut desk, &id, 20);
    assert_eq!(desk.called, ["check", "file"]);
    assert_eq!(desk.var(&id, "filed"), Some(json!("ok:file")));
    assert_eq!(desk.view(&id).steps[1].path.as_deref(), Some("took else: file"));
}
