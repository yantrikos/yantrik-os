//! `set_backend`, and the grade that follows it: where prompts go, what that costs, and that a
//! prompt is never offered at the grade of a backend it is not going to.

use serde_json::json;

use super::{apply, apply_as_agent, bare, fake, homeless, just_started};
use crate::command::{BackendAsk, BackendSet, Command, Found, Next};
use crate::config::{Config, Kind, KINDS};
use crate::gallery::Sidecar;
use crate::plan::Plan;

#[test]
fn set_backend_moves_where_prompts_go_and_what_that_is_graded() {
    let f = super::started("backend", r#"{"backend":{"kind":"fake"},"output_folder":"~/Pictures/Studio"}"#);
    let grade = |f: &super::Fixture| f.app.model(|m| (m.grade(), m.published_grade));
    assert_eq!(grade(&f), ("standard", "standard"));

    // A ComfyUI server on the person's own LAN is a local generation.
    let answer = f
        .act(
            "set_backend",
            json!({ "kind": "comfyui", "base_url": "http://192.168.4.35:8188/", "model": "mine.safetensors" }),
        )
        .unwrap();
    assert_eq!(answer["backend"], json!("comfyui"));
    assert_eq!(answer["configured"], json!(true));
    assert_eq!(answer["generate_is_now_graded"], json!("standard"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(false));
    assert_eq!(answer["where"], json!("a ComfyUI server you reach at http://192.168.4.35:8188"));
    assert_eq!(grade(&f), ("standard", "standard"));
    assert!(f.summary().contains("192.168.4.35"), "{}", f.summary());

    // The choice is written down where this instance was started with, so it survives a restart,
    // and the folder the person already had stays theirs.
    let settings = f.world.places.settings.clone().unwrap();
    let on_disk = Config::parse(&std::fs::read_to_string(&settings).unwrap()).unwrap();
    assert_eq!(on_disk.backend.kind, Kind::ComfyUi);
    assert_eq!(on_disk.backend.base_url, "http://192.168.4.35:8188", "the trailing slash was kept");
    assert_eq!(on_disk.backend.model, "mine.safetensors");
    assert_eq!(on_disk.output_folder, "~/Pictures/Studio");

    // The same kind, pointed somewhere the person does not own, is a different act.
    let answer = f.act("set_backend", json!({ "kind": "comfyui", "base_url": "http://203.0.113.7:8188" })).unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("sensitive"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(true));
    assert!(answer["where"].as_str().unwrap().contains("away from your network"), "{answer}");
    assert_eq!(grade(&f), ("sensitive", "sensitive"));

    // A hosted service is `sensitive` however cheap or trustworthy it is: the words leave the
    // machine and the service may bill for them.
    let answer = f
        .act(
            "set_backend",
            json!({ "kind": "openai-images", "model": "gpt-image-1", "api_key_env": "STUDIO_ABSENT_KEY" }),
        )
        .unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("sensitive"));
    assert_eq!(answer["prompt_leaves_this_machine"], json!(true));
    // The name of a variable that is not set is not said (it may be a key pasted where a name goes).
    assert_eq!(answer["api_key_env"], serde_json::Value::Null);
    assert!(answer["note"].as_str().unwrap().contains("is not set"), "{answer}");
    assert_eq!(grade(&f), ("sensitive", "sensitive"));
    // A generation that cannot be sent says so rather than failing quietly in a worker.
    f.act("generate", json!({ "prompt": "a lighthouse" })).unwrap();
    let notice = f.settled();
    assert!(notice.contains("openai-images could not make the picture"), "{notice}");
    assert!(notice.contains("the environment variable named in studio.json's `api_key_env` is not set"), "{notice}");
    assert!(!notice.contains("STUDIO_ABSENT_KEY"), "{notice}");
    assert_eq!(f.state()["backend"]["api_key_is_set"], json!(false));

    // And back to the placeholder, which is local again.
    let answer = f.act("set_backend", json!({ "kind": "fake" })).unwrap();
    assert_eq!(answer["generate_is_now_graded"], json!("standard"));
    assert!(answer["note"].as_str().unwrap().contains("placeholder"), "{answer}");
    assert_eq!(grade(&f), ("standard", "standard"));
}

#[test]
fn a_backend_that_does_not_exist_leaves_the_app_as_it_was() {
    let f = fake("bad-kind");
    let problem = f.act("set_backend", json!({ "kind": "midjourney" })).unwrap_err();
    for kind in KINDS {
        assert!(problem.contains(kind), "{problem} does not name {kind}");
    }
    assert!(problem.contains("midjourney"), "{problem}");
    // Nothing was written and nothing changed.
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::Fake);
    assert!(!f.world.places.settings.as_ref().unwrap().exists());
    // Nor was a hosted backend with no model saved: the file would have been left holding a
    // configuration that cannot work, and the next start would have found it.
    let problem = f.act("set_backend", json!({ "kind": "openai-images", "base_url": "https://x.example/v1" })).unwrap_err();
    assert!(problem.contains("`model`"), "{problem}");
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::Fake);
    assert!(!f.world.places.settings.as_ref().unwrap().exists());
}

