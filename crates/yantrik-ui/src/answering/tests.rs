use super::*;

fn call(account: &str, model: &str, lane_for: Option<&str>) -> Call {
    Call {
        at: 50,
        rid: 0,
        harness: "hermes".into(),
        account: account.into(),
        model: model.into(),
        effort: None,
        status: 200,
        outcome: String::new(),
        prompt_tokens: 0,
        completion_tokens: 0,
        ms: 1,
        lane_for: lane_for.map(str::to_string),
        dropped: Vec::new(),
    }
}

fn plan(account: &str, model: &str) -> Result<Plan, Refusal> {
    Ok(Plan { account_id: account.into(), model: model.into(), lane: false })
}

fn names(id: &str) -> String {
    match id {
        "ollama-cloud" => "Ollama Cloud".into(),
        "aig" => "AIG".into(),
        other => other.into(),
    }
}

#[test]
fn the_call_log_wins_over_what_the_config_asks_for() {
    let said = resolve(Some(call("ollama-cloud", "deepseek-v4.1-flash", None)), plan("ollama-cloud", "deepseek-v4.1-flash"), names).unwrap();
    assert_eq!(said.words(), "answering \u{b7} Ollama Cloud \u{b7} deepseek-v4.1-flash");
    assert_eq!(said.id(), "ollama-cloud/deepseek-v4.1-flash");
}

#[test]
fn before_any_call_the_gateways_own_route_says_where_it_will_go() {
    let said = resolve(None, plan("aig", "qwen3.8:27b"), names).unwrap();
    assert_eq!(said.words(), "will send to AIG \u{b7} qwen3.8:27b");
    assert_eq!(said.called_at, None);
}

#[test]
fn an_answer_from_the_private_lane_says_so_and_names_the_lane_model() {
    let said = resolve(Some(call("aig", "qwen3.8:27b", Some("ollama-cloud/deepseek-v4.1-flash"))), plan("aig", "qwen3.8:27b"), names).unwrap();
    assert_eq!(said.words(), "answering \u{b7} AIG \u{b7} qwen3.8:27b \u{b7} via your lane");
    let planned = Plan { account_id: "aig".into(), model: "qwen3.8:27b".into(), lane: true };
    assert_eq!(resolve(None, Ok(planned), names).unwrap().words(), "will send to AIG \u{b7} qwen3.8:27b \u{b7} via your lane");
}

#[test]
fn a_refused_route_is_said_even_when_an_older_call_was_answered() {
    let refused = Refusal::new(404, "pick_gone", "Your pick, X, isn't offered by any of your accounts any more.");
    let said = resolve(Some(call("ollama-cloud", "deepseek-v4.1-flash", None)), Err(refused.clone()), names);
    assert_eq!(said, Err(refused));
}

#[test]
fn the_sentence_for_a_gone_pick_names_the_default_and_never_a_slug() {
    // No account named `included-model` exists here: only the model is said.
    let with_default = gone_words("Hermes", "included-model/qwen3.5:27b", Some("home/qwen3:8b"));
    assert_eq!(with_default, "Your pick, qwen3.5:27b, isn't offered by any of your accounts any more. Hermes is using your default, qwen3:8b.");
    let none = gone_words("Hermes", "included-model/qwen3.5:27b", None);
    assert!(none.starts_with("Your pick, qwen3.5:27b, isn't offered by any of your accounts any more."), "{none}");
    assert!(none.contains("no default pick") && none.contains("Hermes"), "{none}");
    assert!(!none.contains("included-model"));
}

#[test]
fn a_new_pick_is_said_as_will_send_until_a_call_to_it_succeeds() {
    // The newest ok call went to Ollama Cloud; the person has since picked qwen on AIG.
    let old = call("ollama-cloud", "deepseek-v4.1-flash", None);
    let said = resolve(Some(old.clone()), plan("aig", "qwen3.8:27b"), names).unwrap();
    assert_eq!(said.words(), "will send to AIG \u{b7} qwen3.8:27b");
    assert_eq!(said.called_at, None);
    // Same account, another model: also a change.
    let said = resolve(Some(old), plan("ollama-cloud", "gpt-oss"), names).unwrap();
    assert_eq!(said.words(), "will send to Ollama Cloud \u{b7} gpt-oss");
    // Once a call to the new route is logged, it reads "answering" again.
    let said = resolve(Some(call("aig", "qwen3.8:27b", None)), plan("aig", "qwen3.8:27b"), names).unwrap();
    assert_eq!(said.words(), "answering \u{b7} AIG \u{b7} qwen3.8:27b");
}

