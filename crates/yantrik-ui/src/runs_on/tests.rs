use super::identity::{from_reported, from_url, ProviderRef};
use super::*;

fn mind(id: &str, name: &str, detail: Option<&str>, answering: bool) -> MindFact {
    MindFact { id: id.into(), name: name.into(), detail: detail.map(String::from), answering, builtin: false }
}

/// VM 520 on 30 Sep 2026, verbatim: every mind's own line, and the companion from config.yaml
/// with nothing saved in Settings.
fn vm_520() -> (Vec<MindFact>, CompanionFact) {
    let minds = vec![
        MindFact { id: "companion".into(), name: "Yantrik Companion".into(), detail: None, answering: false, builtin: true },
        mind("deepseek", "DeepSeek", Some("deepseek-v4.1-flash \u{b7} ollama.com"), false),
        mind("hermes", "Hermes Agent", Some("Hermes 0.14.0 \u{b7} deepseek-v4.1-flash"), false),
        mind("mind", "Yantrik Mind", Some("ollama-cloud:deepseek-v4.1-flash"), true),
        mind("openclaw", "OpenClaw", Some("ollama-cloud/kimi-k3 \u{b7} OpenClaw 2026.9.1"), false),
        mind("pi", "Pi", Some("ollamacloud/deepseek-v4.1-flash \u{b7} pi 0.87.0"), false),
    ];
    let companion = CompanionFact {
        base_url: "https://aig.mycluster.cyou/v1".into(),
        model: "qwen3.8:27b".into(),
        source: "config.yaml".into(),
        provider_name: String::new(),
        ..CompanionFact::default()
    };
    (minds, companion)
}

#[test]
fn vm_520_every_mind_says_what_it_runs_on_and_where_that_is_set() {
    let (minds, companion) = vm_520();
    let rows = resolve(&minds, Some(&companion));
    let got: Vec<(String, &str, String, String)> = rows.iter().map(|r| (r.name.clone(), r.state(), r.runs_on(), r.source())).collect();
    let want = [
        ("Yantrik Mind", "Answering", "Ollama Cloud \u{b7} deepseek-v4.1-flash", "its own settings \u{b7} as it reported"),
        ("Yantrik Companion", "Built in", "Custom endpoint \u{b7} aig.mycluster.cyou \u{b7} qwen3.8:27b", "set in /opt/yantrik/config.yaml"),
        ("DeepSeek", "Attached", "Ollama Cloud \u{b7} deepseek-v4.1-flash", "its own settings \u{b7} as it reported"),
        ("Hermes Agent", "Attached", "provider not reported \u{b7} deepseek-v4.1-flash", "its own settings \u{b7} as it reported"),
        ("OpenClaw", "Attached", "Ollama Cloud \u{b7} kimi-k3", "its own settings \u{b7} as it reported"),
        ("Pi", "Attached", "Ollama Cloud \u{b7} deepseek-v4.1-flash", "its own settings \u{b7} as it reported"),
    ];
    let want: Vec<(String, &str, String, String)> = want.iter().map(|(a, b, c, d)| (a.to_string(), *b, c.to_string(), d.to_string())).collect();
    assert_eq!(got, want);
    assert_eq!(
        summary(&rows),
        "Ollama Cloud runs 4 minds. Custom endpoint \u{b7} aig.mycluster.cyou runs 1 mind. 1 mind does not say what it runs on. No mind here runs on a signed-in account."
    );
    // The thing that started this: nothing here may name Claude, whose sign-in no mind uses.
    for r in &rows {
        assert!(!r.runs_on().contains("Claude") && !r.source().contains("Claude"), "{r:?}");
    }
}

#[test]
fn a_provider_is_never_read_from_a_model_name() {
    // "deepseek-v4.1-flash" holds the catalogue id `deepseek`; DeepSeek does not run these minds.
    assert_eq!(from_reported("deepseek-v4.1-flash"), (ProviderRef::NotReported, Some("deepseek-v4.1-flash".into())));
    assert_eq!(from_reported("qwen3.5:9b").0, ProviderRef::NotReported, "a model with a tag is not provider:model");
    assert_eq!(from_reported("qwen3.5:9b").1.as_deref(), Some("qwen3.5:9b"));
    assert_eq!(from_reported("qwen2.5 on node1").0, ProviderRef::NotReported);
    // An explicit prefix that IS a provider does name it.
    assert_eq!(from_reported("deepseek:deepseek-chat").0.label(), "DeepSeek");
}