#[test]
fn a_hosted_backend_whose_key_is_not_exported_is_saved_and_says_what_is_missing() {
    // Saving it is right: the person may export the key a moment later, and refusing would have
    // them configure the same thing twice. But the summary has to lead with the problem, or the
    // window says "a hosted service" while every generation fails.
    std::env::remove_var("STUDIO_TEST_UNSET_KEY");
    let f = fake("no-key");
    f.act(
        "set_backend",
        json!({ "kind": "openai-images", "model": "gpt-image-1", "api_key_env": "STUDIO_TEST_UNSET_KEY" }),
    )
    .unwrap();
    assert!(f.notice().contains("named in studio.json's `api_key_env`") && !f.notice().contains("STUDIO_TEST_UNSET_KEY"), "{:?}", f.notice());
    assert!(
        f.summary().starts_with("Studio \u{2014} the environment variable"),
        "{}",
        f.summary()
    );
    assert_eq!(f.state()["backend"]["api_key_is_set"], json!(false));
    // The configuration is still on disk, so exporting the key is the only thing left to do, and
    // what is on disk is the variable's name.
    let written = std::fs::read_to_string(f.world.places.settings.as_ref().unwrap()).unwrap();
    assert!(written.contains("STUDIO_TEST_UNSET_KEY"));
}

#[test]
fn a_backend_with_nowhere_to_write_it_down_says_it_will_not_survive_a_restart() {
    let f = homeless("no-home", r#"{"backend":{"kind":"fake"}}"#);
    f.act("set_backend", json!({ "kind": "comfyui", "base_url": "http://127.0.0.1:8188" })).unwrap();
    assert!(f.notice().contains("will not survive a restart"), "{:?}", f.notice());
    // The change still took effect for this session, which is what was asked for.
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::ComfyUi);
}

#[test]
fn a_workflow_file_that_is_not_the_api_format_is_refused_before_it_is_saved() {
    let f = fake("workflow");
    let editor_format = f.world.dir.join("from-the-editor.json");
    std::fs::write(&editor_format, r#"{"last_node_id": 9, "nodes": [{"id": 3, "type": "KSampler"}], "links": []}"#)
        .unwrap();
    let problem = f
        .act(
            "set_backend",
            json!({
                "kind": "comfyui",
                "base_url": "http://127.0.0.1:8188",
                "workflow": editor_format.display().to_string(),
            }),
        )
        .unwrap_err();
    assert!(problem.contains("API Format"), "{problem}");
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::Fake, "the broken workflow was saved anyway");
    assert!(!f.world.places.settings.as_ref().unwrap().exists());
}

/// The surface is graded when it is served: a machine already pointed at a hosted service must not
/// offer a prompt at the local grade, not even for the first call.
#[test]
fn a_hosted_backend_is_graded_sensitive_from_the_first_call() {
    let hosted = Config::parse(
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_ABSENT_KEY"}}"#,
    )
    .unwrap();
    let f = just_started("hosted-start", hosted, |_| {});
    // Before the first command after the surface is served, the two actions that send a prompt are
    // refused, so none can be offered at the grade they were declared with.
    assert!(!f.app.model(|m| m.graded));
    let problem = f.act("generate", json!({ "prompt": "a lighthouse" })).unwrap_err();
    assert!(problem.contains("still starting"), "{problem}");
    let problem = f.act("variations", json!({ "of": "x.png" })).unwrap_err();
    assert!(problem.contains("still starting"), "{problem}");
    assert!(f.app.model(|m| m.queue.is_empty()));
    // Anything that sends nothing is not held up.
    assert!(f.act("refresh", json!({})).is_ok());

    f.until("the grade to be published", |f| f.app.model(|m| m.graded));
    assert_eq!(f.app.model(|m| m.published_grade), "sensitive");
    // Now it is taken (and fails for want of a key, which is the backend's answer, not a refusal).
    f.act("generate", json!({ "prompt": "a lighthouse" })).unwrap();
    assert!(f.settled().contains("could not make the picture"));
}

