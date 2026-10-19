//! Asking a provider which models it serves: the one request path.
//!
//! Settings' Connect button, the row's Test and Models actions, the MODELS
//! list and first-boot onboarding all ask through [`list_models`]. It checks
//! the key and fetches the list in one request, so "the key works" and "here
//! is what you can pick" can never disagree.
//!
//! Blocking: call it off the UI thread.
//!
//! The key goes in a header and nowhere else. No message or log line here
//! carries it, or the URL's userinfo; errors name the host only.

use std::time::Duration;

use yantrik_ml::KNOWN_PROVIDERS;

/// How long to wait for a connection, and for the whole answer.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

/// One model a provider says it serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListedModel {
    /// What goes in a request's `model` field.
    pub id: String,
    /// What to show; the id when the provider gives no display name.
    pub name: String,
    /// Size on disk, from Ollama's native list. 0 when unknown.
    pub size_bytes: u64,
}

/// Why no list came back, in words a person can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListError {
    /// 401 or 403.
    KeyRefused,
    /// 404: nothing answers at the models path.
    NoModelList,
    /// DNS failure, connection refused, TLS failure.
    Unreachable { host: String },
    /// Connected, or tried to, and gave up waiting.
    TimedOut { host: String },
    /// Any other HTTP status.
    Status(u16),
    /// A 200 whose body is not a model list.
    NotAList,
    /// The URL still has a part the person must fill in, or is not a URL.
    BadUrl,
    /// The URL's sign-in details cannot be told from its host (a key holding a `/`, `?` or `#`):
    /// it is neither sent nor named.
    UnclearAddress,
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeyRefused => write!(f, "The key was refused. Check it and try again."),
            Self::NoModelList => write!(f, "There is no model list at this address. Check the URL."),
            Self::Unreachable { host } => write!(f, "Could not reach {host}."),
            Self::TimedOut { host } => write!(f, "No answer from {host} in time."),
            Self::Status(code) => write!(f, "The provider answered with an error (HTTP {code})."),
            Self::NotAList => write!(f, "The provider answered, but not with a model list."),
            Self::BadUrl => write!(f, "That URL is not complete. Fill in any part in braces, like {{account_id}}."),
            Self::UnclearAddress => write!(f, "That URL holds sign-in details that hide where it goes. Put the key in the key field instead."),
        }
    }
}

/// Ask the provider at `base_url` for its models, authenticating as
/// `auth_type` ("bearer", "x-api-key" or "none") with `api_key`.
pub(crate) fn list_models(
    base_url: &str,
    api_key: Option<&str>,
    auth_type: &str,
) -> Result<Vec<ListedModel>, ListError> {
    let base = listing_base(base_url);
    let host = host_of(&base);
    if host.is_empty() && !crate::runs_on::identity::host(&base).is_empty() {
        return Err(ListError::UnclearAddress);
    }
    if host.is_empty() || base.contains('{') || !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(ListError::BadUrl);
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build();

    let paths = candidate_paths(&base);
    let mut last = ListError::NoModelList;
    for url in &paths {
        let mut request = agent.get(url);
        let key = api_key.filter(|k| !k.is_empty());
        match (auth_type, key) {
            (_, None) | ("none", _) => {}
            ("x-api-key", Some(k)) => {
                request = request.set("x-api-key", k).set("anthropic-version", "2023-06-01");
            }
            (_, Some(k)) => request = request.set("Authorization", &format!("Bearer {k}")),
        }
        match request.call() {
            Ok(response) => {
                let body = response.into_string().map_err(|_| ListError::NotAList)?;
                let models = parse_models(&body)?;
                tracing::info!(host = %host, count = models.len(), "Provider listed its models");
                return Ok(models);
            }
            // Only a missing path is worth another try; everything else is the answer.
            Err(ureq::Error::Status(404, _)) => last = ListError::NoModelList,
            Err(ureq::Error::Status(code, _)) => {
                let err = if code == 401 || code == 403 { ListError::KeyRefused } else { ListError::Status(code) };
                tracing::warn!(host = %host, status = code, "Provider refused the model list");
                return Err(err);
            }
            Err(ureq::Error::Transport(t)) => {
                let err = transport_error(&t, &host);
                tracing::warn!(host = %host, kind = ?t.kind(), "Could not list the provider's models");
                return Err(err);
            }
        }
    }
    tracing::warn!(host = %host, tried = paths.len(), "No model list at the provider's address");
    Err(last)
}

