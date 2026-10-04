//! Whose each tab is, and how much the editor keeps open.
//!
//! VM 520, 4 October 2026: a mind hit the eight-tab refusal, then closed the tab in front to make
//! room. It was the person's, opened and saved by them. These pin what fixed that: every tab
//! records who opened it, `describe` says whether that was the reader, and the refusal names the
//! caller's own oldest saved tab, never a modified one. And the eight itself, which had no
//! reason behind it, is a byte budget now.
use super::*;
use crate::owner::{self, Opener};

fn tab(path: Option<&str>, text: &str, baseline: &str, by: &Opener) -> Document {
    Document {
        path: path.map(|n| std::env::temp_dir().join(format!("owner-{n}.txt"))),
        text: text.into(),
        baseline: baseline.into(),
        opened_by: by.clone(),
        ..Document::blank()
    }
}

fn agent(token: &str) -> Opener {
    Opener::Agent { token: Some(yantrik_ipc_transport::reach::token_digest(token)) }
}

#[test]
fn opened_by_you_is_true_only_for_the_caller_that_opened_the_tab() {
    let a = agent("tok-a");
    assert_eq!(a.owns(&a.clone()), Some(true));
    assert_eq!(agent("tok-b").owns(&a), Some(false), "another agent's tab is not this one's");
    assert_eq!(Opener::Person.owns(&a), Some(false), "an agent's tab is not the person's");
    assert_eq!(a.owns(&Opener::Person), Some(false), "the person's tab is not the agent's");
    assert_eq!(Opener::Person.owns(&Opener::Person), Some(true));
    assert_eq!(
        Opener::Agent { token: None }.owns(&a),
        None,
        "an agent whose call carried no token is not guessed at"
    );
    assert_eq!((Opener::Person.label(), a.label()), ("person", "agent"));
}

#[test]
fn the_open_budget_is_bytes_not_eight_tabs() {
    let small: Vec<Document> = (0..40).map(|_| Document::blank()).collect();
    assert!(document::room_for(&small, 1000), "forty small tabs leave room for more");
    let file = "x".repeat(document::MAX_BYTES);
    let mut big: Vec<Document> = (0..63).map(|_| tab(None, &file, "", &Opener::Person)).collect();
    assert!(document::room_for(&big, document::MAX_BYTES), "a 64th 1 MiB file fits");
    big.push(tab(None, &file, "", &Opener::Person));
    assert_eq!(document::open_bytes(&big), document::MAX_OPEN_BYTES);
    assert!(!document::room_for(&big, 1), "one byte past 64 MiB is refused");
    assert!(document::room_for(&big, 0), "an empty tab holds nothing and still opens");
    let many: Vec<Document> = (0..document::MAX_TABS).map(|_| Document::blank()).collect();
    assert!(!document::room_for(&many, 0), "the strip's ceiling still holds");
    assert!(
        owner::no_room(&many, &Opener::Person).starts_with("128 tabs are already open"),
        "{}",
        owner::no_room(&many, &Opener::Person)
    );
    assert!(owner::no_room(&big, &Opener::Person).starts_with("The open tabs would hold more than the 64 MiB"));
}

#[test]
fn recovery_is_bounded_by_the_budget_and_keeps_no_token() {
    let dir = fixture("recovery-bounds");
    let path = dir.join("drafts.json");
    let many: Vec<Document> = (0..20).map(|i| tab(None, &format!("draft {i}"), "", &Opener::Person)).collect();
    document::checkpoint(&path, &many).unwrap();
    assert_eq!(document::recover(&path).unwrap().len(), 20, "more than eight drafts come back");

    let too_many: Vec<Document> =
        (0..=document::MAX_TABS).map(|i| tab(None, &format!("d{i}"), "", &Opener::Person)).collect();
    document::checkpoint(&path, &too_many).unwrap();
    let err = document::recover(&path).unwrap_err();
    assert!(err.contains("too many documents"), "{err}");

    let file = "y".repeat(document::MAX_BYTES);
    let over: Vec<Document> = (0..65).map(|_| tab(None, &file, "", &Opener::Person)).collect();
    document::checkpoint(&path, &over).unwrap();
    let err = document::recover(&path).unwrap_err();
    assert!(err.contains("more text than the editor keeps open"), "{err}");

    // Larger than the editor could have written: not read at all, never parsed.
    std::fs::write(&path, vec![b' '; 4097]).unwrap();
    let err = document::recover_within(&path, 4096).unwrap_err();
    assert!(err.contains("larger than the editor ever writes"), "{err}");

    // A recovered tab is still the agent's by label, but no live agent's: its token was never
    // written down.
    let a = agent("tok-a");
    document::checkpoint(&path, &[tab(None, "the agent's draft", "", &a)]).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let Opener::Agent { token: Some(digest) } = &a else { unreachable!() };
    assert!(!written.contains(digest.as_str()) && !written.contains("tok-a"), "{written}");
    let back = document::recover(&path).unwrap();
    assert_eq!(back[0].opened_by.label(), "agent");
    assert_eq!(a.owns(&back[0].opened_by), Some(false));
}