/// A picture is made by the backend it was asked against, whatever is configured by the time its
/// turn comes: an approved sentence is never sent somewhere the person did not approve.
#[test]
fn a_queued_picture_goes_to_the_backend_it_was_asked_against() {
    let (_world, mut m) = bare("queued-backend", r#"{"backend":{"kind":"fake"}}"#);
    apply(&mut m, Command::Generate(Some(Box::new(Plan::new("a lighthouse")))));
    assert_eq!(m.queue.len(), 1);
    let hosted = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://203.0.113.7:8188"}}"#).unwrap();
    apply(&mut m, Command::BackendSet(Ok(BackendSet { config: hosted, written: true })));
    assert_eq!(m.backend.kind(), "comfyui", "the new backend is the one configured now");
    assert_eq!(m.queue[0].backend.kind(), "fake", "a picture already asked for moved to the new backend");
    assert_eq!(m.published_grade, "sensitive");
}

/// A variation approved against one backend and found, by the worker that read its record, to be
/// landing against another is asked for again.
#[test]
fn a_variation_is_not_sent_to_a_backend_changed_while_its_record_was_read() {
    let (world, mut m) = bare("variation-backend", r#"{"backend":{"kind":"fake"}}"#);
    let path = world.dir.join("x.png");
    let sidecar = Sidecar {
        prompt: "a lighthouse".into(),
        negative: String::new(),
        seed: 1,
        backend: "fake".into(),
        model: String::new(),
        seconds: 0.1,
        width: 64,
        height: 64,
        sent: String::new(),
        created: "2026-10-27T00:00:00+00:00".into(),
        made_from: String::new(),
        steps: Some(4),
        cfg: None,
        made_by: "yantrik-studio".into(),
    };
    let asked = Next::Variations { of: "x.png".into(), count: 1, fill: false, generation: m.backend_generation };
    // The backend is changed while the record is being read.
    let hosted = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://203.0.113.7:8188"}}"#).unwrap();
    apply(&mut m, Command::BackendSet(Ok(BackendSet { config: hosted, written: true })));
    let fx = apply_as_agent(
        &mut m,
        Command::Resolved { next: asked, result: Ok(Found { path, sidecar: Some(sidecar) }) },
    );
    assert!(m.queue.is_empty(), "the sentence was queued against a backend it was not approved for");
    assert!(m.notice.contains("backend was changed"), "{}", m.notice);
    assert!(fx.requests().iter().any(|r| r.name() == "settle"), "the call was not answered");
}

#[test]
fn while_a_change_is_being_written_nothing_is_sent_and_a_second_change_waits() {
    let (_world, mut m) = bare("changing", r#"{"backend":{"kind":"fake"}}"#);
    let ask = || BackendAsk {
        kind: "comfyui".into(),
        base_url: "http://127.0.0.1:8188".into(),
        model: String::new(),
        api_key_env: String::new(),
        workflow: String::new(),
    };
    let fx = apply(&mut m, Command::SetBackend(Box::new(ask())));
    assert!(m.changing_backend);
    assert_eq!(fx.requests().iter().filter(|r| r.name() == "spawn").count(), 1);

    apply(&mut m, Command::Generate(Some(Box::new(Plan::new("a lighthouse")))));
    assert!(m.queue.is_empty(), "a prompt was queued while the choice of where it goes was being written");
    assert!(m.notice.contains("changing where pictures are made"), "{}", m.notice);

    let fx = apply(&mut m, Command::SetBackend(Box::new(ask())));
    assert_eq!(fx.requests().iter().filter(|r| r.name() == "spawn").count(), 0, "a second write was started");
    assert!(m.notice.contains("already changing"), "{}", m.notice);

    // The write fails: the app is no longer waiting for it, and nothing changed.
    apply(&mut m, Command::BackendSet(Err("the disk is full".into())));
    assert!(!m.changing_backend);
    assert_eq!(m.config.backend.kind, Kind::Fake);
    assert_eq!(m.notice, "the disk is full");
}

/// Where Studio sends prompts, and which credential goes with them, is the person's choice: an
/// agent that could name the host and the variable could have the value of any environment variable
/// sent to a host it picked as a Bearer token. Every agent call is refused, and nothing is
/// connected to.
#[test]
fn an_agent_cannot_change_where_prompts_go_and_no_request_ever_leaves() {
    use std::net::TcpListener;
    const SECRET_VARIABLE: &str = "STUDIO_TEST_AGENT_SECRET";
    std::env::set_var(SECRET_VARIABLE, "sk-not-a-real-secret-4471");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    let f = fake("agent-backend");
    for call in [
        json!({ "kind": "openai-images", "base_url": url, "api_key_env": SECRET_VARIABLE, "model": "x" }),
        json!({ "kind": "comfyui", "base_url": url }),
        json!({ "kind": "fake" }),
        json!({ "kind": "openai-images", "model": "x", "api_key_env": "HOME" }),
    ] {
        assert_eq!(f.agent("set_backend", call.clone()), Err(crate::update::AGENT_REFUSED.to_string()), "{call}");
    }
    assert_eq!(
        crate::update::AGENT_REFUSED,
        "Changing where Studio sends prompts is the person's choice; ask them to edit ~/.config/yantrik/studio.json."
    );
    // Nothing changed, nothing was written, and the notice is the same sentence.
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::Fake);
    assert!(!f.world.places.settings.as_ref().unwrap().exists());
    // The framework refused it before the app was asked, so the app has nothing to say.
    assert_eq!(f.notice(), "");
    // So a generation afterwards goes nowhere near the listener.
    f.agent("generate", json!({ "prompt": "hello", "width": 64, "height": 64 })).unwrap();
    assert_eq!(f.settled(), "");
    assert!(listener.accept().is_err(), "a request reached the listener an agent named");

    // The person's own call, with the same arguments, is unchanged.
    let answer = f.act("set_backend", json!({ "kind": "comfyui", "base_url": url })).unwrap();
    assert_eq!(answer["backend"], json!("comfyui"));
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::ComfyUi);
    std::env::remove_var(SECRET_VARIABLE);
}

/// The same refusal through the framework's real dispatch (the registry the socket serves, with
/// the authority a test pins), not a copy of the wrapper: an agent's call in every mode the
/// framework has, granted or not, is refused with the one sentence before any grade, approval or
/// handler is reached; the configuration file is byte-identical afterwards; and an in-process
/// listener the call names gets no connection.
#[test]
fn the_real_dispatch_refuses_an_agents_set_backend_in_every_mode() {
    use std::net::TcpListener;
    use yantrik_testing::Call;
    const SECRET_VARIABLE: &str = "STUDIO_TEST_DISPATCH_SECRET";
    std::env::set_var(SECRET_VARIABLE, "sk-not-a-real-secret-9921");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());

    let f = fake("dispatch-backend");
    // The person's own choice, written down: the file the agent must not be able to change.
    f.act("set_backend", json!({ "kind": "comfyui", "base_url": "http://127.0.0.1:8188" })).unwrap();
    let settings = f.world.places.settings.clone().unwrap();
    let before = std::fs::read(&settings).unwrap();

    for mode in ["plan", "ask", "auto", "bypass", "bypass_all"] {
        for granted in [false, true] {
            let mut call = Call::new(
                "set_backend",
                json!({ "kind": "openai-images", "base_url": url, "api_key_env": SECRET_VARIABLE, "model": "x" }),
            )
            .as_agent(super::AGENT)
            .in_mode(mode);
            if granted {
                call = call.granted();
            }
            let refused = f.app.act_as(call).unwrap_err();
            assert_eq!(refused, crate::update::AGENT_REFUSED, "mode {mode}, granted {granted}");
        }
    }
    assert_eq!(std::fs::read(&settings).unwrap(), before, "the configuration file changed");
    assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::ComfyUi);
    assert!(f.app.model(|m| !m.changing_backend), "a worker was started for a refused call");
    assert_eq!(f.app.jobs_waiting(), 0);
    assert!(listener.accept().is_err(), "a request reached the listener an agent named");
    std::env::remove_var(SECRET_VARIABLE);
}