/// The model to start on from a fresh list: `preferred` (the saved model, or
/// the catalogue's default) when the provider serves it, otherwise the first.
pub(crate) fn pick_model<'a>(models: &'a [ListedModel], preferred: &[&str]) -> Option<&'a str> {
    preferred
        .iter()
        .filter(|p| !p.is_empty())
        .find_map(|p| models.iter().find(|m| m.id == *p))
        .or_else(|| models.first())
        .map(|m| m.id.as_str())
}

/// Parse any of the three shapes a model list comes in:
/// OpenAI `{"data":[{"id"}]}`, Anthropic `{"data":[{"id","display_name"}]}`,
/// Ollama `{"models":[{"name","size"}]}`, and the bare array some
/// OpenAI-compatible servers return.
pub(crate) fn parse_models(body: &str) -> Result<Vec<ListedModel>, ListError> {
    let json: serde_json::Value = serde_json::from_str(body).map_err(|_| ListError::NotAList)?;
    let (items, ollama) = if let Some(a) = json.get("models").and_then(|v| v.as_array()) {
        (a, true)
    } else if let Some(a) = json.get("data").and_then(|v| v.as_array()) {
        (a, false)
    } else if let Some(a) = json.as_array() {
        (a, false)
    } else {
        return Err(ListError::NotAList);
    };

    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    for item in items {
        let raw = if ollama {
            item.get("name").or_else(|| item.get("model"))
        } else {
            item.get("id")
        };
        let Some(raw) = raw.and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else { continue };
        // Gemini's compatibility list says "models/gemini-…"; requests take the bare id.
        let id = raw.strip_prefix("models/").unwrap_or(raw).to_string();
        if !seen.insert(id.clone()) {
            continue;
        }
        let name = item
            .get("display_name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| id.clone());
        let size_bytes = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        models.push(ListedModel { id, name, size_bytes });
    }
    Ok(models)
}

/// What a test request with the key found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyCheck {
    /// The provider accepted the key — it answered, or said the key's quota is spent (429),
    /// which it only says to a key it knows.
    Works,
    /// 401 or 403.
    Refused,
    /// Anything else: the key could not be confirmed either way. A sentence for the person.
    Unconfirmed(String),
}

/// Whether the provider's model list answers with no key at all. NVIDIA's and OpenRouter's do,
/// and then a list fetched with a key says nothing about the key: Connect said "Connected: 81
/// models" on VM 520 with `nvapi-not-a-real-key` in the field.
pub(crate) fn list_is_public(base_url: &str, auth_type: &str) -> bool {
    list_models(base_url, None, auth_type).is_ok()
}

