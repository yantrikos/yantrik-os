//! The ask sits at the point of effect, in each tool, and nowhere is there a way round it
//! (#1311, decision 3). A scan of the sources, in the manner of `smtp_client`'s: every call
//! that does the thing is inside the call that asks, or comes after it in the same function.

fn production(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

/// Every occurrence of `effect` in `source` has `asks` before it, with no new function between.
fn asked_before(source: &str, file: &str, effect: &str, asks: &str) {
    let text = production(source);
    let mut seen = 0;
    for (at, _) in text.match_indices(effect) {
        seen += 1;
        let before = &text[..at];
        let start = before.rfind(asks).unwrap_or_else(|| panic!("{file}: `{effect}` is not preceded by `{asks}`"));
        assert!(!before[start..].contains("fn "), "{file}: `{effect}` is in another function than the one that asks (`{asks}`)");
    }
    assert!(seen > 0, "{file}: no `{effect}` at all; the scan is looking at the wrong thing");
}

fn absent(source: &str, file: &str, needle: &str) {
    assert!(!production(source).contains(needle), "{file} calls `{needle}` itself, round the ask");
}

#[test]
fn mail_is_asked_about_right_before_the_store_and_expunge() {
    let email = include_str!("../email.rs");
    // The tools reach the mailbox only through the asks, which hold no session across the card.
    for door in ["imap_client::delete_message(", "imap_client::move_message("] {
        absent(email, "email.rs", door);
    }
    for call in ["asks::mail::delete(", "asks::mail::move_to(\"email_move\"", "asks::mail::move_to(\"email_archive\""] {
        assert!(production(email).contains(call), "email.rs does not ask with `{call}`");
    }
    assert!(!production(email).contains("let (cached, account, mut session)"), "a tool holds its session across the ask");
    let mail = include_str!("mail.rs");
    asked_before(mail, "asks/mail.rs", ".open()", "act(q, card");
    asked_before(mail, "asks/mail.rs", ".seen(&mut session", ".open()");
    asked_before(mail, "asks/mail.rs", "apply(door, &mut session)", ".seen(&mut session");
    asked_before(mail, "asks/mail.rs", "d.delete(s,", "change(Question::DeleteForGood");
    // And the IMAP functions that do it have no other caller than the real door.
    let tools = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![tools];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("imap_door.rs") && !path.ends_with("imap_client.rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for door in ["imap_client::delete_message(", "imap_client::move_message("] {
                    assert!(!production(&text).contains(door), "{} reaches the IMAP effect `{door}` round the ask", path.display());
                }
            }
        }
    }
}

#[test]
fn calendar_changes_are_asked_about_before_the_backend_is_touched() {
    let tool = include_str!("../calendar.rs");
    absent(tool, "calendar.rs", "backend.delete(");
    // The one other `backend.update(` records the id Google gave a newly created event; it is in the
    // create tool, ahead of the delete tool, and changes nothing the person had.
    let prod = production(tool);
    assert_eq!(prod.matches("backend.update(").count(), 1, "calendar.rs updates the backend outside the ask");
    assert!(prod.find("backend.update(") < prod.find("struct CalendarDeleteEventTool"), "that update is in the create tool");
    assert!(production(tool).contains("asks::calendar::delete("), "delete does not ask");
    assert!(production(tool).contains("asks::calendar::update("), "update does not ask");
    let helper = include_str!("calendar.rs");
    asked_before(helper, "asks/calendar.rs", "backend.delete(", "act(Question::DeleteForGood");
    asked_before(helper, "asks/calendar.rs", "backend.update(", "act(Question::ChangeItem");
}

#[test]
fn chat_messages_are_asked_about_right_before_they_are_sent() {
    asked_before(include_str!("../whatsapp.rs"), "whatsapp.rs", "crate::whatsapp::send_message(", "asks::messages::send(");
    asked_before(include_str!("../telegram.rs"), "telegram.rs", "crate::telegram::send_message(", "asks::messages::send(");
}

#[test]
fn planned_work_is_stored_only_inside_the_ask() {
    asked_before(include_str!("../recipe.rs"), "recipe.rs", "RecipeStore::create(", "asks::plan(");
    absent(include_str!("../recipe.rs"), "recipe.rs", "recipe_templates::start(");
    let scheduler = include_str!("../scheduler.rs");
    asked_before(scheduler, "scheduler.rs", "Scheduler::create(", "asks::plan(");
    asked_before(scheduler, "scheduler.rs", "Scheduler::update(", "asks::plan(");
    let automation = include_str!("../automation.rs");
    asked_before(automation, "automation.rs", "AutomationStore::create(", "asks::plan(");
    asked_before(automation, "automation.rs", "Scheduler::create(", "asks::plan(");
    asked_before(automation, "automation.rs", "Scheduler::update(", "asks::plan(");
    asked_before(automation, "automation.rs", "AutomationStore::set_enabled(", "asks::plan(");
    asked_before(include_str!("../task_queue.rs"), "task_queue.rs", "TaskQueue::enqueue(", "asks::plan(");
    // A run the model starts is the helper's, which asks first.
    let plans = include_str!("plans.rs");
    asked_before(plans, "asks/plans.rs", "recipe_templates::start(conn, id_or_name, vars, None))", "super::plan(&run_card(");
}

#[test]
fn forgetting_a_topic_changes_nothing_before_the_ask() {
    absent(include_str!("../memory.rs"), "memory.rs", "tombstoned");
    let helper = include_str!("forget_topic.rs");
    asked_before(helper, "asks/forget_topic.rs", "SET consolidation_status = 'tombstoned'", "act(Question::DeleteForGood");
    asked_before(helper, "asks/forget_topic.rs", "DELETE FROM cortex_entities", "act(Question::DeleteForGood");
    asked_before(helper, "asks/forget_topic.rs", "SET status = 'cancelled'", "act(Question::DeleteForGood");
}