#[test]
fn the_refusal_names_the_callers_oldest_saved_tab_and_never_a_modified_one() {
    let (a, b) = (agent("tok-a"), agent("tok-b"));
    let docs = vec![
        tab(Some("person"), "kept", "kept", &Opener::Person), // 0: the person's, saved
        tab(None, "draft", "", &a),                           // 1: A's, never written
        tab(Some("a-changed"), "new", "old", &a),             // 2: A's, changed since saved
        tab(Some("a-saved"), "done", "done", &a),             // 3: A's, saved
        tab(Some("b-saved"), "b", "b", &b),                   // 4: B's, saved
    ];
    let told = owner::no_room(&docs, &a);
    assert!(told.contains("The oldest saved tab you opened is 3 ("), "{told}");
    assert!(told.ends_with("`select_tab index=3` then `close` makes room."), "{told}");
    for modified in ["index=1", "index=2", "index=0"] {
        assert!(!told.contains(modified), "{modified}: {told}");
    }
    let told = owner::no_room(&docs, &b);
    assert!(told.contains("`select_tab index=4` then `close`"), "each agent is pointed at its own: {told}");
    let told = owner::no_room(&docs, &Opener::Person);
    assert!(told.contains("The oldest saved tab is 0 ("), "the person may close any: {told}");

    // Only modified tabs of its own: nothing is named to close.
    let told = owner::no_room(&docs[..3], &a);
    assert!(told.contains("Every tab you opened has unsaved changes"), "{told}");
    assert!(!told.contains("select_tab index="), "{told}");
}

#[test]
fn an_agent_that_opened_no_tab_is_told_so_and_shown_what_the_person_could_close() {
    let docs = vec![
        tab(None, "typing", "", &Opener::Person),           // 0: the person's, unsaved
        tab(Some("old"), "x", "x", &Opener::Person),        // 1: the person's, saved
        tab(Some("theirs"), "b", "b", &agent("tok-b")),     // 2: another agent's
    ];
    let told = owner::no_room(&docs, &agent("tok-c"));
    assert!(
        told.contains("None of the open tabs were opened by you; the person can close one (oldest saved: 1 "),
        "{told}"
    );
    assert!(!told.contains("select_tab"), "it is not handed a call to close someone else's tab: {told}");
    // The name goes into `notice`, which anyone reads back: a file outside the home is not named.
    assert!(told.contains("(hidden)") && !told.contains("owner-old"), "{told}");
    let told = owner::no_room(&docs[..1], &agent("tok-c"));
    assert!(told.contains("every one has unsaved changes"), "{told}");
}

