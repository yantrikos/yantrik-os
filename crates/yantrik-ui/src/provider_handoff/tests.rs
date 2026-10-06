use super::*;

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("handoff-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config/yantrik")).unwrap();
        Home(dir)
    }
    fn deepseek(&self) -> PathBuf {
        self.0.join(".config/yantrik/deepseek.json")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const KEY: &str = "nvapi-SECRET-4f3e2d1c";

fn nim() -> ProviderStoreEntry {
    ProviderStoreEntry {
        id: "p-nim".into(),
        name: "NVIDIA NIM".into(),
        provider_type: "nvidia-nim".into(),
        base_url: "https://integrate.api.nvidia.com/v1".into(),
        api_key: Some(KEY.into()),
        auth_type: "bearer".into(),
        is_primary: true,
        is_fallback: false,
        model: "nvidia/nemotron-3-super-120b-a12b".into(),
    }
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn deepseek_is_given_the_address_model_and_key_and_keeps_the_rest_of_its_file() {
    let home = Home::new("merge");
    std::fs::write(home.deepseek(), r#"{"base_url":"https://api.deepseek.com/v1","api_key_env":"DEEPSEEK_API_KEY","max_steps":12,"decider":{"kind":"jev"}}"#).unwrap();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();

    let written = json(&home.deepseek());
    assert_eq!(written["base_url"], "https://integrate.api.nvidia.com/v1");
    assert_eq!(written["model"], "nvidia/nemotron-3-super-120b-a12b");
    assert_eq!(written["api_key"], KEY, "the key is in the file the harness reads");
    assert!(written.get("api_key_env").is_none(), "a variable naming a key it no longer uses is dropped");
    assert_eq!(written["max_steps"], 12, "the person's other settings stay");
    assert_eq!(written["decider"]["kind"], "jev");
}

#[test]
fn the_key_is_in_the_600_file_and_nowhere_the_person_or_a_log_can_read_it() {
    let home = Home::new("secret");
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    let card = plan.card(&home.0);
    assert!(!card.contains(KEY) && card.contains("~/.config/yantrik/deepseek.json"), "{card}");
    assert!(!format!("{plan:?}").contains(KEY), "not even in Debug");
    let m = apply(&home.0, &plan).unwrap();
    let marker_text = std::fs::read_to_string(marker_path(&home.0, "deepseek")).unwrap();
    assert!(!marker_text.contains(KEY), "{marker_text}");
    assert_eq!(m.provider_name, "NVIDIA NIM");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [home.deepseek(), marker_path(&home.0, "deepseek")] {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{} is {mode:o}", p.display());
        }
    }
}

#[test]
fn revert_puts_back_the_persons_own_file_even_after_two_assignments() {
    let home = Home::new("revert");
    let original = r#"{"base_url":"https://api.deepseek.com/v1","api_key_env":"DEEPSEEK_API_KEY"}"#;
    std::fs::write(home.deepseek(), original).unwrap();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();
    // A second assignment must not keep the first assignment as "the original".
    let mut other = nim();
    other.name = "OpenRouter".into();
    other.provider_type = "openrouter".into();
    other.base_url = "https://openrouter.ai/api/v1".into();
    other.model = "x/y".into();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &other).unwrap();
    apply(&home.0, &plan).unwrap();
    assert!(row_line(&home.0, "deepseek").contains("OpenRouter · x/y"));

    revert(&home.0, "deepseek").unwrap();
    assert_eq!(std::fs::read_to_string(home.deepseek()).unwrap(), original);
    assert!(marker(&home.0, "deepseek").is_none());
    assert_eq!(row_line(&home.0, "deepseek"), "Provider: its own settings");
}

#[test]
fn a_harness_with_no_file_of_its_own_has_the_written_one_removed_on_revert() {
    let home = Home::new("fresh");
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();
    assert!(home.deepseek().exists());
    revert(&home.0, "deepseek").unwrap();
    assert!(!home.deepseek().exists(), "there was nothing before, so there is nothing after");
}

#[test]
fn a_provider_with_no_model_chosen_is_refused_with_what_to_do() {
    let home = Home::new("nomodel");
    let mut p = nim();
    p.model.clear();
    let err = adapter_for("deepseek").unwrap().plan(&home.0, &p).unwrap_err();
    assert!(err.contains("pick one"), "{err}");
    assert!(!home.deepseek().exists(), "nothing written");
}

#[test]
fn a_file_that_is_not_json_is_left_alone() {
    let home = Home::new("garbage");
    std::fs::write(home.deepseek(), "not json at all").unwrap();
    assert!(adapter_for("deepseek").unwrap().plan(&home.0, &nim()).is_err());
    assert_eq!(std::fs::read_to_string(home.deepseek()).unwrap(), "not json at all");
}

#[test]
fn only_harnesses_with_an_adapter_offer_it() {
    assert!(adapter_for("deepseek").is_some());
    for id in ["pi", "hermes", "openclaw", "companion", "mind"] {
        assert!(adapter_for(id).is_some(), "{id} can be given Yantrik models");
    }
    assert!(adapter_for("echo").is_none(), "a harness nobody wrote an adapter for keeps its own settings");
    let home = Home::new("line");
    assert_eq!(row_line(&home.0, "echo"), "");
    assert_eq!(row_line(&home.0, "hermes"), "Provider: its own settings");
}

#[test]
fn a_known_provider_at_another_address_is_allowed_and_the_card_says_whose_it_is_not() {
    let home = Home::new("override");
    let mut own_box = nim();
    own_box.base_url = "http://my-nim.lan:8000/v1".into();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &own_box).unwrap();
    assert_eq!(plan.destination, "http://my-nim.lan:8000/v1");
    let card = plan.card(&home.0);
    assert!(card.contains("my-nim.lan:8000 (not NVIDIA NIM's own address, integrate.api.nvidia.com)"), "{card}");
    assert!(plan.sentence(&home.0).contains("not NVIDIA NIM's own address"));

    // Its own address, and local runtimes and Custom, say nothing of the kind.
    let own = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    assert!(own.own_address.is_none() && !own.card(&home.0).contains("not NVIDIA"));
    let mut ollama = nim();
    ollama.provider_type = "ollama".into();
    ollama.base_url = "http://192.168.4.35:11434/v1".into();
    ollama.api_key = None;
    assert_eq!(address(&ollama), ("http://192.168.4.35:11434/v1".to_string(), None));
    let mut custom = nim();
    custom.provider_type = "custom".into();
    custom.base_url = "https://my-gateway.example/v1".into();
    assert_eq!(address(&custom).1, None);
}

