use super::*;

fn call(account: &str, model: &str, lane_for: Option<&str>) -> Call {
    Call {
        at: 50,
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
