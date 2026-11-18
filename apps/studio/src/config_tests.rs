use super::*;

fn write(text: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "studio-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(FILE_NAME);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_machine_with_no_config_file_runs_the_fake_backend_and_says_so() {
    let config = Config::unconfigured();
    assert!(!config.configured);
    assert_eq!(config.backend.kind, Kind::Fake);
    let facts = config.facts(false);
    assert_eq!(facts.kind, "fake");
    assert_eq!(facts.grade, "standard");
    assert!(!facts.prompt_leaves);
    assert!(facts.note.contains("No backend is configured"), "{facts:?}");
    assert!(facts.note.contains(FILE_NAME), "{facts:?}");
}

#[test]
fn the_config_file_the_documentation_describes_parses() {
    let config = Config::parse(
        r#"{"backend": {"kind": "comfyui", "base_url": "http://192.168.4.35:8188",
            "model": "dreamshaperXL_v21.safetensors"}}"#,
    )
    .unwrap();
    assert_eq!(config.backend.kind, Kind::ComfyUi);
    assert_eq!(config.backend.base_url, "http://192.168.4.35:8188");
    assert_eq!(config.backend.model, "dreamshaperXL_v21.safetensors");
    assert!(config.configured);
}

#[test]
fn a_trailing_slash_does_not_make_the_urls_the_app_builds_wrong() {
    let config = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://10.0.0.5:8188/"}}"#).unwrap();
    assert_eq!(config.backend.base_url, "http://10.0.0.5:8188");
}

#[test]
fn a_hosted_backend_needs_a_model_and_says_which_key_it_wants() {
    let problem = Config::parse(r#"{"backend":{"kind":"openai-images"}}"#).unwrap_err();
    assert!(problem.contains("`model`"), "{problem}");

    let config = Config::parse(
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"MY_KEY"}}"#,
    )
    .unwrap();
    assert_eq!(config.backend.api_key_env, "MY_KEY");
    assert_eq!(config.backend.base_url, DEFAULT_OPENAI_URL);
    assert_eq!(config.facts(false).grade, "sensitive");
    assert!(config.facts(false).prompt_leaves);
}

#[test]
fn an_unknown_kind_names_the_ones_that_would_work() {
    let problem = Config::parse(r#"{"backend":{"kind":"midjourney"}}"#).unwrap_err();
    assert!(problem.contains("midjourney"), "{problem}");
    for kind in KINDS {
        assert!(problem.contains(kind), "{problem} does not name {kind}");
    }
}

#[test]
fn broken_json_is_reported_rather_than_swallowed() {
    let problem = Config::parse("{ not json").unwrap_err();
    assert!(problem.contains("JSON"), "{problem}");
    // `read` is the boundary that has to survive a bad file: the app must still open.
    let path = write("{ not json");
    let read = Config::read(&path);
    assert_eq!(Config { refused: None, ..read.clone() }, Config::unconfigured());
    // And says so, with the file and the reason, for the window and `describe` to carry.
    let said = read.refused.expect("a file that was not used says so");
    assert!(said.contains("was not used") && said.contains("not valid JSON"), "{said}");
}

#[test]
fn the_keys_a_person_writes_by_hand_are_forgiven() {
    let config = Config::parse(
        r#"{"output_folder":"/srv/pictures","backend":{"type":"OpenAI_Images","url":"https://llm.example/v1/","checkpoint":"gpt-image-1"}}"#,
    )
    .unwrap();
    assert_eq!(config.backend.kind, Kind::OpenAiImages);
    assert_eq!(config.backend.base_url, "https://llm.example/v1");
    assert_eq!(config.backend.model, "gpt-image-1");
    assert_eq!(config.output_folder, "/srv/pictures");
}

#[test]
fn what_set_backend_writes_is_what_the_file_reader_reads() {
    let config = Config::parse(
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"K","base_url":"https://x.example/v1"}}"#,
    )
    .unwrap();
    assert_eq!(
        Config::parse(&serde_json::to_string(&config.to_json()).unwrap()).unwrap(),
        config
    );
}

#[test]
fn a_server_on_the_lan_is_yours_and_one_on_the_internet_is_not() {
    for local in [
        "http://127.0.0.1:8188",
        "http://localhost:8188",
        "http://192.168.4.35:8188",
        "http://10.8.0.2:8188",
        "http://172.16.0.9:8188",
        "http://172.31.255.254:8188",
        "http://169.254.7.7:8188",
        "http://[::1]:8188",
        "http://[fc00::5]:8188",
        "http://[::ffff:192.168.1.4]:8188",
    ] {
        assert!(stays_on_a_network_you_own(local), "{local} is your own network");
    }
    for away in [
        "https://api.openai.com/v1",
        "http://8.8.8.8:8188",
        "http://172.32.0.1:8188", // just outside 172.16.0.0/12
        "http://11.0.0.1:8188",
        "http://192.169.4.35:8188",
        "http://[2001:4860:4860::8888]:8188",
    ] {
        assert!(!stays_on_a_network_you_own(away), "{away} is not your own network");
    }
}

#[test]
fn a_url_that_only_looks_local_is_not_believed() {
    assert!(!stays_on_a_network_you_own("http://replica.example/?redirect=localhost"));
    assert!(!stays_on_a_network_you_own("https://127.0.0.1.localhost.example:8188"));
    // Not a URL at all: the answer is "away", which asks rather than sends.
    assert!(!stays_on_a_network_you_own(""));
    assert!(!stays_on_a_network_you_own("192.168.1.1"));
    // A hostname that is not a literal address is not resolved in order to grade it.
    assert!(!stays_on_a_network_you_own("http://gpu.home.arpa:8188"));
}