/// The person's card says which credential goes with the prompt, by name and never by value.
#[test]
fn the_persons_card_names_the_credential_that_goes_with_the_prompt() {
    const NAMED: &str = "STUDIO_TEST_CARD_SECRET";
    std::env::set_var(NAMED, "sk-this-value-must-never-be-said-3303");
    let (set_backend, _) =
        crate::intents::actions().into_iter().find(|(a, _)| a.name == "set_backend").unwrap();
    let explains = |args: serde_json::Value| set_backend.explainer.as_ref().unwrap().sentence(&args);
    assert_eq!(
        explains(json!({ "kind": "openai-images", "base_url": "https://img.example/v1", "api_key_env": NAMED })),
        "After this, prompts and the value of $STUDIO_TEST_CARD_SECRET go to img.example."
    );
    // The variable's default name, when none is given.
    let default_set = std::env::var_os("OPENAI_API_KEY").is_some();
    assert_eq!(
        explains(json!({ "kind": "openai" })),
        if default_set {
            "After this, prompts and the value of $OPENAI_API_KEY go to api.openai.com."
        } else {
            "After this, prompts and the value of the environment variable you named go to api.openai.com."
        }
    );
    // A name that is not set is not said: it may be a key pasted where a name goes.
    assert_eq!(
        explains(json!({ "kind": "openai-images", "api_key_env": "STUDIO_TEST_NEVER_SET_VARIABLE" })),
        "After this, prompts and the value of the environment variable you named go to api.openai.com."
    );
    // Only a name that is a name is said; anything else gets no sentence rather than a guess.
    for odd in ["A B", "X; rm -rf", "a\nb", "$(id)"] {
        assert_eq!(explains(json!({ "kind": "openai-images", "api_key_env": odd })), "", "{odd:?}");
    }
    // A kind that sends no key says what it always said.
    assert_eq!(explains(json!({ "kind": "fake" })), "After this, prompts stay on this machine.");
    assert_eq!(
        explains(json!({ "kind": "comfyui", "base_url": "http://gpu-box.lan:8188" })),
        "After this, prompts go to the ComfyUI server at gpu-box.lan."
    );
    for kind in ["openai-images", "comfyui", "fake"] {
        let said = explains(json!({ "kind": kind, "api_key_env": NAMED }));
        assert!(!said.contains("sk-this-value"), "the card says a value: {said}");
    }
    std::env::remove_var(NAMED);
}