#[test]
fn four_spellings_are_one_provider() {
    for text in [
        "ollama-cloud:deepseek-v4.1-flash",
        "deepseek-v4.1-flash \u{b7} ollama.com",
        "ollamacloud/deepseek-v4.1-flash \u{b7} pi 0.87.0",
        "ollama-cloud/kimi-k3 \u{b7} OpenClaw 2026.9.1",
        "ollama_cloud/x",
        "https://ollama.com/v1",
    ] {
        let (p, _) = from_reported(text);
        assert_eq!(p.label(), "Ollama Cloud", "{text}");
        assert_eq!(p.key().as_deref(), Some("ollama-cloud"), "{text}");
    }
}

#[test]
fn addresses_are_named_by_the_catalogue_or_by_their_host() {
    assert_eq!(from_url("https://integrate.api.nvidia.com/v1").label(), "NVIDIA NIM");
    assert_eq!(from_url("http://localhost:11434/v1").label(), "Ollama");
    assert_eq!(from_url("http://192.168.4.35:11434/v1"), ProviderRef::Local("192.168.4.35:11434".into()), "another machine's port is not \"Ollama\"");
    assert_eq!(from_url("https://aig.mycluster.cyou/v1"), ProviderRef::Custom("aig.mycluster.cyou".into()));
    assert_eq!(from_reported("192.168.4.35:11434").0, ProviderRef::Local("192.168.4.35:11434".into()));
    assert_eq!(from_url(""), ProviderRef::NotReported);
}

#[test]
fn a_companion_with_no_address_says_nothing_is_set_up_and_a_saved_one_is_named() {
    let minds = [MindFact { id: "companion".into(), name: "Yantrik Companion".into(), detail: None, answering: true, builtin: true }];
    let rows = resolve(&minds, None);
    assert_eq!((rows[0].runs_on().as_str(), rows[0].source().as_str()), ("Nothing set up", "add a provider under Providers"));
    let saved = CompanionFact { base_url: "https://integrate.api.nvidia.com/v1".into(), model: "nvidia/x".into(), source: "saved provider".into(), provider_name: "NIM work".into(), ..CompanionFact::default() };
    let rows = resolve(&minds, Some(&saved));
    assert_eq!(rows[0].runs_on(), "NVIDIA NIM \u{b7} nvidia/x");
    assert_eq!(rows[0].source(), "your saved provider \u{201c}NIM work\u{201d}");
    assert!(!summary(&rows).contains("does not say"), "the companion with an address is not silent");
}

#[test]
fn a_mind_that_says_nothing_is_not_given_a_provider() {
    let rows = resolve(&[mind("x", "X", None, false)], None);
    assert_eq!(rows[0].runs_on(), "provider not reported");
    assert_eq!(rows[0].source(), "its own settings \u{b7} as it reported");
}

#[test]
fn vm_520_the_providers_in_use_are_listed_though_none_is_saved() {
    let (minds, companion) = vm_520();
    let rows = resolve(&minds, Some(&companion));
    let in_use = in_use_not_saved(&rows, Some(&companion), &[]);
    assert_eq!(in_use.len(), 2, "{in_use:#?}");
    let ollama = &in_use[0];
    assert_eq!(ollama.label, "Ollama Cloud");
    assert_eq!(ollama.used_by, ["Yantrik Mind", "DeepSeek", "OpenClaw", "Pi"]);
    assert_eq!(ollama.models, ["deepseek-v4.1-flash", "kimi-k3"]);
    assert_eq!(ollama.where_set, "each mind keeps its own key in its own settings");
    assert_eq!(
        (ollama.preset.as_str(), ollama.base_url.as_str(), ollama.model.as_str()),
        ("ollama-cloud", "https://ollama.com/v1", "deepseek-v4.1-flash")
    );
    let aig = &in_use[1];
    assert_eq!(aig.label, "Custom endpoint \u{b7} aig.mycluster.cyou");
    assert_eq!(aig.used_by, ["Yantrik Companion"]);
    assert_eq!(aig.where_set, "set in /opt/yantrik/config.yaml \u{b7} used by the built-in companion");
    assert_eq!(
        (aig.preset.as_str(), aig.base_url.as_str(), aig.model.as_str()),
        ("custom", "https://aig.mycluster.cyou/v1", "qwen3.8:27b")
    );
    // Hermes names no provider: it is in no row rather than guessed into one.
    assert!(!in_use.iter().any(|u| u.used_by.iter().any(|n| n == "Hermes Agent")));
}