#[test]
fn the_approval_sentence_says_where_the_key_goes_and_never_the_key() {
    let home = Home::new("sentence");
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    let s = plan.sentence(&home.0);
    assert!(s.contains("integrate.api.nvidia.com") && s.contains("~/.config/yantrik/deepseek.json"), "{s}");
    assert!(!s.contains(KEY));
    let mut plain = nim();
    plain.provider_type = "custom".into();
    plain.base_url = "http://gateway.example.com/v1".into();
    let card = adapter_for("deepseek").unwrap().plan(&home.0, &plain).unwrap().card(&home.0);
    assert!(card.contains("plain http"), "{card}");
}

#[test]
fn revert_refuses_a_record_naming_files_it_never_writes_and_moves_nothing() {
    let home = Home::new("tamper");
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();
    let victim = home.0.join("important.txt");
    std::fs::write(&victim, "keep me").unwrap();
    let mut m = marker(&home.0, "deepseek").unwrap();
    m.files.push(Touched { path: victim.clone(), backup: None });
    std::fs::write(marker_path(&home.0, "deepseek"), serde_json::to_string(&m).unwrap()).unwrap();
    let err = revert(&home.0, "deepseek").unwrap_err();
    assert!(err.contains("never writes"), "{err}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
    assert!(home.deepseek().exists(), "nothing was undone");
}

#[test]
fn the_persons_copy_survives_an_apply_that_failed_after_writing() {
    let home = Home::new("partial");
    let original = r#"{"api_key_env":"DEEPSEEK_API_KEY"}"#;
    std::fs::write(home.deepseek(), original).unwrap();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();
    // As if the marker had never been written: the config holds the key, the copy the original.
    std::fs::remove_file(marker_path(&home.0, "deepseek")).unwrap();
    let plan = adapter_for("deepseek").unwrap().plan(&home.0, &nim()).unwrap();
    apply(&home.0, &plan).unwrap();
    assert_eq!(std::fs::read_to_string(backup_path(&home.deepseek())).unwrap(), original, "the copy was not replaced by the assigned file");
    revert(&home.0, "deepseek").unwrap();
    assert_eq!(std::fs::read_to_string(home.deepseek()).unwrap(), original);
}

#[test]
fn local_is_decided_by_the_address_not_its_first_characters() {
    assert!(is_local("http://127.0.0.1:11434/v1"));
    assert!(is_local("http://172.20.1.5:8000/v1"), "172.16/12 is private");
    assert!(is_local("http://[::1]:11434"));
    assert!(is_local("http://ollama-box.local:11434"));
    assert!(!is_local("http://10.example.com/v1"), "a hostname beginning with 10. is not an address");
    assert!(!is_local("http://127.attacker.net/v1"));
    assert!(!is_local("http://8.8.8.8/v1"));
}

// ── Use Yantrik models ──────────────────────────────────────────────

fn offer() -> Offer {
    let caps = yantrik_ml::model_caps::caps_for("groq", "openai/gpt-oss-120b", Some(131_072), Some(true));
    Offer {
        token: Token(yantrik_gateway::tokens::mint().unwrap()),
        model: "free-groq/openai/gpt-oss-120b".into(),
        models: vec![crate::ai_accounts::models::CatalogueModel {
            id: "free-groq/openai/gpt-oss-120b".into(),
            account: "free-groq".into(),
            model: "openai/gpt-oss-120b".into(),
            name: "openai/gpt-oss-120b".into(),
            caps,
        }],
    }
}

#[test]
fn a_harness_given_yantrik_models_holds_a_gateway_token_never_a_key() {
    let home = Home::new("gw-deepseek");
    std::fs::write(home.deepseek(), r#"{"base_url":"https://api.deepseek.com/v1","api_key":"sk-old-SECRET","max_steps":12}"#).unwrap();
    let o = offer();
    let plan = adapter_for("deepseek").unwrap().plan_gateway(&home.0, &o).unwrap();
    let card = plan.card(&home.0);
    assert!(card.contains("token") && card.contains("never a key") && !card.contains(&o.token.0), "{card}");
    assert!(card.contains("free-groq/openai/gpt-oss-120b now"), "what `picked` is now: {card}");
    assert!(!format!("{plan:?}").contains(&o.token.0), "not even in Debug");
    apply(&home.0, &plan).unwrap();
    let written = json(&home.deepseek());
    assert_eq!(written["base_url"], "http://127.0.0.1:7460/v1");
    assert_eq!(written["model"], "picked", "whatever is picked in the ask bar, at each call");
    assert_eq!(written["api_key"], o.token.0.as_str());
    assert_eq!(written["max_steps"], 12, "the rest of its file stays");
    assert_eq!(crate::gateway::tokens().verify(&o.token.0).unwrap().harness, "deepseek", "the gateway takes it from now on");
    assert!(uses_gateway(&home.0, "deepseek"));
    assert!(row_line(&home.0, "deepseek").contains("Yantrik models"));

    revert(&home.0, "deepseek").unwrap();
    assert!(crate::gateway::tokens().verify(&o.token.0).is_none(), "Revert withdraws the token");
    assert_eq!(json(&home.deepseek())["api_key"], "sk-old-SECRET", "and puts the person's own file back");
    assert!(!uses_gateway(&home.0, "deepseek"));
}

#[test]
fn pi_gets_one_provider_of_its_own_beside_the_ones_the_person_keeps() {
    let home = Home::new("gw-pi");
    std::fs::create_dir_all(home.0.join(".pi/agent")).unwrap();
    std::fs::write(home.0.join(".pi/agent/models.json"), r#"{"providers":{"ollama":{"baseUrl":"http://localhost:11434/v1","api":"openai-completions","models":[]}}}"#).unwrap();
    let o = offer();
    let plan = adapter_for("pi").unwrap().plan_gateway(&home.0, &o).unwrap();
    assert!(!plan.private_context, "a coding agent keeps no memory of the person");
    apply(&home.0, &plan).unwrap();
    let models = json(&home.0.join(".pi/agent/models.json"));
    assert!(models["providers"]["ollama"].is_object(), "the person's own provider stays");
    let ours = &models["providers"]["yantrik"];
    assert_eq!(ours["baseUrl"], "http://127.0.0.1:7460/v1");
    assert_eq!(ours["apiKey"], o.token.0.as_str());
    assert_eq!(ours["models"][0]["id"], "picked");
    assert_eq!(ours["models"][1]["id"], "free-groq/openai/gpt-oss-120b");
    assert_eq!(ours["models"][1]["reasoning"], true);
    let pi = json(&home.0.join(".config/yantrik/pi.json"));
    assert_eq!((pi["provider"].as_str(), pi["model"].as_str()), (Some("yantrik"), Some("picked")));
    revert(&home.0, "pi").unwrap();
    assert!(json(&home.0.join(".pi/agent/models.json"))["providers"].get("yantrik").is_none());
    assert!(!home.0.join(".config/yantrik/pi.json").exists(), "a file that was not there is removed");
}

#[test]
fn hermes_and_openclaw_are_written_their_own_way_and_json5_is_left_alone() {
    let home = Home::new("gw-hermes");
    std::fs::create_dir_all(home.0.join(".hermes")).unwrap();
    std::fs::write(home.0.join(".hermes/config.yaml"), "model: anthropic/claude-opus\nmemory:\n  provider: yantrikdb\n").unwrap();
    let o = offer();
    let plan = adapter_for("hermes").unwrap().plan_gateway(&home.0, &o).unwrap();
    assert!(plan.private_context, "Hermes keeps a memory of the person");
    assert!(plan.card(&home.0).contains("comments are not kept"));
    let yaml: serde_yaml::Value = serde_yaml::from_str(&plan.writes[0].content).unwrap();
    assert_eq!(yaml["model"]["default"].as_str(), Some("picked"));
    assert_eq!(yaml["model"]["provider"].as_str(), Some("custom"));
    assert_eq!(yaml["model"]["base_url"].as_str(), Some("http://127.0.0.1:7460/v1"));
    assert_eq!(yaml["memory"]["provider"].as_str(), Some("yantrikdb"), "the rest of its settings stay");

    std::fs::create_dir_all(home.0.join(".openclaw")).unwrap();
    std::fs::write(home.0.join(".openclaw/openclaw.json"), "{ // json5\n gateway: {} }").unwrap();
    let refused = adapter_for("openclaw").unwrap().plan_gateway(&home.0, &o).unwrap_err();
    assert!(refused.contains("not plain JSON"), "{refused}");
    std::fs::write(home.0.join(".openclaw/openclaw.json"), r#"{"gateway":{"http":{}}}"#).unwrap();
    let plan = adapter_for("openclaw").unwrap().plan_gateway(&home.0, &o).unwrap();
    let ours: serde_json::Value = serde_json::from_str(&plan.writes[0].content).unwrap();
    assert_eq!(ours["model"], "yantrik/picked");
    let own: serde_json::Value = serde_json::from_str(&plan.writes[1].content).unwrap();
    assert_eq!(own["models"]["providers"]["yantrik"]["apiKey"], o.token.0.as_str());
    assert!(own["gateway"]["http"].is_object());
}

#[test]
fn the_mind_is_sent_its_provider_never_a_file_and_the_companion_gets_a_file_of_its_own() {
    let home = Home::new("gw-mind");
    let o = offer();
    let plan = adapter_for("mind").unwrap().plan_gateway(&home.0, &o).unwrap();
    assert!(plan.writes.is_empty());
    let body: serde_json::Value = serde_json::from_str(&plan.mind_post.as_ref().unwrap().0).unwrap();
    assert_eq!(body["base_url"], "http://127.0.0.1:7460/v1");
    assert_eq!(body["model"], "picked");
    assert_eq!(body["api_key"], o.token.0.as_str());
    assert_eq!(body["private_context"], true);
    assert!(plan.card(&home.0).contains("its own provider setting"));
    assert!(!format!("{plan:?}").contains(&o.token.0));

    let plan = adapter_for("companion").unwrap().plan_gateway(&home.0, &o).unwrap();
    assert_eq!(plan.writes[0].path, home.0.join(".config/yantrik/companion-models.json"));
    assert!(adapter_for("companion").unwrap().plan(&home.0, &nim()).is_err(), "the built-in runs on the primary provider");
    assert!(adapter_for("pi").unwrap().plan(&home.0, &nim()).is_err());
    assert!(adapter_for("deepseek").unwrap().takes_saved_provider());
    assert!(!adapter_for("pi").unwrap().takes_saved_provider());
}