/// M2 of #1162: the panel, the bubble, the Lens and `describe` all read `answering`, which asks the
/// gateway for what the mind's own config asks (`asked_for`), so a config that names a model is
/// followed everywhere and not only on the runs-on card.
#[test]
fn every_face_asks_for_what_the_minds_config_asks() {
    let home = std::env::temp_dir().join(format!("answering-asked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".hermes")).unwrap();
    let config = |default: &str| format!("model:\n  base_url: {}\n  default: {default}\n", yantrik_gateway::base_url());

    std::fs::write(home.join(".hermes/config.yaml"), config("picked")).unwrap();
    assert_eq!(asked_for(&home, "hermes"), yantrik_gateway::PICKED);

    std::fs::write(home.join(".hermes/config.yaml"), config("aig/qwen3.8:27b")).unwrap();
    assert_eq!(asked_for(&home, "hermes"), "aig/qwen3.8:27b");

    // A config pointed elsewhere cannot name a model to the gateway: `picked`, as before.
    std::fs::write(home.join(".hermes/config.yaml"), "model:\n  base_url: https://openrouter.ai/api/v1\n  default: x/y\n").unwrap();
    assert_eq!(asked_for(&home, "hermes"), yantrik_gateway::PICKED);
    let _ = std::fs::remove_dir_all(&home);
}

/// #1162: Hermes's own detail line named no model after a restart, and the old one after a pick
/// changed, until its next turn. The shell says it: the head is Hermes's, the model is the route's.
#[test]
fn a_detail_line_names_the_answering_model_whatever_the_mind_announced() {
    for announced in [Some("Hermes 0.14.0"), Some("Hermes 0.14.0 \u{b7} deepseek-v4.1-flash"), Some("Hermes 0.14.0 \u{b7} picked")] {
        assert_eq!(detail_with_model(announced, "Hermes Agent", Some("qwen3.8:27b")), "Hermes 0.14.0 \u{b7} qwen3.8:27b");
    }
    assert_eq!(detail_with_model(None, "Hermes", Some("qwen3.8:27b")), "Hermes \u{b7} qwen3.8:27b");
    assert_eq!(detail_with_model(Some("  "), "Hermes", Some("m")), "Hermes \u{b7} m");
}

/// #1162 L1: Pi and DeepSeek announce their model first. It is never the head, and what they say
/// after it (Pi's version, DeepSeek's host and who picks) stays.
#[test]
fn a_mind_that_announces_its_model_first_keeps_its_name_and_the_rest() {
    assert_eq!(detail_with_model(Some("picked \u{b7} pi 1.2"), "Pi", Some("qwen3.8:27b")), "Pi \u{b7} qwen3.8:27b \u{b7} pi 1.2");
    assert_eq!(
        detail_with_model(Some("aig/qwen3.8:27b \u{b7} pi 1.2"), "Pi", Some("qwen3.8:27b")),
        "Pi \u{b7} qwen3.8:27b \u{b7} pi 1.2",
        "a model set in the session is not shown beside the shell's"
    );
    assert_eq!(
        detail_with_model(Some("picked \u{b7} aig.example.cyou \u{b7} gate picks"), "DeepSeek", Some("deepseek-v4.1-flash")),
        "DeepSeek \u{b7} deepseek-v4.1-flash \u{b7} aig.example.cyou \u{b7} gate picks"
    );
    assert!(!detail_with_model(Some("picked \u{b7} pi 1.2"), "Pi", Some("m")).starts_with("picked"));
}

/// #1162 L2: a refused route names no model, and the mind's stale one does not stand.
#[test]
fn a_refused_route_names_no_model_on_the_detail_line() {
    assert_eq!(detail_with_model(Some("Hermes 0.14.0 \u{b7} deepseek-v4.1-flash"), "Hermes", None), "Hermes 0.14.0");
    assert_eq!(detail_with_model(Some("picked \u{b7} pi 1.2"), "Pi", None), "Pi \u{b7} pi 1.2");
    assert!(REFUSED.contains("refuses"));
}

/// #1162 note: the head a harness announced goes through the host's sanitiser, and a look-alike
/// of the separator cannot join two segments into one.
#[test]
fn a_harness_head_is_sanitised_and_cannot_fake_the_separator() {
    let line = detail_with_model(Some("Hermes \u{2219} gpt-5\u{202E}\n\u{200B}x"), "Hermes", Some("qwen3.8:27b"));
    assert_eq!(line, "Hermes - gpt-5 x \u{b7} qwen3.8:27b");
    let long = detail_with_model(Some(&format!("Hermes {}", "y".repeat(5000))), "Hermes", Some("m"));
    assert!(line.chars().all(|c| !c.is_control()) && long.split(" \u{b7} ").next().unwrap().chars().count() <= 120, "{long}");
}