/// The row above the chat composer's Send: where the message text goes, from the same facts as
/// every other surface, and nothing at all when the provider is not known.
#[test]
fn the_composer_line_says_where_words_go_or_nothing() {
    let (minds, companion) = vm_520();
    let rows = resolve(&minds, Some(&companion));
    let line = |name: &str| rows.iter().find(|r| r.name == name).unwrap().destination();
    assert_eq!(line("Yantrik Mind"), "Sends your message and conversation context to: Ollama Cloud \u{b7} deepseek-v4.1-flash");
    assert_eq!(line("Yantrik Companion"), "Sends your message and conversation context to: aig.mycluster.cyou \u{b7} qwen3.8:27b");
    // Hermes names a model and no provider: no line rather than a guessed one.
    assert_eq!(line("Hermes Agent"), "");

    let companion_at = |url: &str| {
        let c = CompanionFact { base_url: url.into(), model: "qwen3.5:9b".into(), source: "config.yaml".into(), provider_name: String::new(), ..CompanionFact::default() };
        resolve(&minds[..1], Some(&c))[0].destination()
    };
    assert_eq!(companion_at("http://localhost:11434/v1"), "Sends your message and conversation context to: Ollama at localhost:11434 \u{b7} qwen3.5:9b");
    assert_eq!(companion_at("http://127.0.0.1:8341/v1"), "Sends your message and conversation context to: 127.0.0.1:8341 \u{b7} qwen3.5:9b");
    assert_eq!(companion_at("http://192.168.4.35:11434/v1"), "Sends your message and conversation context to: 192.168.4.35:11434, on this network \u{b7} qwen3.5:9b");
    assert_eq!(companion_at(""), "", "nothing set up, nothing said");

    // A local runtime named only in a mind's own words could be on any machine.
    let ollama = resolve(&[mind("x", "X", Some("ollama:qwen3.5:9b"), true)], None);
    assert_eq!(ollama[0].destination(), "Sends your message and conversation context to: Ollama \u{b7} qwen3.5:9b");
}

/// Security review of #648, H1: "Stays on this machine" was said where it could be false. It is
/// said for a model in this process with no fallback and nothing else: every API, loopback
/// included, is named as it was seen, a proxy or a cloud model behind it as forwarding, and a
/// fallback by name.
#[test]
fn stays_on_this_machine_is_said_only_when_it_is_certain() {
    let builtin = [MindFact { id: "companion".into(), name: "Yantrik Companion".into(), detail: None, answering: true, builtin: true }];
    let line = |c: CompanionFact| resolve(&builtin, Some(&c))[0].destination();
    let at = |url: &str, model: &str| CompanionFact { base_url: url.into(), model: model.into(), source: "config.yaml".into(), ..CompanionFact::default() };

    // The one certain case: a model in this process, with no fallback.
    assert_eq!(line(CompanionFact { backend: Backend::InProcess, model: "qwen3.5-4b".into(), ..CompanionFact::default() }), "Stays on this machine \u{b7} qwen3.5-4b");
    // A plain loopback API is named as it was seen, never "Stays": a daemon here can forward.
    for (url, seen) in [
        ("http://localhost:11434/v1", "Ollama at localhost:11434"),
        ("http://127.0.0.1:11434/v1", "Ollama at 127.0.0.1:11434"),
        ("http://127.0.0.1:8341/v1", "127.0.0.1:8341"),
        ("http://[::1]:8000/v1", "vLLM at [::1]:8000"),
    ] {
        let said = line(at(url, "qwen3.5:9b"));
        assert_eq!(said, format!("Sends your message and conversation context to: {seen} \u{b7} qwen3.5:9b"), "{url}");
        assert!(!said.contains("Stays"), "{url}: {said}");
    }

    // The Claude CLI sends to Anthropic, whatever address config.yaml also holds.
    assert_eq!(
        line(CompanionFact { backend: Backend::ClaudeCli, ..at("http://localhost:11434/v1", "sonnet") }),
        "Sends your message and conversation context to: Anthropic (Claude CLI) \u{b7} sonnet"
    );
    // A fallback could take the words elsewhere: it is named, and nothing claims to stay.
    assert_eq!(
        line(CompanionFact { fallback: Some("Ollama Cloud".into()), ..at("http://localhost:11434/v1", "qwen3.5:9b") }),
        "Sends your message and conversation context to: Ollama at localhost:11434 \u{b7} qwen3.5:9b \u{b7} falls back to Ollama Cloud"
    );
    assert_eq!(
        line(CompanionFact { fallback: Some("Ollama at localhost:8341".into()), ..at("http://127.0.0.1:8341/v1", "qwen3.5:9b") }),
        "Sends your message and conversation context to: 127.0.0.1:8341 \u{b7} qwen3.5:9b \u{b7} falls back to Ollama at localhost:8341"
    );
    assert_eq!(
        line(CompanionFact { backend: Backend::InProcess, model: "m".into(), fallback: Some("aig.mycluster.cyou".into()), ..CompanionFact::default() }),
        "Runs in this process \u{b7} m \u{b7} falls back to aig.mycluster.cyou"
    );
    // A proxy on this machine forwards the words on: LiteLLM on its own port by name or by IP,
    // LiteLLM set up as such on another port, and a cloud model through a local daemon.
    assert_eq!(line(at("http://localhost:4000/v1", "gpt-x")), "Sends your message and conversation context to: a proxy that forwards it on, through LiteLLM Proxy at localhost:4000 \u{b7} gpt-x");
    assert_eq!(line(at("http://127.0.0.1:4000/v1", "gpt-x")), "Sends your message and conversation context to: a proxy that forwards it on, through LiteLLM Proxy at 127.0.0.1:4000 \u{b7} gpt-x");
    assert_eq!(
        line(CompanionFact { provider_kind: "litellm".into(), ..at("http://127.0.0.1:4100/v1", "gpt-x") }),
        "Sends your message and conversation context to: a proxy that forwards it on, through 127.0.0.1:4100 \u{b7} gpt-x"
    );
    assert_eq!(
        line(at("http://localhost:11434/v1", "gpt-oss:120b-cloud")),
        "Sends your message and conversation context to: Ollama Cloud, through Ollama at localhost:11434 \u{b7} gpt-oss:120b-cloud"
    );
    assert_eq!(
        line(at("http://127.0.0.1:11434/v1", "gpt-oss:120b-cloud")),
        "Sends your message and conversation context to: Ollama Cloud, through Ollama at 127.0.0.1:11434 \u{b7} gpt-oss:120b-cloud"
    );
    assert_eq!(
        line(at("http://127.0.0.1:9999/v1", "kimi-k2:cloud")),
        "Sends your message and conversation context to: Ollama Cloud, through 127.0.0.1:9999 \u{b7} kimi-k2:cloud"
    );
    // A mind's own word for a loopback address is about its machine, not this one.
    let said = |detail: &str| resolve(&[mind("x", "X", Some(detail), true)], None)[0].destination();
    assert_eq!(said("127.0.0.1:9000 \u{b7} qwen3.5:9b"), "Sends your message and conversation context to: 127.0.0.1:9000 \u{b7} qwen3.5:9b");
    assert_eq!(said("http://localhost:11434/v1 \u{b7} qwen3.5:9b"), "Sends your message and conversation context to: Ollama \u{b7} qwen3.5:9b");
    // A bare `localhost:11434` names no host it can be held to: nothing is said rather than "Stays".
    assert_eq!(said("localhost:11434 \u{b7} qwen3.5:9b"), "");
    // The fallback's own words, from config.yaml.
    assert_eq!(fallback_label("llamacpp", None), "llama.cpp in this process");
    assert_eq!(fallback_label("api", Some("http://localhost:8341/v1")), "localhost:8341");
    assert_eq!(fallback_label("api", Some("http://localhost:11434/v1")), "Ollama at localhost:11434");
    assert_eq!(fallback_label("api", Some("https://ollama.com/v1")), "Ollama Cloud");
}