/// Send the smallest request that needs the key: one chat completion of one token with
/// `model`, in the shape a real companion turn sends ([`key_check_body`]), and through the same
/// refusal handling (`yantrik_ml::refusal`): a sampling parameter the model refuses is dropped
/// and remembered for the turns that follow, so "Connected" means a real turn will work. Only
/// for OpenAI-compatible providers — the ones whose lists can be public.
pub(crate) fn check_key(base_url: &str, api_key: &str, model: &str) -> KeyCheck {
    let base = listing_base(base_url);
    let host = host_of(&base);
    if host.is_empty() {
        return KeyCheck::Unconfirmed(ListError::UnclearAddress.to_string());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout(TOTAL_TIMEOUT)
        .build();
    let url = format!("{base}/chat/completions");
    let post = |body: &serde_json::Value| -> anyhow::Result<Result<Result<(), ureq::Transport>, (u16, String)>> {
        Ok(match agent.post(&url).set("Authorization", &format!("Bearer {api_key}")).send_json(body) {
            Ok(_) => Ok(Ok(())),
            Err(ureq::Error::Status(code, resp)) => Err((code, resp.into_string().unwrap_or_default())),
            Err(ureq::Error::Transport(t)) => Ok(Err(t)),
        })
    };
    let answer = match yantrik_ml::refusal::send_dropping_refused(&base, model, &key_check_body(&base, model), post) {
        Ok(answer) => answer,
        Err(e) => return KeyCheck::Unconfirmed(format!("The test request could not be sent ({e}).")),
    };
    match answer {
        Ok(Ok(())) | Err((429, _)) => {
            tracing::info!(host = %host, "Provider accepted the key on a test request");
            KeyCheck::Works
        }
        Err((code, _)) if code == 401 || code == 403 => {
            tracing::warn!(host = %host, status = code, "Provider refused the key on a test request");
            KeyCheck::Refused
        }
        Err((code, said)) => {
            tracing::warn!(host = %host, status = code, "A test request could not confirm the key");
            let reason = yantrik_ml::refusal::provider_message(&said)
                .map(|m| format!(": {}", yantrik_ml::refusal::excerpt(&m, Some(api_key)).trim_end_matches('.')))
                .unwrap_or_default();
            KeyCheck::Unconfirmed(format!(
                "The provider answered a test request with HTTP {code}{reason}, so the key could not be confirmed."
            ))
        }
        Ok(Err(t)) => KeyCheck::Unconfirmed(transport_error(&t, &host).to_string()),
    }
}

/// The test request: one short message, one token back, built by the companion's own request
/// builder with the settings a turn uses, so it carries the same parameters a real turn does.
/// It costs the person next to nothing, and it is the least a provider will answer that still
/// needs the key.
fn key_check_body(base: &str, model: &str) -> serde_json::Value {
    let llm = yantrik_ml::ApiLLM::new(base, None, model);
    let config = yantrik_ml::GenerationConfig { max_tokens: 1, ..Default::default() };
    llm.chat_completions_body(&[yantrik_ml::ChatMessage::user("Reply with OK.")], &config, false)
}

/// The OpenAI-compatible address for a saved base URL: a native one whose API has no
/// OpenAI-style routes (Anthropic's bare host, Gemini's generateContent host) is given the
/// provider's OpenAI-compatible endpoint. Listing uses it, and so does a harness that speaks
/// chat completions (`provider_handoff`).
pub(crate) fn openai_base(base_url: &str) -> String {
    listing_base(base_url)
}

/// A native base URL whose API has no OpenAI-style /models (Anthropic's bare
/// host, Gemini's generateContent host) is listed at the provider's
/// OpenAI-compatible endpoint instead.
fn listing_base(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    KNOWN_PROVIDERS
        .iter()
        .find(|p| p.openai_compat_base_url.is_some() && p.default_base_url.trim_end_matches('/') == base)
        .map(|p| p.openai_base_url().to_string())
        .unwrap_or_else(|| base.to_string())
}

/// Where to look. An OpenAI-compatible base lists at `/models`. A bare local
/// address (`http://host:11434`) is Ollama-style first, then OpenAI-style.
fn candidate_paths(base: &str) -> Vec<String> {
    let after_scheme = base.split_once("://").map_or(base, |(_, rest)| rest);
    let has_path = after_scheme.contains('/');
    if !has_path && base.starts_with("http://") {
        vec![format!("{base}/api/tags"), format!("{base}/v1/models")]
    } else {
        vec![format!("{base}/models")]
    }
}

/// The host errors and logs name: the drawable one (`identity::shown_host`). Empty for a URL whose
/// sign-in details cannot be told from its host, which is then refused (`UnclearAddress`) rather
/// than sent, or named by a part of its key ("Could not reach user:p.").
fn host_of(url: &str) -> String {
    crate::runs_on::identity::shown_host(url)
}

fn transport_error(t: &ureq::Transport, host: &str) -> ListError {
    use std::error::Error as _;
    let timed_out = t
        .source()
        .and_then(|s| s.downcast_ref::<std::io::Error>())
        .is_some_and(|e| matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock))
        || t.to_string().contains("timed out");
    match t.kind() {
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => ListError::BadUrl,
        _ if timed_out => ListError::TimedOut { host: host.to_string() },
        _ => ListError::Unreachable { host: host.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An error or log line names the host by the drawable address only, and a URL whose host
    /// cannot be told from its sign-in details is refused, never sent (review of #844, finding 2).
    #[test]
    fn errors_never_name_a_part_of_the_sign_in_details() {
        for url in crate::runs_on::tests::HOSTILE_URLS {
            crate::runs_on::tests::assert_clean(url, &host_of(url));
            for e in [ListError::Unreachable { host: host_of(url) }, ListError::TimedOut { host: host_of(url) }] {
                crate::runs_on::tests::assert_clean(url, &e.to_string());
            }
        }
        assert_eq!(host_of("https://user:pw@aig.example/v1?api-key=x#f"), "aig.example");
        assert_eq!(host_of("https://user:pw@[fd00::1]:8443/v1"), "[fd00::1]:8443");
        assert_eq!(list_models("https://user:p/w@aig.example/v1", Some("k"), "bearer"), Err(ListError::UnclearAddress));
        assert_eq!(list_models("https://user:p/w@[::1]:4000/v1", None, "none"), Err(ListError::UnclearAddress));
        match check_key("https://user:p/w@aig.example/v1", "k", "m") {
            KeyCheck::Unconfirmed(said) => crate::runs_on::tests::assert_clean("https://user:p/w@aig.example/v1", &said),
            other => panic!("{other:?}"),
        }
    }
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    const OPENAI: &str = r#"{"object":"list","data":[
        {"id":"gpt-4o-mini","object":"model","owned_by":"openai"},
        {"id":"gpt-4o","object":"model","owned_by":"openai"}]}"#;
    const ANTHROPIC: &str = r#"{"data":[
        {"type":"model","id":"claude-sonnet-5-5","display_name":"Claude Sonnet 5.5","created_at":"2026-09-28T00:00:00Z"},
        {"type":"model","id":"claude-haiku-4-5-20251001","display_name":"Claude Haiku 4.5"}],
        "has_more":false,"first_id":"claude-sonnet-5-5","last_id":"claude-haiku-4-5-20251001"}"#;
    const OLLAMA: &str = r#"{"models":[
        {"name":"nemotron-3-nano:4b","model":"nemotron-3-nano:4b","size":2800000000,"details":{"parameter_size":"4B"}},
        {"name":"qwen3.5:27b","model":"qwen3.5:27b","size":17000000000}]}"#;

    #[test]
    fn an_openai_list_is_ids() {
        let models = parse_models(OPENAI).unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["gpt-4o-mini", "gpt-4o"]);
        assert_eq!(models[0].name, "gpt-4o-mini");
    }

    #[test]
    fn an_anthropic_list_carries_display_names() {
        let models = parse_models(ANTHROPIC).unwrap();
        assert_eq!(models[0].id, "claude-sonnet-5-5");
        assert_eq!(models[0].name, "Claude Sonnet 5.5");
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn an_ollama_list_is_names_with_sizes() {
        let models = parse_models(OLLAMA).unwrap();
        assert_eq!(models[0].id, "nemotron-3-nano:4b");
        assert_eq!(models[0].size_bytes, 2_800_000_000);
        assert_eq!(models[1].id, "qwen3.5:27b");
    }

    #[test]
    fn gemini_prefixes_and_bare_arrays_and_duplicates_are_handled() {
        let gemini = r#"{"object":"list","data":[{"id":"models/gemini-3.8-flash"},{"id":"models/gemini-3.8-flash"}]}"#;
        let models = parse_models(gemini).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-3.8-flash");
        let bare = r#"[{"id":"meta-llama/Llama-3.3-70B-Instruct-Turbo","type":"chat"}]"#;
        assert_eq!(parse_models(bare).unwrap()[0].id, "meta-llama/Llama-3.3-70B-Instruct-Turbo");
    }

    #[test]
    fn a_body_that_is_not_a_list_says_so() {
        assert_eq!(parse_models("<html>hi</html>"), Err(ListError::NotAList));
        assert_eq!(parse_models(r#"{"error":"nope"}"#), Err(ListError::NotAList));
    }

    #[test]
    fn the_catalogue_default_is_picked_when_listed_otherwise_the_first() {
        let models = parse_models(OPENAI).unwrap();
        assert_eq!(pick_model(&models, &["", "gpt-4o"]), Some("gpt-4o"));
        assert_eq!(pick_model(&models, &["gone-model"]), Some("gpt-4o-mini"));
        assert_eq!(pick_model(&[], &["gpt-4o"]), None);
    }

    #[test]
    fn native_urls_list_at_their_compatible_endpoint() {
        assert_eq!(
            listing_base("https://generativelanguage.googleapis.com/"),
            "https://generativelanguage.googleapis.com/v1beta/openai"
        );
        assert_eq!(listing_base("https://api.anthropic.com"), "https://api.anthropic.com/v1");
        assert_eq!(listing_base("https://api.openai.com/v1/"), "https://api.openai.com/v1");
        assert_eq!(candidate_paths("https://api.openai.com/v1"), ["https://api.openai.com/v1/models"]);
        assert_eq!(
            candidate_paths("http://192.168.4.35:11434"),
            ["http://192.168.4.35:11434/api/tags", "http://192.168.4.35:11434/v1/models"]
        );
    }

    #[test]
    fn hosts_never_carry_userinfo() {
        assert_eq!(host_of("https://user:secret@api.example.com/v1"), "api.example.com");
        assert_eq!(host_of("http://localhost:11434"), "localhost:11434");
    }

    #[test]
    fn every_error_is_a_sentence() {
        let all = [
            ListError::KeyRefused,
            ListError::NoModelList,
            ListError::Unreachable { host: "api.example.com".into() },
            ListError::TimedOut { host: "api.example.com".into() },
            ListError::Status(500),
            ListError::NotAList,
            ListError::BadUrl,
        ];
        for e in all {
            let s = e.to_string();
            assert!(s.ends_with('.'), "{s}");
            assert!(s.chars().next().unwrap().is_uppercase(), "{s}");
        }
        assert!(ListError::KeyRefused.to_string().contains("key was refused"));
        assert!(ListError::NoModelList.to_string().contains("Check the URL"));
    }

    /// A one-shot HTTP server on a local port: answers each request with the
    /// next `(status, body)` and hands back what it was sent.
    fn serve(replies: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let _ = tx.send(read_request(&mut stream));
                let reason = match status { 200 => "OK", 401 => "Unauthorized", 404 => "Not Found", _ => "Error" };
                let reply = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        (base, rx)
    }

    /// One whole request: the head, then as many body bytes as its Content-Length says. ureq
    /// writes the head and a JSON body separately, so one read can end before the body arrives.
    fn read_request(stream: &mut std::net::TcpStream) -> String {
        let mut got = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let text = String::from_utf8_lossy(&got).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if body.len() >= length {
                    return text;
                }
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return String::from_utf8_lossy(&got).to_string(),
                Ok(n) => got.extend_from_slice(&buf[..n]),
            }
        }
    }

    #[test]
    fn a_key_is_confirmed_by_a_one_token_request_and_a_refusal_is_named() {
        let (base, sent) = serve(vec![(200, r#"{"choices":[{"message":{"content":"OK"}}]}"#)]);
        assert_eq!(check_key(&format!("{base}/v1"), "nvapi-good", "m/x"), KeyCheck::Works);
        let request = sent.recv().unwrap();
        assert!(request.starts_with("POST /v1/chat/completions "), "{request}");
        assert!(request.to_lowercase().contains("authorization: bearer nvapi-good"), "{request}");
        assert_eq!(key_check_body(&base, "m/x")["max_tokens"], 1, "one token, not a conversation");
        assert_eq!(key_check_body(&base, "m/x")["model"], "m/x");

        let (base, _) = serve(vec![(401, r#"{"error":"bad key nvapi-not-a-real-key"}"#)]);
        assert_eq!(check_key(&format!("{base}/v1"), "nvapi-not-a-real-key", "m/x"), KeyCheck::Refused);

        let (base, _) = serve(vec![(429, r#"{"error":"quota"}"#)]);
        assert_eq!(check_key(&format!("{base}/v1"), "k", "m"), KeyCheck::Works, "429 is said only to a key it knows");

        let (base, _) = serve(vec![(404, r#"{"error":"no such model"}"#)]);
        match check_key(&format!("{base}/v1"), "k", "m") {
            KeyCheck::Unconfirmed(why) => assert!(why.contains("HTTP 404"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    /// The request line and body a test request put on the wire.
    fn body_of(request: &str) -> serde_json::Value {
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap_or("")).unwrap()
    }

    #[test]
    fn a_model_that_refuses_temperature_is_asked_again_without_it_and_the_key_is_confirmed() {
        // VM 542, 16 Oct 2026: the key test passed and every turn after it was refused. Now the
        // test request carries what a turn carries, and is refused the same way, and recovers.
        let refusal = r#"{"error":{"code":"invalid_request_error","message":"`temperature` is deprecated for this model."}}"#;
        let (base, sent) = serve(vec![(400, refusal), (200, r#"{"choices":[{"message":{"content":"OK"}}]}"#)]);
        let base = format!("{base}/v1");
        assert_eq!(check_key(&base, "sk-ant-good", "claude-sonnet-5-5"), KeyCheck::Works);
        let first = body_of(&sent.recv().unwrap());
        let retry = body_of(&sent.recv().unwrap());
        assert!(first.get("temperature").is_some(), "{first}");
        assert!(retry.get("temperature").is_none(), "{retry}");
        // And the companion's turns in this process leave it out from the start.
        assert_eq!(yantrik_ml::refusal::remembered(&base, "claude-sonnet-5-5"), vec!["temperature"]);
        let turn = yantrik_ml::ApiLLM::new(base.as_str(), None, "claude-sonnet-5-5")
            .chat_completions_body(&[yantrik_ml::ChatMessage::user("hi")], &Default::default(), true);
        assert!(turn.get("temperature").is_none(), "{turn}");
    }

    #[test]
    fn a_refusal_that_is_not_about_the_key_says_what_the_provider_said() {
        let (base, _) = serve(vec![(400, r#"{"error":{"message":"model: no-such-model not found"}}"#)]);
        match check_key(&format!("{base}/v1"), "k", "no-such-model") {
            KeyCheck::Unconfirmed(why) => assert!(why.contains("HTTP 400: model: no-such-model not found"), "{why}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_test_request_carries_the_parameters_a_real_turn_carries() {
        // A companion turn, as `handle_message` builds its config from the companion's settings.
        let llm = yantrik_companion::config::LLMConfig::default();
        let turn_config = yantrik_ml::GenerationConfig {
            max_tokens: llm.max_tokens,
            temperature: llm.temperature,
            top_p: Some(0.9),
            max_context: Some(llm.max_context_tokens),
            ..Default::default()
        };
        let without_content = |mut b: serde_json::Value| {
            for k in ["messages", "max_tokens", "stream"] {
                b.as_object_mut().unwrap().remove(k);
            }
            b
        };
        for base in ["https://api.anthropic.com/v1", "https://api.openai.com/v1", "https://integrate.api.nvidia.com/v1"] {
            let turn = yantrik_ml::ApiLLM::new(base, None, "m")
                .chat_completions_body(&[yantrik_ml::ChatMessage::user("hi")], &turn_config, true);
            assert_eq!(without_content(key_check_body(base, "m")), without_content(turn), "{base}");
        }
    }

    #[test]
    fn a_list_anyone_can_read_is_told_apart_from_one_that_needs_the_key() {
        let (base, _) = serve(vec![(200, OPENAI)]);
        assert!(list_is_public(&format!("{base}/v1"), "bearer"));
        let (base, _) = serve(vec![(401, r#"{"error":"auth"}"#)]);
        assert!(!list_is_public(&format!("{base}/v1"), "bearer"));
    }

    #[test]
    fn a_listed_provider_is_asked_once_with_its_key_in_the_header() {
        let (base, sent) = serve(vec![(200, OPENAI)]);
        let models = list_models(&format!("{base}/v1"), Some("sk-test-123"), "bearer").unwrap();
        assert_eq!(models.len(), 2);
        let request = sent.recv().unwrap();
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert!(request.to_lowercase().contains("authorization: bearer sk-test-123"), "{request}");
    }

    #[test]
    fn anthropic_style_auth_sends_the_version_header() {
        let (base, sent) = serve(vec![(200, ANTHROPIC)]);
        let models = list_models(&format!("{base}/v1"), Some("sk-ant-1"), "x-api-key").unwrap();
        assert_eq!(models[0].name, "Claude Sonnet 5.5");
        let request = sent.recv().unwrap().to_lowercase();
        assert!(request.contains("x-api-key: sk-ant-1"), "{request}");
        assert!(request.contains("anthropic-version: 2023-06-01"), "{request}");
        assert!(!request.contains("authorization:"), "{request}");
    }

    #[test]
    fn a_refused_key_is_named_but_never_repeated() {
        let key = "sk-live-SECRET-9f8e7d";
        let (base, _sent) = serve(vec![(401, r#"{"error":{"message":"Incorrect API key sk-live-SECRET-9f8e7d"}}"#)]);
        let err = list_models(&format!("{base}/v1"), Some(key), "bearer").unwrap_err();
        assert_eq!(err, ListError::KeyRefused);
        let message = err.to_string();
        assert!(!message.contains(key), "{message}");
        assert!(!message.contains("SECRET"), "{message}");
    }

    #[test]
    fn a_missing_list_says_check_the_url() {
        let (base, _sent) = serve(vec![(404, "{}")]);
        assert_eq!(list_models(&format!("{base}/v1"), None, "none"), Err(ListError::NoModelList));
    }

    #[test]
    fn a_bare_local_address_tries_ollama_then_openai() {
        let (base, sent) = serve(vec![(404, "{}"), (200, OPENAI)]);
        let models = list_models(&base, None, "none").unwrap();
        assert_eq!(models[0].id, "gpt-4o-mini");
        assert!(sent.recv().unwrap().starts_with("GET /api/tags "));
        assert!(sent.recv().unwrap().starts_with("GET /v1/models "));
    }

    #[test]
    fn nobody_listening_is_could_not_reach() {
        // Bind and drop, so the port is closed.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let err = list_models(&format!("http://127.0.0.1:{port}/v1"), Some("sk-x"), "bearer").unwrap_err();
        assert_eq!(err, ListError::Unreachable { host: format!("127.0.0.1:{port}") });
        assert!(!err.to_string().contains("sk-x"));
    }

    #[test]
    fn an_unfilled_url_is_caught_before_any_request() {
        let url = "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1";
        assert_eq!(list_models(url, Some("t"), "bearer"), Err(ListError::BadUrl));
        assert_eq!(list_models("not a url", None, "none"), Err(ListError::BadUrl));
    }
}
