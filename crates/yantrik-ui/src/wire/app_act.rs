//! One press carried out by an app, and what the app said about it.
//!
//! `forward_to_app` starts the app and calls its action, and logs the answer: right for a
//! notification button whose card is gone. A row on a card that was pressed has to say when the
//! app could not do it ("Not opened: ..."), so this one brings the app's answer back to the
//! line the press came from.

use std::time::{Duration, Instant};

use slint::ComponentHandle;

use yantrik_ipc_transport::SyncRpcClient;

use super::for_you::Say;
use super::notifications::button_route;
use crate::App;

/// How long a cold app has to open its socket.
const COLD_START: Duration = Duration::from_secs(5);
/// How long the app has to answer: a message is read from the mail server first.
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// The actions only the shell's own press runs, as (app, action): an app declares them
/// `shell_only`, `app.act` refuses them to every caller, and `app.press` runs them for the shell
/// alone. This is the one place in the tree that sends `app.press` (a source scan test below holds
/// it to that), so the companion's `app_action`, a recipe, a notification button's relay and
/// every other path that sends `app.act` for someone else cannot reach one.
const PRESSED: &[(&str, &str)] = &[("email", "open_message_id")];

/// The method that carries `action` to `surface`: `app.press` for a press only the shell makes,
/// `app.act` for everything else.
fn method_for(surface: &str, action: &str) -> &'static str {
    if PRESSED.contains(&(surface, action)) {
        "app.press"
    } else {
        "app.act"
    }
}

/// The key of an app's `describe` state that says its first read has landed, for an app that
/// answers a call before it (Notes reads its library after its socket is up, and refuses a call
/// while it does). Email handles it inside the app; an app not listed is asked at once.
fn ready_key(app: &str) -> Option<&'static str> {
    (app == "notes").then_some("library_ready")
}

/// Whether `describe` (an app's answer) says its first read has landed, by `key`.
fn is_ready(describe: &serde_json::Value, key: &str) -> bool {
    describe["state"][key].as_bool().unwrap_or(false)
}

/// What a press says on its line: nothing when the app did it, else why it did not.
pub fn said(reply: &Result<serde_json::Value, String>) -> String {
    match reply {
        Ok(v) if v["accepted"].as_bool().unwrap_or(true) => String::new(),
        Ok(_) => "Not opened: the app did not accept it.".to_string(),
        Err(why) => format!("Not opened: {}.", why.trim().trim_end_matches('.')),
    }
}