/// Security review of #648, H1(b): the backend is what `bridge` builds, by its own predicates. A
/// spelling it reads as an API ("Candle", " llamacpp") is an API here too, and never "stays".
#[test]
fn the_backend_is_the_one_the_bridge_builds() {
    use yantrik_companion::config::LLMConfig;
    let of = |name: &str| Backend::of(&LLMConfig { backend: name.into(), ..LLMConfig::default() });
    assert_eq!(of("candle"), Backend::InProcess);
    assert_eq!(of("llamacpp"), Backend::InProcess);
    assert_eq!(of("claude-cli"), Backend::ClaudeCli);
    for api in ["api", "ollama", "Candle", " llamacpp", "LLAMACPP", "Claude-CLI", "claude_cli"] {
        assert_eq!(of(api), Backend::Api, "{api:?} is an ApiLLM in bridge::build_companion");
    }
    // And the fallback: `llamacpp` exactly is the embedded model, anything else an API.
    assert_eq!(fallback_label("llamacpp", None), "llama.cpp in this process");
    assert_eq!(fallback_label("Llamacpp", Some("http://127.0.0.1:8341/v1")), "127.0.0.1:8341");
}

#[test]
fn a_saved_provider_is_not_listed_again_as_in_use() {
    let (minds, companion) = vm_520();
    let rows = resolve(&minds, Some(&companion));
    let in_use = in_use_not_saved(&rows, Some(&companion), &["https://ollama.com/v1".to_string()]);
    let labels: Vec<&str> = in_use.iter().map(|u| u.label.as_str()).collect();
    assert_eq!(labels, ["Custom endpoint \u{b7} aig.mycluster.cyou"]);
}