/// Through the real window and surface. Called from the window test, which owns the platform.
pub(super) fn tabs_know_who_opened_them(
    ui: &TextEditorApp,
    s: &State,
    published: &[(Action, Handler)],
    dir: &Path,
) {
    use yantrik_app_runtime::control::AgentTokenScope;
    let as_agent = |token: &str| AgentTokenScope::enter(Some(token.to_string()));
    if ui.get_dialog() != 0 {
        action(ui, s, "cancel");
    }
    {
        let mut b = s.borrow_mut();
        b.docs = vec![Document::blank()];
        b.active = 0;
        paint(ui, &mut b, true);
    }
    let tabs = || view(ui, s).state["tabs"].clone();

    // From the window: the person's.
    ui.invoke_action("new".into());
    let seen = tabs();
    assert_eq!(seen[1]["opened_by"], "person", "{seen}");
    assert_eq!(seen[1]["opened_by_you"], true, "{seen}");

    // Through the surface with an agent's token: that agent's, and only that agent's.
    {
        let _a = as_agent("tok-owner-a");
        let answer = act_on(published, "new", serde_json::json!({ "text": "the mind's notes\n" })).unwrap();
        assert_eq!((answer["opened_by"].clone(), answer["opened_by_you"].clone()), ("agent".into(), true.into()));
        let seen = tabs();
        assert_eq!(seen[2]["opened_by"], "agent", "{seen}");
        assert_eq!(seen[2]["opened_by_you"], true, "{seen}");
        assert_eq!(seen[1]["opened_by_you"], false, "the person's tab is not the agent's: {seen}");
    }
    {
        let _b = as_agent("tok-owner-b");
        let seen = tabs();
        assert_eq!(seen[2]["opened_by_you"], false, "nor another agent's: {seen}");
    }
    let seen = tabs();
    assert_eq!((seen[2]["opened_by_you"].clone(), seen[1]["opened_by_you"].clone()), (false.into(), true.into()));

    // Opened through the worker, the asker is read when it asks; `save_as` keeps the tab its.
    let file = dir.join("owner-open.txt");
    std::fs::write(&file, "opened by an agent\n").unwrap();
    {
        let _a = as_agent("tok-owner-a");
        open(ui, s, file.clone());
        assert!(settle(ui, s));
        assert_eq!(s.borrow().docs.len(), 4);
        assert_eq!(s.borrow().docs[3].opened_by.label(), "agent");
        crate::save(ui, s, Some(dir.join("owner-saved-as.txt")), false);
        assert!(settle(ui, s));
        let b = s.borrow();
        assert!(b.docs[3].path.as_ref().unwrap().ends_with("owner-saved-as.txt"));
        assert_eq!(Opener::calling().owns(&b.docs[3].opened_by), Some(true), "save_as kept it the agent's");
    }

    // The budget full: 0 the person's saved, 1 the person's unsaved, 2 A's unsaved, 3 A's saved,
    // then the person's saved files up to exactly 64 MiB.
    let full = "z".repeat(document::MAX_BYTES);
    {
        let mut b = s.borrow_mut();
        b.docs[0] = Document {
            path: Some(dir.join("owner-person.txt")),
            text: "kept\n".into(),
            baseline: "kept\n".into(),
            ..Document::blank()
        };
        b.docs[1].text = "the person typing".into();
        let mut left = document::MAX_OPEN_BYTES - document::open_bytes(&b.docs);
        let mut n = 0;
        while left > 0 {
            let take = left.min(document::MAX_BYTES);
            b.docs.push(Document {
                path: Some(dir.join(format!("filler-{n}.txt"))),
                text: full[..take].to_string(),
                baseline: full[..take].to_string(),
                ..Document::blank()
            });
            left -= take;
            n += 1;
        }
        b.active = 0;
        paint(ui, &mut b, true);
    }
    let one_more = |token: Option<&str>| {
        let _t = AgentTokenScope::enter(token.map(str::to_string));
        act_on(published, "new", serde_json::json!({ "text": "one more" })).expect_err("no room")
    };
    let told = one_more(Some("tok-owner-a"));
    assert!(told.contains("The oldest saved tab you opened is 3 ("), "{told}");
    assert!(told.ends_with("`select_tab index=3` then `close` makes room."), "{told}");
    assert!(!told.contains("index=2") && !told.contains("index=1"), "never a modified tab: {told}");
    assert_eq!(ui.get_notice().to_string(), told, "said in the window too");
    let told = one_more(Some("tok-owner-b"));
    assert!(
        told.contains("None of the open tabs were opened by you; the person can close one (oldest saved: 0 "),
        "{told}"
    );
    let told = one_more(None);
    assert!(told.contains("The oldest saved tab is 0 ("), "{told}");
    // The same, for a file opened through the worker.
    {
        let _a = as_agent("tok-owner-a");
        let other = dir.join("owner-one-more.txt");
        std::fs::write(&other, "more\n").unwrap();
        open(ui, s, other);
        settle(ui, s);
        assert!(ui.get_notice().contains("`select_tab index=3` then `close`"), "{}", ui.get_notice());
    }

    // Following the hint makes room, and an agent may not close the person's unsaved tab.
    {
        let _a = as_agent("tok-owner-a");
        act_on(published, "select_tab", serde_json::json!({ "index": 3 })).unwrap();
        let closed = act_on(published, "close", serde_json::json!({})).unwrap();
        assert_eq!(closed["closed"], true, "{closed}");
        act_on(published, "new", serde_json::json!({ "text": "one more" })).expect("room was made");
        act_on(published, "select_tab", serde_json::json!({ "index": 1 })).unwrap();
        let err = act_on(published, "close", serde_json::json!({})).expect_err("the person's unsaved tab");
        assert!(err.contains("opened by the person and has unsaved changes"), "{err}");
        assert_eq!(ui.get_dialog(), 0, "no question was put over the person's work");
        // A saved tab of the person's is not held here; the Mind keeps its own hold on that.
        act_on(published, "select_tab", serde_json::json!({ "index": 0 })).unwrap();
        let closed = act_on(published, "close", serde_json::json!({})).unwrap();
        assert_eq!(closed["closed"], true, "{closed}");
    }

    let mut b = s.borrow_mut();
    b.docs = vec![Document::blank()];
    b.active = 0;
    paint(ui, &mut b, true);
    ui.set_notice("".into());
}