/// The app's own second lock, driven as the framework would drive it if a call ever reached the
/// command without the dispatch's refusal: an agent's `SetBackend` is refused with the sentence, no
/// worker is started, and nothing is written.
#[test]
fn the_apps_own_lock_refuses_an_agents_set_backend() {
    use yantrik_app::effects::Request;
    let (world, mut m) = bare("second-lock", r#"{"backend":{"kind":"fake"}}"#);
    let ask = BackendAsk {
        kind: "openai-images".into(),
        base_url: "http://203.0.113.7:1".into(),
        model: "x".into(),
        api_key_env: "HOME".into(),
        workflow: String::new(),
    };
    let fx = apply_as_agent(&mut m, Command::SetBackend(Box::new(ask.clone())));
    assert!(matches!(fx.requests().last(), Some(Request::Settle(Err(r))) if r.sentence == crate::update::AGENT_REFUSED));
    assert_eq!(m.notice, crate::update::AGENT_REFUSED);
    assert!(fx.requests().iter().all(|r| r.name() != "spawn"), "a worker was started for a refused call");
    assert!(!m.changing_backend);
    assert_eq!(m.config.backend.kind, Kind::Fake);
    assert!(!world.places.settings.as_ref().unwrap().exists(), "something was written");
    // The person's own call is not held up by it.
    let fx = apply(&mut m, Command::SetBackend(Box::new(ask)));
    assert!(m.changing_backend);
    assert_eq!(fx.requests().iter().filter(|r| r.name() == "spawn").count(), 1);
}

/// A key variable's name has to be a name: refused with a plain sentence by `set_backend`, and by
/// the reader of a hand-edited file, so the card and what happens agree.
#[test]
fn a_key_variable_has_to_be_named_like_one() {
    let f = fake("variable-names");
    for odd in ["A B", "1KEY", "KEY-NAME", "KEY;rm", "$(id)", "k\u{e9}y"] {
        let problem = f
            .act("set_backend", json!({ "kind": "openai-images", "model": "x", "api_key_env": odd }))
            .unwrap_err();
        assert!(problem.contains("has to be the name of an environment variable"), "{odd:?}: {problem}");
        assert!(!f.world.places.settings.as_ref().unwrap().exists(), "{odd:?} was written down");
        assert_eq!(f.app.model(|m| m.config.backend.kind), Kind::Fake);
        let from_file = Config::parse(&format!(
            r#"{{"backend":{{"kind":"openai-images","model":"x","api_key_env":{}}}}}"#,
            serde_json::to_string(odd).unwrap()
        ))
        .unwrap_err();
        assert!(from_file.contains("has to be the name of an environment variable"), "{odd:?}: {from_file}");
    }
    for good in ["OPENAI_API_KEY", "_x", "a1_B2"] {
        assert!(Config::parse(&format!(r#"{{"backend":{{"kind":"openai-images","model":"x","api_key_env":"{good}"}}}}"#)).is_ok(), "{good}");
    }
    // None named is the default, and the card and the behaviour agree on it.
    assert!(f.act("set_backend", json!({ "kind": "openai-images", "model": "x" })).is_ok());
    assert_eq!(f.app.model(|m| m.config.backend.api_key_env.clone()), "OPENAI_API_KEY");
}