#[test]
fn the_grade_of_generate_follows_the_backend() {
    let comfy_local =
        Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
            .unwrap();
    assert_eq!(comfy_local.facts(false).grade, "standard");
    assert!(!comfy_local.facts(false).prompt_leaves);

    // The same app, pointed at a ComfyUI that is not on a network the person owns: now the
    // prompt is leaving, and the grade has to follow it.
    let comfy_away =
        Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"https://gpu.example.com"}}"#)
            .unwrap();
    assert_eq!(comfy_away.facts(false).grade, "sensitive");
    assert!(comfy_away.facts(false).prompt_leaves);

    let hosted =
        Config::parse(r#"{"backend":{"kind":"openai-images","model":"gpt-image-1"}}"#).unwrap();
    assert_eq!(hosted.facts(false).grade, "sensitive");

    let fake = Config::parse(r#"{"backend":{"kind":"fake"}}"#).unwrap();
    assert_eq!(fake.facts(false).grade, "standard");
    assert!(!fake.facts(false).prompt_leaves);
}

#[test]
fn a_comfyui_on_the_lan_is_described_as_being_on_the_lan() {
    let config =
        Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
            .unwrap();
    let facts = config.facts(false);
    assert!(facts.place.contains("192.168.4.35"), "{facts:?}");
    assert!(facts.place.contains("you reach"), "{facts:?}");
    assert_eq!(facts.note, "");
}

#[test]
fn a_hosted_backend_with_no_key_exported_says_which_variable_is_missing() {
    let config = Config::parse(
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_TEST_ABSENT_KEY"}}"#,
    )
    .unwrap();
    std::env::remove_var("STUDIO_TEST_ABSENT_KEY");
    let facts = config.facts(false);
    // The file was complete enough to parse; it is the environment that is missing something,
    // and those two are reported differently on purpose.
    assert!(facts.configured);
    assert_eq!(facts.grade, "sensitive");
    assert!(facts.note.contains("named in studio.json's `api_key_env`") && !facts.note.contains("STUDIO_TEST_ABSENT_KEY"), "{facts:?}");
    // The note names the variable, never a value; there is no value to leak here, and the
    // assertion below is the one that would catch a regression that printed one.
    assert!(!facts.note.contains("api_key="), "{facts:?}");
}

#[test]
fn the_state_names_the_variable_holding_the_key_but_never_its_value() {
    let sentinel = "sk-sentinel-that-must-not-appear";
    std::env::set_var("STUDIO_TEST_KEY", sentinel);
    let config = Config::parse(
        r#"{"backend":{"kind":"openai-images","model":"gpt-image-1","api_key_env":"STUDIO_TEST_KEY"}}"#,
    )
    .unwrap();
    let state = serde_json::to_string(&config.state(true)).unwrap();
    assert!(state.contains("STUDIO_TEST_KEY"), "{state}");
    assert!(!state.contains(sentinel), "the key reached describe: {state}");
    assert!(state.contains("\"api_key_is_set\":true"), "{state}");
    std::env::remove_var("STUDIO_TEST_KEY");
}

#[test]
fn a_key_is_read_from_the_environment_and_only_from_there() {
    let backend = Backend {
        api_key_env: "STUDIO_TEST_KEY_2".into(),
        ..Backend::defaults_for(Kind::OpenAiImages)
    };
    std::env::remove_var("STUDIO_TEST_KEY_2");
    let problem = backend.api_key().unwrap_err();
    assert!(problem.contains("named in studio.json's `api_key_env`") && !problem.contains("STUDIO_TEST_KEY_2"), "{problem}");
    assert!(!problem.contains('='), "the message must not look like a key: {problem}");

    std::env::set_var("STUDIO_TEST_KEY_2", "   ");
    assert!(backend.api_key().unwrap_err().contains("empty"));

    std::env::set_var("STUDIO_TEST_KEY_2", "sk-abc");
    assert_eq!(backend.api_key().unwrap(), "sk-abc");
    std::env::remove_var("STUDIO_TEST_KEY_2");

    let unnamed = Backend { api_key_env: "".into(), ..Backend::defaults_for(Kind::OpenAiImages) };
    assert!(unnamed.api_key().is_err());
}

#[test]
fn a_saved_config_is_read_back_as_the_same_config() {
    // A private XDG_CONFIG_HOME, so the test does not write into the person's own config and
    // does not read whatever is already there.
    //
    // This is the only test in the binary that moves a process-wide variable, and it moves this
    // one: nothing else here asserts on where the configuration lives, and every other test is
    // handed its places (`engine::Places`) instead of looking them up. `HOME` and
    // `XDG_PICTURES_DIR` are read by tests that do assert on them, which is why those are
    // tested against whatever this machine says rather than against a value set here.
    let dir = std::env::temp_dir().join(format!("studio-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    let path = config_path().unwrap();
    assert!(path.starts_with(&dir));

    let mut config = Config::parse(r#"{"backend":{"kind":"comfyui","base_url":"http://192.168.4.35:8188"}}"#)
        .unwrap();
    config.output_folder = "/srv/pictures".into();
    let saved_at = config.save_to(&path).unwrap();
    assert_eq!(saved_at, path);
    assert!(saved_at.is_file());
    assert!(!saved_at.with_extension("json.new").exists(), "the temporary file was left behind");
    assert_eq!(Config::load(), config);
    std::env::remove_var("XDG_CONFIG_HOME");
}
