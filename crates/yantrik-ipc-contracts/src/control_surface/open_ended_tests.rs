//! The tests of the open-ended grade of an action. Separate from control_surface.rs, which is over
//! the file-size limit, so that file does not grow.

use super::*;

#[test]
fn an_open_ended_action_says_so_once_at_the_end_of_its_description() {
    let run = Action::new("run", "Type a command line into the shell").open_ended();
    assert_eq!(run.description, "Type a command line into the shell. What it runs can do anything you can.");
    let again = run.clone().open_ended();
    assert_eq!(again.description, run.description, "said once, however often it is declared");
    assert_eq!(with_open_ended("Run it.  "), "Run it. What it runs can do anything you can.");
    assert_eq!(with_open_ended(""), OPEN_ENDED);
    assert_eq!(run.schema()["description"], run.description.as_str(), "published as it reads");
}