/// Ask `app` to do `action` with `args`, starting it if it is closed, and put its answer on the
/// line `key` names through `say`. Asked on a thread of its own, never the UI thread.
pub fn act(ui: &App, app: &str, action: &str, args: serde_json::Value, key: &str, say: Say) {
    let installed = crate::apps::Catalogue::shared().get();
    let Some((surface, opens_as)) = button_route(app, &installed) else {
        say(ui, key, "Not opened: this desktop does not open that app.");
        return;
    };
    let address = format!("app-{surface}");
    if !yantrik_app_runtime::service::is_up(&address) {
        match opens_as {
            Some(name) => ui.invoke_launch_app(name.into()),
            None => {
                say(ui, key, "Not opened: that app is closed and cannot be started.");
                return;
            }
        }
    }
    let (weak, key, action) = (ui.as_weak(), key.to_string(), action.to_string());
    let _ = std::thread::Builder::new().name("yos-app-act".into()).spawn(move || {
        let deadline = Instant::now() + COLD_START;
        while Instant::now() < deadline && !yantrik_app_runtime::service::is_up(&address) {
            std::thread::sleep(Duration::from_millis(150));
        }
        let reply = if yantrik_app_runtime::service::is_up(&address) {
            SyncRpcClient::clear_breaker(&yantrik_ipc_transport::RpcServer::default_address(&address));
            let client = SyncRpcClient::for_service(&address).with_timeout(ANSWER_WITHIN);
            // A cold app is up before it has read what the press names; ask once it has, within
            // the same start-up allowance.
            if let Some(key) = ready_key(&surface) {
                while Instant::now() < deadline
                    && !client.call("app.describe", serde_json::json!({})).is_ok_and(|d| is_ready(&d, key))
                {
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
            client
                .call(method_for(&surface, &action), serde_json::json!({ "action": action, "args": args }))
                .map_err(|e| e.message)
        } else {
            Err("the app did not open".to_string())
        };
        let words = said(&reply);
        let _ = weak.upgrade_in_event_loop(move |ui| say(&ui, &key, &words));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_open_is_sent_as_a_press() {
        assert_eq!(method_for("email", "open_message_id"), "app.press");
        for (app, action) in [("email", "open_message"), ("notes", "open_note"), ("calendar", "show_date"), ("notes", "open_message_id")] {
            assert_eq!(method_for(app, action), "app.act", "{app}.{action}");
        }
    }

    /// Every `.rs` and `.py` file under `dir` (not `target`, not `.git`), as (path from `root`, text).
    fn sources(dir: &std::path::Path, root: &std::path::Path) -> Vec<(String, String)> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return found };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !matches!(&*name, "target" | ".git" | "node_modules") {
                    found.extend(sources(&path, root));
                }
            } else if name.ends_with(".rs") || name.ends_with(".py") {
                if let (Ok(text), Ok(rel)) = (std::fs::read_to_string(&path), path.strip_prefix(root)) {
                    found.push((rel.display().to_string(), text));
                }
            }
        }
        found
    }

    fn repository() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
    }

    /// Every file under the repository that holds the method's name in quotes, as paths from the
    /// repository root.
    fn files_naming(method: &str) -> Vec<String> {
        let root = repository();
        let needle = format!("\"{method}\"");
        let mut found: Vec<String> = sources(&root, &root).into_iter().filter(|(_, text)| text.contains(&needle)).map(|(path, _)| path).collect();
        found.sort();
        found
    }

    /// The non-test code of this crate, by file: each file up to its test module (`#[cfg(test)]`
    /// on a `mod`), without the files that are only tests. A `#[cfg(test)]` on a single item is
    /// kept: a helper made for tests is code too.
    fn shell_code() -> Vec<(String, String)> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let root = repository();
        let before_tests = |text: &str| {
            let lines: Vec<&str> = text.lines().collect();
            let end = (0..lines.len())
                .find(|&i| lines[i] == "#[cfg(test)]" && lines.get(i + 1).is_some_and(|next| next.starts_with("mod ") || next.starts_with("#[path")))
                .unwrap_or(lines.len());
            lines[..end].join("\n")
        };
        sources(&src, &root)
            .into_iter()
            .filter(|(path, _)| !path.ends_with("tests.rs") && !path.contains("/tests/"))
            .map(|(path, text)| (path, before_tests(&text)))
            .collect()
    }

    /// The files of `shell_code` with a line that satisfies `hit`, sorted.
    fn files_with_a_line(hit: impl Fn(&str) -> bool) -> Vec<String> {
        let mut found: Vec<String> = shell_code().into_iter().filter(|(_, code)| code.lines().any(&hit)).map(|(path, _)| path).collect();
        found.sort();
        found
    }

    /// `app.press` is the shell's own press and nothing else's. The built-in companion runs inside
    /// this process, so a path that merely sent `app.press` from here would be the companion's:
    /// only this file sends it. The one other file that names it is an app's own dispatch, which
    /// answers it.
    #[test]
    fn only_the_shells_press_sends_app_press() {
        assert_eq!(
            files_naming("app.press"),
            ["crates/yantrik-ui/src/wire/app_act.rs", "frameworks/yantrik-app/src/control.rs"],
            "a new sender of app.press can open any message by folder and id: it belongs in wire/app_act.rs and nowhere else"
        );
        let own = include_str!("app_act.rs");
        let code = own.split("#[cfg(test)]").next().unwrap();
        let sends: Vec<&str> = code.lines().filter(|l| l.contains("\"app.press\"")).collect();
        assert_eq!(sends.len(), 1, "one place in this file picks the method: {sends:?}");
    }

    /// Who can make this file send `app.press`: `method_for` sends it for any caller that names
    /// `email.open_message_id`. So the shell's own code that names that action, and the code that
    /// builds the opening that names it, are held to the two places that are the person's press on
    /// a meeting brief's row. A notice action, a For you card or a control action that came to
    /// produce one from a poster's reference would let a Mind open any message by folder and id.
    #[test]
    fn only_the_brief_can_make_the_shell_press_open_a_message() {
        // A construction, not a match arm that reads one: the name comes after any `=>` on its line.
        let builds_a_message_opening = |l: &str| l.find("Opening::Message {").is_some_and(|at| l.find("=>").map_or(true, |arrow| at > arrow));
        assert_eq!(
            files_with_a_line(builds_a_message_opening),
            ["crates/yantrik-ui/src/wire/meeting_brief.rs"],
            "`Opening::Message` is built from the brief's own mail only, never from a poster's reference"
        );
        assert_eq!(
            files_with_a_line(|l| l.contains("\"open_message_id\"")),
            ["crates/yantrik-ui/src/wire/app_act.rs", "crates/yantrik-ui/src/wire/for_you.rs"],
            "the press of an opening is the one For you row and the one table in app_act.rs"
        );
        // Not only the quoted literal: a method built with `concat!` or `format!` is caught too.
        assert_eq!(
            files_with_a_line(|l| l.contains("\"app.\"") && l.contains("\"press\"")),
            Vec::<String>::new(),
            "no file builds the name `app.press` from pieces: it is spelled out once, in app_act.rs"
        );
    }

    #[test]
    fn a_cold_notes_is_waited_for_until_its_library_has_loaded() {
        assert_eq!(ready_key("notes"), Some("library_ready"));
        assert_eq!(ready_key("email"), None, "Email waits inside the app");
        assert!(!is_ready(&serde_json::json!({ "state": { "library_ready": false } }), "library_ready"));
        assert!(!is_ready(&serde_json::json!({ "state": {} }), "library_ready"), "no word is not ready");
        assert!(is_ready(&serde_json::json!({ "state": { "library_ready": true } }), "library_ready"));
    }

    #[test]
    fn an_accepted_press_says_nothing_and_a_refused_one_says_why() {
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": true, "settled": true }))), "");
        assert_eq!(said(&Ok(serde_json::json!({ "accepted": false }))), "Not opened: the app did not accept it.");
        assert_eq!(said(&Err("no message with that id.".into())), "Not opened: no message with that id.");
    }
}
