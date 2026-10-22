//! An agent is never told a path it may not read (#443), in any answer or in `describe`, though
//! every answer is finished off the UI thread now. The library is under the temporary directory,
//! outside the home: the person sees its paths, an agent does not.

use serde_json::json;

use super::open;

#[test]
fn an_agent_is_told_the_track_and_never_a_path_outside_the_home() {
    let Some(f) = open("agent-paths") else { return };
    let dir = f.dir_text();
    let mine = f.act("play", json!({"index": 3})).unwrap();
    assert!(mine["path"].as_str().is_some_and(|p| p.ends_with("Alpha 002.mp3")), "{mine}");
    let theirs = f.agent("play", json!({"index": 3})).unwrap();
    assert_eq!(theirs["track"], "Alpha 002", "{theirs}");
    assert_eq!(theirs["path"], "(hidden)", "{theirs}");
    assert!(!theirs.to_string().contains(&dir), "{theirs}");
    for (name, args) in [("pause", json!({})), ("play", json!({})), ("seek", json!({"seconds": 1.0})), ("set_volume", json!({"percent": 50})), ("previous", json!({}))] {
        let answer = f.agent(name, args).unwrap();
        assert!(!answer.to_string().contains(&dir), "{name}: {answer}");
    }
    let scanned = f.agent("rescan", json!({})).unwrap();
    assert_eq!(scanned["folders"][0]["path"], "(hidden)", "{scanned}");
    assert!(!scanned.to_string().contains(&dir), "{scanned}");
    let described = f.agent_state();
    assert!(!described.to_string().contains(&dir), "describe hides them too: {described}");
    assert_eq!(described["path"], "(hidden)");
    assert_eq!(described["folders"][0]["path"], "(hidden)");
    // The person's own describe has them all.
    assert!(f.state()["path"].as_str().is_some_and(|p| p.starts_with(&dir)));
    assert!(f.state()["folders"][0]["path"].as_str().is_some_and(|p| p.starts_with(&dir)));
}

#[test]
fn an_unreadable_folder_is_named_to_the_person_and_hidden_from_an_agent() {
    let Some(f) = open("agent-status") else { return };
    let gone = f.dir.join("not-there");
    f.app.ui({
        let gone = gone.to_string_lossy().into_owned();
        move |ui| ui.invoke_scan_add_folder(gone.into())
    });
    f.quiet();
    // A folder that is not there is refused; one that cannot be read is scanned and named.
    assert!(f.notice().starts_with("Not a folder"), "{}", f.notice());
    let locked = f.dir.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    f.app.ui({
        let locked = locked.to_string_lossy().into_owned();
        move |ui| ui.invoke_scan_add_folder(locked.into())
    });
    f.quiet();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    }
    let person = f.act("rescan", json!({})).unwrap();
    let agent = f.agent("rescan", json!({})).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    if person["unreadable"].as_array().is_some_and(|u| u.is_empty()) {
        eprintln!("this account reads a mode-0 folder (root); the unreadable case is skipped");
        return;
    }
    assert!(person["summary"].as_str().unwrap().contains(&locked.to_string_lossy().into_owned()), "{person}");
    assert!(agent["summary"].as_str().unwrap().ends_with("could not read this folder: (hidden)"), "{agent}");
    assert_eq!(agent["unreadable"][0]["path"], "(hidden)");
    assert!(f.agent_state()["status"].as_str().unwrap().contains("(hidden)"));
    assert!(f.state()["status"].as_str().unwrap().contains(&locked.to_string_lossy().into_owned()));
}

#[test]
fn mpv_playing_another_file_than_asked_is_named_to_the_person_and_hidden_from_an_agent() {
    let Some(f) = open("instead") else { return };
    let elsewhere = f.dir.join("elsewhere.mp3");
    std::fs::write(&elsewhere, b"").unwrap();
    let err = f.act("play_file", json!({"path": elsewhere})).unwrap_err();
    assert!(err.contains("mpv is playing /var/elsewhere/secret.mp3 instead"), "{err}");
    // An agent may name only a file in the home, so this part needs one there.
    let home = yantrik_foundation::place::home();
    let in_home = home.join(format!("yantrik-music-test-{}", std::process::id()));
    if std::fs::create_dir_all(&in_home).is_err() {
        eprintln!("the home is not writable here; the agent's play_file case is skipped");
        return;
    }
    let asked = in_home.join("elsewhere.mp3");
    std::fs::write(&asked, b"").unwrap();
    let err = f.agent("play_file", json!({"path": asked}));
    let _ = std::fs::remove_dir_all(&in_home);
    let err = err.unwrap_err();
    assert!(err.contains("mpv is playing (hidden) instead"), "{err}");
    assert!(!err.contains("/var/elsewhere"), "{err}");
}

#[test]
fn an_agent_may_name_only_a_file_in_the_home_and_is_told_so_before_it_is_looked_at() {
    let Some(f) = open("agent-file") else { return };
    let err = f.agent("play_file", json!({"path": f.track("Beta", 1)})).unwrap_err();
    assert!(!err.contains("not a sound file") && !err.contains("no file"), "the rule speaks first: {err}");
    assert_eq!(f.notice(), err, "said in the window too");
    assert_eq!(f.state()["queue_length"], 0, "nothing was queued");
    let missing = f.agent("play_file", json!({"path": "/etc/passwd"})).unwrap_err();
    assert!(!missing.contains("root:"), "{missing}");
}

/// An agent's `play_file` of a path outside the home is refused in the rule's words, with the
/// path it gave said as one line of plain text (#937 delta review, finding C).
#[test]
fn an_agents_refused_path_is_said_as_one_line_of_plain_text() {
    let Some(f) = open("agent-name") else { return };
    let odd = f.dir.join("two\nlines\u{202E}.mp3");
    std::fs::write(&odd, b"").unwrap();
    let err = f.agent("play_file", json!({"path": odd})).unwrap_err();
    assert!(err.contains("two\u{FFFD}lines\u{FFFD}.mp3"), "{err}");
    for said in [err, f.notice(), f.summary()] {
        assert!(!said.contains('\n') && !said.contains('\u{202E}'), "{said:?}");
    }
}
