//! The ask sits at the point of effect, in each tool, and nowhere is there a way round it
//! (#1311, decision 3). A scan of the sources, in the manner of `smtp_client`'s: every call
//! that does the thing is inside the closure the call that asks runs (`inside_ask`), or, where a
//! helper carries the ask, comes after it in the same function (`asked_before`).

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

/// Where the call that opens at `open` (the index of its `(`) closes: parentheses balanced,
/// string literals and line comments skipped.
fn call_end(text: &str, open: usize) -> usize {
    let bytes = text.as_bytes();
    let (mut depth, mut i) = (0usize, open);
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("a call that never closes");
}

/// Every occurrence of `effect` is inside the closure the ask runs (#1313, L6): within the
/// parentheses of the nearest `asks` before it, and after a closure head (`||` or `|a, b|`) in
/// them, so it is not merely somewhere after the ask in the same function, nor in the card's own
/// arguments, nor in something that runs whatever the answer.
fn inside_ask(source: &str, file: &str, effect: &str, asks: &str) {
    let text = production(source);
    let mut seen = 0;
    for (at, _) in text.match_indices(effect) {
        seen += 1;
        let start = text[..at].rfind(asks).unwrap_or_else(|| panic!("{file}: `{effect}` is not preceded by `{asks}`"));
        let open = start + text[start..].find('(').unwrap_or_else(|| panic!("{file}: `{asks}` is not a call"));
        let end = call_end(text, open);
        assert!(at < end, "{file}: `{effect}` is after the call that asks (`{asks}`), not inside it");
        let head = regex_lite_closure(&text[open..at]);
        assert!(head, "{file}: `{effect}` is inside `{asks}` but not in the closure it runs");
    }
    assert!(seen > 0, "{file}: no `{effect}` at all; the scan is looking at the wrong thing");
}

/// Whether `text` holds a closure head: `||`, or `|` names `|`.
fn regex_lite_closure(text: &str) -> bool {
    text.contains("||")
        || text.match_indices('|').any(|(at, _)| {
            let rest = &text[at + 1..];
            rest.find('|').is_some_and(|close| rest[..close].chars().all(|c| c.is_alphanumeric() || c == '_' || c == ',' || c == ' ' || c == '&' || c == ':'))
        })
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
    inside_ask(mail, "asks/mail.rs", ".open()", "act(q, card");
    asked_before(mail, "asks/mail.rs", ".seen(&mut session", ".open()");
    asked_before(mail, "asks/mail.rs", "apply(door, &mut session)", ".seen(&mut session");
    inside_ask(mail, "asks/mail.rs", "apply(door, &mut session)", "act(q, card");
    inside_ask(mail, "asks/mail.rs", "d.delete(s,", "change(Question::DeleteForGood");
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
    inside_ask(helper, "asks/calendar.rs", "backend.delete(", "act(Question::DeleteForGood");
    inside_ask(helper, "asks/calendar.rs", "backend.update(", "act(Question::ChangeItem");
}

#[test]
fn chat_messages_are_asked_about_right_before_they_are_sent() {
    inside_ask(include_str!("../whatsapp.rs"), "whatsapp.rs", "crate::whatsapp::send_message(", "asks::messages::send(");
    inside_ask(include_str!("../telegram.rs"), "telegram.rs", "crate::telegram::send_message(", "asks::messages::send(");
}

#[test]
fn planned_work_is_stored_only_inside_the_ask() {
    inside_ask(include_str!("../recipe.rs"), "recipe.rs", "RecipeStore::create(", "asks::plan(");
    absent(include_str!("../recipe.rs"), "recipe.rs", "recipe_templates::start(");
    let scheduler = include_str!("../scheduler.rs");
    inside_ask(scheduler, "scheduler.rs", "Scheduler::create(", "asks::plan(");
    inside_ask(scheduler, "scheduler.rs", "Scheduler::update(", "asks::plan(");
    let automation = include_str!("../automation.rs");
    inside_ask(automation, "automation.rs", "AutomationStore::create(", "asks::plan(");
    inside_ask(automation, "automation.rs", "Scheduler::create(", "asks::plan(");
    inside_ask(automation, "automation.rs", "Scheduler::update(", "asks::plan(");
    inside_ask(automation, "automation.rs", "AutomationStore::set_enabled(", "asks::plan(");
    inside_ask(include_str!("../task_queue.rs"), "task_queue.rs", "TaskQueue::enqueue(", "asks::plan(");
    // A run the model starts is the helper's, which asks first.
    let plans = include_str!("plans.rs");
    inside_ask(plans, "asks/plans.rs", "recipe_templates::start(&db.conn(), id_or_name, vars, None))", "super::plan(&run_card(");
}

#[test]
fn forgetting_a_topic_changes_nothing_before_the_ask() {
    absent(include_str!("../memory.rs"), "memory.rs", "tombstoned");
    let helper = include_str!("forget_topic.rs");
    inside_ask(helper, "asks/forget_topic.rs", "SET consolidation_status = 'tombstoned'", "act(Question::DeleteForGood");
    inside_ask(helper, "asks/forget_topic.rs", "DELETE FROM cortex_entities", "act(Question::DeleteForGood");
    inside_ask(helper, "asks/forget_topic.rs", "SET status = 'cancelled'", "act(Question::DeleteForGood");
}

/// The scan itself (#1313, L6): an effect that merely comes after the ask is caught.
#[test]
#[should_panic(expected = "after the call that asks")]
fn an_effect_after_the_ask_and_not_inside_it_is_caught() {
    inside_ask("fn f() { let _ = asks::plan(&card, || Ok(())); Scheduler::create(1); }", "x.rs", "Scheduler::create(", "asks::plan(");
}

#[test]
#[should_panic(expected = "not in the closure")]
fn an_effect_in_the_cards_own_arguments_is_caught() {
    inside_ask("fn f() { asks::plan(&Scheduler::create(1), || Ok(())) }", "x.rs", "Scheduler::create(", "asks::plan(");
}

#[test]
fn an_effect_inside_the_closure_passes_whatever_the_strings_and_comments_hold() {
    let source = "fn f() { asks::plan(&card(\"a ) b\"), || { // )\n Scheduler::create(1) }) }";
    inside_ask(source, "x.rs", "Scheduler::create(", "asks::plan(");
    inside_ask("fn f() { change(Question::X, |d, s| d.delete(s, 1)) }", "x.rs", "d.delete(s,", "change(Question::X");
}
