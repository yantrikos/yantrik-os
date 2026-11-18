//! The tests of the surface's own state rule (the locked desktop refusing every action but one) and
//! of the per-call explainer. Separate from the dispatch tests in the parent only to keep
//! registry.rs from growing; the helpers are the parent's.

use super::*;

/// The refusal the shell's rule answers with while the desktop waits for the person (#203).
const LOCKED: &str = "LOCKED: the desktop is waiting for the person to sign in";

/// A shell-shaped surface whose rule allows one action and refuses every other one — the
/// shape of the locked desktop, with an allow-list of one so both sides can be tested.
fn ruled_surface(ran: Rc<Cell<bool>>) -> Registry {
    let mut reg = surface(
        "shell",
        Some(Box::new(|| View::new("Shell \u{2014} lock screen"))),
        vec![
            (Action::new("open_lens", "Open the ask bar").risk("safe"), {
                let ran = ran.clone();
                Box::new(move |_| {
                    ran.set(true);
                    Ok(Value::Null)
                })
            }),
            (
                Action::new("read_clock", "Read the clock").risk("safe"),
                Box::new(|_| Ok(Value::Null)),
            ),
            (
                Action::new("installer_install", "Erase a disk and install").risk("dangerous"),
                Box::new(|_| Ok(Value::Null)),
            ),
        ],
    );
    reg.set_state_rule(Box::new(|name| {
        if name == "read_clock" {
            Ok(())
        } else {
            Err(LOCKED.to_string())
        }
    }));
    reg
}

#[test]
fn a_state_rule_refuses_before_the_arguments_and_the_gate_are_read() {
    let ran = Rc::new(Cell::new(false));
    let reg = ruled_surface(ran.clone());

    // A safe action, a well-formed call, and every authority open: refused anyway, in the
    // rule's own sentence — this is the call from the bug report, `yos act shell open_lens`
    // on a machine standing at its login screen.
    let err = reg.act("open_lens", &json!({}), None, "shell#1", &open()).unwrap_err();
    assert_eq!(err, LOCKED);
    assert!(!ran.get(), "the handler must not have run");

    // A malformed call gets the same sentence, not an argument complaint: nothing about the
    // call is read while the rule refuses.
    let err = reg
        .act("installer_install", &json!({"junk": 1}), None, "shell#2", &in_mode("ask", false))
        .unwrap_err();
    assert_eq!(err, LOCKED, "not GRANT:, not an argument refusal: {err}");

    // And the allow-list side: an action the rule names passes, and the ordinary dispatch
    // decides it as it always would.
    assert!(reg.act("read_clock", &json!({}), None, "shell#3", &open()).is_ok());
}

#[test]
fn a_state_rule_refuses_before_a_grant_is_spent_on_the_call() {
    // `check_call` is what the dispatch asks before spending a person's Allow. A call the
    // rule will refuse must not use the Allow up on the way — the lesson of #154, applied
    // to the rule of #203.
    let reg = ruled_surface(Rc::new(Cell::new(false)));
    assert_eq!(reg.check_call("open_lens", &json!({})).unwrap_err(), LOCKED);
    assert_eq!(reg.check_call("read_clock", &json!({})), Ok("safe"), "the allow-list passes");
}

#[test]
fn a_surface_with_no_state_rule_is_held_to_nothing_by_one() {
    // Every surface but the shell's: `notes_at` has no rule, and its act runs — pinned here
    // so adding the field cannot quietly change the dispatch for the other thirty surfaces.
    let answer = notes_at("Kernel asks")
        .act("rename", &json!({ "to": "ok" }), None, "notes#1", &open())
        .unwrap();
    assert_eq!(answer["accepted"], true);
}

/// #137: the sentence about ONE call, with the arguments it carries — and the ways there is
/// nothing to say: an action that declared no explainer, an action the surface does not
/// have, and an app whose honest sentence about THESE arguments is empty. All three are
/// answers, not failures of the lookup; only the first two are refusals.
#[test]
fn an_explainer_speaks_per_call_and_only_when_the_app_declared_one() {
    let reg = surface(
        "studio",
        None,
        vec![
            (
                Action::new("set_backend", "Choose where pictures are made from now on")
                    .arg(Param::text("kind"))
                    .explain(|args| match args["kind"].as_str().unwrap_or_default() {
                        "fake" => "After this, prompts stay on this machine.".to_string(),
                        "openai-images" => {
                            "After this, prompts go to api.openai.com and may cost money."
                                .to_string()
                        }
                        _ => String::new(),
                    }),
                Box::new(|_| Ok(json!("never reached"))),
            ),
            (
                Action::new("refresh", "Fetch the gallery again"),
                Box::new(|_| Ok(json!("never reached"))),
            ),
        ],
    );

    assert_eq!(
        reg.explain("set_backend", &json!({ "kind": "fake" })).unwrap(),
        "After this, prompts stay on this machine."
    );
    assert_eq!(
        reg.explain("set_backend", &json!({ "kind": "openai-images" })).unwrap(),
        "After this, prompts go to api.openai.com and may cost money."
    );
    // A call the app has nothing honest to say about: an empty sentence, not a refusal.
    assert_eq!(reg.explain("set_backend", &json!({ "kind": "comfyui" })).unwrap(), "");

    // No explainer declared: a refusal that says so, which is what tells an asker to draw
    // no line — and the same for an action this surface does not have, in `act`'s words.
    assert!(reg.explain("refresh", &json!({})).unwrap_err().contains("`refresh` says nothing"));
    assert!(reg.explain("nope", &json!({})).unwrap_err().starts_with("unknown action `nope`"));

    // The arguments arrive as sent — no defaults filled in, no checks run — because the
    // sentence is about the call the caller means to make, not one the dispatch accepted.
    assert_eq!(reg.explain("set_backend", &json!({ "kind": "fake", "junk": 1 })).unwrap(),
        "After this, prompts stay on this machine.");
}
